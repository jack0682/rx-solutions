//! Opt-in real release-service passage. The driver builds this test from current
//! sources and runs it in a validated runtime image, never in a fixture pretending
//! to contain a qualified native installation.
#![cfg(unix)]
use rx_domain::types::*;
use rx_solution_catalog::DeviceCatalog;
use rx_storage::SqliteRepository;
use rx_supervisor::{
    Result,
    builtin::release_programs,
    execution::Application,
    model::*,
    process::{Backend, OsProcesses, SpawnFailure},
    registered::{RegisteredSupervisor, catalog_reference},
    registration::*,
    use_assessment::{
        ConditionState, NoWorkUseProvider, ReportOrigin, StatusObservationRequest,
        StatusObservationResult, UseScope, WorkUseState,
    },
};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    rc::Rc,
    time::{Duration, Instant},
};
#[path = "support/dependency_passage.rs"]
mod dependency_passage;

fn n(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn uid() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn now() -> TimePoint {
    TimePoint {
        clock_id: "unix-utc-ns".into(),
        ticks_ns: Counter(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64,
        ),
    }
}
struct PassageRecoveryPolicy {
    component: Id,
}
impl RecoveryAuthority for PassageRecoveryPolicy {
    fn may_dispose(&self, request: &DispositionRequest) -> bool {
        request.target.registration == self.component
            && request.kind == DispositionKind::ConfirmedClosure
            && matches!(&request.evidence, RecoveryEvidence::OwnedChildExit{report,..} if report.actor==n("passage/owned-process-operator"))
    }
    fn may_resume(&self, disposition: &Disposition, request: &ResumeRequest) -> bool {
        disposition.target.registration == self.component
            && request.actor == n("passage/owned-process-operator")
    }
}
#[derive(Clone)]
struct Owned(Rc<RefCell<OsProcesses>>);
impl Owned {
    fn new(path: PathBuf) -> Self {
        Self(Rc::new(RefCell::new(OsProcesses::new(path).unwrap())))
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        if Rc::strong_count(&self.0) != 1 {
            return;
        }
        let mut b = self.0.borrow_mut();
        for id in b.owned_instances() {
            let _ = b.terminate(&id, true);
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                if matches!(b.exited(&id), Ok(Some(_))) {
                    let _ = b.forget_exited(&id);
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}
impl Backend for Owned {
    fn observe_status(
        &mut self,
        launch: &Launch,
        request: &StatusObservationRequest,
    ) -> Result<StatusObservationResult> {
        self.0.borrow_mut().observe_status(launch, request)
    }
    fn spawn(
        &mut self,
        l: &Launch,
        a: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<u32, SpawnFailure> {
        self.0.borrow_mut().spawn(l, a)
    }
    fn pid(&self, i: &Id) -> Option<u32> {
        self.0.borrow().pid(i)
    }
    fn owns(&self, i: &Id) -> bool {
        self.0.borrow().owns(i)
    }
    fn forget_exited(&mut self, i: &Id) -> Result<()> {
        self.0.borrow_mut().forget_exited(i)
    }
    fn exited(&mut self, i: &Id) -> Result<Option<Option<i32>>> {
        self.0.borrow_mut().exited(i)
    }
    fn ready(&mut self, l: &Launch) -> Result<bool> {
        self.0.borrow_mut().ready(l)
    }
    fn terminate(&mut self, i: &Id, f: bool) -> Result<()> {
        self.0.borrow_mut().terminate(i, f)
    }
}
fn plan() -> Plan {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    Plan {
        schema: n("rx.solutions-process-plan.v1"),
        id: uid(),
        environment: Environment::Simulation,
        profiles: vec![],
        processes: vec![Process {
            id: n("status"),
            program: n("rx/status-http"),
            parameters: [
                (n("bind"), "127.0.0.1".into()),
                (n("port"), port.to_string()),
            ]
            .into(),
            depends_on: vec![],
            startup_timeout_ms: Counter(10000),
            shutdown_timeout_ms: Counter(3000),
            restart_limit: Counter(0),
            restart_backoff_ms: Counter(100),
        }],
    }
}
fn emit(stage: &str, basis: &str, data: Value) {
    println!(
        "{}",
        json!({"stage":stage,"manager_pid":std::process::id(),"basis":basis,"data":data})
    );
}
fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}
type Managed = RegisteredSupervisor<SqliteRepository, Owned, SoftwareOnly, SqliteRepository>;
fn open(data: &Path, config: &Value, kind: &str) -> (Managed, Owned) {
    let root = Path::new("/opt/rx");
    let programs = release_programs(root).unwrap();
    let p: Plan = serde_json::from_value(config["plan"].clone()).unwrap();
    let id: Id = serde_json::from_value(config["component"].clone()).unwrap();
    let registry = Registry::new(SqliteRepository::open(data.join("registration.db")).unwrap());
    let b = Owned::new(data.join(format!("{kind}-logs")));
    let support = DeviceCatalog::decode(
        &std::fs::read(root.join("catalogs/device-support.v1.json")).unwrap(),
    )
    .unwrap();
    let s = RegisteredSupervisor::open(
        SqliteRepository::open(data.join(format!("{kind}.db"))).unwrap(),
        b.clone(),
        SoftwareOnly,
        p,
        programs,
        &support,
        registry,
        id,
    )
    .unwrap();
    (s, b)
}
fn ready(s: &mut Managed) -> Status {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let report = s.tick().unwrap();
        if report.state.records[&n("status")].phase == Phase::ProcessReady {
            return report;
        }
        assert!(Instant::now() < deadline, "{report:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn stopped(s: &mut Managed) -> Status {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let report = s.tick().unwrap();
        if report.all_exited {
            return report;
        }
        assert!(Instant::now() < deadline, "{report:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "worker invoked only by the real registration passage"]
fn passage_worker() {
    let data = PathBuf::from(std::env::var("RX_PASSAGE_DATA").unwrap());
    let stage = std::env::var("RX_PASSAGE_STAGE").unwrap();
    if stage == "dependencies" {
        dependency_passage::run(&data);
        return;
    }
    if stage == "register" {
        let programs = release_programs(Path::new("/opt/rx")).unwrap();
        let mut r = Registry::new(SqliteRepository::open(data.join("registration.db")).unwrap());
        let registered = r
            .register(Declaration {
                label: n("release-status"),
                catalog: catalog_reference(&programs[&n("rx/status-http")]).unwrap(),
            })
            .unwrap();
        let view = r.query(&registered.registration.id).unwrap();
        assert!(view.executions.is_empty());
        let config = json!({"component":registered.registration.id,"declaration":registered.registration.declaration,"plan":plan()});
        std::fs::write(
            data.join("config.json"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        emit(
            "registered",
            "atomic registration commit; zero executions; no supervisor or Cell required",
            json!(view),
        );
        return;
    }
    let config = read_json(&data.join("config.json"));
    if stage == "normal" {
        let (mut s, b) = open(&data, &config, "normal");
        let report = ready(&mut s);
        let record = &report.state.records[&n("status")];
        let admission = &report.execution_admission[&n("status")];
        assert!(matches!(
            admission.application,
            Application::ReportedAtStart { .. }
        ));
        let receipt = json!(admission);
        assert_eq!(
            receipt["application"]["receipt"]["evidence"]["basis"],
            "NO_REQUIREMENTS"
        );
        assert_ne!(config["component"], json!(record.instance));
        let p: Plan = serde_json::from_value(config["plan"].clone()).unwrap();
        let port = &p.processes[0].parameters[&n("port")];
        let mut stream = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream.write_all(b"GET /health HTTP/1.0\r\n\r\n").unwrap();
        let mut response = String::new();
        stream.take(65536).read_to_string(&mut response).unwrap();
        let health: Value =
            serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(health["supervisor_instance"], json!(record.instance));
        assert_eq!(health["hardware_processes_started_by_entrypoint"], 0);
        assert_eq!(health["physical_qualification"], "NOT_PERFORMED");
        emit(
            "admitted-and-observed",
            "full F1 NoRequirements receipt plus instance-correlated HTTP report; no resource enforcement or work-use permission",
            json!({"registration":s.query().unwrap(),"lifecycle":report,"health":health}),
        );
        let before_history = s.history().unwrap();
        let not_ready = s
            .assess_use(
                UseScope {
                    operating_area: n("passage/diagnostics"),
                    role: n("diagnostics/operator-connected"),
                },
                &NoWorkUseProvider,
            )
            .unwrap();
        assert_eq!(
            s.state().unwrap().records[&n("status")].phase,
            Phase::ProcessReady
        );
        assert_eq!(
            not_ready.functional_readiness.state(),
            ConditionState::NotMet
        );
        let delegation = not_ready
            .functional_readiness
            .conditions()
            .iter()
            .find(|c| c.name == n("operator/api-delegation"))
            .unwrap();
        assert_eq!(delegation.origin, ReportOrigin::ReleaseDeclaration);
        assert_eq!(delegation.observed, Some(json!("NOT_CONNECTED")));
        assert_eq!(
            not_ready.work_use_permission.state(),
            WorkUseState::Unsupported
        );
        emit(
            "process-ready-but-use-conditions-not-met",
            "current instance is ProcessReady, but the component reports the static release declaration NOT_CONNECTED; this is not a failed live operator-connectivity probe",
            json!(not_ready),
        );
        let reported_ready = s
            .assess_use(
                UseScope {
                    operating_area: n("passage/diagnostics"),
                    role: n("diagnostics/support-summary"),
                },
                &NoWorkUseProvider,
            )
            .unwrap();
        assert_eq!(
            reported_ready.functional_readiness.state(),
            ConditionState::Satisfied
        );
        assert!(
            reported_ready
                .functional_readiness
                .conditions()
                .iter()
                .any(|c| c.name == n("report/current-instance")
                    && c.state == ConditionState::Satisfied)
        );
        assert_eq!(
            reported_ready.work_use_permission.state(),
            WorkUseState::Unsupported
        );
        assert!(
            reported_ready
                .work_use_permission
                .conditions()
                .iter()
                .any(|c| c.name == n("work-use/operating-area-provider"))
        );
        assert_eq!(s.history().unwrap(), before_history);
        assert_eq!(
            s.query().unwrap().functional_readiness.state(),
            ConditionState::NotEvaluated
        );
        emit(
            "reported-conditions-met-but-work-use-unavailable",
            "readonly support-summary report matches authored conditions; counts are startup audit/catalog derived, instance is environment derived; positive operating-area provider is unconnected, not an authority-issued denial or grant",
            json!(reported_ready),
        );
        s.request_stop().unwrap();
        let report = stopped(&mut s);
        assert!(b.0.borrow().owned_instances().is_empty());
        emit(
            "terminated",
            "owned child exit observed, committed and reaped; registration and history retained",
            json!({"registration":s.query().unwrap(),"lifecycle":report,"history":s.history().unwrap()}),
        );
        return;
    }
    if stage == "reopen" {
        let uid: Id = serde_json::from_value(config["component"].clone()).unwrap();
        let mut registry =
            Registry::new(SqliteRepository::open(data.join("registration.db")).unwrap());
        let view = registry.query(&uid).unwrap();
        assert_eq!(
            json!(view.registration.registration.declaration),
            config["declaration"]
        );
        assert_eq!(view.executions.len(), 1);
        assert_eq!(
            view.executions[0].last_observed.state,
            ExecutionState::Exited
        );
        drop(registry);
        let (mut s, b) = open(&data, &config, "normal");
        let report = s.tick().unwrap();
        assert!(report.all_exited);
        assert!(b.0.borrow().owned_instances().is_empty());
        assert!(matches!(
            report.execution_admission[&n("status")].application,
            Application::Unconfirmed { .. }
        ));
        emit(
            "reopened-in-new-manager-process",
            "same registration/declaration and exit history read from SQLite; no PID adoption, replay or recovered resource receipt",
            json!({"registration":s.query().unwrap(),"lifecycle":report}),
        );
        return;
    }
    if stage == "loss" {
        let mut config = config;
        config["plan"] = json!(plan());
        std::fs::write(
            data.join("loss-config.json"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        let (mut s, mut b) = open(&data, &config, "loss");
        let report = ready(&mut s);
        let instance = report.state.records[&n("status")].instance.clone().unwrap();
        assert!(b.owns(&instance));
        // Kill only the currently owned disposable diagnostic child, never a saved PID.
        b.terminate(&instance, true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let report = s.tick().unwrap();
            if report.state.records[&n("status")].phase == Phase::Exited {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(20));
        }
        let view = s.query().unwrap();
        assert_eq!(
            view.registration.registration.state,
            RegistrationState::Accepted
        );
        assert_eq!(view.executions.len(), 2);
        assert!(b.0.borrow().owned_instances().is_empty());
        emit(
            "abnormal-child-exit",
            "owned child forcibly exited for this software-only test; nonzero/signal exit observed, registration retained",
            json!({"registration":view,"state":s.state().unwrap()}),
        );
        return;
    }
    if stage == "manager-reopen" {
        let mut config = config;
        config["plan"] = json!(plan());
        let (mut s, b) = open(&data, &config, "restart");
        let report = ready(&mut s);
        let instance = report.state.records[&n("status")].instance.clone().unwrap();
        let (store, mut old_backend, _, registry) = s.into_parts();
        drop(store);
        drop(registry);
        // Keep the old real child handle solely to clean up the test. The new
        // supervisor receives a fresh backend and cannot adopt that process.
        let (mut reopened, fresh) = open(&data, &config, "restart");
        let report = reopened.tick().unwrap();
        assert_eq!(report.state.records[&n("status")].phase, Phase::Unknown);
        assert!(fresh.0.borrow().owned_instances().is_empty());
        assert!(b.owns(&instance));
        emit(
            "lost-manager-ownership",
            "manager object and both stores reopened with fresh backend while original real child still exists; UNKNOWN, no saved-PID adoption",
            json!({"registration":reopened.query().unwrap(),"lifecycle":report}),
        );
        let component: Id = serde_json::from_value(config["component"].clone()).unwrap();
        let original = reopened
            .query()
            .unwrap()
            .executions
            .into_iter()
            .find(|e| e.binding.instance == instance)
            .unwrap();
        let (store, new_backend, _, mut registry) = reopened.into_parts();
        drop(store);
        drop(new_backend);
        let history = registry.history(&component).unwrap();
        let target = registry.recovery_target(&component, &instance).unwrap();
        assert!(
            old_backend
                .0
                .borrow_mut()
                .observe_recovery_exit(&instance)
                .is_err(),
            "live child cannot produce exit evidence"
        );
        old_backend.terminate(&instance, false).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !matches!(old_backend.exited(&instance), Ok(Some(_))) {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(20));
        }
        let witness = old_backend
            .0
            .borrow_mut()
            .observe_recovery_exit(&instance)
            .unwrap();
        assert!(old_backend.0.borrow().owned_instances().is_empty());
        let request = DispositionRequest {
            target,
            kind: DispositionKind::ConfirmedClosure,
            evidence: RecoveryEvidence::OwnedChildExit {
                report: RecoveryReport {
                    actor: n("passage/owned-process-operator"),
                    scope: n("host/direct-child-exit"),
                    observed_at: witness.observed_at().clone(),
                    procedure: n("owned-child-cooperative-stop-exit-and-retirement"),
                },
                witness,
            },
        };
        assert!(registry.dispose(&request, &DenyRecovery).is_err());
        let policy = PassageRecoveryPolicy {
            component: component.clone(),
        };
        let disposition = registry.dispose(&request, &policy).unwrap();
        let view = registry.query(&component).unwrap();
        assert_eq!(
            *view
                .executions
                .iter()
                .find(|e| e.binding.instance == instance)
                .unwrap(),
            original
        );
        assert_eq!(
            registry.history(&component).unwrap()[..history.len()],
            history
        );
        emit(
            "explicit-disposition",
            "old live owner observed actual direct child exit and retired its handle; default authority rejected; scoped policy accepted a separate record; original UNKNOWN is unchanged",
            json!({"registration":view,"disposition":disposition}),
        );

        let programs = release_programs(Path::new("/opt/rx")).unwrap();
        let support = DeviceCatalog::decode(
            &std::fs::read("/opt/rx/catalogs/device-support.v1.json").unwrap(),
        )
        .unwrap();
        let denied_backend = Owned::new(data.join("no-resume-logs"));
        let mut denied = RegisteredSupervisor::open(
            SqliteRepository::open(data.join("no-resume.db")).unwrap(),
            denied_backend.clone(),
            SoftwareOnly,
            plan(),
            programs.clone(),
            &support,
            registry,
            component.clone(),
        )
        .unwrap();
        let denied_report = denied.tick().unwrap();
        assert_eq!(
            denied_report.state.records[&n("status")].phase,
            Phase::StartFailed
        );
        assert!(denied_backend.0.borrow().owned_instances().is_empty());
        emit(
            "disposition-alone-rejected",
            "confirmed closure does not automatically authorize a new assignment",
            json!(denied_report),
        );
        let (store, backend, _, mut registry) = denied.into_parts();
        drop(store);
        drop(backend);
        let next_plan = plan();
        let resume_request = ResumeRequest {
            id: uid(),
            disposition: disposition.id.clone(),
            actor: n("passage/owned-process-operator"),
            requested_at: now(),
            next_run: next_plan.id.clone(),
        };
        let permit = registry
            .request_resume(&component, resume_request.clone(), &policy)
            .unwrap();
        let replay = permit.clone();
        let next_backend = Owned::new(data.join("resumed-logs"));
        let mut resumed = RegisteredSupervisor::open_with_resume(
            SqliteRepository::open(data.join("resumed.db")).unwrap(),
            next_backend.clone(),
            SoftwareOnly,
            next_plan.clone(),
            programs.clone(),
            &support,
            registry,
            component.clone(),
            permit,
        )
        .unwrap();
        assert!(matches!(
            resumed.execution_admission().unwrap()[&n("status")].application,
            Application::NotApplied
        ));
        let report = ready(&mut resumed);
        let next_instance = report.state.records[&n("status")].instance.clone().unwrap();
        assert_ne!(instance, next_instance);
        assert_eq!(
            json!(report.execution_admission[&n("status")])["application"]["receipt"]["request"]["instance"],
            json!(next_instance)
        );
        let view = resumed.query().unwrap();
        assert_eq!(
            *view
                .executions
                .iter()
                .find(|e| e.binding.instance == instance)
                .unwrap(),
            original
        );
        assert_eq!(
            view.recovery.resume_requests[0].consumed_by,
            Some(next_instance)
        );
        emit(
            "explicitly-resumed-new-instance",
            "one explicit request consumed atomically with new assignment; new run/instance, fresh lifecycle authority and F1 receipt; original outcome remains unresolved",
            json!({"registration":view,"lifecycle":report}),
        );
        resumed.request_stop().unwrap();
        let stopped_report = stopped(&mut resumed);
        assert!(next_backend.0.borrow().owned_instances().is_empty());
        emit(
            "resumed-child-terminated",
            "new owned child exit confirmed without rewriting the old UNKNOWN",
            json!({"registration":resumed.query().unwrap(),"lifecycle":stopped_report}),
        );
        let (store, backend, _, mut registry) = resumed.into_parts();
        drop(store);
        drop(backend);
        let error = registry
            .request_resume(&component, resume_request, &policy)
            .unwrap_err();
        emit(
            "second-resume-request-rejected",
            "confirmed disposition's explicit resume has already been consumed",
            json!({"error":error.to_string()}),
        );
        let replay_backend = Owned::new(data.join("replay-logs"));
        let mut replayed = RegisteredSupervisor::open_with_resume(
            SqliteRepository::open(data.join("replay.db")).unwrap(),
            replay_backend.clone(),
            SoftwareOnly,
            next_plan,
            programs,
            &support,
            registry,
            component,
            replay,
        )
        .unwrap();
        let report = replayed.tick().unwrap();
        assert_eq!(report.state.records[&n("status")].phase, Phase::StartFailed);
        assert!(replay_backend.0.borrow().owned_instances().is_empty());
        emit(
            "consumed-permit-rejected",
            "a retained token cannot consume the confirmed disposition twice",
            json!(report),
        );
        drop(old_backend);
        drop(b);
        return;
    }
    panic!("unknown passage stage");
}

#[test]
#[ignore = "requires validated /opt/rx release; run tools/registration_passage.py"]
fn real_registration_passage() {
    let data = PathBuf::from(
        std::env::var("RX_PASSAGE_DATA").expect("dedicated evidence directory required"),
    );
    assert!(
        !data.join("registration.db").exists(),
        "use a fresh evidence directory; old records must not be overwritten"
    );
    std::fs::create_dir_all(&data).unwrap();
    for stage in [
        "register",
        "normal",
        "reopen",
        "loss",
        "manager-reopen",
        "dependencies",
    ] {
        let result = Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "passage_worker", "--nocapture"])
            .env("RX_PASSAGE_DATA", &data)
            .env("RX_PASSAGE_STAGE", stage)
            .output()
            .unwrap();
        print!("{}", String::from_utf8_lossy(&result.stdout));
        eprint!("{}", String::from_utf8_lossy(&result.stderr));
        assert!(result.status.success(), "stage {stage} failed");
    }
    emit(
        "passage-complete",
        "all asserted transitions executed; component registration persists independently of manager process lifetime",
        json!({"limitations":["readiness is only authored comparison of component self-report, not independent functional or physical qualification","positive operating-area work-use provider connection unsupported; host has no grant issuer","actual device operations, collaborative resource binding and multi-hop dependency execution unsupported","external investigation provider after total manager-process/Child-handle loss unsupported","direct child exit does not establish descendant termination or resource recovery","multi-host unsupported","Linux resource enforcement not implemented",
                "positive consumer-side binding acceptance unsupported; diagnostic tracking is not a work binding",
                "dependency availability is checked at explicit checkpoints, not continuously between samples",
                "control-effect group-target signal race remains unresolved","no physical equipment qualification"],"evidence":"actual Linux service and SQLite, owned-exit disposition, explicit new-instance resume, scoped self-report/work-use assessments and diagnostic dependency consumption; no synthetic release installation"}),
    );
}
