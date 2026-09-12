//! Synchronous bounded private pipes for the release-owned bridge, usable inside a blocking Host gate.
use super::protocol::*;
use crate::{Clock, HostError, Result};
use rx_domain::{canonical, types::*};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    time::{Duration, Instant},
};

/// Supplied by release composition, never by an operation or site JSON.
pub struct Executable {
    pub path: PathBuf,
    pub sha256: Digest,
    pub environment: BTreeMap<String, String>,
}
pub struct Process<C: Clock> {
    child: Option<Child>,
    input: Option<ChildStdin>,
    output: Option<ChildStdout>,
    buffer: Vec<u8>,
    directory: Option<tempfile::TempDir>,
    instance: Id,
    clock: C,
    sequence: Counter,
    faulted: bool,
    closing: bool,
}
fn invalid(e: impl std::fmt::Display) -> HostError {
    HostError::NativeUnknown(e.to_string())
}
#[cfg(unix)]
fn nonblocking(fd: &impl std::os::fd::AsFd) -> Result<()> {
    let flags = rustix::fs::fcntl_getfl(fd).map_err(invalid)?;
    rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK).map_err(invalid)
}
#[cfg(unix)]
fn wait(
    fd: &impl std::os::fd::AsFd,
    flags: rustix::event::PollFlags,
    deadline: Instant,
) -> Result<()> {
    loop {
        let left = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| invalid("bridge I/O deadline"))?;
        let timeout = rustix::event::Timespec {
            tv_sec: left.as_secs() as i64,
            tv_nsec: i64::from(left.subsec_nanos()),
        };
        let mut fds = [rustix::event::PollFd::new(fd, flags)];
        match rustix::event::poll(&mut fds, Some(&timeout)) {
            Ok(0) => return Err(invalid("bridge I/O deadline")),
            Ok(_) => return Ok(()),
            Err(rustix::io::Errno::INTR) => continue,
            Err(e) => return Err(invalid(e)),
        }
    }
}
impl<C: Clock> Process<C> {
    #[cfg(unix)]
    pub fn spawn(
        executable: Executable,
        config: &Configuration,
        runtime: &Path,
        clock: C,
    ) -> Result<Self> {
        config.joints()?;
        if !runtime.is_absolute()
            || !std::fs::symlink_metadata(runtime)
                .map_err(invalid)?
                .is_dir()
            || runtime.is_symlink()
        {
            return Err(HostError::Invalid(
                "owned runtime directory required".into(),
            ));
        }
        let leaf = executable
            .path
            .file_name()
            .and_then(|p| p.to_str())
            .ok_or_else(|| invalid("executable name"))?;
        let content = rx_package::directory::read_relative_file(
            executable
                .path
                .parent()
                .ok_or_else(|| invalid("executable parent"))?,
            &rx_package::PackagePath::new(leaf).map_err(invalid)?,
            16 * 1024 * 1024,
        )
        .map_err(invalid)?;
        if !executable.path.is_absolute()
            || rx_package::content_digest(&content) != executable.sha256
        {
            return Err(HostError::Invalid("bridge executable pin differs".into()));
        }
        let allowed = [
            "LD_LIBRARY_PATH",
            "AMENT_PREFIX_PATH",
            "COLCON_PREFIX_PATH",
            "PYTHONPATH",
            "RMW_IMPLEMENTATION",
            "ROS_AUTOMATIC_DISCOVERY_RANGE",
            "ROS_STATIC_PEERS",
            "FASTRTPS_DEFAULT_PROFILES_FILE",
            "CYCLONEDDS_URI",
            "ZENOH_SESSION_CONFIG_URI",
            "ROS_LOG_DIR",
        ];
        if executable
            .environment
            .iter()
            .any(|(k, v)| !allowed.contains(&k.as_str()) || v.len() > 8192)
        {
            return Err(HostError::Invalid("bridge release environment".into()));
        }
        let directory = tempfile::Builder::new()
            .prefix("rx-jtc-")
            .tempdir_in(runtime)
            .map_err(invalid)?;
        let binary = directory.path().join("bridge");
        std::fs::write(&binary, content).map_err(invalid)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o500))
            .map_err(invalid)?;
        let cfg = directory.path().join("configuration.json");
        std::fs::write(&cfg, canonical::bytes(config).map_err(invalid)?).map_err(invalid)?;
        let mut child = Command::new(&binary)
            .arg(&cfg)
            .env_clear()
            .envs(&executable.environment)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(invalid)?;
        let input = child.stdin.take().ok_or_else(|| invalid("bridge stdin"))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| invalid("bridge stdout"))?;
        let mut process = Self {
            child: Some(child),
            input: Some(input),
            output: Some(output),
            buffer: vec![],
            directory: Some(directory),
            instance: crate::journal::id(),
            clock,
            sequence: Counter(0),
            faulted: false,
            closing: false,
        };
        nonblocking(process.input.as_ref().expect("owned input"))?;
        nonblocking(process.output.as_ref().expect("owned output"))?;
        let line = process.read_line(Instant::now() + Duration::from_secs(3))?;
        let hello: Reply = canonical::decode_json(&line).map_err(invalid)?;
        hello.ready(config, &process.clock)?;
        process.instance = hello.bridge_instance;
        Ok(process)
    }
    #[cfg(not(unix))]
    pub fn spawn(
        _executable: Executable,
        _config: &Configuration,
        _runtime: &Path,
        _clock: C,
    ) -> Result<Self> {
        Err(HostError::Invalid(
            "ROS bridge pipes currently require Unix".into(),
        ))
    }
    #[cfg(unix)]
    fn read_line(&mut self, deadline: Instant) -> Result<Vec<u8>> {
        loop {
            if let Some(at) = self.buffer.iter().position(|b| *b == b'\n') {
                let line = self.buffer.drain(..=at).collect::<Vec<_>>();
                return Ok(line[..line.len() - 1].to_vec());
            }
            if self.buffer.len() >= 1_048_576 {
                return Err(invalid("bridge reply size limit"));
            }
            let output = self
                .output
                .as_mut()
                .ok_or_else(|| invalid("bridge output closed"))?;
            wait(output, rustix::event::PollFlags::IN, deadline)?;
            let mut data = [0; 4096];
            match output.read(&mut data) {
                Ok(0) => return Err(invalid("bridge output ended before reply")),
                Ok(n) => {
                    if self.buffer.len() + n > 1_048_576 {
                        return Err(invalid("bridge reply size limit"));
                    }
                    self.buffer.extend_from_slice(&data[..n]);
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    continue;
                }
                Err(e) => return Err(invalid(e)),
            }
        }
    }
    #[cfg(unix)]
    fn write_line(&mut self, mut bytes: &[u8], deadline: Instant) -> Result<()> {
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| invalid("bridge input closed"))?;
        while !bytes.is_empty() {
            wait(input, rustix::event::PollFlags::OUT, deadline)?;
            match input.write(bytes) {
                Ok(0) => return Err(invalid("bridge input closed")),
                Ok(n) => bytes = &bytes[n..],
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    continue;
                }
                Err(e) => return Err(invalid(e)),
            }
        }
        Ok(())
    }
    pub fn is_faulted(&self) -> bool {
        self.faulted
    }
}
impl<C: Clock> Transport for Process<C> {
    fn instance(&self) -> &Id {
        &self.instance
    }
    #[cfg(unix)]
    fn exchange(
        &mut self,
        command: &str,
        body: serde_json::Value,
        until: &TimePoint,
    ) -> Result<Reply> {
        if self.faulted || self.closing {
            return Err(invalid("bridge is faulted/closing"));
        }
        let now = self.clock.now();
        if !self.clock.healthy()
            || now.clock_id != until.clock_id
            || until.ticks_ns <= now.ticks_ns
            || until.ticks_ns.0 - now.ticks_ns.0 > 1_000_000_000
        {
            return Err(HostError::Stale);
        }
        self.sequence = self.sequence.increment().map_err(invalid)?;
        let request = serde_json::json!({"schema":"rx.ros-jtc-request.v1","bridge_instance":self.instance,"sequence":self.sequence,"expires_at_ns":until.ticks_ns,"command":command,"body":body});
        let mut bytes = canonical::bytes(&request).map_err(invalid)?;
        if bytes.len() >= 1_048_576 {
            return Err(invalid("bridge command size limit"));
        }
        bytes.push(b'\n');
        let deadline = Instant::now()
            + Duration::from_nanos(until.ticks_ns.0 - now.ticks_ns.0)
            + Duration::from_millis(100);
        let outcome = (|| {
            self.write_line(&bytes, deadline)?;
            let line = self.read_line(deadline)?;
            let reply: Reply = canonical::decode_json(&line).map_err(invalid)?;
            reply.validate(Some(&self.instance), Some(self.sequence), &self.clock)?;
            Ok(reply)
        })();
        if outcome.is_err() {
            self.faulted = true;
        }
        outcome
    }
    #[cfg(not(unix))]
    fn exchange(&mut self, _: &str, _: serde_json::Value, _: &TimePoint) -> Result<Reply> {
        Err(HostError::Guard)
    }
    fn try_close(&mut self) -> Result<bool> {
        self.closing = true;
        self.input.take();
        let Some(child) = self.child.as_mut() else {
            return Ok(true);
        };
        if child.try_wait().map_err(invalid)?.is_some() {
            self.child.take();
            self.output.take();
            self.directory.take();
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
impl<C: Clock> Drop for Process<C> {
    fn drop(&mut self) {
        self.input.take();
        self.output.take();
        if let Some(mut child) = self.child.take() {
            let directory = self.directory.take();
            std::thread::spawn(move || {
                let _ = child.wait();
                drop(directory);
            });
        }
    }
}
