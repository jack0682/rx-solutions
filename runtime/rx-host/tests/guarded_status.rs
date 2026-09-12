#![cfg(target_os = "linux")]

use rx_domain::{
    intent::Intent,
    types::{Id, Name, TimePoint},
};
use rx_host::service::{self, config::Loaded};
use rx_host::{
    Binding, Environment, Guard, NativeAdapter, NativeCapture,
    native::{LocalProtection, NativeShutdown},
    simulation::{FileDevice, ManualClock},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use std::{
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};
#[allow(dead_code)]
#[path = "support/host_service_fixture.rs"]
mod fixture;

fn bounded_output(mut command: Command) -> Output {
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            // Test-only process cleanup is never a service stop acceptance assertion.
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!("invalid-env Host fixture exceeded deadline: {output:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn invalid_guarded_writer_fails_before_passive_native_open_or_rpc_admission() {
    let (root, path, clock) = fixture::fixture();
    let loaded = Loaded::read(&path).unwrap();
    service::initialize_with(&loaded, clock, &service::Builtin).unwrap();
    assert!(!loaded.config.data_directory.join("device").exists());
    for with_instance in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rx-hostd"));
        command
            .arg("run")
            .arg(&path)
            .env(
                "RX_PROCESS_STATUS_PATH",
                "relative-status-is-forbidden.json",
            )
            .env_remove("RX_PROCESS_INSTANCE_ID")
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if with_instance {
            command.env("RX_PROCESS_INSTANCE_ID", fixture::id().as_str());
        }
        let output = bounded_output(command);
        assert!(!output.status.success(), "{output:?}");
        assert!(
            !loaded.config.data_directory.join("device").exists(),
            "native opened before invalid status was rejected"
        );
        assert!(!root.path().join("runtime/host-status.json").exists());
    }
}

struct Held {
    native: FileDevice,
    safe: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
}
impl Drop for Held {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}
impl NativeAdapter for Held {
    fn environment(&self) -> Environment {
        Environment::Simulation
    }
    fn protection(&self) -> Arc<dyn LocalProtection> {
        self.native.protection()
    }
    fn guard(&self, intent: &Intent, now: &TimePoint) -> rx_host::Result<Guard> {
        self.native.guard(intent, now)
    }
    fn can_handover(&self, resources: &[Name]) -> bool {
        self.native.can_handover(resources)
    }
    fn shutdown_snapshot(&self, resources: &[Name]) -> rx_host::Result<NativeShutdown> {
        let mut snapshot = self.native.shutdown_snapshot(resources)?;
        snapshot.safe_to_drop = self.safe.load(Ordering::SeqCst);
        Ok(snapshot)
    }
    fn submit(
        &mut self,
        operation: &Id,
        invocation: &Id,
        intent: &Intent,
    ) -> rx_host::Result<NativeCapture> {
        self.native.submit(operation, invocation, intent)
    }
    fn lookup(
        &mut self,
        operation: &Id,
        invocation: &Id,
    ) -> rx_host::Result<Option<NativeCapture>> {
        self.native.lookup(operation, invocation)
    }
}
struct HeldFactory {
    safe: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
    status: PathBuf,
    backup: PathBuf,
    wait_for_heartbeat: bool,
}
impl service::AdapterFactory<ManualClock> for HeldFactory {
    type Adapter = Held;
    fn validate(
        &self,
        backend: &service::config::Backend,
        bindings: &[Binding],
    ) -> service::Result<()> {
        <service::Builtin as service::AdapterFactory<ManualClock>>::validate(
            &service::Builtin,
            backend,
            bindings,
        )
    }
    fn open_passive(
        &self,
        _: &service::config::Backend,
        path: &Path,
        clock: ManualClock,
    ) -> service::Result<Held> {
        let native = FileDevice::open(path.join("device"), clock)?;
        let starting: serde_json::Value = serde_json::from_slice(&std::fs::read(&self.status)?)?;
        assert_eq!(starting["state"]["kind"], "STARTING");
        // Keep the last committed bytes for restoration, but make every subsequent
        // report fail while a real passive adapter is already owned by this call.
        std::fs::rename(&self.status, &self.backup)?;
        std::fs::create_dir(&self.status)?;
        if self.wait_for_heartbeat {
            // This fixture runs with two workers so the independent 200ms reporter
            // fails before open_passive returns and Host installs its callback target.
            std::thread::sleep(Duration::from_millis(500));
        }
        Ok(Held {
            native,
            safe: self.safe.clone(),
            dropped: self.dropped.clone(),
        })
    }
}

#[test]
fn reporter_failure_after_native_open_waits_for_explicit_adapter_drop_permission() {
    for mode in ["ready-write", "heartbeat-before-host"] {
        let root = tempfile::tempdir().unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "held_adapter_startup_status_failure_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("RX_TEST_GUARDED_FAILURE_MODE", mode)
            .env("RX_PROCESS_STATUS_PATH", root.path().join("guarded.json"))
            .env("RX_PROCESS_INSTANCE_ID", fixture::id().as_str())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = bounded_output(command);
        assert!(output.status.success(), "{mode}: {output:?}");
    }
}

#[test]
#[ignore = "isolated environment fixture invoked by the bounded parent test"]
fn held_adapter_startup_status_failure_fixture() {
    let mode = std::env::var("RX_TEST_GUARDED_FAILURE_MODE").unwrap();
    assert!(matches!(
        mode.as_str(),
        "ready-write" | "heartbeat-before-host"
    ));
    // Ready-write uses one worker: after open_passive returns there is no pending
    // await until the synchronous Ready publication fails. The second case lets
    // the independent heartbeat fail while open_passive is still blocked.
    let mut runtime = if mode == "ready-write" {
        tokio::runtime::Builder::new_current_thread()
    } else {
        let mut runtime = tokio::runtime::Builder::new_multi_thread();
        runtime.worker_threads(2);
        runtime
    };
    runtime
        .enable_all()
        .build()
        .unwrap()
        .block_on(held_adapter_failure_case(mode));
}

async fn held_adapter_failure_case(mode: String) {
    let (_root, path, clock) = fixture::fixture();
    let loaded = Loaded::read(&path).unwrap();
    let legacy_status = loaded.config.runtime_directory.join("host-status.json");
    let guarded_status = PathBuf::from(std::env::var_os("RX_PROCESS_STATUS_PATH").unwrap());
    let backup = guarded_status.with_extension("last-good");
    let safe = Arc::new(AtomicBool::new(false));
    let dropped = Arc::new(AtomicBool::new(false));
    let factory = HeldFactory {
        safe: safe.clone(),
        dropped: dropped.clone(),
        status: guarded_status.clone(),
        backup: backup.clone(),
        wait_for_heartbeat: mode == "heartbeat-before-host",
    };
    service::initialize_with(&loaded, clock.clone(), &factory).unwrap();
    let task = tokio::spawn(service::run_with(
        loaded,
        clock,
        factory,
        std::future::pending(),
    ));
    let waiting = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            assert!(
                !task.is_finished(),
                "service returned before adapter drop permission"
            );
            assert!(
                !dropped.load(Ordering::SeqCst),
                "adapter dropped with safe_to_drop=false"
            );
            if let Ok(bytes) = std::fs::read(&legacy_status) {
                let status: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                if status["phase"] == "STOP_WAITING_FOR_ADAPTER" {
                    break status;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(waiting["admission_open"], false);
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(!task.is_finished());
    assert!(!dropped.load(Ordering::SeqCst));
    // A later successful write must preserve the failure, not change this to STOPPED.
    std::fs::remove_dir(&guarded_status).unwrap();
    std::fs::rename(&backup, &guarded_status).unwrap();
    safe.store(true, Ordering::SeqCst);
    let result = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err());
    assert!(dropped.load(Ordering::SeqCst));
    let final_status: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&guarded_status).unwrap()).unwrap();
    assert_eq!(final_status["state"]["kind"], "ATTENTION");
    let final_legacy: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&legacy_status).unwrap()).unwrap();
    assert_eq!(final_legacy["phase"], "STOPPED_WITH_SERVICE_ERROR");
    assert_eq!(final_legacy["stop"]["safe_to_drop"], true);
}
