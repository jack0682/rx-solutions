//! First OS enforcement scope: per-process virtual address space, not RAM or
//! reservation. A private gated child prevents target exec before observation.
use super::*;
use crate::execution::{
    Capacity, Decision, Evidence, KernelObservation, Receipt, Request, Requirement,
};
use rustix::process::{Pid, Resource, Rlimit, prlimit};
use std::{
    io::{BufRead, BufReader},
    os::{fd::OwnedFd, unix::net::UnixStream},
    time::Instant,
};

const GATE: &str = include_str!("linux_gate.py");

fn rejected(request: &Request, reason: impl AsRef<str>) -> Decision {
    Decision::Rejected {
        unmet: request.reject(reason.as_ref()),
    }
}
fn policy(request: &Request, launch: &Launch) -> std::result::Result<(Name, u64), String> {
    if request.instance() != &launch.instance || request.process() != &launch.selection {
        return Err("request and launch instance/selection differ".into());
    }
    if launch.effect != Effect::NonActuating
        || launch.executable != std::path::Path::new("/usr/bin/python3")
    {
        return Err("AddressSpaceBytes currently supports only release-verified /usr/bin/python3 non-actuating programs".into());
    }
    let mut selected = None;
    for (key, requirement) in &request.requirements().0 {
        match requirement {
            Requirement::NotRequired => {}
            Requirement::UpperBound {
                resource: Capacity::AddressSpaceBytes,
                amount,
            } if amount.0 > 0 && amount.0 < u64::MAX => {
                if selected.replace((key.clone(), amount.0)).is_some() {
                    return Err(
                        "multiple address-space declarations unsupported; whole bundle not applied"
                            .into(),
                    );
                }
            }
            _ => {
                return Err(format!(
                    "unsupported requirement {key}: only AddressSpaceBytes UpperBound is implemented; memory/CPU capacity, reservations and access grants are not applied"
                ));
            }
        }
    }
    selected.ok_or_else(|| "no supported address-space requirement".into())
}

/// procfs is read from the parent, never a child-supplied success report.
fn observed_limit(pid: u32) -> Result<(u64, u64)> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/limits"))?;
    let row = text
        .lines()
        .find_map(|l| l.strip_prefix("Max address space"))
        .ok_or_else(|| Error::Reconciliation("address-space kernel limit row absent".into()))?;
    let parts: Vec<_> = row.split_whitespace().collect();
    if parts.len() != 3 || parts[2] != "bytes" {
        return Err(Error::Reconciliation(
            "address-space kernel limit format differs".into(),
        ));
    }
    let value = |s: &str| {
        s.parse::<u64>().map_err(|_| {
            Error::Reconciliation("finite address-space kernel limit not observed".into())
        })
    };
    Ok((value(parts[0])?, value(parts[1])?))
}

trait Kernel {
    fn gate(&self) -> &str {
        GATE
    }
    fn apply(&self, pid: u32, amount: u64) -> Result<(Option<u64>, Option<u64>)> {
        let pid = Pid::from_raw(
            i32::try_from(pid).map_err(|_| Error::Invalid("child PID range".into()))?,
        )
        .ok_or_else(|| Error::Invalid("zero child PID".into()))?;
        let old = prlimit(
            Some(pid),
            Resource::As,
            Rlimit {
                current: Some(amount),
                maximum: Some(amount),
            },
        )
        .map_err(std::io::Error::from)?;
        Ok((old.current, old.maximum))
    }
    fn observe(&self, pid: u32) -> Result<(u64, u64)> {
        observed_limit(pid)
    }
}
struct LinuxKernel;
impl Kernel for LinuxKernel {}

impl OsProcesses {
    pub(super) fn spawn_limited(
        &mut self,
        launch: &Launch,
        request: &Request,
        authorize: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<Decision, SpawnFailure> {
        let (requirement, amount) = match policy(request, launch) {
            Ok(v) => v,
            Err(reason) => return Ok(rejected(request, reason)),
        };
        self.spawn_gated(
            launch,
            request,
            requirement,
            amount,
            authorize,
            &LinuxKernel,
        )
    }

    fn retain_uncertain(&mut self, launch: &Launch, child: Child, reason: String) -> SpawnFailure {
        self.effects.insert(launch.instance.clone(), launch.effect);
        self.children.insert(launch.instance.clone(), child);
        SpawnFailure::Uncertain(Error::Reconciliation(reason))
    }

    fn rollback_gate(
        &mut self,
        launch: &Launch,
        request: &Request,
        mut child: Child,
        reason: String,
    ) -> std::result::Result<Decision, SpawnFailure> {
        // EXEC was not authorized, or the trusted gate explicitly reported exec
        // failure. It has no descendants or reservations. Confirm exit; never
        // translate a failed signal or elapsed timeout into rollback success.
        let _ = child.kill();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => {
                    return Ok(rejected(
                        request,
                        format!(
                            "{reason}; gated child exit confirmed, no policy remains from this attempt"
                        ),
                    ));
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                _ => {
                    return Err(self.retain_uncertain(
                        launch,
                        child,
                        format!("{reason}; gated child rollback unconfirmed"),
                    ));
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_gated(
        &mut self,
        launch: &Launch,
        request: &Request,
        requirement: Name,
        amount: u64,
        authorize: &mut dyn FnMut() -> bool,
        kernel: &impl Kernel,
    ) -> std::result::Result<Decision, SpawnFailure> {
        if self.children.contains_key(&launch.instance) {
            return Err(SpawnFailure::Uncertain(Error::Reconciliation(
                "instance already has a child handle".into(),
            )));
        }
        let prepare = (|| -> Result<_> {
            verify(&launch.executable, launch.executable_sha256)?;
            for (path, digest) in &launch.files {
                verify(path, *digest)?;
            }
            let stdout_path = self.logs.join(format!("{}.stdout.log", launch.instance));
            let stdout = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&stdout_path)?;
            let stderr = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(self.logs.join(format!("{}.stderr.log", launch.instance)))?;
            let (parent, gate) = UnixStream::pair()?;
            parent.set_read_timeout(Some(Duration::from_secs(5)))?;
            parent.set_write_timeout(Some(Duration::from_secs(5)))?;
            let payload = serde_json::to_string(&serde_json::json!({
                "executable": launch.executable, "arguments": launch.arguments,
                "stdout": stdout_path.canonicalize()? }))
            .map_err(|e| Error::Invalid(e.to_string()))?;
            let mut command = Command::new("/usr/bin/python3");
            command
                .args(["-c", kernel.gate(), &payload])
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("LANG", "C.UTF-8")
                .env("PYTHONDONTWRITEBYTECODE", "1")
                .env("RX_PROCESS_INSTANCE_ID", launch.instance.as_str())
                .stdin(Stdio::from(OwnedFd::from(gate.try_clone()?)))
                .stdout(Stdio::from(OwnedFd::from(gate)))
                .stderr(stderr);
            use std::os::unix::process::CommandExt;
            command.process_group(0);
            drop(stdout); // Reserved with create_new; the gate opens only this log.
            Ok((command, parent))
        })();
        let (mut command, mut channel) = match prepare {
            Ok(v) => v,
            Err(error) => {
                return Ok(rejected(
                    request,
                    format!("launch preparation failed: {error}"),
                ));
            }
        };
        if !authorize() {
            return Ok(rejected(
                request,
                "startup authority unavailable before gated child creation",
            ));
        }
        let mut child = match command.spawn() {
            Ok(v) => v,
            Err(error) => {
                return Ok(rejected(
                    request,
                    format!("gated child creation failed: {error}"),
                ));
            }
        };
        drop(command); // Close the parent's copies of the child's socket ends.
        let pid = child.id();
        let mut reader = match channel.try_clone() {
            Ok(stream) => BufReader::new(stream),
            Err(error) => return self.rollback_gate(launch, request, child, error.to_string()),
        };
        let mut message = String::new();
        if reader.read_line(&mut message).is_err() || message != "GATED\n" {
            return self.rollback_gate(
                launch,
                request,
                child,
                "private child gate unavailable before policy application".into(),
            );
        }
        let previous = match kernel.apply(pid, amount) {
            Ok(v) => v,
            Err(error) => {
                return self.rollback_gate(
                    launch,
                    request,
                    child,
                    format!("kernel application failed: {error}"),
                );
            }
        };
        match kernel.observe(pid) {
            Ok((soft, hard)) if soft == amount && hard == amount => {}
            _ => {
                return self.rollback_gate(
                    launch,
                    request,
                    child,
                    "parent kernel observation did not confirm the complete policy".into(),
                );
            }
        }
        if !authorize() {
            return self.rollback_gate(
                launch,
                request,
                child,
                "startup authority changed after application, before EXEC".into(),
            );
        }
        if channel.write_all(b"EXEC\n").is_err() {
            // A write error may follow a partial delivery. EXEC entry is no
            // longer provably absent; keep ownership and uncertainty.
            return Err(self.retain_uncertain(
                launch,
                child,
                "EXEC authorization delivery uncertain".into(),
            ));
        }
        message.clear();
        match reader.read_line(&mut message) {
            Ok(_) if message.starts_with("EXEC_ERROR:") => {
                return self.rollback_gate(launch, request, child, message.trim().into());
            }
            Ok(0) => {}
            _ => {
                return Err(self.retain_uncertain(
                    launch,
                    child,
                    "target exec acknowledgement unavailable".into(),
                ));
            }
        }
        // EOF alone could mean a killed gate. Kernel argv must show the target,
        // not the bootstrap, and the same owned PID must still be observable.
        let actual = std::fs::read(format!("/proc/{pid}/cmdline"));
        use std::os::unix::ffi::OsStrExt;
        let mut expected = launch.executable.as_os_str().as_bytes().to_vec();
        expected.push(0);
        for arg in &launch.arguments {
            expected.extend_from_slice(arg.as_bytes());
            expected.push(0);
        }
        if !matches!(actual, Ok(ref v) if *v == expected)
            || !matches!(kernel.observe(pid), Ok((soft, hard)) if soft == amount && hard == amount)
            || !matches!(child.try_wait(), Ok(None))
        {
            return Err(self.retain_uncertain(
                launch,
                child,
                "final target/PID/kernel limit could not be confirmed after EXEC".into(),
            ));
        }
        let observation = KernelObservation::observed(request, pid, requirement, amount, previous);
        let receipt = match Receipt::reported(
            request,
            Evidence::LinuxRlimit {
                observation: Box::new(observation),
            },
        ) {
            Ok(v) => v,
            Err(error) => return Err(self.retain_uncertain(launch, child, error.to_string())),
        };
        self.effects.insert(launch.instance.clone(), launch.effect);
        self.children.insert(launch.instance.clone(), child);
        Ok(Decision::Admitted { pid, receipt })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    fn n(v: &str) -> Name {
        Name::new(v).unwrap()
    }
    fn id() -> Id {
        Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
    }
    fn fixture(root: &std::path::Path) -> (Launch, Request) {
        let executable = PathBuf::from("/usr/bin/python3");
        let marker = root.join("target-entered");
        let program = Program {
            id: n("test/resource"),
            effect: Effect::NonActuating,
            executable_sha256: Digest::from_bytes(
                Sha256::digest(std::fs::read(&executable).unwrap()).into(),
            ),
            executable,
            files: BTreeMap::new(),
            fixed_arguments: vec![
                "-c".into(),
                format!(
                    "from pathlib import Path; import time; Path({:?}).write_text('entered'); time.sleep(30)",
                    marker.to_str().unwrap()
                ),
            ],
            arguments: BTreeMap::new(),
            ready: ReadyProbe::AliveOnly,
            execution_requirements: None,
            functional_readiness: None,
            decision_policy: None,
        };
        let process = Process {
            id: n("probe"),
            program: program.id.clone(),
            parameters: BTreeMap::new(),
            depends_on: vec![],
            startup_timeout_ms: Counter(1000),
            shutdown_timeout_ms: Counter(1000),
            restart_limit: Counter(0),
            restart_backoff_ms: Counter(100),
        };
        let launch = process.launch(&program, id()).unwrap();
        let request = Request::new(
            program.id,
            process.id,
            launch.instance.clone(),
            Digest::from_bytes([1; 32]),
            crate::execution::Requirements(
                [(
                    n("usage/address-space"),
                    Requirement::UpperBound {
                        resource: Capacity::AddressSpaceBytes,
                        amount: Counter(67108864),
                    },
                )]
                .into(),
            ),
        );
        (launch, request)
    }
    struct Fault {
        mode: &'static str,
        pid: Cell<u32>,
        script: String,
    }
    impl Fault {
        fn new(mode: &'static str) -> Self {
            let script=match mode {
                "exec" => GATE.replace("os.execv(request[\"executable\"],", "os.execv(\"/absent/rx-f8-executable\","),
                "death" => "import os,sys,signal\nos.write(1,b'GATED\\n')\nsys.stdin.buffer.readline(6)\nos.kill(os.getpid(),signal.SIGKILL)\n".into(),
                _ => GATE.into(),
            };
            Self {
                mode,
                pid: Cell::new(0),
                script,
            }
        }
    }
    impl Kernel for Fault {
        fn gate(&self) -> &str {
            &self.script
        }
        fn apply(&self, pid: u32, amount: u64) -> Result<(Option<u64>, Option<u64>)> {
            self.pid.set(pid);
            let prior = LinuxKernel.apply(pid, amount)?;
            if self.mode == "permission" {
                // Actual EPERM after the child hard ceiling was lowered.
                return LinuxKernel.apply(pid, amount * 2);
            }
            Ok(prior)
        }
        fn observe(&self, pid: u32) -> Result<(u64, u64)> {
            if self.mode == "observation" {
                return Err(Error::Reconciliation(
                    "injected observation failure after application".into(),
                ));
            }
            LinuxKernel.observe(pid)
        }
    }
    fn rejected_without_target(
        mode: &'static str,
        allowed: &mut dyn FnMut() -> bool,
        expected: &str,
    ) {
        let root = tempfile::tempdir().unwrap();
        let (launch, request) = fixture(root.path());
        let mut backend = OsProcesses::new(root.path().join("logs")).unwrap();
        let kernel = Fault::new(mode);
        let result = backend.spawn_gated(
            &launch,
            &request,
            n("usage/address-space"),
            67108864,
            allowed,
            &kernel,
        );
        let Ok(Decision::Rejected { unmet }) = result else {
            panic!("must reject after confirmed rollback")
        };
        assert_eq!(unmet.len(), 1);
        assert!(unmet[0].reason.contains(expected), "{}", unmet[0].reason);
        assert!(unmet[0].reason.contains("exit confirmed"));
        assert!(backend.owned_instances().is_empty());
        assert!(!root.path().join("target-entered").exists());
        assert!(!std::path::Path::new(&format!("/proc/{}", kernel.pid.get())).exists());
        println!("{mode}: {}", unmet[0].reason);
    }
    #[test]
    fn actual_kernel_permission_failure_after_lowering_rolls_back_before_target() {
        rejected_without_target("permission", &mut || true, "kernel application failed");
    }
    #[test]
    fn observation_failure_after_application_confirms_no_remaining_child_policy() {
        rejected_without_target("observation", &mut || true, "observation did not confirm");
    }
    #[test]
    fn authority_change_after_application_prevents_exec_and_confirms_rollback() {
        let mut calls = 0;
        rejected_without_target(
            "normal",
            &mut || {
                calls += 1;
                calls == 1
            },
            "authority changed after application",
        );
        assert_eq!(calls, 2);
    }
    #[test]
    fn exec_syscall_failure_is_not_an_admission() {
        rejected_without_target("exec", &mut || true, "EXEC_ERROR:2");
    }
    #[test]
    fn gate_death_in_exec_window_cannot_mint_a_receipt_from_eof() {
        let root = tempfile::tempdir().unwrap();
        let (launch, request) = fixture(root.path());
        let mut backend = OsProcesses::new(root.path().join("logs")).unwrap();
        let kernel = Fault::new("death");
        let result = backend.spawn_gated(
            &launch,
            &request,
            n("usage/address-space"),
            67108864,
            &mut || true,
            &kernel,
        );
        assert!(matches!(result, Err(SpawnFailure::Uncertain(_))));
        assert!(!root.path().join("target-entered").exists());
        assert_eq!(backend.owned_instances(), vec![launch.instance.clone()]);
        // EOF and an unconfirmed exec do not promise that waitpid already has
        // an exit result. Retain the owner until the exit is actually observed.
        let deadline = Instant::now() + Duration::from_secs(5);
        while backend.exited(&launch.instance).unwrap().is_none() {
            assert!(
                Instant::now() < deadline,
                "owned gate exit was not observed"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        backend.forget_exited(&launch.instance).unwrap();
    }
}
