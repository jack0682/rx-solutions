use rx_domain::types::*;
use rx_solution_catalog::DeviceCatalog;
use rx_storage::SqliteRepository;
use rx_supervisor::{Supervisor, execution::*, model::*, process::OsProcesses};
use std::{collections::BTreeMap, path::Path};

fn n(value: &str) -> Name {
    Name::new(value).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn catalog() -> DeviceCatalog {
    DeviceCatalog::decode(include_bytes!("../../../catalogs/device-support.v1.json")).unwrap()
}
fn fixture(code: &str) -> (Plan, Program) {
    use sha2::{Digest as _, Sha256};
    let executable = Path::new("/usr/bin/python3");
    let hash = std::fs::read(executable)
        .map(|bytes| Digest::from_bytes(Sha256::digest(bytes).into()))
        .unwrap_or(Digest::from_bytes([0; 32]));
    let program = Program {
        id: n("test/address-space"),
        effect: Effect::NonActuating,
        executable: executable.into(),
        executable_sha256: hash,
        files: BTreeMap::new(),
        fixed_arguments: vec!["-u".into(), "-c".into(), code.into()],
        arguments: BTreeMap::new(),
        ready: ReadyProbe::AliveOnly,
        execution_requirements: Some(Requirements(
            [(
                n("usage/address-space"),
                Requirement::UpperBound {
                    resource: Capacity::AddressSpaceBytes,
                    amount: Counter(67108864),
                },
            )]
            .into(),
        )),
        functional_readiness: None,
        decision_policy: None,
    };
    let plan = Plan {
        schema: n("rx.solutions-process-plan.v1"),
        id: id(),
        environment: Environment::Simulation,
        profiles: vec![],
        processes: vec![Process {
            id: n("probe"),
            program: program.id.clone(),
            parameters: BTreeMap::new(),
            depends_on: vec![],
            startup_timeout_ms: Counter(1000),
            shutdown_timeout_ms: Counter(1000),
            restart_limit: Counter(0),
            restart_backoff_ms: Counter(100),
        }],
    };
    (plan, program)
}
fn open(
    root: &Path,
    plan: Plan,
    program: Program,
) -> Supervisor<SqliteRepository, OsProcesses, SoftwareOnly> {
    Supervisor::open(
        SqliteRepository::open(root.join("supervisor.db")).unwrap(),
        OsProcesses::new(root.join("logs")).unwrap(),
        SoftwareOnly,
        plan,
        [(program.id.clone(), program)].into(),
        &catalog(),
    )
    .unwrap()
}
#[test]
fn unsupported_reserved_capacity_rejects_the_entire_bundle() {
    #[cfg(target_os = "linux")]
    let _serial = linux::SERIAL.lock().unwrap();
    let root = tempfile::tempdir().unwrap();
    let (plan, mut program) = fixture("raise SystemExit('must never execute')");
    program.execution_requirements.as_mut().unwrap().0.insert(
        n("capacity/memory"),
        Requirement::ReservedCapacity {
            resource: Capacity::MemoryBytes,
            amount: Counter(1024),
        },
    );
    let mut s = open(root.path(), plan, program);
    let report = s.tick().unwrap();
    let status = &report.execution_admission[&n("probe")];
    assert!(matches!(status.application, Application::NotApplied));
    assert_eq!(status.not_applied_reasons.len(), 2);
    println!(
        "unsupported_bundle={}",
        serde_json::to_string(status).unwrap()
    );
    assert!(
        status
            .not_applied_reasons
            .iter()
            .all(|r| r.reason.contains("unsupported"))
    );
    assert_eq!(report.state.records[&n("probe")].phase, Phase::StartFailed);
    assert!(report.state.records[&n("probe")].pid.is_none());
    let (_, backend, _) = s.into_parts();
    assert!(backend.owned_instances().is_empty());
}

#[cfg(not(target_os = "linux"))]
#[test]
fn address_space_is_rejected_on_non_linux_without_an_ignore_escape() {
    let root = tempfile::tempdir().unwrap();
    let (plan, program) = fixture("raise SystemExit('never execute')");
    let mut s = open(root.path(), plan, program);
    let report = s.tick().unwrap();
    let status = &report.execution_admission[&n("probe")];
    assert!(matches!(status.application, Application::NotApplied));
    assert_eq!(status.not_applied_reasons.len(), 1);
    assert!(
        status.not_applied_reasons[0]
            .reason
            .contains("unsupported on this platform")
    );
    assert_eq!(report.state.records[&n("probe")].phase, Phase::StartFailed);
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use rx_supervisor::process::Backend;
    use std::{
        process::Command,
        time::{Duration, Instant},
    };
    const RUN: &str = "import time; time.sleep(30)";
    // Reopen/ownership probes share a test process. Concurrent fork/exec can
    // temporarily inherit another fixture's writer-lock descriptor; isolate
    // these probes instead of adding retries to the product's lock boundary.
    pub(super) static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    fn stop(s: &mut Supervisor<SqliteRepository, OsProcesses, SoftwareOnly>) {
        s.request_stop().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !s.tick().unwrap().all_exited {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    fn actual_rx_child_limit_is_observed_by_a_separate_process_and_binds_allocations() {
        let _serial = SERIAL.lock().unwrap();
        let root = tempfile::tempdir().unwrap();
        let code = "import mmap,time\ntry:\n m=mmap.mmap(-1,134217728); print('UNEXPECTED_ALLOCATION')\nexcept OSError as e:\n print('ALLOCATION_ERRNO='+str(e.errno),flush=True)\ntime.sleep(30)";
        let (plan, program) = fixture(code);
        let mut s = open(root.path(), plan, program);
        let report = s.tick().unwrap();
        let record = &report.state.records[&n("probe")];
        assert_eq!(record.phase, Phase::Starting, "{:?}", report.blocked);
        let pid = record.pid.unwrap();
        let instance = record.instance.as_ref().unwrap();
        let evidence = serde_json::to_value(&report.execution_admission[&n("probe")]).unwrap();
        assert_eq!(
            evidence["application"]["receipt"]["evidence"]["basis"],
            "LINUX_RLIMIT"
        );
        let observed = Command::new("/usr/bin/python3").args(["-c",
            "import os,sys,json; print(json.dumps({'observer_pid':os.getpid(),'parent_pid':os.getppid(),'limits':open('/proc/'+sys.argv[1]+'/limits').read(),'own_limits':open('/proc/self/limits').read()}))",
            &pid.to_string()]).output().unwrap();
        assert!(observed.status.success());
        let value: serde_json::Value = serde_json::from_slice(&observed.stdout).unwrap();
        assert_ne!(value["observer_pid"], std::process::id());
        assert_eq!(value["parent_pid"], std::process::id());
        let row = value["limits"]
            .as_str()
            .unwrap()
            .lines()
            .find(|v| v.starts_with("Max address space"))
            .unwrap();
        assert_eq!(
            row.split_whitespace().skip(3).collect::<Vec<_>>(),
            vec!["67108864", "67108864", "bytes"]
        );
        println!("outside_supervisor_observation={value}");
        let path = root
            .path()
            .join("logs")
            .join(format!("{instance}.stdout.log"));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let output = std::fs::read_to_string(&path).unwrap();
            assert!(!output.contains("UNEXPECTED_ALLOCATION"));
            if output.contains("ALLOCATION_ERRNO=12") {
                println!("RX child allocation result: {output}");
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        stop(&mut s);
        let state = s.state().unwrap();
        let resources = state.records[&n("probe")].resources.as_ref().unwrap();
        assert_eq!(
            resources.lifetime,
            ResourceLifetime::DirectChildExitedDescendantsUnassessed
        );
        assert_eq!(resources.capacity_reservation, "NONE_CREATED");
        assert_eq!(resources.physical_handover, "NOT_ASSESSED");
        assert!(!Path::new(&format!("/proc/{pid}")).exists());
    }

    #[test]
    fn saved_limit_history_never_restores_a_current_receipt_after_owner_loss() {
        let _serial = SERIAL.lock().unwrap();
        let root = tempfile::tempdir().unwrap();
        let (plan, program) = fixture(RUN);
        let mut s = open(root.path(), plan.clone(), program.clone());
        let started = s.tick().unwrap();
        let old = started.state.records[&n("probe")]
            .resources
            .clone()
            .unwrap();
        assert!(old.last_observed.is_some());
        let instance = started.state.records[&n("probe")].instance.clone().unwrap();
        let (store, mut owner, _) = s.into_parts();
        drop(store);
        let mut next = open(root.path(), plan, program);
        let report = next.tick().unwrap();
        assert_eq!(report.state.records[&n("probe")].phase, Phase::Unknown);
        assert!(matches!(
            report.execution_admission[&n("probe")].application,
            Application::Unconfirmed { .. }
        ));
        let history = report.state.records[&n("probe")]
            .resources
            .as_ref()
            .unwrap();
        assert_eq!(history.last_observed, old.last_observed);
        assert_eq!(history.lifetime, ResourceLifetime::Unconfirmed);
        assert_eq!(history.current_enforcement, "NOT_ESTABLISHED_BY_HISTORY");
        // Cleanup is by the retained original Child owner, never by the new
        // supervisor adopting a persisted PID.
        owner.terminate(&instance, true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while owner.exited(&instance).unwrap().is_none() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        owner.forget_exited(&instance).unwrap();
    }

    #[test]
    fn executable_integrity_failure_is_a_named_no_application_rejection() {
        let _serial = SERIAL.lock().unwrap();
        let root = tempfile::tempdir().unwrap();
        let (plan, mut program) = fixture(RUN);
        program.executable_sha256 = Digest::from_bytes([0; 32]);
        let mut s = open(root.path(), plan, program);
        let report = s.tick().unwrap();
        assert_eq!(report.state.records[&n("probe")].phase, Phase::StartFailed);
        assert!(
            report.execution_admission[&n("probe")].not_applied_reasons[0]
                .reason
                .contains("launch preparation failed")
        );
        assert!(s.into_parts().1.owned_instances().is_empty());
    }
}
