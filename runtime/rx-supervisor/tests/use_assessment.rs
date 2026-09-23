#![cfg(unix)]
use rx_domain::types::*;
use rx_solution_catalog::DeviceCatalog;
use rx_storage::SqliteRepository;
use rx_supervisor::{
    Result,
    execution::{Application, Requirements},
    model::*,
    process::{Backend, OsProcesses, SpawnFailure},
    registered::{RegisteredSupervisor, catalog_reference},
    registration::{Declaration, Registry},
    use_assessment::*,
};
use sha2::{Digest as _, Sha256};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};
fn n(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn hash(p: &Path) -> Digest {
    Digest::from_bytes(Sha256::digest(std::fs::read(p).unwrap()).into())
}
fn support() -> DeviceCatalog {
    DeviceCatalog::decode(include_bytes!("../../../catalogs/device-support.v1.json")).unwrap()
}
fn scope(role: &str) -> UseScope {
    UseScope {
        operating_area: n("test/diagnostic-area"),
        role: n(role),
    }
}
fn contract() -> ReadinessContract {
    let base: BTreeMap<_, _> = [
        (n("report/instance"), ReadinessCondition::InstanceMatches),
        (
            n("report/schema"),
            ReadinessCondition::Equals {
                field: n("schema"),
                expected: ExpectedValue::Text("rx.solutions-status.v1".into()),
                origin: ReportOrigin::ReleaseDeclaration,
            },
        ),
        (
            n("report/count"),
            ReadinessCondition::Unsigned {
                field: n("support_profiles"),
                origin: ReportOrigin::ReleaseDeclaration,
            },
        ),
    ]
    .into();
    let mut linked = base.clone();
    linked.insert(
        n("operator/delegation"),
        ReadinessCondition::Equals {
            field: n("operator_api_delegation"),
            expected: ExpectedValue::Text("CONNECTED".into()),
            origin: ReportOrigin::ReleaseDeclaration,
        },
    );
    let mut unsupported = base.clone();
    unsupported.insert(
        n("calibration/physical"),
        ReadinessCondition::Unsupported {
            reason: "physical calibration observation is not supported".into(),
        },
    );
    ReadinessContract(
        [
            (
                n("diagnostics/summary"),
                ReadinessProfile {
                    endpoint: StatusEndpoint::SupportSummary,
                    conditions: base,
                },
            ),
            (
                n("diagnostics/operator"),
                ReadinessProfile {
                    endpoint: StatusEndpoint::SupportSummary,
                    conditions: linked,
                },
            ),
            (
                n("diagnostics/calibrated"),
                ReadinessProfile {
                    endpoint: StatusEndpoint::SupportSummary,
                    conditions: unsupported,
                },
            ),
        ]
        .into(),
    )
}
struct Observing {
    os: OsProcesses,
    saved: Option<Box<StatusObservation>>,
    capture: bool,
    replay: bool,
    requests: usize,
}
#[derive(Clone)]
struct Owned(Rc<RefCell<Observing>>);
impl Owned {
    fn new(path: PathBuf) -> Self {
        Self(Rc::new(RefCell::new(Observing {
            os: OsProcesses::new(path).unwrap(),
            saved: None,
            capture: false,
            replay: false,
            requests: 0,
        })))
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        if Rc::strong_count(&self.0) != 1 {
            return;
        }
        let mut b = self.0.borrow_mut();
        for i in b.os.owned_instances() {
            let _ = b.os.terminate(&i, true);
            let end = Instant::now() + Duration::from_secs(2);
            while Instant::now() < end {
                if matches!(b.os.exited(&i), Ok(Some(_))) {
                    let _ = b.os.forget_exited(&i);
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
impl Backend for Owned {
    fn spawn(
        &mut self,
        l: &Launch,
        a: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<u32, SpawnFailure> {
        self.0.borrow_mut().os.spawn(l, a)
    }
    fn pid(&self, i: &Id) -> Option<u32> {
        self.0.borrow().os.pid(i)
    }
    fn owns(&self, i: &Id) -> bool {
        self.0.borrow().os.owns(i)
    }
    fn forget_exited(&mut self, i: &Id) -> Result<()> {
        self.0.borrow_mut().os.forget_exited(i)
    }
    fn exited(&mut self, i: &Id) -> Result<Option<Option<i32>>> {
        self.0.borrow_mut().os.exited(i)
    }
    fn ready(&mut self, l: &Launch) -> Result<bool> {
        self.0.borrow_mut().os.ready(l)
    }
    fn terminate(&mut self, i: &Id, f: bool) -> Result<()> {
        self.0.borrow_mut().os.terminate(i, f)
    }
    fn observe_status(
        &mut self,
        l: &Launch,
        r: &StatusObservationRequest,
    ) -> Result<StatusObservationResult> {
        let mut b = self.0.borrow_mut();
        b.requests += 1;
        if b.replay {
            b.replay = false;
            return Ok(StatusObservationResult::Observed(
                b.saved.take().expect("captured response"),
            ));
        }
        let result = b.os.observe_status(l, r)?;
        if b.capture {
            b.capture = false;
            if let StatusObservationResult::Observed(value) = result {
                b.saved = Some(value);
                return Ok(StatusObservationResult::NotEvaluated {
                    condition: n("test/withheld-observation"),
                    reason: "test retains response for a subsequent replay attack".into(),
                });
            }
        }
        Ok(result)
    }
}
type Managed = RegisteredSupervisor<SqliteRepository, Owned, SoftwareOnly, SqliteRepository>;
struct Fixture {
    dir: tempfile::TempDir,
    program: Program,
    plan: Plan,
    component: Id,
    backend: Owned,
    managed: Managed,
}
fn setup(declared: bool, alive_only: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("report.py");
    let mode = dir.path().join("mode");
    std::fs::write(&mode, "normal").unwrap();
    std::fs::write(&script,r#"import json,os,sys
from http.server import BaseHTTPRequestHandler,HTTPServer
from pathlib import Path
class Handler(BaseHTTPRequestHandler):
 def log_message(self,*args):pass
 def do_GET(self):
  mode=Path(MODE_FILE).read_text()
  value={'schema':'rx.solutions-status.v1','phase':'SOFTWARE_READY_UNCOMMISSIONED','supervisor_instance':os.environ['RX_PROCESS_INSTANCE_ID'],'support_profiles':2,'operator_api_delegation':'NOT_CONNECTED'}
  if self.path!='/health':
   if mode=='wrong-instance':value['supervisor_instance']='another-instance'
   if mode=='missing':del value['support_profiles']
   if mode=='connected':value['operator_api_delegation']='CONNECTED'
  body=json.dumps(value).encode();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
HTTPServer(('127.0.0.1',int(sys.argv[2])),Handler).serve_forever()
"#.replace("MODE_FILE", &serde_json::to_string(mode.to_string_lossy().as_ref()).unwrap())).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let python = PathBuf::from("/usr/bin/python3");
    let program = Program {
        id: n("test/report"),
        effect: Effect::NonActuating,
        executable: python.clone(),
        executable_sha256: hash(&python),
        files: [(script.clone(), hash(&script))].into(),
        fixed_arguments: vec![script.to_string_lossy().into()],
        arguments: [(
            n("port"),
            Argument::Port {
                flag: "--port".into(),
            },
        )]
        .into(),
        ready: if alive_only {
            ReadyProbe::AliveOnly
        } else {
            ReadyProbe::HttpStatus {
                port_parameter: n("port"),
            }
        },
        execution_requirements: Some(Requirements(BTreeMap::new())),
        functional_readiness: declared.then(contract),
    };
    let plan = Plan {
        schema: n("rx.solutions-process-plan.v1"),
        id: id(),
        environment: Environment::Simulation,
        profiles: vec![],
        processes: vec![Process {
            id: n("report"),
            program: program.id.clone(),
            parameters: [(n("port"), port.to_string())].into(),
            depends_on: vec![],
            startup_timeout_ms: Counter(3000),
            shutdown_timeout_ms: Counter(1000),
            restart_limit: Counter(0),
            restart_backoff_ms: Counter(100),
        }],
    };
    let mut registry =
        Registry::new(SqliteRepository::open(dir.path().join("registry.db")).unwrap());
    let component = registry
        .register(Declaration {
            label: n("fixture-report"),
            catalog: catalog_reference(&program).unwrap(),
        })
        .unwrap()
        .registration
        .id;
    let backend = Owned::new(dir.path().join("logs"));
    let managed = RegisteredSupervisor::open(
        SqliteRepository::open(dir.path().join("state.db")).unwrap(),
        backend.clone(),
        SoftwareOnly,
        plan.clone(),
        [(program.id.clone(), program.clone())].into(),
        &support(),
        registry,
        component.clone(),
    )
    .unwrap();
    Fixture {
        dir,
        program,
        plan,
        component,
        backend,
        managed,
    }
}
fn ready(f: &mut Fixture) {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let status = f.managed.tick().unwrap();
        if status.state.records[&n("report")].phase == Phase::ProcessReady {
            return;
        }
        assert!(Instant::now() < end, "{status:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}
static CASES: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn registration_and_f1_admission_do_not_evaluate_or_grant_use() {
    let _case = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let mut f = setup(true, false);
    let before = f.managed.query().unwrap();
    assert_eq!(
        before.functional_readiness.state(),
        ConditionState::NotEvaluated
    );
    assert_eq!(
        before.work_use_permission.state(),
        WorkUseState::NotEvaluated
    );
    ready(&mut f);
    assert!(matches!(
        f.managed.execution_admission().unwrap()[&n("report")].application,
        Application::ReportedAtStart { .. }
    ));
    let view = f.managed.query().unwrap();
    assert_eq!(
        view.functional_readiness.state(),
        ConditionState::NotEvaluated
    );
    assert_eq!(view.work_use_permission.state(), WorkUseState::NotEvaluated);
    assert_eq!(f.backend.0.borrow().requests, 0);
    println!(
        "registered/started/F1 receipt is not an assessment: {}",
        serde_json::to_string(&view).unwrap()
    );
}
#[test]
fn alive_only_and_undeclared_programs_stay_unsupported() {
    let _case = CASES.lock().unwrap_or_else(|e| e.into_inner());
    for declared in [false, true] {
        let mut f = setup(declared, true);
        ready(&mut f);
        let view = f
            .managed
            .assess_use(scope("diagnostics/summary"), &NoWorkUseProvider)
            .unwrap();
        assert_eq!(
            f.managed.state().unwrap().records[&n("report")].phase,
            Phase::ProcessReady
        );
        assert_eq!(
            view.functional_readiness.state(),
            ConditionState::Unsupported
        );
        assert_eq!(view.work_use_permission.state(), WorkUseState::Unsupported);
        assert!(!view.functional_readiness.conditions().is_empty());
        println!(
            "alive-only declared={declared}: {}",
            serde_json::to_string(&view).unwrap()
        );
    }
}
#[test]
fn reported_conditions_and_work_use_are_separate_and_read_only() {
    let _case = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let mut f = setup(true, false);
    ready(&mut f);
    let state = serde_json::to_value(f.managed.state().unwrap()).unwrap();
    let history = f.managed.history().unwrap();
    let unavailable = f
        .managed
        .assess_use(scope("diagnostics/operator"), &NoWorkUseProvider)
        .unwrap();
    assert_eq!(
        unavailable.functional_readiness.state(),
        ConditionState::NotMet
    );
    let condition = unavailable
        .functional_readiness
        .conditions()
        .iter()
        .find(|c| c.name == n("operator/delegation"))
        .unwrap();
    assert_eq!(condition.origin, ReportOrigin::ReleaseDeclaration);
    assert_eq!(condition.observed, Some(serde_json::json!("NOT_CONNECTED")));
    let summary = f
        .managed
        .assess_use(scope("diagnostics/summary"), &NoWorkUseProvider)
        .unwrap();
    assert_eq!(
        summary.functional_readiness.state(),
        ConditionState::Satisfied
    );
    assert_eq!(
        summary.work_use_permission.state(),
        WorkUseState::Unsupported
    );
    assert!(
        summary
            .work_use_permission
            .conditions()
            .iter()
            .any(|c| c.name == n("work-use/operating-area-provider"))
    );
    assert_eq!(
        serde_json::to_value(f.managed.state().unwrap()).unwrap(),
        state
    );
    assert_eq!(f.managed.history().unwrap(), history);
    assert_eq!(
        f.managed.query().unwrap().functional_readiness.state(),
        ConditionState::NotEvaluated
    );
    println!(
        "ProcessReady but report conditions not met: {}",
        serde_json::to_string(&unavailable).unwrap()
    );
    println!(
        "reported conditions met, no work-use provider: {}",
        serde_json::to_string(&summary).unwrap()
    );
}
#[test]
fn not_evaluated_not_met_and_unsupported_have_named_conditions() {
    let _case = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let mut f = setup(true, false);
    ready(&mut f);
    f.backend.0.borrow_mut().capture = true;
    let unknown = f
        .managed
        .assess_use(scope("diagnostics/summary"), &NoWorkUseProvider)
        .unwrap();
    assert_eq!(
        unknown.functional_readiness.state(),
        ConditionState::NotEvaluated
    );
    assert!(
        unknown
            .functional_readiness
            .conditions()
            .iter()
            .all(|c| c.state == ConditionState::NotEvaluated)
    );
    std::fs::write(f.dir.path().join("mode"), "missing").unwrap();
    let unmet = f
        .managed
        .assess_use(scope("diagnostics/summary"), &NoWorkUseProvider)
        .unwrap();
    assert_eq!(unmet.functional_readiness.state(), ConditionState::NotMet);
    assert!(
        unmet
            .functional_readiness
            .conditions()
            .iter()
            .any(|c| c.name == n("report/count") && c.state == ConditionState::NotMet)
    );
    std::fs::write(f.dir.path().join("mode"), "normal").unwrap();
    let unsupported = f
        .managed
        .assess_use(scope("diagnostics/calibrated"), &NoWorkUseProvider)
        .unwrap();
    assert_eq!(
        unsupported.functional_readiness.state(),
        ConditionState::Unsupported
    );
    assert!(
        unsupported
            .functional_readiness
            .conditions()
            .iter()
            .any(|c| c.name == n("calibration/physical") && c.state == ConditionState::Unsupported)
    );
    println!(
        "named states: {}",
        serde_json::json!([
            unknown.functional_readiness,
            unmet.functional_readiness,
            unsupported.functional_readiness
        ])
    );
}
#[test]
fn another_instance_and_previous_assessment_response_cannot_be_promoted() {
    let _case = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let mut f = setup(true, false);
    ready(&mut f);
    std::fs::write(f.dir.path().join("mode"), "wrong-instance").unwrap();
    let wrong = f
        .managed
        .assess_use(scope("diagnostics/summary"), &NoWorkUseProvider)
        .unwrap();
    assert_eq!(wrong.functional_readiness.state(), ConditionState::NotMet);
    assert!(
        wrong
            .functional_readiness
            .conditions()
            .iter()
            .any(|c| c.name == n("report/instance") && c.state == ConditionState::NotMet)
    );
    assert!(
        wrong
            .functional_readiness
            .conditions()
            .iter()
            .filter(|c| c.name != n("report/instance"))
            .all(|c| c.state == ConditionState::NotEvaluated)
    );
    std::fs::write(f.dir.path().join("mode"), "normal").unwrap();
    f.backend.0.borrow_mut().capture = true;
    f.managed
        .assess_use(scope("diagnostics/summary"), &NoWorkUseProvider)
        .unwrap();
    f.backend.0.borrow_mut().replay = true;
    let replay = f
        .managed
        .assess_use(scope("diagnostics/summary"), &NoWorkUseProvider)
        .unwrap();
    assert_eq!(replay.functional_readiness.state(), ConditionState::NotMet);
    assert_eq!(
        replay.functional_readiness.conditions()[0].name,
        n("report/assessment-binding")
    );
    println!(
        "negative old observation: {}",
        serde_json::to_string(&replay).unwrap()
    );
}
struct AreaDenied;
impl WorkUsePort for AreaDenied {
    fn assess(&self, request: &WorkUseRequest) -> WorkUseReply {
        assert_eq!(request.readiness().state(), ConditionState::Satisfied);
        WorkUseReply::Denied {
            decision_reference: n("test-area/decision-1"),
            conditions: [(
                n("role/assignment"),
                "simulated area policy has not assigned this diagnostic role".into(),
            )]
            .into(),
        }
    }
}
struct InvalidPort;
impl WorkUsePort for InvalidPort {
    fn assess(&self, _: &WorkUseRequest) -> WorkUseReply {
        WorkUseReply::Denied {
            decision_reference: n("test-area/invalid"),
            conditions: BTreeMap::new(),
        }
    }
}
#[test]
fn reported_area_denial_differs_from_unconnected_or_invalid_provider() {
    let _case = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let mut f = setup(true, false);
    ready(&mut f);
    let denied = f
        .managed
        .assess_use(scope("diagnostics/summary"), &AreaDenied)
        .unwrap();
    assert_eq!(denied.work_use_permission.state(), WorkUseState::Denied);
    assert_eq!(
        denied.work_use_permission.conditions()[0].origin,
        ReportOrigin::OperatingAreaReport
    );
    let missing = f
        .managed
        .assess_use(scope("diagnostics/summary"), &NoWorkUseProvider)
        .unwrap();
    assert_eq!(
        missing.work_use_permission.state(),
        WorkUseState::Unsupported
    );
    let invalid = f
        .managed
        .assess_use(scope("diagnostics/summary"), &InvalidPort)
        .unwrap();
    assert_eq!(
        invalid.work_use_permission.state(),
        WorkUseState::Unsupported
    );
    assert_eq!(
        invalid.work_use_permission.conditions()[0].name,
        n("work-use/provider-response")
    );
    println!(
        "denied versus unconnected: {}",
        serde_json::json!([
            denied.work_use_permission,
            missing.work_use_permission,
            invalid.work_use_permission
        ])
    );
}
#[test]
fn declared_conditions_cannot_be_lowered_through_site_or_existing_catalog_pin() {
    let _case = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let f = setup(true, false);
    let mut site = serde_json::to_value(&f.plan).unwrap();
    site["processes"][0]["functional_readiness"] = serde_json::json!({});
    assert!(serde_json::from_value::<Plan>(site).is_err());
    let mut no_correlation = f.program.clone();
    no_correlation.functional_readiness = Some(ReadinessContract(
        [(
            n("diagnostics/summary"),
            ReadinessProfile {
                endpoint: StatusEndpoint::Health,
                conditions: BTreeMap::new(),
            },
        )]
        .into(),
    ));
    assert!(catalog_reference(&no_correlation).is_err());
    let (store, backend, _, registry) = f.managed.into_parts();
    drop(store);
    drop(backend);
    let mut lower = f.program.clone();
    lower.functional_readiness = None;
    let result = RegisteredSupervisor::open(
        SqliteRepository::open(f.dir.path().join("lower.db")).unwrap(),
        Owned::new(f.dir.path().join("lower-logs")),
        SoftwareOnly,
        f.plan,
        [(lower.id.clone(), lower)].into(),
        &support(),
        registry,
        f.component,
    );
    assert!(
        result
            .err()
            .unwrap()
            .to_string()
            .contains("catalog declaration differs")
    );
}
#[test]
fn new_owner_does_not_restore_a_readiness_assessment_or_permission() {
    let _case = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let mut f = setup(true, false);
    ready(&mut f);
    assert_eq!(
        f.managed
            .assess_use(scope("diagnostics/summary"), &NoWorkUseProvider)
            .unwrap()
            .functional_readiness
            .state(),
        ConditionState::Satisfied
    );
    let (store, backend, _, registry) = f.managed.into_parts();
    drop(store);
    drop(backend);
    drop(registry);
    let registry = Registry::new(SqliteRepository::open(f.dir.path().join("registry.db")).unwrap());
    let mut reopened = RegisteredSupervisor::open(
        SqliteRepository::open(f.dir.path().join("state.db")).unwrap(),
        Owned::new(f.dir.path().join("fresh")),
        SoftwareOnly,
        f.plan,
        [(f.program.id.clone(), f.program)].into(),
        &support(),
        registry,
        f.component,
    )
    .unwrap();
    assert_eq!(
        reopened.query().unwrap().functional_readiness.state(),
        ConditionState::NotEvaluated
    );
    let view = reopened
        .assess_use(scope("diagnostics/summary"), &NoWorkUseProvider)
        .unwrap();
    assert_eq!(
        view.functional_readiness.state(),
        ConditionState::NotEvaluated
    );
    assert_eq!(view.work_use_permission.state(), WorkUseState::Unsupported);
    assert_eq!(
        reopened.state().unwrap().records[&n("report")].phase,
        Phase::Unknown
    );
}

#[test]
fn omitted_declaration_preserves_legacy_serialization_and_closed_child_cannot_report_ready() {
    let _case = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let mut f = setup(false, false);
    assert!(
        serde_json::to_value(&f.program)
            .unwrap()
            .get("functional_readiness")
            .is_none()
    );
    ready(&mut f);
    assert_eq!(
        f.managed
            .assess_use(scope("diagnostics/summary"), &NoWorkUseProvider)
            .unwrap()
            .functional_readiness
            .state(),
        ConditionState::Unsupported
    );
    drop(f);
    let mut f = setup(true, false);
    ready(&mut f);
    let instance = f.managed.state().unwrap().records[&n("report")]
        .instance
        .clone()
        .unwrap();
    f.backend.terminate(&instance, true).unwrap();
    let end = Instant::now() + Duration::from_secs(2);
    while !matches!(f.backend.exited(&instance), Ok(Some(_))) {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(5));
    }
    let view = f
        .managed
        .assess_use(scope("diagnostics/summary"), &NoWorkUseProvider)
        .unwrap();
    assert_eq!(view.functional_readiness.state(), ConditionState::NotMet);
    assert!(
        view.functional_readiness
            .conditions()
            .iter()
            .any(|c| c.name == n("execution/current-child"))
    );
}
