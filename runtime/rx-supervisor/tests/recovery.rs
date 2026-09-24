#![cfg(unix)]
use rx_domain::types::*;
use rx_ports::{Repository, Transaction};
use rx_solution_catalog::DeviceCatalog;
use rx_storage::SqliteRepository;
use rx_supervisor::{
    Result,
    execution::{Application, Requirements},
    model::*,
    process::{Backend, OsProcesses, SpawnFailure},
    registered::{RegisteredSupervisor, catalog_reference},
    registration::*,
};
use sha2::{Digest as _, Sha256};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
fn n(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn now() -> TimePoint {
    TimePoint {
        clock_id: "unix-utc-ns".into(),
        ticks_ns: Counter(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64,
        ),
    }
}
fn support() -> DeviceCatalog {
    DeviceCatalog::decode(include_bytes!("../../../catalogs/device-support.v1.json")).unwrap()
}
fn program() -> Program {
    let path = PathBuf::from("/bin/sleep");
    Program {
        id: n("test/observer"),
        effect: Effect::NonActuating,
        executable_sha256: Digest::from_bytes(Sha256::digest(std::fs::read(&path).unwrap()).into()),
        executable: path,
        files: BTreeMap::new(),
        fixed_arguments: vec!["60".into()],
        arguments: BTreeMap::new(),
        ready: ReadyProbe::AliveOnly,
        functional_readiness: None,
        decision_policy: None,
        execution_requirements: Some(Requirements(BTreeMap::new())),
    }
}
fn plan() -> Plan {
    Plan {
        schema: n("rx.solutions-process-plan.v1"),
        id: id(),
        environment: Environment::Simulation,
        profiles: vec![],
        processes: vec![Process {
            id: n("observer"),
            program: program().id,
            parameters: BTreeMap::new(),
            depends_on: vec![],
            startup_timeout_ms: Counter(1000),
            shutdown_timeout_ms: Counter(1000),
            restart_limit: Counter(0),
            restart_backoff_ms: Counter(100),
        }],
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
        for i in b.owned_instances() {
            let _ = b.terminate(&i, true);
            let end = Instant::now() + Duration::from_secs(2);
            while Instant::now() < end {
                if matches!(b.exited(&i), Ok(Some(_))) {
                    let _ = b.forget_exited(&i);
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
            let r = f(tx)?;
            if fail.swap(false, Ordering::SeqCst) {
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
struct Policy {
    component: Id,
}
impl RecoveryAuthority for Policy {
    fn may_dispose(&self, r: &DispositionRequest) -> bool {
        let actor = match &r.evidence {
            RecoveryEvidence::OwnedChildExit { report, .. }
            | RecoveryEvidence::Investigation { report, .. } => Some(&report.actor),
            _ => None,
        };
        r.target.registration == self.component && actor == Some(&n("test/recovery-owner"))
    }
    fn may_resume(&self, d: &Disposition, r: &ResumeRequest) -> bool {
        d.target.registration == self.component && r.actor == n("test/recovery-owner")
    }
}
// Keep real child creation and store-reopen probes isolated from unrelated
// cases in this test process. Parallel macOS execution produced transient
// writer-lock refusals after drop; storage semantics and CAS tests are unchanged.
static REAL_PROCESS_CASES: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Case {
    dir: tempfile::TempDir,
    registry: Registry<FaultRepo>,
    owner: Owned,
    component: Id,
    old_plan: Plan,
    target: ObservationRef,
    fail: Arc<AtomicBool>,
}
fn setup() -> Case {
    let dir = tempfile::tempdir().unwrap();
    let fail = Arc::new(AtomicBool::new(false));
    let p = plan();
    let mut registry = Registry::new(FaultRepo {
        inner: SqliteRepository::open(dir.path().join("registry.db")).unwrap(),
        fail: fail.clone(),
    });
    let component = registry
        .register(Declaration {
            label: n("observer"),
            catalog: catalog_reference(&program()).unwrap(),
        })
        .unwrap()
        .registration
        .id;
    let owner = Owned::new(dir.path().join("old-logs"));
    let mut s = RegisteredSupervisor::open(
        SqliteRepository::open(dir.path().join("old.db")).unwrap(),
        owner.clone(),
        SoftwareOnly,
        p.clone(),
        [(program().id, program())].into(),
        &support(),
        registry,
        component.clone(),
    )
    .unwrap();
    s.tick().unwrap();
    s.tick().unwrap();
    let instance = s.state().unwrap().records[&n("observer")]
        .instance
        .clone()
        .unwrap();
    let (store, b, _, r) = s.into_parts();
    drop(store);
    drop(b);
    drop(r);
    let registry = Registry::new(FaultRepo {
        inner: SqliteRepository::open(dir.path().join("registry.db")).unwrap(),
        fail: fail.clone(),
    });
    let mut s = RegisteredSupervisor::open(
        SqliteRepository::open(dir.path().join("old.db")).unwrap(),
        Owned::new(dir.path().join("reopen-logs")),
        SoftwareOnly,
        p.clone(),
        [(program().id, program())].into(),
        &support(),
        registry,
        component.clone(),
    )
    .unwrap();
    assert_eq!(
        s.tick().unwrap().state.records[&n("observer")].phase,
        Phase::Unknown
    );
    let (store, b, _, mut registry) = s.into_parts();
    drop(store);
    drop(b);
    let target = registry.recovery_target(&component, &instance).unwrap();
    Case {
        dir,
        registry,
        owner,
        component,
        old_plan: p,
        target,
        fail,
    }
}
fn proof(c: &mut Case) -> OwnedExit {
    c.owner.terminate(&c.target.instance, true).unwrap();
    let end = Instant::now() + Duration::from_secs(3);
    while !matches!(c.owner.exited(&c.target.instance), Ok(Some(_))) {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(5));
    }
    c.owner
        .0
        .borrow_mut()
        .observe_recovery_exit(&c.target.instance)
        .unwrap()
}
fn request(c: &Case, w: OwnedExit) -> DispositionRequest {
    let report = RecoveryReport {
        actor: n("test/recovery-owner"),
        scope: n("host/direct-child-exit"),
        observed_at: w.observed_at().clone(),
        procedure: n("owned-child-exit-and-retirement"),
    };
    DispositionRequest {
        target: c.target.clone(),
        kind: DispositionKind::ConfirmedClosure,
        evidence: RecoveryEvidence::OwnedChildExit { report, witness: w },
    }
}
fn resume(d: &Disposition, p: &Plan) -> ResumeRequest {
    ResumeRequest {
        id: id(),
        disposition: d.id.clone(),
        actor: n("test/recovery-owner"),
        requested_at: now(),
        next_run: p.id.clone(),
    }
}
type Managed = RegisteredSupervisor<SqliteRepository, Owned, SoftwareOnly, FaultRepo>;
fn open_case(
    c: Case,
    p: Plan,
    permit: Option<ResumePermit>,
    label: &str,
) -> (tempfile::TempDir, Managed, Owned, Arc<AtomicBool>) {
    let b = Owned::new(c.dir.path().join(format!("{label}-logs")));
    let store = SqliteRepository::open(c.dir.path().join(format!("{label}.db"))).unwrap();
    let s = if let Some(token) = permit {
        RegisteredSupervisor::open_with_resume(
            store,
            b.clone(),
            SoftwareOnly,
            p,
            [(program().id, program())].into(),
            &support(),
            c.registry,
            c.component,
            token,
        )
    } else {
        RegisteredSupervisor::open(
            store,
            b.clone(),
            SoftwareOnly,
            p,
            [(program().id, program())].into(),
            &support(),
            c.registry,
            c.component,
        )
    }
    .unwrap();
    (c.dir, s, b, c.fail)
}

#[test]
fn weak_observations_and_default_authority_cannot_dispose() {
    let _scenario = REAL_PROCESS_CASES.lock().unwrap_or_else(|p| p.into_inner());
    let mut c = setup();
    let policy = Policy {
        component: c.component.clone(),
    };
    for evidence in [
        RecoveryEvidence::None,
        RecoveryEvidence::ObservationOnly {
            report: RecoveryReport {
                actor: n("test/recovery-owner"),
                scope: n("host/direct-child-exit"),
                observed_at: now(),
                procedure: n("observation"),
            },
            observation: WeakObservation::Timeout,
        },
        RecoveryEvidence::ObservationOnly {
            report: RecoveryReport {
                actor: n("test/recovery-owner"),
                scope: n("host/direct-child-exit"),
                observed_at: now(),
                procedure: n("observation"),
            },
            observation: WeakObservation::CurrentIdle,
        },
        RecoveryEvidence::ObservationOnly {
            report: RecoveryReport {
                actor: n("test/recovery-owner"),
                scope: n("host/direct-child-exit"),
                observed_at: now(),
                procedure: n("observation"),
            },
            observation: WeakObservation::ProcessAbsent,
        },
        RecoveryEvidence::ObservationOnly {
            report: RecoveryReport {
                actor: n("test/recovery-owner"),
                scope: n("host/direct-child-exit"),
                observed_at: now(),
                procedure: n("observation"),
            },
            observation: WeakObservation::ElapsedTime,
        },
    ] {
        let r = DispositionRequest {
            target: c.target.clone(),
            kind: DispositionKind::ConfirmedClosure,
            evidence,
        };
        let e = c.registry.dispose(&r, &policy).unwrap_err();
        println!("negative weak evidence {:?}: {e}", r.evidence);
    }
    assert!(
        c.owner
            .0
            .borrow_mut()
            .observe_recovery_exit(&c.target.instance)
            .is_err()
    );
    let w = proof(&mut c);
    let r = request(&c, w);
    let e = c.registry.dispose(&r, &DenyRecovery).unwrap_err();
    println!("negative default authority: {e}");
    assert!(
        c.registry
            .query(&c.component)
            .unwrap()
            .recovery
            .dispositions
            .is_empty()
    );
    assert!(
        c.owner
            .0
            .borrow_mut()
            .observe_recovery_exit(&c.target.instance)
            .is_err(),
        "absence after retirement cannot mint a second witness"
    );
}
#[test]
fn confirmed_disposition_preserves_unknown_and_does_not_automatically_open_assignment() {
    let _scenario = REAL_PROCESS_CASES.lock().unwrap_or_else(|p| p.into_inner());
    let mut c = setup();
    let original = c.registry.query(&c.component).unwrap().executions;
    let history = c.registry.history(&c.component).unwrap();
    let policy = Policy {
        component: c.component.clone(),
    };
    let w = proof(&mut c);
    let r = request(&c, w);
    let d = c.registry.dispose(&r, &policy).unwrap();
    let v = c.registry.query(&c.component).unwrap();
    assert_eq!(v.executions, original);
    assert_eq!(v.recovery.dispositions[0], d);
    assert!(matches!(
        v.recovery.new_execution,
        RecoveryGate::ExplicitResumeRequired
    ));
    assert_eq!(d.past_outcome, PastOutcome::Unresolved);
    assert_eq!(d.resource_recovery, ResourceRecovery::NotClaimed);
    assert_eq!(d.work_use_permission, WorkUsePermission::Unsupported);
    assert_eq!(
        c.registry.history(&c.component).unwrap()[..history.len()],
        history
    );
    let token_request = resume(&d, &plan());
    assert!(
        c.registry
            .request_resume(&c.component, token_request, &DenyRecovery)
            .is_err()
    );
    println!(
        "separate disposition and original: {}",
        serde_json::to_string(&v).unwrap()
    );
    let (_dir, mut s, b, _) = open_case(c, plan(), None, "automatic-denied");
    let report = s.tick().unwrap();
    println!("negative disposition alone: {:?}", report.blocked);
    assert_eq!(
        report.state.records[&n("observer")].phase,
        Phase::StartFailed
    );
    assert!(b.0.borrow().owned_instances().is_empty());
}
#[test]
fn explicit_resume_consumes_once_and_uses_new_instance_without_restoring_receipts() {
    let _scenario = REAL_PROCESS_CASES.lock().unwrap_or_else(|p| p.into_inner());
    let mut c = setup();
    let old_instance = c.target.instance.clone();
    let original = c.registry.query(&c.component).unwrap().executions[0].clone();
    let policy = Policy {
        component: c.component.clone(),
    };
    let w = proof(&mut c);
    let r = request(&c, w);
    let d = c.registry.dispose(&r, &policy).unwrap();
    let p = plan();
    let req = resume(&d, &p);
    let token = c
        .registry
        .request_resume(&c.component, req.clone(), &policy)
        .unwrap();
    // A retry may recover a still-unconsumed permit only with identical intent and authority.
    let retry = c
        .registry
        .request_resume(&c.component, req.clone(), &policy)
        .unwrap();
    assert!(
        c.registry
            .request_resume(&c.component, resume(&d, &plan()), &policy)
            .is_err()
    );
    let component = c.component.clone();
    let (dir, mut s, b, fail) = open_case(c, p.clone(), Some(token), "resumed");
    assert!(matches!(
        s.execution_admission().unwrap()[&n("observer")].application,
        Application::NotApplied
    ));
    let report = s.tick().unwrap();
    let next = report.state.records[&n("observer")]
        .instance
        .clone()
        .unwrap();
    assert_ne!(old_instance, next);
    assert!(b.owns(&next));
    assert!(matches!(
        report.execution_admission[&n("observer")].application,
        Application::ReportedAtStart { .. }
    ));
    let v = s.query().unwrap();
    assert_eq!(
        *v.executions
            .iter()
            .find(|e| e.binding.instance == old_instance)
            .unwrap(),
        original
    );
    assert_eq!(v.recovery.resume_requests[0].consumed_by, Some(next));
    println!(
        "explicit new execution: {}",
        serde_json::to_string(&v).unwrap()
    );
    let (store, backend, _, mut registry) = s.into_parts();
    drop(store);
    drop(backend);
    let e = registry
        .request_resume(&component, req, &policy)
        .unwrap_err();
    println!("negative second resume: {e}");
    let c = Case {
        dir,
        registry,
        owner: b,
        component,
        old_plan: p.clone(),
        target: r.target,
        fail,
    };
    let (_dir, mut replay, b, _) = open_case(c, p, Some(retry), "replay-denied");
    let report = replay.tick().unwrap();
    assert_eq!(
        report.state.records[&n("observer")].phase,
        Phase::StartFailed
    );
    assert!(b.0.borrow().owned_instances().is_empty());
    println!("negative consumed permit replay: {:?}", report.blocked);
}
#[test]
fn unable_to_resolve_is_not_success_or_permission_to_resume() {
    let _scenario = REAL_PROCESS_CASES.lock().unwrap_or_else(|p| p.into_inner());
    let mut c = setup();
    let original = c.registry.query(&c.component).unwrap().executions;
    let policy = Policy {
        component: c.component.clone(),
    };
    let finding = c.registry.investigate(&c.target).unwrap();
    let observed_at = finding.observed_at().clone();
    let mut r = DispositionRequest {
        target: c.target.clone(),
        kind: DispositionKind::ConfirmedClosure,
        evidence: RecoveryEvidence::Investigation {
            report: RecoveryReport {
                actor: n("test/recovery-owner"),
                scope: n("host/execution-investigation"),
                observed_at,
                procedure: n("kernel-investigation"),
            },
            finding: Box::new(finding),
        },
    };
    let e = c.registry.dispose(&r, &policy).unwrap_err();
    println!("negative investigation labeled confirmed: {e}");
    r.kind = DispositionKind::UnableToResolve;
    let d = c.registry.dispose(&r, &policy).unwrap();
    assert!(
        c.registry
            .request_resume(&c.component, resume(&d, &plan()), &policy)
            .is_err()
    );
    let mut forged = serde_json::to_value(&d).unwrap();
    forged["past_outcome"] = serde_json::json!("SUCCESS");
    assert!(serde_json::from_value::<Disposition>(forged).is_err());
    let v = c.registry.query(&c.component).unwrap();
    assert_eq!(v.executions, original);
    assert!(matches!(
        v.recovery.new_execution,
        RecoveryGate::BlockedUnresolved
    ));
    assert!(
        v.recovery
            .limitations
            .iter()
            .any(|s| s.contains("legacy records without it cannot be backfilled"))
    );
    println!(
        "unable-to-resolve preserves unknown: {}",
        serde_json::to_string(&v).unwrap()
    );
}
#[test]
fn witness_scope_time_target_actor_and_stale_revision_are_checked() {
    let _scenario = REAL_PROCESS_CASES.lock().unwrap_or_else(|p| p.into_inner());
    let mut c = setup();
    let policy = Policy {
        component: c.component.clone(),
    };
    let w = proof(&mut c);
    let mut r = request(&c, w);
    r.target.revision = Counter(r.target.revision.0 + 1);
    assert!(c.registry.dispose(&r, &policy).is_err());
    r.target = c.target.clone();
    if let RecoveryEvidence::OwnedChildExit { report, .. } = &mut r.evidence {
        report.actor = n("untrusted/actor");
    }
    assert!(c.registry.dispose(&r, &policy).is_err());
    if let RecoveryEvidence::OwnedChildExit { report, .. } = &mut r.evidence {
        report.actor = n("test/recovery-owner");
        report.scope = n("physical/result");
    }
    assert!(c.registry.dispose(&r, &policy).is_err());
    if let RecoveryEvidence::OwnedChildExit { report, .. } = &mut r.evidence {
        report.scope = n("host/direct-child-exit");
        report.observed_at.ticks_ns = Counter(0);
    }
    assert!(c.registry.dispose(&r, &policy).is_err());
    if let RecoveryEvidence::OwnedChildExit { report, witness } = &mut r.evidence {
        report.observed_at = witness.observed_at().clone();
    }
    r.target.instance = id();
    assert!(c.registry.dispose(&r, &policy).is_err());
    r.target = c.target.clone();
    assert!(
        c.registry
            .query(&c.component)
            .unwrap()
            .recovery
            .dispositions
            .is_empty()
    );
    c.registry.dispose(&r, &policy).unwrap();
    println!(
        "negative scope/time/target/actor/revision attempts all rejected before valid disposition"
    );
}
#[test]
fn disposition_and_resume_consumption_rollback_without_half_records() {
    let _scenario = REAL_PROCESS_CASES.lock().unwrap_or_else(|p| p.into_inner());
    let mut c = setup();
    let policy = Policy {
        component: c.component.clone(),
    };
    let w = proof(&mut c);
    let r = request(&c, w);
    let history = c.registry.history(&c.component).unwrap();
    c.fail.store(true, Ordering::SeqCst);
    assert!(c.registry.dispose(&r, &policy).is_err());
    assert!(
        c.registry
            .query(&c.component)
            .unwrap()
            .recovery
            .dispositions
            .is_empty()
    );
    assert_eq!(c.registry.history(&c.component).unwrap(), history);
    let d = c.registry.dispose(&r, &policy).unwrap();
    let p = plan();
    let token = c
        .registry
        .request_resume(&c.component, resume(&d, &p), &policy)
        .unwrap();
    let retry = token.clone();
    let component = c.component.clone();
    let target = c.target.clone();
    let (dir, mut s, b, fail) = open_case(c, p.clone(), Some(token), "rollback");
    fail.store(true, Ordering::SeqCst);
    let report = s.tick().unwrap();
    assert_eq!(
        report.state.records[&n("observer")].phase,
        Phase::StartFailed
    );
    assert!(b.0.borrow().owned_instances().is_empty());
    assert!(
        s.query().unwrap().recovery.resume_requests[0]
            .consumed_by
            .is_none()
    );
    assert_eq!(s.query().unwrap().executions.len(), 1);
    let (store, backend, _, registry) = s.into_parts();
    drop(store);
    drop(backend);
    let c = Case {
        dir,
        registry,
        owner: b,
        component,
        old_plan: p.clone(),
        target,
        fail,
    };
    let (_dir, mut s, b, _) = open_case(c, p, Some(retry), "retry");
    s.tick().unwrap();
    assert_eq!(b.0.borrow().owned_instances().len(), 1);
    println!("atomic rollback retained unconsumed request; explicit retry assigned once");
}
#[test]
fn disposed_observation_cannot_be_overwritten_by_an_old_supervisor() {
    let _scenario = REAL_PROCESS_CASES.lock().unwrap_or_else(|p| p.into_inner());
    let mut c = setup();
    let policy = Policy {
        component: c.component.clone(),
    };
    let w = proof(&mut c);
    let r = request(&c, w);
    c.registry.dispose(&r, &policy).unwrap();
    let original = c.registry.query(&c.component).unwrap().executions;
    let history = c.registry.history(&c.component).unwrap();
    let mut store = SqliteRepository::open(c.dir.path().join("old.db")).unwrap();
    store
        .transact(|tx| {
            let key = n("supervisor/state");
            let old = tx.get(&key)?.unwrap();
            let mut doc = old.document;
            doc.value["records"]["observer"]["error"] =
                serde_json::json!("late changed report after disposition");
            tx.put(&key, Some(old.revision), &doc)?;
            Ok(())
        })
        .unwrap();
    let result = RegisteredSupervisor::open(
        store,
        Owned::new(c.dir.path().join("late")),
        SoftwareOnly,
        c.old_plan,
        [(program().id, program())].into(),
        &support(),
        c.registry,
        c.component.clone(),
    );
    let e = result.err().unwrap();
    println!("negative historical overwrite: {e}");
    assert!(e.to_string().contains("instead of rewriting history"));
    let mut registry =
        Registry::new(SqliteRepository::open(c.dir.path().join("registry.db")).unwrap());
    assert_eq!(registry.query(&c.component).unwrap().executions, original);
    assert_eq!(registry.history(&c.component).unwrap(), history);
}
#[test]
fn old_run_prepared_instance_and_automatic_restart_budget_are_not_recovery_routes() {
    let _scenario = REAL_PROCESS_CASES.lock().unwrap_or_else(|p| p.into_inner());
    let mut c = setup();
    let policy = Policy {
        component: c.component.clone(),
    };
    let w = proof(&mut c);
    let r = request(&c, w);
    let d = c.registry.dispose(&r, &policy).unwrap();
    assert!(
        c.registry
            .request_resume(&c.component, resume(&d, &c.old_plan), &policy)
            .is_err()
    );
    let p = plan();
    let token = c
        .registry
        .request_resume(&c.component, resume(&d, &p), &policy)
        .unwrap();
    // Existing saved state (including any forged PREPARED old instance) must never be a resume target.
    let mut store = SqliteRepository::open(c.dir.path().join("old.db")).unwrap();
    store
        .transact(|tx| {
            let key = n("supervisor/state");
            let row = tx.get(&key)?.unwrap();
            let mut doc = row.document;
            doc.value["records"]["observer"]["phase"] = serde_json::json!("PREPARED");
            tx.put(&key, Some(row.revision), &doc)?;
            Ok(())
        })
        .unwrap();
    let result = RegisteredSupervisor::open_with_resume(
        store,
        Owned::new(c.dir.path().join("bad")),
        SoftwareOnly,
        p,
        [(program().id, program())].into(),
        &support(),
        c.registry,
        c.component.clone(),
        token,
    );
    let e = result.err().unwrap();
    println!("negative reused saved instance: {e}");
    assert!(e.to_string().contains("fresh execution store"));
    let registry = Registry::new(FaultRepo {
        inner: SqliteRepository::open(c.dir.path().join("registry.db")).unwrap(),
        fail: c.fail,
    });
    let mut p = plan();
    p.processes[0].restart_limit = Counter(1);
    // An otherwise valid pending token is deliberately reissued under its original request.
    let mut registry = registry;
    let record = registry
        .query(&c.component)
        .unwrap()
        .recovery
        .resume_requests[0]
        .clone();
    let token = registry
        .request_resume(&c.component, record.request, &policy)
        .unwrap();
    let result = RegisteredSupervisor::open_with_resume(
        SqliteRepository::open(c.dir.path().join("auto.db")).unwrap(),
        Owned::new(c.dir.path().join("auto")),
        SoftwareOnly,
        p,
        [(program().id, program())].into(),
        &support(),
        registry,
        c.component,
        token,
    );
    assert!(
        result
            .err()
            .unwrap()
            .to_string()
            .contains("automatic restart budget must be zero")
    );
}

#[test]
fn missing_original_pid_binding_requires_another_provider_not_an_inferred_closure() {
    let _scenario = REAL_PROCESS_CASES.lock().unwrap_or_else(|p| p.into_inner());
    let mut c = setup();
    let policy = Policy {
        component: c.component.clone(),
    };
    let w = proof(&mut c);
    let mut repo = c.registry.into_repository();
    repo.transact(|tx| {
        let key = n(&format!(
            "components/execution/{}/{}",
            c.component, c.target.instance
        ));
        let row = tx.get(&key)?.unwrap();
        let mut doc = row.document;
        doc.value["last_observed"]["pid"] = serde_json::Value::Null;
        tx.put(&key, Some(row.revision), &doc)?;
        Ok(())
    })
    .unwrap();
    c.registry = Registry::new(repo);
    c.target = c
        .registry
        .recovery_target(&c.component, &c.target.instance)
        .unwrap();
    let r = request(&c, w);
    let error = c.registry.dispose(&r, &policy).unwrap_err();
    assert!(error.to_string().contains("matching original instance/PID"));
    println!("negative missing original PID: {error}");
    assert!(
        c.registry
            .query(&c.component)
            .unwrap()
            .recovery
            .dispositions
            .is_empty()
    );
}
