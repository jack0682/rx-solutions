//! OS operations are outside storage transactions. A saved PID is never sufficient to signal a process.
use crate::{Error, Result, model::*};
use rx_domain::{canonical, types::*};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::Duration,
};
#[cfg(target_os = "linux")]
mod linux_limits;
pub enum SpawnFailure {
    NotStarted(Error),
    Uncertain(Error),
}
#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
enum SignalOutcome {
    Accepted,
    OwnedNonActuatingExit,
}
#[cfg(unix)]
fn signal_outcome(
    accepted: bool,
    effect: Option<Effect>,
    observe_exit: impl FnOnce() -> std::io::Result<Option<std::process::ExitStatus>>,
) -> Result<SignalOutcome> {
    if accepted {
        return Ok(SignalOutcome::Accepted);
    }
    // A failed signal is not success. Only a positive observation of this same
    // owned non-actuating Child's exit can settle the existing already-exited
    // case. Control effects and unavailable/live observations remain errors.
    if effect == Some(Effect::NonActuating) && observe_exit()?.is_some() {
        return Ok(SignalOutcome::OwnedNonActuatingExit);
    }
    Err(Error::Reconciliation(
        "owned process signal was not confirmed".into(),
    ))
}
pub trait Backend {
    /// Read component self-report data in the current owned-child context.
    /// The default has no functional observation source; process readiness is
    /// never promoted here. Positive observations cannot be minted by callers.
    fn observe_status(
        &mut self,
        _launch: &Launch,
        _request: &crate::use_assessment::StatusObservationRequest,
    ) -> Result<crate::use_assessment::StatusObservationResult> {
        Ok(
            crate::use_assessment::StatusObservationResult::Unsupported {
                condition: Name::new("readiness/observation-provider").expect("literal"),
                reason: "backend has no functional self-report observation provider".into(),
            },
        )
    }
    fn spawn(
        &mut self,
        launch: &Launch,
        authorize: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<u32, SpawnFailure>;
    /// Whole-bundle resource admission and process creation are one boundary.
    /// Rejected and NotStarted MUST leave no policies/reservations or child behind.
    /// If rollback/creation cannot be confirmed, return Uncertain, not Rejected.
    /// Recheck authorize immediately before any application/exec. Future OS
    /// backends own the resulting resource lifetime alongside the child handle.
    /// The default implements no cgroup, rlimit, reservation or device policy.
    fn spawn_with_requirements(
        &mut self,
        launch: &Launch,
        request: &crate::execution::Request,
        authorize: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<crate::execution::Decision, SpawnFailure> {
        use crate::execution::{Decision, Evidence, Receipt};
        let unknown = request.unknown();
        if !unknown.is_empty() {
            return Ok(Decision::Rejected { unmet: unknown });
        }
        if request.requirements().needs_enforcement() {
            return Ok(Decision::Rejected {
                unmet: request
                    .reject("host execution enforcement is not implemented by this backend"),
            });
        }
        let receipt = Receipt::reported(request, Evidence::NoRequirements)
            .map_err(SpawnFailure::NotStarted)?;
        self.spawn(launch, authorize).map(|pid| Decision::Admitted {
            pid,
            receipt,
            identity: None,
        })
    }
    fn pid(&self, instance: &Id) -> Option<u32>;
    fn owns(&self, instance: &Id) -> bool;
    fn forget_exited(&mut self, instance: &Id) -> Result<()>;
    fn exited(&mut self, instance: &Id) -> Result<Option<Option<i32>>>;
    fn ready(&mut self, launch: &Launch) -> Result<bool>;
    fn guarded_status(&mut self, _launch: &Launch) -> Result<Option<GuardedObservation>> {
        Err(Error::Invalid(
            "backend has no guarded status reader".into(),
        ))
    }
    fn terminate(&mut self, instance: &Id, force: bool) -> Result<()>;
}
pub struct OsProcesses {
    children: BTreeMap<Id, Child>,
    effects: BTreeMap<Id, Effect>,
    observations: BTreeMap<Id, GuardedObservation>,
    logs: PathBuf,
}
impl OsProcesses {
    /// Positive evidence from this live owner's actual non-actuating Child.
    /// Absence of a PID/handle or an elapsed deadline is never sufficient.
    /// Retires only this direct child handle, not descendants or shared resources.
    pub fn observe_recovery_exit(
        &mut self,
        instance: &Id,
    ) -> Result<crate::registration::OwnedExit> {
        if self.effects.get(instance) != Some(&Effect::NonActuating) {
            return Err(Error::Reconciliation(
                "recovery evidence requires an owned non-actuating child".into(),
            ));
        }
        let child = self.children.get_mut(instance).ok_or_else(|| {
            Error::Reconciliation(
                "owned Child handle absent; external investigation provider is unsupported".into(),
            )
        })?;
        let status = child
            .try_wait()?
            .ok_or_else(|| Error::Reconciliation("owned child has not exited".into()))?;
        let pid = child.id();
        let ticks_ns = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| Error::Invalid(e.to_string()))?
                .as_nanos(),
        )
        .map_err(|e| Error::Invalid(e.to_string()))?;
        let observed_at = TimePoint {
            clock_id: "unix-utc-ns".into(),
            ticks_ns: Counter(ticks_ns),
        };
        self.forget_exited(instance)?;
        Ok(crate::registration::OwnedExit::observed(
            instance.clone(),
            pid,
            status.code(),
            observed_at,
        ))
    }
    pub fn owned_instances(&self) -> Vec<Id> {
        self.children.keys().cloned().collect()
    }
    pub fn new(logs: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&logs)?;
        Ok(Self {
            children: BTreeMap::new(),
            effects: BTreeMap::new(),
            observations: BTreeMap::new(),
            logs,
        })
    }
}
pub fn verify(path: &std::path::Path, expected: Digest) -> Result<()> {
    if !path.is_absolute() || !path.is_file() {
        return Err(Error::Invalid(
            "program path must be an absolute regular file".into(),
        ));
    }
    let bytes = std::fs::read(path)?;
    if Digest::from_bytes(Sha256::digest(bytes).into()) != expected {
        return Err(Error::Invalid(format!(
            "program file integrity differs: {}",
            path.display()
        )));
    }
    Ok(())
}
impl Backend for OsProcesses {
    fn spawn_with_requirements(
        &mut self,
        launch: &Launch,
        request: &crate::execution::Request,
        authorize: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<crate::execution::Decision, SpawnFailure> {
        use crate::execution::{Decision, Evidence, Receipt};
        if !request.requirements().needs_enforcement() {
            let receipt = Receipt::reported(request, Evidence::NoRequirements)
                .map_err(SpawnFailure::NotStarted)?;
            let pid = self.spawn(launch, authorize)?;
            let identity = self
                .children
                .get_mut(&launch.instance)
                .and_then(|child| crate::process_identity::capture(child, request));
            return Ok(Decision::Admitted {
                pid,
                receipt,
                identity,
            });
        }
        #[cfg(target_os = "linux")]
        {
            self.spawn_limited(launch, request, authorize)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Ok(Decision::Rejected { unmet: request.reject("OS enforcement unsupported on this platform; Linux AddressSpaceBytes is the only implemented usage ceiling") })
        }
    }
    fn observe_status(
        &mut self,
        launch: &Launch,
        request: &crate::use_assessment::StatusObservationRequest,
    ) -> Result<crate::use_assessment::StatusObservationResult> {
        use crate::use_assessment::{StatusObservation, StatusObservationResult as R};
        let named = |s| Name::new(s).expect("literal");
        if launch.effect != Effect::NonActuating
            || !matches!(launch.ready, ReadyProbe::HttpStatus { .. })
        {
            return Ok(R::Unsupported{condition:named("readiness/status-source"),reason:"only non-actuating HttpStatus programs have this provider; AliveOnly remains a liveness probe".into()});
        }
        let Some(child) = self.children.get_mut(&launch.instance) else {
            return Ok(R::NotEvaluated {
                condition: named("execution/current-owner"),
                reason: "no current owned Child; saved identity is not observation evidence".into(),
            });
        };
        if self.effects.get(&launch.instance) != Some(&Effect::NonActuating) {
            return Ok(R::Unsupported {
                condition: named("report/owned-effect"),
                reason: "the owned child is not a non-actuating observation source".into(),
            });
        }
        if request.instance() != Some(&launch.instance) || request.pid() != Some(child.id()) {
            return Ok(R::NotMet {
                condition: named("report/owner-binding"),
                reason: "assessment instance/PID differs from owned launch".into(),
            });
        }
        if child.try_wait()?.is_some() {
            return Ok(R::NotMet {
                condition: named("execution/current-child"),
                reason: "owned child has exited; no current functional report".into(),
            });
        }
        let Some(port) = launch.port else {
            return Ok(R::Unsupported {
                condition: named("report/http-binding"),
                reason: "no authored HTTP port binding".into(),
            });
        };
        let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
        let Ok(mut stream) =
            TcpStream::connect_timeout(&address.into(), Duration::from_millis(100))
        else {
            return Ok(R::NotEvaluated {
                condition: named("report/transport"),
                reason: "no HTTP response obtained; declared conditions not evaluated".into(),
            });
        };
        stream.set_read_timeout(Some(Duration::from_millis(200)))?;
        stream.set_write_timeout(Some(Duration::from_millis(100)))?;
        stream.write_all(
            format!(
                "GET {} HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                request.endpoint().path()
            )
            .as_bytes(),
        )?;
        let mut bytes = vec![];
        if stream.take(1_048_577).read_to_end(&mut bytes).is_err() {
            return Ok(R::NotEvaluated {
                condition: named("report/transport"),
                reason: "complete HTTP response not obtained; no readiness inferred".into(),
            });
        }
        if bytes.len() > 1_048_576 {
            return Ok(R::NotMet {
                condition: named("report/size"),
                reason: "component report exceeds bounded response size".into(),
            });
        }
        let Some(split) = bytes.windows(4).position(|s| s == b"\r\n\r\n") else {
            return Ok(R::NotMet {
                condition: named("report/http-format"),
                reason: "malformed HTTP response".into(),
            });
        };
        if !bytes.starts_with(b"HTTP/1.0 200 ") && !bytes.starts_with(b"HTTP/1.1 200 ") {
            return Ok(R::NotMet {
                condition: named("report/http-status"),
                reason: "requested diagnostic endpoint did not return HTTP 200".into(),
            });
        }
        let Ok(reported) = canonical::decode_json::<serde_json::Value>(&bytes[split + 4..]) else {
            return Ok(R::NotMet {
                condition: named("report/json-format"),
                reason: "component response is not unambiguous JSON".into(),
            });
        };
        if self
            .children
            .get_mut(&launch.instance)
            .expect("owned child retained")
            .try_wait()?
            .is_some()
        {
            return Ok(R::NotMet {
                condition: named("execution/current-child"),
                reason: "owned child exited while its response was read".into(),
            });
        }
        Ok(R::Observed(Box::new(
            StatusObservation::captured(request, reported).map_err(Error::Invalid)?,
        )))
    }
    fn spawn(
        &mut self,
        l: &Launch,
        authorize: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<u32, SpawnFailure> {
        self.spawn_local(l, authorize)
            .map_err(SpawnFailure::NotStarted)
    }
    fn pid(&self, instance: &Id) -> Option<u32> {
        self.children.get(instance).map(Child::id)
    }
    fn owns(&self, instance: &Id) -> bool {
        self.children.contains_key(instance)
    }
    fn forget_exited(&mut self, instance: &Id) -> Result<()> {
        let child = self
            .children
            .get_mut(instance)
            .ok_or_else(|| Error::Reconciliation("unknown process handle".into()))?;
        if child.try_wait()?.is_none() {
            return Err(Error::Reconciliation(
                "cannot discard a live process handle".into(),
            ));
        }
        self.children.remove(instance);
        self.effects.remove(instance);
        self.observations.remove(instance);
        Ok(())
    }
    fn exited(&mut self, instance: &Id) -> Result<Option<Option<i32>>> {
        let child = self
            .children
            .get_mut(instance)
            .ok_or_else(|| Error::Reconciliation("no owned process handle".into()))?;
        Ok(child.try_wait()?.map(|exit| exit.code()))
    }
    fn ready(&mut self, l: &Launch) -> Result<bool> {
        self.ready_local(l)
    }
    fn guarded_status(&mut self, l: &Launch) -> Result<Option<GuardedObservation>> {
        let ReadyProbe::GuardedStatus(binding) = &l.ready else {
            return Err(Error::Invalid("guarded status binding required".into()));
        };
        let child = self
            .children
            .get(&l.instance)
            .ok_or_else(|| Error::Reconciliation("no owned child for status read".into()))?;
        let observation = rx_service_status::read(binding, &l.instance, child.id())
            .map_err(|e| Error::Reconciliation(e.to_string()))?;
        if let Some(next) = &observation {
            if let Some(previous) = self.observations.get(&l.instance)
                && (next.sequence < previous.sequence
                    || next.observed_at.clock_id != previous.observed_at.clock_id
                    || next.observed_at.ticks_ns < previous.observed_at.ticks_ns
                    || (next.sequence == previous.sequence
                        && next.payload_digest != previous.payload_digest))
            {
                return Err(Error::Reconciliation(
                    "guarded status cut regressed or changed".into(),
                ));
            }
            self.observations.insert(l.instance.clone(), next.clone());
        }
        Ok(observation)
    }
    fn terminate(&mut self, instance: &Id, force: bool) -> Result<()> {
        self.terminate_local(instance, force)
    }
}
impl OsProcesses {
    fn spawn_local(&mut self, l: &Launch, authorize: &mut dyn FnMut() -> bool) -> Result<u32> {
        if self.children.contains_key(&l.instance) {
            return Err(Error::Invalid("instance already owned".into()));
        }
        verify(&l.executable, l.executable_sha256)?;
        for (p, h) in &l.files {
            verify(p, *h)?;
        }
        let stdout = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(self.logs.join(format!("{}.stdout.log", l.instance)))?;
        let stderr = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(self.logs.join(format!("{}.stderr.log", l.instance)))?;
        let mut command = Command::new(&l.executable);
        command
            .args(&l.arguments)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("LANG", "C.UTF-8")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("RX_PROCESS_INSTANCE_ID", l.instance.as_str())
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr);
        if let ReadyProbe::GuardedStatus(binding) = &l.ready {
            command.env("RX_PROCESS_STATUS_PATH", binding.path());
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        if !authorize() {
            return Err(Error::Invalid(
                "startup authority unavailable at OS boundary".into(),
            ));
        }
        let child = command.spawn()?;
        let pid = child.id();
        self.children.insert(l.instance.clone(), child);
        self.effects.insert(l.instance.clone(), l.effect);
        Ok(pid)
    }
    fn ready_local(&mut self, l: &Launch) -> Result<bool> {
        if !self.children.contains_key(&l.instance) {
            return Ok(false);
        }
        match &l.ready {
            ReadyProbe::AliveOnly => Ok(true),
            ReadyProbe::GuardedStatus(_) => Ok(self
                .guarded_status(l)?
                .is_some_and(|o| o.state == GuardedState::Ready)),
            ReadyProbe::HttpStatus { .. } => {
                let address = SocketAddrV4::new(
                    Ipv4Addr::LOCALHOST,
                    l.port
                        .ok_or_else(|| Error::Invalid("probe port missing".into()))?,
                );
                let Ok(mut stream) =
                    TcpStream::connect_timeout(&address.into(), Duration::from_millis(50))
                else {
                    return Ok(false);
                };
                stream.set_read_timeout(Some(Duration::from_millis(100)))?;
                stream.set_write_timeout(Some(Duration::from_millis(100)))?;
                stream.write_all(
                    b"GET /health HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                )?;
                let mut bytes = vec![];
                if stream.take(1_048_577).read_to_end(&mut bytes).is_err() {
                    return Ok(false);
                }
                if bytes.len() > 1_048_576 {
                    return Ok(false);
                }
                let Some(split) = bytes.windows(4).position(|s| s == b"\r\n\r\n") else {
                    return Ok(false);
                };
                if !bytes.starts_with(b"HTTP/1.0 200 ") && !bytes.starts_with(b"HTTP/1.1 200 ") {
                    return Ok(false);
                }
                let Ok(value) = canonical::decode_json::<serde_json::Value>(&bytes[split + 4..])
                else {
                    return Ok(false);
                };
                Ok(value["schema"] == "rx.solutions-status.v1"
                    && value["phase"] == "SOFTWARE_READY_UNCOMMISSIONED"
                    && value["supervisor_instance"] == l.instance.as_str())
            }
        }
    }
    fn terminate_local(&mut self, instance: &Id, force: bool) -> Result<()> {
        let effect = self.effects.get(instance).copied();
        let guarded = effect == Some(Effect::ProtocolGuardedService);
        if guarded && force {
            return Err(Error::Invalid(
                "protocol-guarded services cannot be force-killed".into(),
            ));
        }
        let child = self.children.get_mut(instance).ok_or_else(|| {
            Error::Reconciliation(
                "cannot signal a recorded PID without its live child handle".into(),
            )
        })?;
        // No concurrent reaper exists. An exited child remains unreaped until this owner checks it.
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        #[cfg(unix)]
        {
            let target = if guarded {
                child.id().to_string()
            } else {
                format!("-{}", child.id())
            };
            let status = Command::new("/bin/kill")
                .env_clear()
                .args([if force { "-KILL" } else { "-TERM" }, "--", &target])
                .status()?;
            let _ = signal_outcome(status.success(), effect, || child.try_wait())?;
        }
        #[cfg(not(unix))]
        {
            let _ = force;
            return Err(Error::Invalid(
                "OS process termination is supported only on Unix in this draft".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod signal_tests {
    use super::*;
    struct OwnedTestChild(Child);
    impl Drop for OwnedTestChild {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
            }
            let _ = self.0.wait();
        }
    }
    fn running() -> OwnedTestChild {
        OwnedTestChild(Command::new("/bin/sleep").arg("30").spawn().unwrap())
    }
    fn exited() -> OwnedTestChild {
        let mut child = OwnedTestChild(Command::new("/usr/bin/true").spawn().unwrap());
        assert!(child.0.wait().unwrap().success());
        child
    }
    #[test]
    fn signal_acceptance_does_not_claim_child_exit() {
        let mut child = running();
        assert!(child.0.try_wait().unwrap().is_none());
        assert_eq!(
            signal_outcome(true, Some(Effect::NonActuating), || panic!(
                "acceptance is not an exit observation"
            ))
            .unwrap(),
            SignalOutcome::Accepted
        );
        assert!(child.0.try_wait().unwrap().is_none());
    }
    #[test]
    fn failed_signal_uses_only_confirmed_owned_non_actuating_exit() {
        let mut child = exited();
        assert_eq!(
            signal_outcome(false, Some(Effect::NonActuating), || child.0.try_wait()).unwrap(),
            SignalOutcome::OwnedNonActuatingExit
        );
    }
    #[test]
    fn failed_signal_with_live_or_unconfirmed_child_remains_error() {
        let mut child = running();
        assert!(signal_outcome(false, Some(Effect::NonActuating), || child.0.try_wait()).is_err());
        assert!(
            signal_outcome(false, Some(Effect::NonActuating), || Err(
                std::io::Error::other("injected observation unavailability")
            ))
            .is_err()
        );
        assert!(child.0.try_wait().unwrap().is_none());
    }
    #[test]
    fn failed_signal_keeps_control_and_unknown_effect_errors() {
        let mut child = exited();
        for effect in [
            Some(Effect::ProtocolGuardedService),
            Some(Effect::RequiresPlatformAuthority),
            None,
        ] {
            assert!(
                signal_outcome(false, effect, || panic!(
                    "control path must not use non-actuating fallback"
                ))
                .is_err()
            );
            assert!(child.0.try_wait().unwrap().is_some());
        }
    }
}
