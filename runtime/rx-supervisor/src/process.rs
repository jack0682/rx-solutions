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
pub enum SpawnFailure {
    NotStarted(Error),
    Uncertain(Error),
}
pub trait Backend {
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
        self.spawn(launch, authorize)
            .map(|pid| Decision::Admitted { pid, receipt })
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
        let guarded = self.effects.get(instance) == Some(&Effect::ProtocolGuardedService);
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
            if !status.success() {
                return Err(Error::Reconciliation(
                    "owned process signal was not confirmed".into(),
                ));
            }
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
