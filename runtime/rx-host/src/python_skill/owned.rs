//! Owns one already-authorized runner and its private acknowledgement/result channel.
//! Collection never starts another command or restores custody from a saved PID.
use super::*;
use std::process::Child;

pub(super) struct OwnedCall {
    channel: UnixStream,
    child: Child,
    group: rustix::process::Pid,
    deadline: Instant,
    reaped: bool,
}
impl OwnedCall {
    pub fn start(
        release: &ReleasePython,
        state: &Path,
        action: &str,
        bytes: &[u8],
        timeout: Duration,
    ) -> Result<Self> {
        pinned(&release.executable, release.executable_digest)?;
        pinned(&release.runner, release.runner_digest)?;
        pinned(
            &release.runner.with_file_name("python_environment.py"),
            release.verifier_digest,
        )?;
        let (channel, peer) = UnixStream::pair().map_err(unknown)?;
        let stdout = peer.try_clone().map_err(unknown)?;
        channel.set_write_timeout(Some(timeout)).map_err(unknown)?;
        let child = Command::new(&release.executable)
            .args(["-I", "-S", "-B"])
            .arg(&release.runner)
            .arg(action)
            .arg(state)
            .env_clear()
            .stdin(Stdio::from(OwnedFd::from(peer)))
            .stdout(Stdio::from(OwnedFd::from(stdout)))
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(unknown)?;
        let group = rustix::process::Pid::from_raw(child.id() as i32).ok_or(HostError::Guard)?;
        let mut call = Self {
            channel,
            child,
            group,
            deadline: Instant::now() + timeout,
            reaped: false,
        };
        call.channel.write_all(bytes).map_err(unknown)?;
        call.channel.shutdown(Shutdown::Write).map_err(unknown)?;
        Ok(call)
    }
    fn read(&mut self, buffer: &mut [u8], deadline: Instant) -> Result<usize> {
        self.channel
            .set_read_timeout(Some(
                deadline
                    .checked_duration_since(Instant::now())
                    .ok_or_else(|| unknown("Python execution deadline"))?,
            ))
            .map_err(unknown)?;
        self.channel.read(buffer).map_err(unknown)
    }
    pub fn entry(&mut self, timeout: Duration) -> Result<Value> {
        let deadline = (Instant::now() + timeout).min(self.deadline);
        let mut output = Vec::new();
        // Do not prefetch completion bytes into a temporary buffered reader.
        loop {
            let mut byte = [0];
            if self.read(&mut byte, deadline)? == 0 {
                return Err(unknown("Python entry acknowledgement missing"));
            }
            if byte[0] == b'\n' {
                break;
            }
            output.push(byte[0]);
            if output.len() > 65536 {
                return Err(unknown("Python entry exceeds bound"));
            }
        }
        canonical::decode_json(&output).map_err(unknown)
    }
    pub fn finish(mut self) -> Result<Value> {
        let mut output = Vec::new();
        let mut buffer = [0; 8192];
        loop {
            let count = self.read(&mut buffer, self.deadline)?;
            if count == 0 {
                break;
            }
            output.extend_from_slice(&buffer[..count]);
            if output.len() > 131072 {
                return Err(unknown("Python reply exceeds bound"));
            }
        }
        loop {
            if let Some(status) = self.child.try_wait().map_err(unknown)? {
                self.reaped = true;
                if !status.success() {
                    return Err(unknown("Python helper exited without confirmed receipt"));
                }
                break;
            }
            if Instant::now() >= self.deadline {
                return Err(unknown("Python process exit deadline"));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        if rustix::process::test_kill_process_group(self.group).is_ok() {
            return Err(unknown("Python process group custody remains unresolved"));
        }
        canonical::decode_json(&output).map_err(unknown)
    }
}
impl Drop for OwnedCall {
    fn drop(&mut self) {
        if !self.reaped {
            // Retain the unreaped leader while signaling; never signal a recycled PID.
            let _ = rustix::process::kill_process_group(self.group, rustix::process::Signal::KILL);
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
