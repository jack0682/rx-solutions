//! Private-pipe bridge to the pinned, persistent BT planner. Never an execution authority.
use crate::{
    ValidatedSnapshot,
    frame::{Identity, Request},
};
use rx_domain::{canonical, types::Digest};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
};
const MAX_COMMAND: usize = 4_194_304;
const MAX_REPLY: usize = 1_000_000;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum State {
    Ready,
    Running,
    Success,
    Failure,
    Stale,
    Halted,
    Fault,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    schema: String,
    sequence: rx_domain::types::Counter,
    pub state: State,
    pub requests: Vec<Request>,
    pub fault: Option<String>,
}
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("BT process I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("BT boundary: {0}")]
    Invalid(String),
    #[error("BT process deadline exceeded")]
    Timeout,
    #[error("BT frame expired before transmission")]
    Stale,
}
fn invalid(why: impl Into<String>) -> Error {
    Error::Invalid(why.into())
}
pub struct Executable {
    pub path: PathBuf,
    pub sha256: Digest,
}
pub struct EngineProcess {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    identity: Identity,
    sequence: u64,
    failed: bool,
    halted: bool,
}
impl EngineProcess {
    /// Deployment supplies an immutable release binary. Site/process data never supplies argv.
    pub async fn spawn(
        executable: Executable,
        snapshot: &ValidatedSnapshot,
    ) -> Result<Self, Error> {
        verify_executable(&executable.path, executable.sha256).await?;
        let mut command = Command::new(&executable.path);
        command.env_clear();
        Self::spawn_command(command, snapshot).await
    }
    #[cfg(feature = "test-harness")]
    pub async fn spawn_simulation(
        command: Command,
        snapshot: &ValidatedSnapshot,
    ) -> Result<Self, Error> {
        if snapshot.data().checked_at.clock_id != "test-clock" {
            return Err(invalid("simulation clock required"));
        }
        Self::spawn_command(command, snapshot).await
    }
    async fn spawn_command(
        mut command: Command,
        snapshot: &ValidatedSnapshot,
    ) -> Result<Self, Error> {
        let bytes = canonical::bytes(snapshot.process()).map_err(|e| invalid(e.to_string()))?;
        if Digest::from_bytes(Sha256::digest(&bytes).into()) != snapshot.data().resolved.sha256
            || bytes.len() as u64 != snapshot.data().resolved.size_bytes.0
        {
            return Err(invalid("resolved input bytes differ"));
        }
        let xml = rx_process::bt_xml::generate(snapshot.process()).map_err(invalid)?;
        if bytes.len() > MAX_REPLY || xml.len() > MAX_REPLY {
            return Err(invalid("bootstrap artifact limit"));
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn()?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| invalid("stdin pipe missing"))?;
        let output = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| invalid("stdout pipe missing"))?,
        );
        let mut process = Self {
            child,
            input,
            output,
            identity: snapshot.context_identity(),
            sequence: 0,
            failed: false,
            halted: false,
        };
        let value = serde_json::json!({"schema":"rx.bt-command.v1","sequence":"1","command":"INITIALIZE","identity":process.identity,"resolved":snapshot.process(),"xml":xml});
        let reply = process.exchange(value).await?;
        if reply.state != State::Ready || !reply.requests.is_empty() || reply.fault.is_some() {
            return Err(invalid("initialization emitted work or failed"));
        }
        Ok(process)
    }
    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }
    pub fn identity(&self) -> &Identity {
        &self.identity
    }
    pub async fn step(&mut self, snapshot: &ValidatedSnapshot) -> Result<Reply, Error> {
        if self.halted {
            return Err(invalid("planner is halted"));
        }
        if snapshot.context_identity() != self.identity {
            self.failed = true;
            let _ = self.child.start_kill();
            return Err(invalid("context changed; explicit replacement required"));
        }
        let frame = snapshot
            .frame(&self.identity)
            .map_err(|error| match error {
                crate::Error::Expired => Error::Stale,
                other => invalid(other.to_string()),
            })?;
        let reply = self.exchange(serde_json::json!({"schema":"rx.bt-command.v1","sequence":self.next_sequence()?.to_string(),"command":"STEP","frame":frame})).await?;
        if !matches!(
            reply.state,
            State::Running | State::Success | State::Failure | State::Stale | State::Fault
        ) {
            self.failed = true;
            let _ = self.child.start_kill();
            return Err(invalid("unexpected step response"));
        }
        Ok(reply)
    }
    pub async fn halt(&mut self) -> Result<Reply, Error> {
        self.halted = true;
        let reply = self.exchange(serde_json::json!({"schema":"rx.bt-command.v1","sequence":self.next_sequence()?.to_string(),"command":"HALT"})).await?;
        if !matches!(reply.state, State::Halted | State::Fault) {
            self.failed = true;
            let _ = self.child.start_kill();
            return Err(invalid("unexpected halt response"));
        }
        Ok(reply)
    }
    /// Closing a planner does not confirm native stop or resource handover.
    pub async fn close(mut self) -> Result<Reply, Error> {
        self.halted = true;
        let reply = self.exchange(serde_json::json!({"schema":"rx.bt-command.v1","sequence":self.next_sequence()?.to_string(),"command":"CLOSE"})).await?;
        let status = tokio::time::timeout(Duration::from_secs(2), self.child.wait())
            .await
            .map_err(|_| Error::Timeout)??;
        if !status.success() || reply.state != State::Halted {
            return Err(invalid("planner close failed"));
        }
        Ok(reply)
    }
    /// Discard CLOSE's run-pause suggestion only after P confirmed this exact visit complete.
    pub(crate) async fn retire(self, proof: crate::client::CompletedVisit) -> Result<(), Error> {
        if proof.identity != self.identity || proof.part.as_str().is_empty() {
            return Err(invalid("completed visit does not match planner"));
        }
        self.close().await?;
        Ok(())
    }
    fn next_sequence(&self) -> Result<u64, Error> {
        self.sequence
            .checked_add(1)
            .ok_or_else(|| invalid("IPC sequence exhausted"))
    }
    async fn exchange(&mut self, value: serde_json::Value) -> Result<Reply, Error> {
        if self.failed {
            return Err(invalid("failed planner cannot be reused"));
        }
        let sequence = self.next_sequence()?;
        let mut bytes = serde_json::to_vec(&value).map_err(|e| invalid(e.to_string()))?;
        if bytes.len() > MAX_COMMAND {
            return Err(invalid("IPC command limit"));
        }
        bytes.push(b'\n');
        let result = tokio::time::timeout(Duration::from_secs(2), async {
            self.input.write_all(&bytes).await?;
            self.input.flush().await?;
            let response = read_reply(&mut self.output).await?;
            let reply: Reply =
                canonical::decode_json(&response).map_err(|e| invalid(e.to_string()))?;
            if reply.schema != "rx.bt-reply.v1"
                || reply.sequence.0 != sequence
                || reply.requests.len() > 32
                || reply.requests.iter().any(|r| r.identity != self.identity)
                || (reply.state == State::Fault) != reply.fault.is_some()
                || reply.fault.as_ref().is_some_and(|s| s.len() > 4096)
                || (matches!(reply.state, State::Ready | State::Stale)
                    && !reply.requests.is_empty())
                || (matches!(reply.state, State::Halted | State::Fault)
                    && reply
                        .requests
                        .iter()
                        .any(|r| r.kind != crate::frame::RequestKind::PauseExecutor))
            {
                return Err(invalid("IPC response correlation/state differs"));
            }
            Ok(reply)
        })
        .await
        .map_err(|_| Error::Timeout)
        .and_then(|v| v);
        match &result {
            Ok(reply) => {
                self.sequence = sequence;
                if reply.state == State::Fault {
                    self.failed = true;
                }
            }
            Err(_) => {
                self.failed = true;
                let _ = self.child.start_kill();
            }
        }
        result
    }
}
async fn read_reply(reader: &mut BufReader<ChildStdout>) -> Result<Vec<u8>, Error> {
    let mut result = Vec::new();
    loop {
        let bytes = reader.fill_buf().await?;
        if bytes.is_empty() {
            return Err(invalid("planner pipe closed or reply truncated"));
        }
        let end = bytes.iter().position(|b| *b == b'\n');
        let length = end.unwrap_or(bytes.len());
        if result.len() + length > MAX_REPLY {
            return Err(invalid("IPC response limit"));
        }
        result.extend_from_slice(&bytes[..length]);
        reader.consume(length + usize::from(end.is_some()));
        if end.is_some() {
            return Ok(result);
        }
    }
}
async fn verify_executable(path: &Path, expected: Digest) -> Result<(), Error> {
    if !path.is_absolute() {
        return Err(invalid("absolute release binary path required"));
    }
    let metadata = tokio::fs::symlink_metadata(path).await?;
    if !metadata.is_file() || metadata.len() > 134_217_728 {
        return Err(invalid("regular release binary and size bound required"));
    }
    let mut file = tokio::fs::File::open(path).await?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 16384];
    let mut total = 0_u64;
    loop {
        let n = file.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > 134_217_728 {
            return Err(invalid("release binary changed or exceeds size bound"));
        }
        hash.update(&buffer[..n]);
    }
    if Digest::from_bytes(hash.finalize().into()) != expected {
        return Err(invalid("release binary digest differs"));
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crate::clock::{Clock, LinuxBoottime};
    use std::sync::Arc;
    fn view(clock: Arc<LinuxBoottime>) -> ValidatedSnapshot {
        let mut raw: rx_process_contract::execution::ExecutionSnapshot = canonical::decode_json(
            include_bytes!("../tests/fixtures/restored-v1/snapshot.json"),
        )
        .unwrap();
        let process: rx_process_contract::ResolvedProcess = canonical::decode_json(include_bytes!(
            "../tests/fixtures/restored-v1/resolved.json"
        ))
        .unwrap();
        let now = clock.now().unwrap();
        raw.checked_at = now.clone();
        raw.valid_until = rx_domain::types::TimePoint {
            clock_id: now.clock_id,
            ticks_ns: rx_domain::types::Counter(now.ticks_ns.0 + 100_000_000),
        };
        assert!(!raw.request_admission_allowed);
        let frontier = rx_process_contract::execution_validation::validate(&raw, &process).unwrap();
        ValidatedSnapshot {
            raw,
            process: Arc::new(process),
            frontier,
            deadline: std::time::Instant::now() + Duration::from_millis(100),
            clock,
        }
    }
    #[tokio::test]
    #[ignore = "Linux validation image supplies RX_BT_ENGINE_BINARY"]
    async fn verified_product_binary_uses_live_linux_clock_and_preserves_one_process() {
        let path = PathBuf::from(std::env::var("RX_BT_ENGINE_BINARY").expect("engine binary path"));
        let clock = Arc::new(LinuxBoottime::new().unwrap());
        assert!(
            EngineProcess::spawn(
                Executable {
                    path: path.clone(),
                    sha256: Digest::from_bytes([0; 32])
                },
                &view(clock.clone())
            )
            .await
            .is_err()
        );
        let sha256 = Digest::from_bytes(Sha256::digest(std::fs::read(&path).unwrap()).into());
        let mut process = EngineProcess::spawn(Executable { path, sha256 }, &view(clock.clone()))
            .await
            .unwrap();
        let pid = process.pid().unwrap();
        for _ in 0..3 {
            let snapshot = view(clock.clone());
            assert!(
                snapshot
                    .data()
                    .progress
                    .operations
                    .values()
                    .all(|op| op.operation.outcome() == rx_domain::operation::Outcome::NotExecuted)
            );
            let reply = process.step(&snapshot).await.unwrap();
            assert_eq!(reply.state, State::Failure);
            assert!(reply.requests.is_empty());
            assert_eq!(process.pid(), Some(pid));
        }
        let reply = process.close().await.unwrap();
        assert_eq!(reply.state, State::Halted);
        assert_eq!(reply.requests.len(), 1);
        assert_eq!(
            reply.requests[0].kind,
            crate::frame::RequestKind::PauseExecutor
        );
    }
}
