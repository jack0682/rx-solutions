//! Python program execution behind the existing Host gate. Registration wiring is separate.
//! Current adapter qualification is simulation-only; support facts remain independently supplied.
use crate::{
    Clock, Environment, Guard, HostError, LocalHandover, NativeCapture, Result, native::*,
};
use rx_domain::{
    canonical,
    host_snapshot::SourceObservation,
    intent::{Body, Intent},
    types::*,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    net::Shutdown,
    os::{
        fd::OwnedFd,
        unix::{net::UnixStream, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

pub struct ReleasePython {
    pub executable: PathBuf,
    pub executable_digest: Digest,
    pub runner: PathBuf,
    pub runner_digest: Digest,
    pub verifier_digest: Digest,
}
pub struct Program {
    pub environment: PathBuf,
    pub environment_digest: Digest,
    pub input: Value,
    pub intent: Intent,
}
pub struct PythonSkill<N, C> {
    release: ReleasePython,
    programs: BTreeMap<Digest, Program>,
    state: PathBuf,
    support: N,
    clock: C,
    uncertain: BTreeSet<Id>,
}
fn unknown(e: impl std::fmt::Display) -> HostError {
    HostError::NativeUnknown(e.to_string())
}
fn pinned(path: &Path, expected: Digest) -> Result<Vec<u8>> {
    if !path.is_absolute() {
        return Err(HostError::Guard);
    }
    let raw = rx_package::directory::read_relative_file(
        path.parent().ok_or(HostError::Guard)?,
        &rx_package::PackagePath::new(
            path.file_name()
                .and_then(|s| s.to_str())
                .ok_or(HostError::Guard)?,
        )
        .map_err(unknown)?,
        16 * 1024 * 1024,
    )
    .map_err(unknown)?;
    if rx_package::content_digest(&raw) != expected {
        return Err(HostError::Guard);
    }
    Ok(raw)
}
fn journal_bytes(root: &Path, relative: &str) -> Result<Vec<u8>> {
    rx_package::directory::read_relative_file(
        root,
        &rx_package::PackagePath::new(relative).map_err(unknown)?,
        131072,
    )
    .map_err(unknown)
}
pub fn validate_program(program: &Program) -> Result<()> {
    program.intent.normalized().map_err(unknown)?;
    if program.intent.execution_timeout_ms.0 > 60000 {
        return Err(HostError::Guard);
    }
    let Body::Program(goal) = &program.intent.body else {
        return Err(HostError::Guard);
    };
    let manifest = pinned(
        &program.environment.join("environment.json"),
        goal.program.sha256,
    )?;
    let value: Value = canonical::decode_json(&manifest).map_err(unknown)?;
    if value["environment_digest"] != program.environment_digest.to_string()
        || goal.program.size_bytes.0 != manifest.len() as u64
    {
        return Err(HostError::Guard);
    }
    let input = canonical::bytes(&program.input).map_err(unknown)?;
    if rx_package::content_digest(&input) != goal.parameter_set.sha256
        || input.len() as u64 != goal.parameter_set.size_bytes.0
    {
        return Err(HostError::Guard);
    }
    Ok(())
}
impl<N: NativeAdapter, C: Clock> PythonSkill<N, C> {
    pub fn open(
        release: ReleasePython,
        program: Program,
        state: PathBuf,
        support: N,
        clock: C,
    ) -> Result<Self> {
        Self::open_library(release, vec![program], state, support, clock)
    }
    /// One native owner and one uncertainty set across all explicitly pinned programs.
    pub fn open_library(
        release: ReleasePython,
        programs: Vec<Program>,
        state: PathBuf,
        support: N,
        clock: C,
    ) -> Result<Self> {
        if support.environment() != Environment::Simulation
            || !state.is_absolute()
            || state.is_symlink()
            || !state.is_dir()
            || programs.is_empty()
            || programs.len() > 16
        {
            return Err(HostError::Invalid(
                "Python SDK adapter requires an owned simulation journal and bounded execution"
                    .into(),
            ));
        }
        let mut selected = BTreeMap::new();
        for program in programs {
            validate_program(&program)?;
            let digest = program.intent.digest().map_err(unknown)?;
            if selected.insert(digest, program).is_some() {
                return Err(HostError::Invalid("duplicate Python program Intent".into()));
            }
        }
        pinned(&release.executable, release.executable_digest)?;
        pinned(&release.runner, release.runner_digest)?;
        pinned(
            &release.runner.with_file_name("python_environment.py"),
            release.verifier_digest,
        )?;
        let mut uncertain = BTreeSet::new();
        for item in fs::read_dir(&state).map_err(unknown)? {
            let item = item.map_err(unknown)?;
            if item.file_name() == "captures" {
                continue;
            }
            let id = Id::new(item.file_name().to_string_lossy()).map_err(unknown)?;
            uncertain.insert(id); // Reopening does not establish process/native custody.
        }
        if state.join("captures").is_symlink() {
            return Err(HostError::Guard);
        }
        fs::create_dir_all(state.join("captures")).map_err(unknown)?;
        Ok(Self {
            release,
            programs: selected,
            state,
            support,
            clock,
            uncertain,
        })
    }
    fn call(&self, action: &str, request: &Value, timeout: Duration) -> Result<Value> {
        pinned(&self.release.executable, self.release.executable_digest)?;
        pinned(&self.release.runner, self.release.runner_digest)?;
        pinned(
            &self.release.runner.with_file_name("python_environment.py"),
            self.release.verifier_digest,
        )?;
        let (mut channel, peer) = UnixStream::pair().map_err(unknown)?;
        let stdout = peer.try_clone().map_err(unknown)?;
        let deadline = Instant::now() + timeout;
        channel.set_write_timeout(Some(timeout)).map_err(unknown)?;
        let mut child = Command::new(&self.release.executable)
            .args(["-I", "-S", "-B"])
            .arg(&self.release.runner)
            .arg(action)
            .arg(&self.state)
            .env_clear()
            .stdin(Stdio::from(OwnedFd::from(peer)))
            .stdout(Stdio::from(OwnedFd::from(stdout)))
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(unknown)?;
        let group = rustix::process::Pid::from_raw(child.id() as i32).ok_or(HostError::Guard)?;
        let mut reaped = false;
        let result = (|| {
            channel
                .write_all(&canonical::bytes(request).map_err(unknown)?)
                .map_err(unknown)?;
            channel.shutdown(Shutdown::Write).map_err(unknown)?;
            let mut output = Vec::new();
            let mut buffer = [0u8; 8192];
            loop {
                channel
                    .set_read_timeout(Some(
                        deadline
                            .checked_duration_since(Instant::now())
                            .ok_or_else(|| unknown("Python execution deadline"))?,
                    ))
                    .map_err(unknown)?;
                let count = channel.read(&mut buffer).map_err(unknown)?;
                if count == 0 {
                    break;
                }
                output.extend_from_slice(&buffer[..count]);
                if output.len() > 131072 {
                    return Err(unknown("Python reply exceeds bound"));
                }
            }
            loop {
                if let Some(status) = child.try_wait().map_err(unknown)? {
                    reaped = true;
                    if !status.success() {
                        return Err(unknown("Python helper exited without confirmed receipt"));
                    }
                    break;
                }
                if Instant::now() >= deadline {
                    return Err(unknown("Python process exit deadline"));
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            canonical::decode_json::<Value>(&output).map_err(unknown)
        })();
        // Only this freshly owned group is signaled. Process death is not physical stop evidence.
        if !reaped {
            // Retain the unreaped leader PID while signaling its group; never signal a recycled PID.
            let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
            let _ = child.kill();
            let _ = child.wait();
        }
        if rustix::process::test_kill_process_group(group).is_ok() {
            return Err(unknown("Python process group custody remains unresolved"));
        }
        result
    }
    fn capture(
        &self,
        operation: &Id,
        invocation: &Id,
        request: &Value,
        value: Value,
    ) -> Result<Option<NativeCapture>> {
        for key in [
            "operation",
            "invocation",
            "intent_digest",
            "environment_digest",
            "device_session",
        ] {
            if value[key] != request[key] {
                return Err(HostError::Conflict);
            }
        }
        if value["status"] != "RETURNED" {
            return Ok(None);
        }
        let path = self.state.join("captures").join(operation.as_str());
        if path.exists() {
            return canonical::decode_json(&journal_bytes(
                &self.state,
                &format!("captures/{operation}/capture.json"),
            )?)
            .map(Some)
            .map_err(unknown);
        }
        let capture = NativeCapture {
            status_schema: Name::new("rx.python.returned.v1").map_err(unknown)?,
            status: 0,
            native_id: Some(invocation.to_string()),
            captured_at: self.clock.now(),
            device_session: Id::new(request["device_session"].as_str().ok_or(HostError::Guard)?)
                .map_err(unknown)?,
        };
        rx_package::directory::publish_files(
            BTreeMap::from([(
                rx_package::PackagePath::new("capture.json").map_err(unknown)?,
                canonical::bytes(&capture).map_err(unknown)?,
            )]),
            &path,
        )
        .map_err(unknown)?;
        Ok(Some(capture))
    }
}
impl<N: NativeAdapter, C: Clock> NativeAdapter for PythonSkill<N, C> {
    fn prepare_shutdown(&mut self, resources: &[Name]) -> Result<()> {
        if !self.uncertain.is_empty() {
            return Err(HostError::Guard);
        }
        self.support.prepare_shutdown(resources)
    }
    fn shutdown_snapshot(&self, resources: &[Name]) -> Result<NativeShutdown> {
        if !self.uncertain.is_empty() {
            return Err(HostError::Guard);
        }
        self.support.shutdown_snapshot(resources)
    }
    fn environment(&self) -> Environment {
        self.support.environment()
    }
    fn protection(&self) -> Arc<dyn LocalProtection> {
        self.support.protection()
    }
    fn observe_sources(&self, cell: &Name, sources: &[Name]) -> Result<Vec<SourceObservation>> {
        self.support.observe_sources(cell, sources)
    }
    fn guard(&self, intent: &Intent, now: &TimePoint) -> Result<Guard> {
        if !self
            .programs
            .contains_key(&intent.digest().map_err(unknown)?)
            || !self.uncertain.is_empty()
        {
            return Err(HostError::Guard);
        }
        self.support.guard(intent, now)
    }
    fn can_handover(&self, resources: &[Name]) -> bool {
        self.uncertain.is_empty() && self.support.can_handover(resources)
    }
    fn handover_snapshot(&self, resources: &[Name]) -> Result<LocalHandover> {
        if !self.uncertain.is_empty() {
            return Err(HostError::Guard);
        }
        self.support.handover_snapshot(resources)
    }
    fn submit(&mut self, _: &Id, _: &Id, _: &Intent) -> Result<NativeCapture> {
        Err(HostError::Guard)
    }
    fn submit_with_context(
        &mut self,
        operation: &Id,
        invocation: &Id,
        intent: &Intent,
        context: &NativeDispatch,
    ) -> Result<NativeCapture> {
        let now = self.clock.now();
        let guard = self.guard(intent, &now)?;
        if guard.device_session != context.device_session
            || now.clock_id != context.expires_at.clock_id
            || now.ticks_ns >= context.expires_at.ticks_ns
        {
            return Err(HostError::Stale);
        }
        #[cfg(target_os = "linux")]
        let (dispatch_clock, clock_id) = ("BOOTTIME", rustix::time::ClockId::Boottime);
        #[cfg(not(target_os = "linux"))]
        let (dispatch_clock, clock_id) = ("MONOTONIC", rustix::time::ClockId::Monotonic);
        let os_now = rustix::time::clock_gettime(clock_id);
        let deadline = (os_now.tv_sec as u64 * 1_000_000_000 + os_now.tv_nsec as u64)
            .saturating_add(context.expires_at.ticks_ns.0 - now.ticks_ns.0);
        let program = self
            .programs
            .get(&intent.digest().map_err(unknown)?)
            .ok_or(HostError::Guard)?;
        let request = json!({"schema":"rx.python-host-request.v1","operation":operation,"invocation":invocation,
            "intent_digest":intent.digest().map_err(unknown)?,"environment":program.environment,
            "environment_digest":program.environment_digest,"input":program.input,
            "device_session":context.device_session,"dispatch_deadline_ns":deadline.to_string(),"dispatch_clock":dispatch_clock});
        self.uncertain.insert(operation.clone());
        let value = self.call(
            "execute",
            &request,
            Duration::from_millis(intent.execution_timeout_ms.0),
        )?;
        let capture = self
            .capture(operation, invocation, &request, value)?
            .ok_or_else(|| unknown("Python result unknown"))?;
        self.uncertain.remove(operation);
        Ok(capture)
    }
    fn lookup(&mut self, operation: &Id, invocation: &Id) -> Result<Option<NativeCapture>> {
        let path = self.state.join(operation.as_str()).join("request.json");
        if !path.exists() {
            return Ok(None);
        }
        let request: Value = canonical::decode_json(&journal_bytes(
            &self.state,
            &format!("{operation}/request.json"),
        )?)
        .map_err(unknown)?;
        let digest: Digest = serde_json::from_value(request["intent_digest"].clone())
            .map_err(|_| HostError::Conflict)?;
        let program = self.programs.get(&digest).ok_or(HostError::Conflict)?;
        if request["input"] != program.input
            || request["environment"] != program.environment.to_string_lossy().as_ref()
            || request["operation"] != operation.as_str()
            || request["invocation"] != invocation.as_str()
            || request["environment_digest"] != program.environment_digest.to_string()
            || request["intent_digest"] != program.intent.digest().map_err(unknown)?.to_string()
        {
            return Err(HostError::Conflict);
        }
        let value = self.call("lookup", &request, Duration::from_secs(5))?;
        // Lookup can recover a fact, but reopening/lost process custody does not authorize handover.
        self.capture(operation, invocation, &request, value)
    }
}
