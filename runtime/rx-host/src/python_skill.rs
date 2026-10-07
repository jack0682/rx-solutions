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
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

mod owned;
use owned::OwnedCall;

struct Pending {
    invocation: Id,
    request: Value,
    receiver: mpsc::Receiver<std::result::Result<Value, String>>,
    result: Option<std::result::Result<Value, String>>,
}

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
    execution: Option<crate::python_execution::Profile>,
    release: ReleasePython,
    programs: BTreeMap<Digest, Program>,
    state: PathBuf,
    support: N,
    clock: C,
    uncertain: BTreeSet<Id>,
    pending: BTreeMap<Id, Pending>,
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
        Self::open_inner(release, programs, None, state, support, clock)
    }
    pub fn open_execution(
        release: ReleasePython,
        profile: crate::python_execution::Profile,
        state: PathBuf,
        support: N,
        clock: C,
    ) -> Result<Self> {
        profile.validate()?;
        Self::open_inner(release, vec![], Some(profile), state, support, clock)
    }
    fn open_inner(
        release: ReleasePython,
        programs: Vec<Program>,
        execution: Option<crate::python_execution::Profile>,
        state: PathBuf,
        support: N,
        clock: C,
    ) -> Result<Self> {
        if support.environment() != Environment::Simulation
            || !state.is_absolute()
            || state.is_symlink()
            || !state.is_dir()
            || (programs.is_empty() != execution.is_some())
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
            execution,
            release,
            programs: selected,
            state,
            support,
            clock,
            uncertain,
            pending: BTreeMap::new(),
        })
    }
    fn dispatch_request(
        &self,
        operation: &Id,
        invocation: &Id,
        intent: &Intent,
        context: &NativeDispatch,
    ) -> Result<Value> {
        let now = self.clock.now();
        let guard = self.guard_with_input(intent, &now, context.execution.as_ref())?;
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
        let bound = if let Some(input) = &context.execution {
            if input.binding.operation != *operation {
                return Err(HostError::Conflict);
            }
            Some(
                self.execution
                    .as_ref()
                    .ok_or(HostError::Guard)?
                    .program(intent, input)?,
            )
        } else {
            None
        };
        let program = match &bound {
            Some(program) => program,
            None => self
                .programs
                .get(&intent.digest().map_err(unknown)?)
                .ok_or(HostError::Guard)?,
        };
        Ok(
            json!({"schema":"rx.python-host-request.v1","operation":operation,"invocation":invocation,
            "intent_digest":intent.digest().map_err(unknown)?,"environment":program.environment,
            "environment_digest":program.environment_digest,"input":program.input,
            "device_session":context.device_session,"dispatch_deadline_ns":deadline.to_string(),"dispatch_clock":dispatch_clock}),
        )
    }
    fn call(&self, action: &str, request: &Value, timeout: Duration) -> Result<Value> {
        OwnedCall::start(
            &self.release,
            &self.state,
            action,
            &canonical::bytes(request).map_err(unknown)?,
            timeout,
        )?
        .finish()
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
    fn guard_with_input(
        &self,
        intent: &Intent,
        now: &TimePoint,
        input: Option<&BoundInput>,
    ) -> Result<Guard> {
        let Some(input) = input else {
            return self.guard(intent, now);
        };
        self.execution
            .as_ref()
            .ok_or(HostError::Guard)?
            .program(intent, input)?;
        if !self.uncertain.is_empty() {
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
        let request = self.dispatch_request(operation, invocation, intent, context)?;
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
    fn begin_with_context(
        &mut self,
        operation: &Id,
        invocation: &Id,
        intent: &Intent,
        context: &NativeDispatch,
    ) -> Result<NativeSubmission> {
        if context.execution.is_none() {
            return self
                .submit_with_context(operation, invocation, intent, context)
                .map(NativeSubmission::Captured);
        }
        let request = self.dispatch_request(operation, invocation, intent, context)?;
        let bytes = canonical::bytes(&request).map_err(unknown)?;
        self.uncertain.insert(operation.clone());
        let mut call = OwnedCall::start(
            &self.release,
            &self.state,
            "execute-entered",
            &bytes,
            Duration::from_millis(intent.execution_timeout_ms.0),
        )?;
        let now = self.clock.now();
        if now.clock_id != context.expires_at.clock_id
            || now.ticks_ns >= context.expires_at.ticks_ns
        {
            return Err(HostError::Stale);
        }
        let entry = call.entry(Duration::from_nanos(
            context.expires_at.ticks_ns.0 - now.ticks_ns.0,
        ))?;
        if entry.as_object().map(|v| v.len()) != Some(7)
            || entry["schema"] != "rx.python-native-entry.v2"
            || entry["request_sha256"] != rx_package::content_digest(&bytes).to_string()
        {
            return Err(HostError::Conflict);
        }
        for key in [
            "operation",
            "invocation",
            "intent_digest",
            "environment_digest",
            "device_session",
        ] {
            if entry[key] != request[key] {
                return Err(HostError::Conflict);
            }
        }
        let confirmed = self.clock.now();
        if confirmed.clock_id != context.expires_at.clock_id
            || confirmed.ticks_ns >= context.expires_at.ticks_ns
        {
            return Err(HostError::Stale);
        }
        let payload = canonical::bytes(&entry).map_err(unknown)?;
        let proof = rx_process_contract::execution_v2::host_inputs::NativeEntry {
            operation: operation.clone(),
            invocation: invocation.clone(),
            intent_digest: intent.digest().map_err(unknown)?,
            profile_digest: intent.profile_digest,
            device_session: context.device_session.clone(),
            evidence: ArtifactRef {
                schema_id: Name::new("rx.python-native-entry.v2").map_err(unknown)?,
                sha256: rx_package::content_digest(&payload),
                size_bytes: Counter(payload.len() as u64),
            },
            payload,
        };
        proof.validate().map_err(unknown)?;
        let (sender, receiver) = mpsc::channel();
        // This waiter only observes the same owned child; it has no submit capability.
        std::thread::Builder::new()
            .name("rx-python-completion".into())
            .spawn(move || {
                let _ = sender.send(call.finish().map_err(|error| error.to_string()));
            })
            .map_err(unknown)?;
        self.pending.insert(
            operation.clone(),
            Pending {
                invocation: invocation.clone(),
                request,
                receiver,
                result: None,
            },
        );
        Ok(NativeSubmission::Entered(proof))
    }
    fn completed(&mut self) -> Result<Vec<NativeCompletion>> {
        let mut ready = Vec::new();
        for (operation, pending) in &mut self.pending {
            if pending.result.is_none() {
                match pending.receiver.try_recv() {
                    Ok(result) => pending.result = Some(result),
                    Err(mpsc::TryRecvError::Empty) => (),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        pending.result = Some(Err("Python completion custody lost".into()))
                    }
                }
            }
            if let Some(Ok(value)) = &pending.result {
                ready.push((
                    operation.clone(),
                    pending.invocation.clone(),
                    pending.request.clone(),
                    value.clone(),
                ));
            }
        }
        let mut captures = Vec::new();
        for (operation, invocation, request, value) in ready {
            if let Some(capture) = self.capture(&operation, &invocation, &request, value)? {
                captures.push(NativeCompletion {
                    operation,
                    invocation,
                    capture,
                });
            }
        }
        Ok(captures)
    }
    fn acknowledge_completion(&mut self, operation: &Id) {
        if self.pending.remove(operation).is_some() {
            self.uncertain.remove(operation);
        }
    }
    fn lookup(&mut self, operation: &Id, invocation: &Id) -> Result<Option<NativeCapture>> {
        self.lookup_program(operation, invocation, None)
    }
    fn lookup_with_input(
        &mut self,
        operation: &Id,
        invocation: &Id,
        intent: &Intent,
        input: Option<&BoundInput>,
    ) -> Result<Option<NativeCapture>> {
        let Some(input) = input else {
            return self.lookup(operation, invocation);
        };
        if input.binding.operation != *operation {
            return Err(HostError::Conflict);
        }
        let program = self
            .execution
            .as_ref()
            .ok_or(HostError::Guard)?
            .program(intent, input)?;
        self.lookup_program(operation, invocation, Some(&program))
    }
}
impl<N: NativeAdapter, C: Clock> PythonSkill<N, C> {
    fn lookup_program(
        &mut self,
        operation: &Id,
        invocation: &Id,
        bound: Option<&Program>,
    ) -> Result<Option<NativeCapture>> {
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
        let program = bound
            .or_else(|| self.programs.get(&digest))
            .ok_or(HostError::Conflict)?;
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
