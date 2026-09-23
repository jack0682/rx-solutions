#![cfg(unix)]
use rx_domain::types::*;
use rx_solution_catalog::DeviceCatalog;
use rx_storage::SqliteRepository;
use rx_supervisor::{
    Result,
    execution::Requirements,
    model::*,
    process::{Backend, OsProcesses, SpawnFailure},
    registered::{RegisteredSupervisor, catalog_reference},
    registration::diagnostic::*,
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
fn uid() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn hash(p: &Path) -> Digest {
    Digest::from_bytes(Sha256::digest(std::fs::read(p).unwrap()).into())
}
#[derive(Clone)]
struct Owned(Rc<RefCell<OsProcesses>>);
impl Backend for Owned {
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
    fn observe_status(
        &mut self,
        l: &Launch,
        r: &StatusObservationRequest,
    ) -> Result<StatusObservationResult> {
        self.0.borrow_mut().observe_status(l, r)
    }
}
type Managed = RegisteredSupervisor<SqliteRepository, Owned, SoftwareOnly, SqliteRepository>;
struct Fixture {
    dir: tempfile::TempDir,
    managed: Managed,
    backend: Owned,
    reference: RegistrationRef,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let mut b = self.backend.0.borrow_mut();
        for id in b.owned_instances() {
            let _ = b.terminate(&id, true);
            let end = Instant::now() + Duration::from_secs(2);
            while Instant::now() < end {
                if matches!(b.exited(&id), Ok(Some(_))) {
                    let _ = b.forget_exited(&id);
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("report.py");
    std::fs::write(dir.path().join("count"), "2").unwrap();
    // Explicit synthetic self-report fixture; not the installed release audit.
    std::fs::write(&script,r#"import json,os,sys
from pathlib import Path
from http.server import BaseHTTPRequestHandler,HTTPServer
class H(BaseHTTPRequestHandler):
 def log_message(self,*a):pass
 def do_GET(self):
  v={'schema':'rx.solutions-status.v1','phase':'SOFTWARE_READY_UNCOMMISSIONED','supervisor_instance':os.environ['RX_PROCESS_INSTANCE_ID'],'support_profiles':int(Path(__file__).with_name('count').read_text())}
  b=json.dumps(v).encode();self.send_response(200);self.send_header('Content-Length',str(len(b)));self.end_headers();self.wfile.write(b)
HTTPServer(('127.0.0.1',int(sys.argv[2])),H).serve_forever()
"#).unwrap();
    let python = PathBuf::from("/usr/bin/python3");
    let program = Program {
        id: n("test/diagnostic-provider"),
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
        ready: ReadyProbe::HttpStatus {
            port_parameter: n("port"),
        },
        execution_requirements: Some(Requirements(BTreeMap::new())),
        functional_readiness: Some(ReadinessContract(
            [(
                n("diagnostics/support-summary"),
                ReadinessProfile {
                    endpoint: StatusEndpoint::SupportSummary,
                    conditions: [
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
                    .into(),
                },
            )]
            .into(),
        )),
    };
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let plan = Plan {
        schema: n("rx.solutions-process-plan.v1"),
        id: uid(),
        environment: Environment::Simulation,
        profiles: vec![],
        processes: vec![Process {
            id: n("provider"),
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
        Registry::new(SqliteRepository::open(dir.path().join("provider.db")).unwrap());
    let registered = registry
        .register(Declaration {
            label: n("provider"),
            catalog: catalog_reference(&program).unwrap(),
        })
        .unwrap();
    let reference = RegistrationRef::from_registration(&registered);
    let backend = Owned(Rc::new(RefCell::new(
        OsProcesses::new(dir.path().join("logs")).unwrap(),
    )));
    let support =
        DeviceCatalog::decode(include_bytes!("../../../catalogs/device-support.v1.json")).unwrap();
    let managed = Managed::open(
        SqliteRepository::open(dir.path().join("execution.db")).unwrap(),
        backend.clone(),
        SoftwareOnly,
        plan,
        [(program.id.clone(), program)].into(),
        &support,
        registry,
        registered.registration.id,
    )
    .unwrap();
    Fixture {
        dir,
        managed,
        backend,
        reference,
    }
}
fn ready(f: &mut Fixture) {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let s = f.managed.tick().unwrap();
        if s.state.records[&n("provider")].phase == Phase::ProcessReady {
            return;
        }
        assert!(Instant::now() < end, "{s:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn stop(f: &mut Fixture, planned: bool) {
    if planned {
        f.managed.request_stop().unwrap();
    } else {
        let id = f.managed.state().unwrap().records[&n("provider")]
            .instance
            .clone()
            .unwrap();
        f.backend.0.borrow_mut().terminate(&id, true).unwrap();
    }
    let end = Instant::now() + Duration::from_secs(4);
    loop {
        let s = f.managed.tick().unwrap();
        if s.state.records[&n("provider")].phase == Phase::Exited {
            assert!(f.backend.0.borrow().owned_instances().is_empty());
            return;
        }
        assert!(Instant::now() < end, "{s:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn consumer(path: &Path, c: Catalog) -> (Consumer<SqliteRepository>, Id) {
    let mut r = Registry::new(SqliteRepository::open(path).unwrap());
    let id = r
        .register(Declaration {
            label: n("diagnostic-consumer"),
            catalog: c.reference().unwrap(),
        })
        .unwrap()
        .registration
        .id;
    (Consumer::open(r, id.clone(), c).unwrap(), id)
}
fn track(c: &mut Consumer<SqliteRepository>, f: &Fixture, profile: &str) -> TrackedBinding {
    c.track(n(profile), n("test/diagnostics"), Some(f.reference.clone()))
        .unwrap()
}
fn assigned(c: &mut Consumer<SqliteRepository>, b: &TrackedBinding, s: &mut impl Source) -> Run {
    let p = c.assign(&b.id, s).unwrap();
    assert_eq!(p.assessment.state, ConditionState::Satisfied);
    let r = p.run.unwrap();
    assert_eq!(r.phase, RunPhase::Assigned);
    r
}
fn begun(c: &mut Consumer<SqliteRepository>, b: &TrackedBinding, r: &Run, s: &mut impl Source) {
    let p = c.begin(&b.id, &r.id, s).unwrap();
    assert_eq!(p.assessment.state, ConditionState::Satisfied);
    assert_eq!(p.run.unwrap().phase, RunPhase::Running);
}
fn finished(
    c: &mut Consumer<SqliteRepository>,
    b: &TrackedBinding,
    r: &Run,
    s: &mut impl Source,
) -> DiagnosticResult {
    let p = c.finish(&b.id, &r.id, s).unwrap();
    assert_eq!(p.assessment.state, ConditionState::Satisfied);
    assert_eq!(p.run.unwrap().phase, RunPhase::Completed);
    p.result.unwrap()
}
static CASES: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn preparation_input_outlives_provider_and_result_is_durable_without_permission() {
    let _g = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let mut f = fixture();
    let path = f.dir.path().join("consumer.db");
    let (c, id) = consumer(&path, catalog());
    let mut c = c;
    let b = track(&mut c, &f, "snapshot-after-preparation");
    let empty = c.inspect(&b.id, None, None, &mut f.managed).unwrap();
    assert_eq!(empty.binding.generation, None);
    assert_eq!(empty.new_assignment.state, ConditionState::NotEvaluated);
    ready(&mut f);
    let a = assigned(&mut c, &b, &mut f.managed);
    assert_eq!(
        c.inspect(&b.id, Some(&a.id), None, &mut f.managed)
            .unwrap()
            .ongoing
            .state,
        ConditionState::NotEvaluated
    );
    begun(&mut c, &b, &a, &mut f.managed);
    let old = finished(&mut c, &b, &a, &mut f.managed);
    let active = assigned(&mut c, &b, &mut f.managed);
    begun(&mut c, &b, &active, &mut f.managed);
    stop(&mut f, true);
    let before = c.history().unwrap();
    let view = c
        .inspect(&b.id, Some(&active.id), Some(&old.id), &mut f.managed)
        .unwrap();
    assert_eq!(view.new_assignment.state, ConditionState::NotMet);
    assert_eq!(view.ongoing.state, ConditionState::Satisfied);
    assert_eq!(view.result_consumption.state, ConditionState::Satisfied);
    assert_eq!(c.history().unwrap(), before);
    assert!(c.assign(&b.id, &mut f.managed).unwrap().run.is_none());
    let later = finished(&mut c, &b, &active, &mut f.managed);
    assert_eq!(later.body["sample_count"], 1);
    assert_eq!(
        later.body["samples"][0]["report"]["conditions"][0]["state"],
        "SATISFIED"
    );
    assert!(
        c.finish(&b.id, &active.id, &mut f.managed)
            .unwrap()
            .result
            .is_none()
    );
    assert_eq!(c.recorded_result(&old.id).unwrap(), old);
    drop(c.into_registry());
    let mut c = Consumer::open(
        Registry::new(SqliteRepository::open(path).unwrap()),
        id,
        catalog(),
    )
    .unwrap();
    assert_eq!(
        c.consume_result(&old.id, &mut NoSource).unwrap().result,
        Some(old)
    );
    assert_eq!(
        serde_json::to_value(&later).unwrap()["work_use"],
        "UNSUPPORTED"
    );
}

#[test]
fn continuous_loss_blocks_only_dependent_progress_and_current_result_consumption() {
    let _g = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let mut f = fixture();
    ready(&mut f);
    let (mut c, _) = consumer(&f.dir.path().join("consumer.db"), catalog());
    let b = track(&mut c, &f, "current-report-collection");
    let r = assigned(&mut c, &b, &mut f.managed);
    begun(&mut c, &b, &r, &mut f.managed);
    let old = finished(&mut c, &b, &r, &mut f.managed);
    let active = assigned(&mut c, &b, &mut f.managed);
    begun(&mut c, &b, &active, &mut f.managed);
    assert_eq!(
        c.poll(&b.id, &active.id, &mut f.managed)
            .unwrap()
            .run
            .unwrap()
            .samples
            .len(),
        3
    );
    let unrelated = c
        .track(n("catalog-summary"), n("test/diagnostics"), None)
        .unwrap();
    let u = assigned(&mut c, &unrelated, &mut NoSource);
    begun(&mut c, &unrelated, &u, &mut NoSource);
    stop(&mut f, false);
    let view = c
        .inspect(&b.id, Some(&active.id), Some(&old.id), &mut f.managed)
        .unwrap();
    for a in [
        &view.new_assignment,
        &view.ongoing,
        &view.result_consumption,
    ] {
        assert_eq!(a.state, ConditionState::NotMet);
        assert!(!a.conditions[0].reason.is_empty());
    }
    let blocked = c.finish(&b.id, &active.id, &mut f.managed).unwrap();
    assert!(blocked.result.is_none());
    assert_eq!(blocked.run.unwrap().phase, RunPhase::Running);
    assert!(
        c.consume_result(&old.id, &mut f.managed)
            .unwrap()
            .result
            .is_none()
    );
    assert_eq!(c.recorded_result(&old.id).unwrap(), old);
    let independent = finished(&mut c, &unrelated, &u, &mut NoSource);
    assert_eq!(independent.body["sample_count"], 0);
    assert!(independent.body["local_catalog_summary"].is_object());
}

#[test]
fn generation_window_defers_input_until_finish_and_preserves_completed_snapshot() {
    let _g = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let mut f = fixture();
    let (mut c, _) = consumer(&f.dir.path().join("consumer.db"), catalog());
    let b = track(&mut c, &f, "snapshot-at-generation");
    let r = assigned(&mut c, &b, &mut NoSource);
    assert!(r.samples.is_empty());
    begun(&mut c, &b, &r, &mut NoSource);
    let missing = c.finish(&b.id, &r.id, &mut NoSource).unwrap();
    assert_eq!(missing.assessment.state, ConditionState::Unsupported);
    assert!(missing.result.is_none());
    ready(&mut f);
    let value = finished(&mut c, &b, &r, &mut f.managed);
    assert_eq!(value.body["sample_count"], 1);
    let active = assigned(&mut c, &b, &mut NoSource);
    begun(&mut c, &b, &active, &mut NoSource);
    stop(&mut f, false);
    let view = c
        .inspect(&b.id, Some(&active.id), Some(&value.id), &mut f.managed)
        .unwrap();
    assert_eq!(view.new_assignment.state, ConditionState::Satisfied);
    assert_eq!(view.ongoing.state, ConditionState::NotMet);
    assert_eq!(view.result_consumption.state, ConditionState::Satisfied);
}

#[test]
fn same_format_registration_and_same_registration_new_execution_do_not_inherit_binding() {
    let _g = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let mut f = fixture();
    let mut replacement = fixture();
    ready(&mut f);
    ready(&mut replacement);
    let (mut c, _) = consumer(&f.dir.path().join("consumer.db"), catalog());
    let b = track(&mut c, &f, "current-report-collection");
    let r = assigned(&mut c, &b, &mut f.managed);
    begun(&mut c, &b, &r, &mut f.managed);
    let original = c
        .inspect(&b.id, Some(&r.id), None, &mut f.managed)
        .unwrap()
        .binding;
    assert_eq!(
        c.finish(&b.id, &r.id, &mut replacement.managed)
            .unwrap()
            .assessment
            .state,
        ConditionState::NotMet
    );
    let initial = c
        .assess_binding(&b.id, AcceptanceKind::Initial, None, &NoBindingJudgment)
        .unwrap();
    let changed = c
        .assess_binding(
            &b.id,
            AcceptanceKind::Replacement,
            Some(replacement.reference.clone()),
            &NoBindingJudgment,
        )
        .unwrap();
    assert_eq!(initial.state, WorkUseState::Unsupported);
    assert_eq!(changed.state, WorkUseState::Unsupported);
    assert_ne!(initial.request.kind, changed.request.kind);
    assert_eq!(
        c.inspect(&b.id, Some(&r.id), None, &mut f.managed)
            .unwrap()
            .binding,
        original
    );
    stop(&mut f, true);
    f.managed.rearm_software().unwrap();
    let awaiting = c.inspect(&b.id, Some(&r.id), None, &mut f.managed).unwrap();
    assert_eq!(awaiting.ongoing.state, ConditionState::NotEvaluated);
    ready(&mut f);
    let view = c.inspect(&b.id, Some(&r.id), None, &mut f.managed).unwrap();
    assert_eq!(view.ongoing.state, ConditionState::NotMet);
    assert_eq!(
        view.ongoing.conditions[0].name,
        n("dependency/provider-generation")
    );
    assert_eq!(view.binding, original);
}

#[test]
fn author_scope_cannot_be_replaced_and_unknown_is_not_independent() {
    let _g = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    let path = f.dir.path().join("consumer.db");
    let mut authored = catalog();
    authored
        .profiles
        .insert(n("missing"), Dependency::Undeclared);
    authored.profiles.insert(
        n("unknown"),
        Dependency::Unknown {
            reason: "scope investigation pending".into(),
        },
    );
    let (mut c, id) = consumer(&path, authored.clone());
    for (profile, expected) in [
        ("missing", ConditionState::Unsupported),
        ("unknown", ConditionState::NotEvaluated),
    ] {
        let b = c.track(n(profile), n("test/diagnostics"), None).unwrap();
        let a = c.assign(&b.id, &mut NoSource).unwrap();
        assert_eq!(a.assessment.state, expected);
        assert!(a.run.is_none());
        assert!(!a.assessment.conditions[0].reason.is_empty());
    }
    let r = c.into_registry();
    authored
        .profiles
        .insert(n("current-report-collection"), Dependency::Independent);
    assert!(Consumer::open(r, id, authored).is_err());
    let process = serde_json::json!({"id":"report","program":"test/report","parameters":{},"depends_on":[],"startup_timeout_ms":"1000","shutdown_timeout_ms":"1000","restart_limit":"0","restart_backoff_ms":"100","dependency_window":"PREPARATION_ONLY"});
    assert!(serde_json::from_value::<Process>(process).is_err());
}

struct Replay<'a> {
    real: &'a mut Managed,
    saved: Option<SourceReply>,
}
impl Source for Replay<'_> {
    fn observe(&mut self, p: &Probe) -> SourceReply {
        if let Some(old) = self.saved.take() {
            old
        } else {
            self.saved = Some(Source::observe(self.real, p));
            SourceReply::Missing {
                known_lost: false,
                reason: "retain first response to attempt a replay".into(),
            }
        }
    }
}
#[test]
fn captured_response_cannot_be_replayed_for_another_diagnostic_request() {
    let _g = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let mut f = fixture();
    ready(&mut f);
    let (mut c, _) = consumer(&f.dir.path().join("consumer.db"), catalog());
    let b = track(&mut c, &f, "snapshot-after-preparation");
    let mut replay = Replay {
        real: &mut f.managed,
        saved: None,
    };
    assert_eq!(
        c.assign(&b.id, &mut replay).unwrap().assessment.state,
        ConditionState::NotEvaluated
    );
    let rejected = c.assign(&b.id, &mut replay).unwrap();
    assert_eq!(rejected.assessment.state, ConditionState::NotMet);
    assert_eq!(
        rejected.assessment.conditions[0].name,
        n("dependency/observation-context")
    );
    assert!(rejected.run.is_none());
    assert!(c.assign(&b.id, &mut f.managed).unwrap().run.is_some());
}

#[test]
fn current_report_change_and_consumer_retirement_do_not_rewrite_historical_result() {
    let _g = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let mut f = fixture();
    ready(&mut f);
    let path = f.dir.path().join("consumer.db");
    let (mut c, id) = consumer(&path, catalog());
    let b = track(&mut c, &f, "current-report-collection");
    let r = assigned(&mut c, &b, &mut f.managed);
    begun(&mut c, &b, &r, &mut f.managed);
    let old = finished(&mut c, &b, &r, &mut f.managed);
    std::fs::write(f.dir.path().join("count"), "3").unwrap();
    let use_now = c.consume_result(&old.id, &mut f.managed).unwrap();
    assert_eq!(use_now.assessment.state, ConditionState::NotMet);
    assert!(use_now.result.is_none());
    assert_eq!(c.recorded_result(&old.id).unwrap(), old);
    let mut registry = c.into_registry();
    let rev = registry.query(&id).unwrap().registration.revision;
    registry.retire(&id, rev).unwrap();
    let mut c = Consumer::open(registry, id, catalog()).unwrap();
    assert_eq!(
        c.consume_result(&old.id, &mut f.managed)
            .unwrap()
            .assessment
            .conditions[0]
            .name,
        n("consumer/registration")
    );
    assert_eq!(c.recorded_result(&old.id).unwrap(), old);
}

#[test]
fn bounded_collection_keeps_a_finish_slot_and_cannot_overwrite_result() {
    let _g = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let mut f = fixture();
    ready(&mut f);
    let (mut c, _) = consumer(&f.dir.path().join("consumer.db"), catalog());
    let b = track(&mut c, &f, "current-report-collection");
    let r = assigned(&mut c, &b, &mut f.managed);
    begun(&mut c, &b, &r, &mut f.managed);
    for _ in 0..13 {
        assert_eq!(
            c.poll(&b.id, &r.id, &mut f.managed)
                .unwrap()
                .assessment
                .state,
            ConditionState::Satisfied
        );
    }
    assert_eq!(
        c.poll(&b.id, &r.id, &mut f.managed)
            .unwrap()
            .assessment
            .conditions[0]
            .name,
        n("run/sample-capacity")
    );
    let value = finished(&mut c, &b, &r, &mut f.managed);
    assert_eq!(value.body["sample_count"], 16);
    let before = c.history().unwrap();
    assert!(
        c.finish(&b.id, &r.id, &mut f.managed)
            .unwrap()
            .result
            .is_none()
    );
    assert_eq!(c.history().unwrap(), before);
    assert_eq!(c.recorded_result(&value.id).unwrap(), value);
}

struct MalformedSource;
impl Source for MalformedSource {
    fn observe(&mut self, _: &Probe) -> SourceReply {
        SourceReply::Missing {
            known_lost: true,
            reason: String::new(),
        }
    }
}
struct Denial;
impl BindingJudgment for Denial {
    fn assess(&self, r: &AcceptanceRequest) -> AcceptanceReply {
        AcceptanceReply::Denied {
            condition: n("consumer/policy"),
            reason: format!("explicit simulated {:?} refusal", r.kind),
            decision_reference: n("test/decision"),
        }
    }
}
struct MalformedJudgment;
impl BindingJudgment for MalformedJudgment {
    fn assess(&self, _: &AcceptanceRequest) -> AcceptanceReply {
        AcceptanceReply::Unsupported {
            condition: n("test/provider"),
            reason: String::new(),
        }
    }
}
#[test]
fn absent_and_malformed_sources_and_explicit_denial_have_named_distinct_states() {
    let _g = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    let (mut c, _) = consumer(&f.dir.path().join("consumer.db"), catalog());
    let b = track(&mut c, &f, "snapshot-after-preparation");
    let a = c.assign(&b.id, &mut MalformedSource).unwrap();
    assert_eq!(a.assessment.state, ConditionState::Unsupported);
    assert_eq!(
        a.assessment.conditions[0].name,
        n("dependency/provider-response")
    );
    assert!(!a.assessment.conditions[0].reason.is_empty());
    let history = c.history().unwrap();
    let denied = c
        .assess_binding(&b.id, AcceptanceKind::Initial, None, &Denial)
        .unwrap();
    assert_eq!(denied.state, WorkUseState::Denied);
    assert!(denied.decision_reference.is_some());
    let absent = c
        .assess_binding(&b.id, AcceptanceKind::Initial, None, &NoBindingJudgment)
        .unwrap();
    assert_eq!(absent.state, WorkUseState::Unsupported);
    let malformed = c
        .assess_binding(&b.id, AcceptanceKind::Initial, None, &MalformedJudgment)
        .unwrap();
    assert_eq!(malformed.condition, n("binding/provider-response"));
    assert!(
        c.assess_binding(&b.id, AcceptanceKind::Replacement, None, &Denial)
            .is_err()
    );
    assert_eq!(c.history().unwrap(), history);
}

use rx_ports::{Repository, Transaction};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
struct FaultRepo {
    inner: SqliteRepository,
    fail: Arc<AtomicBool>,
}
impl Repository for FaultRepo {
    fn transact<T>(
        &mut self,
        f: impl FnOnce(&mut dyn Transaction) -> rx_ports::Result<T>,
    ) -> rx_ports::Result<T> {
        let fail = &self.fail;
        self.inner.transact(|tx| {
            let before = tx.control_head()?;
            let r = f(tx)?;
            if tx.control_head()? != before && fail.swap(false, Ordering::SeqCst) {
                return Err(rx_ports::StoreError::Unavailable(
                    "injected transaction rollback".into(),
                ));
            }
            Ok(r)
        })
    }
    fn pending_outbox_after(
        &mut self,
        a: Option<&Id>,
        l: usize,
    ) -> rx_ports::Result<Vec<rx_ports::OutboxRecord>> {
        self.inner.pending_outbox_after(a, l)
    }
    fn control_events_after(
        &mut self,
        a: Counter,
        l: usize,
    ) -> rx_ports::Result<Vec<rx_ports::StoredEvent>> {
        self.inner.control_events_after(a, l)
    }
    fn control_snapshot(&mut self) -> rx_ports::Result<(Counter, Vec<rx_ports::Record>)> {
        self.inner.control_snapshot()
    }
    fn journal_head(&mut self) -> rx_ports::Result<Counter> {
        self.inner.journal_head()
    }
    fn pending_outbox(&mut self, l: usize) -> rx_ports::Result<Vec<rx_ports::OutboxRecord>> {
        self.inner.pending_outbox(l)
    }
    fn snapshot(&mut self) -> rx_ports::Result<(Counter, Vec<rx_ports::Record>)> {
        self.inner.snapshot()
    }
    fn events_after(
        &mut self,
        a: Counter,
        l: usize,
    ) -> rx_ports::Result<Vec<rx_ports::StoredEvent>> {
        self.inner.events_after(a, l)
    }
}

#[test]
fn result_run_and_history_commit_atomically_on_storage_failure() {
    let _g = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let fail = Arc::new(AtomicBool::new(false));
    let repo = FaultRepo {
        inner: SqliteRepository::open(dir.path().join("consumer.db")).unwrap(),
        fail: fail.clone(),
    };
    let mut registry = Registry::new(repo);
    let id = registry
        .register(Declaration {
            label: n("atomic-consumer"),
            catalog: catalog().reference().unwrap(),
        })
        .unwrap()
        .registration
        .id;
    let mut c = Consumer::open(registry, id, catalog()).unwrap();
    let b = c
        .track(n("catalog-summary"), n("test/diagnostics"), None)
        .unwrap();
    let r = c.assign(&b.id, &mut NoSource).unwrap().run.unwrap();
    c.begin(&b.id, &r.id, &mut NoSource).unwrap();
    let history = c.history().unwrap();
    fail.store(true, Ordering::SeqCst);
    assert!(c.finish(&b.id, &r.id, &mut NoSource).is_err());
    assert_eq!(c.history().unwrap(), history);
    let after = c
        .inspect(&b.id, Some(&r.id), None, &mut NoSource)
        .unwrap()
        .run
        .unwrap();
    assert_eq!(after.phase, RunPhase::Running);
    assert!(after.result.is_none());
    let registry = c.into_registry();
    let mut repo = registry.into_repository();
    assert!(
        repo.snapshot()
            .unwrap()
            .1
            .iter()
            .all(|r| r.document.schema.as_str() != "rx.diagnostic-result.v1")
    );
    let mut registry = Registry::new(repo);
    let id = registry.list().unwrap()[0].registration.id.clone();
    let mut c = Consumer::open(registry, id, catalog()).unwrap();
    let done = c.finish(&b.id, &r.id, &mut NoSource).unwrap();
    assert_eq!(done.run.unwrap().phase, RunPhase::Completed);
    assert!(done.result.is_some());
}
