//! Own the child, process group and private channel; never reconstruct ownership from a PID.
use super::profile::{Program, invalid};
use crate::{HostError, ProtectionIncident, Result, native::LocalProtection};
use rx_domain::canonical;
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::Shutdown,
    os::{
        fd::OwnedFd,
        unix::{net::UnixStream, process::CommandExt},
    },
    path::Path,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};

struct ChildState {
    child: Child,
    reaped: bool,
}
pub struct Control {
    state: Mutex<ChildState>,
    group: rustix::process::Pid,
}
impl Control {
    fn kill(&self) {
        if let Ok(mut state) = self.state.lock()
            && !state.reaped
        {
            let _ = rustix::process::kill_process_group(self.group, rustix::process::Signal::KILL);
            let _ = state.child.kill();
        }
    }
    fn finish(&self, deadline: Instant) -> Result<bool> {
        loop {
            {
                let mut state = self.state.lock().map_err(|_| HostError::Guard)?;
                if state.reaped {
                    return Err(HostError::Guard);
                }
                if let Some(status) = state.child.try_wait().map_err(invalid)? {
                    state.reaped = true;
                    return Ok(status.success());
                }
            }
            if Instant::now() >= deadline {
                return Err(HostError::Stale);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn reap(&self) {
        self.kill();
        if let Ok(mut state) = self.state.lock()
            && !state.reaped
            && state.child.wait().is_ok()
        {
            state.reaped = true;
        }
    }
    fn quiet(&self) -> bool {
        self.state.lock().is_ok_and(|s| s.reaped)
            && matches!(
                rustix::process::test_kill_process_group(self.group),
                Err(rustix::io::Errno::SRCH)
            )
    }
}
impl Drop for Control {
    fn drop(&mut self) {
        self.reap();
    }
}

#[derive(Default)]
pub struct Protection {
    children: Mutex<Vec<Weak<Control>>>,
}
impl Protection {
    fn retain(&self, child: &Arc<Control>) -> Result<()> {
        let mut children = self.children.lock().map_err(|_| HostError::Guard)?;
        children.retain(|c| c.strong_count() > 0);
        children.push(Arc::downgrade(child));
        Ok(())
    }
}
impl LocalProtection for Protection {
    fn react(&self, _: ProtectionIncident) {
        // Native protection has no access to the Host command gate or its journal transaction.
        if let Ok(children) = self.children.lock() {
            for child in children.iter().filter_map(Weak::upgrade) {
                child.kill();
            }
        }
    }
}
pub struct Finished {
    pub value: std::result::Result<Value, String>,
    pub custody_clear: bool,
}
pub struct Call {
    channel: UnixStream,
    child: Arc<Control>,
    deadline: Instant,
}
impl Call {
    pub fn start(
        program: &Program,
        state: &Path,
        mode: &str,
        bytes: &[u8],
        timeout: Duration,
        protection: &Protection,
    ) -> Result<Self> {
        program.validate(true)?;
        let (channel, peer) = UnixStream::pair().map_err(invalid)?;
        channel.set_write_timeout(Some(timeout)).map_err(invalid)?;
        let stdout = peer.try_clone().map_err(invalid)?;
        let child = Command::new(&program.executable.path)
            .args(&program.arguments)
            .arg(mode)
            .arg(state)
            .env_clear()
            .stdin(Stdio::from(OwnedFd::from(peer)))
            .stdout(Stdio::from(OwnedFd::from(stdout)))
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(invalid)?;
        let group = rustix::process::Pid::from_raw(child.id() as i32).ok_or(HostError::Guard)?;
        let child = Arc::new(Control {
            state: Mutex::new(ChildState {
                child,
                reaped: false,
            }),
            group,
        });
        protection.retain(&child)?;
        let mut result = Self {
            channel,
            child,
            deadline: Instant::now() + timeout,
        };
        result.channel.write_all(bytes).map_err(invalid)?;
        result.channel.shutdown(Shutdown::Write).map_err(invalid)?;
        Ok(result)
    }
    fn read(&mut self, bytes: &mut [u8], deadline: Instant) -> Result<usize> {
        self.channel
            .set_read_timeout(Some(
                deadline
                    .checked_duration_since(Instant::now())
                    .ok_or(HostError::Stale)?,
            ))
            .map_err(invalid)?;
        self.channel.read(bytes).map_err(invalid)
    }
    pub fn entry(&mut self, timeout: Duration) -> Result<Value> {
        let deadline = (Instant::now() + timeout).min(self.deadline);
        let mut bytes = Vec::new();
        loop {
            let mut byte = [0];
            if self.read(&mut byte, deadline)? == 0 {
                return Err(HostError::NativeUnknown("external entry missing".into()));
            }
            if byte[0] == b'\n' {
                break;
            }
            bytes.push(byte[0]);
            if bytes.len() > 65536 {
                return Err(invalid("external entry exceeds bound"));
            }
        }
        canonical::decode_json(&bytes).map_err(invalid)
    }
    fn result(&mut self) -> Result<Value> {
        let mut bytes = Vec::new();
        let mut buffer = [0; 8192];
        loop {
            let n = self.read(&mut buffer, self.deadline)?;
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..n]);
            if bytes.len() > 1_048_576 {
                return Err(invalid("external reply exceeds bound"));
            }
        }
        if !self.child.finish(self.deadline)? {
            return Err(HostError::NativeUnknown(
                "external process exited without reply".into(),
            ));
        }
        canonical::decode_json(&bytes).map_err(invalid)
    }
    pub fn finish(mut self) -> Finished {
        let value = self.result().map_err(|e| e.to_string());
        self.child.reap();
        Finished {
            value,
            custody_clear: self.child.quiet(),
        }
    }
}
impl Drop for Call {
    fn drop(&mut self) {
        self.child.reap();
    }
}
