use rx_domain::types::*;
use rx_ports::{Repository, Transaction};
use rx_solution_catalog::DeviceCatalog;
use rx_storage::SqliteRepository;
use rx_supervisor::{
    Error, Result, Supervisor,
    execution::Requirements,
    model::*,
    process::{Backend, SpawnFailure},
    registered::{RegisteredSupervisor, catalog_reference},
    registration::*,
};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
fn n(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn support() -> DeviceCatalog {
    DeviceCatalog::decode(include_bytes!("../../../catalogs/device-support.v1.json")).unwrap()
}
fn program() -> Program {
    Program {
        id: n("test/observation"),
        effect: Effect::NonActuating,
        executable: "/release/observer".into(),
        executable_sha256: Digest::from_bytes([1; 32]),
        files: BTreeMap::new(),
        fixed_arguments: vec![],
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
            id: n("observation"),
            program: program().id,
            parameters: [(n("port"), "18081".into())].into(),
            depends_on: vec![],
            startup_timeout_ms: Counter(100),
            shutdown_timeout_ms: Counter(100),
            restart_limit: Counter(0),
            restart_backoff_ms: Counter(100),
        }],
    }
}
fn declaration() -> Declaration {
    Declaration {
        label: n("sensor"),
        catalog: catalog_reference(&program()).unwrap(),
    }
}
#[derive(Default)]
struct Effects {
    owned: BTreeMap<Id, Option<Option<i32>>>,
    starts: Vec<Id>,
    stops: Vec<Id>,
    fail_after_spawn: Option<Arc<AtomicBool>>,
}
#[derive(Clone, Default)]
struct Fake(Rc<RefCell<Effects>>);
impl Backend for Fake {
    fn spawn(
        &mut self,
        l: &Launch,
        a: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<u32, SpawnFailure> {
        if !a() {
            return Err(SpawnFailure::NotStarted(Error::Invalid(
                "authority denied".into(),
            )));
        }
        let mut e = self.0.borrow_mut();
        e.starts.push(l.instance.clone());
        e.owned.insert(l.instance.clone(), None);
        if let Some(flag) = &e.fail_after_spawn {
            flag.store(true, Ordering::SeqCst);
        }
        Ok(4242) // Deliberate PID reuse: never used as ownership evidence.
    }
    fn pid(&self, i: &Id) -> Option<u32> {
        self.owns(i).then_some(4242)
    }
    fn owns(&self, i: &Id) -> bool {
        self.0.borrow().owned.contains_key(i)
    }
    fn forget_exited(&mut self, i: &Id) -> Result<()> {
        self.0.borrow_mut().owned.remove(i);
        Ok(())
    }
    fn exited(&mut self, i: &Id) -> Result<Option<Option<i32>>> {
        Ok(self.0.borrow().owned.get(i).copied().flatten())
    }
    fn ready(&mut self, l: &Launch) -> Result<bool> {
        Ok(self.owns(&l.instance))
    }
    fn terminate(&mut self, i: &Id, _: bool) -> Result<()> {
        let mut e = self.0.borrow_mut();
        e.stops.push(i.clone());
        e.owned.insert(i.clone(), Some(Some(0)));
        Ok(())
    }
}
type Managed = RegisteredSupervisor<SqliteRepository, Fake, SoftwareOnly, SqliteRepository>;
fn setup() -> (tempfile::TempDir, Managed, Fake, Plan, Id) {
    let dir = tempfile::tempdir().unwrap();
    let mut registry =
        Registry::new(SqliteRepository::open(dir.path().join("registration.db")).unwrap());
    let component = registry.register(declaration()).unwrap().registration.id;
    let p = plan();
    let backend = Fake::default();
    let s = RegisteredSupervisor::open(
        SqliteRepository::open(dir.path().join("execution.db")).unwrap(),
        backend.clone(),
        SoftwareOnly,
        p.clone(),
        [(program().id, program())].into(),
        &support(),
        registry,
        component.clone(),
    )
    .unwrap();
    (dir, s, backend, p, component)
}
fn stop(s: &mut Managed) {
    s.request_stop().unwrap();
    for _ in 0..3 {
        s.tick().unwrap();
    }
    assert!(s.tick().unwrap().all_exited);
}

#[test]
fn registration_exists_without_execution_or_plan_and_survives_repository_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("registry.db");
    let mut registry = Registry::new(SqliteRepository::open(&path).unwrap());
    let a = registry.register(declaration()).unwrap();
    let b = registry.register(declaration()).unwrap(); // Labels are not identities.
    assert_ne!(a.registration.id, b.registration.id);
    let before = registry.query(&a.registration.id).unwrap();
    assert!(before.executions.is_empty());
    assert_eq!(before.registration.registration.declaration, declaration());
    drop(registry);
    let mut registry = Registry::new(SqliteRepository::open(&path).unwrap());
    let after = registry.query(&a.registration.id).unwrap();
    println!(
        "registration without any execution: {}",
        serde_json::to_string(&after).unwrap()
    );
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(after).unwrap()
    );
    assert_eq!(registry.list().unwrap().len(), 2);
    assert_eq!(registry.history(&a.registration.id).unwrap().len(), 1);
}
#[test]
fn competing_updates_and_retired_reactivation_fail_without_erasing_history() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = Registry::new(SqliteRepository::open(dir.path().join("registry.db")).unwrap());
    let a = r.register(declaration()).unwrap();
    let uid = a.registration.id;
    let mut d = declaration();
    d.label = n("reviewed");
    let next = r.update(&uid, a.revision, d.clone()).unwrap();
    let error = r.update(&uid, a.revision, declaration()).unwrap_err();
    println!("negative competing CAS: {error}");
    assert!(matches!(error, rx_ports::StoreError::RevisionConflict(_)));
    assert_eq!(
        r.query(&uid).unwrap().registration.registration.declaration,
        d
    );
    let retired = r.retire(&uid, next.revision).unwrap();
    let before = r.history(&uid).unwrap();
    let error = r.update(&uid, retired.revision, declaration()).unwrap_err();
    println!("negative retired reactivation: {error}");
    assert_eq!(before, r.history(&uid).unwrap());
    assert_eq!(before.len(), 3);
    assert_eq!(
        r.query(&uid).unwrap().registration.registration.state,
        RegistrationState::Retired
    );
}
#[test]
fn normal_exit_unexpected_handle_loss_and_restart_preserve_registration() {
    for mode in ["normal", "loss", "restart"] {
        let (dir, mut s, b, p, uid) = setup();
        let original = s.query().unwrap().registration.registration;
        s.tick().unwrap();
        s.tick().unwrap();
        let running = s.state().unwrap();
        let instance = running.records[&n("observation")].instance.clone().unwrap();
        assert_ne!(uid, instance);
        if mode == "normal" {
            stop(&mut s);
        } else if mode == "loss" {
            b.0.borrow_mut().owned.clear();
            s.tick().unwrap();
        }
        let (repo, _, _, r) = s.into_parts();
        drop(repo);
        drop(r);
        let r = Registry::new(SqliteRepository::open(dir.path().join("registration.db")).unwrap());
        let fresh = Fake::default();
        let mut s = RegisteredSupervisor::open(
            SqliteRepository::open(dir.path().join("execution.db")).unwrap(),
            fresh.clone(),
            SoftwareOnly,
            p,
            [(program().id, program())].into(),
            &support(),
            r,
            uid.clone(),
        )
        .unwrap();
        let state = s.tick().unwrap();
        let view = s.query().unwrap();
        assert_eq!(view.registration.registration, original);
        assert_eq!(view.executions[0].binding.instance, instance);
        assert_eq!(
            state.state.records[&n("observation")].phase,
            if mode == "normal" {
                Phase::Exited
            } else {
                Phase::Unknown
            }
        );
        assert!(fresh.0.borrow().starts.is_empty());
        assert!(fresh.0.borrow().stops.is_empty());
        assert_eq!(
            view.execution_ownership,
            "NOT_ESTABLISHED_BY_PERSISTENT_RECORDS"
        );
        assert_eq!(
            view.functional_readiness.state(),
            rx_supervisor::use_assessment::ConditionState::NotEvaluated
        );
        assert!(!view.functional_readiness.conditions().is_empty());
        assert!(
            view.functional_readiness
                .conditions()
                .iter()
                .all(|c| c.state == rx_supervisor::use_assessment::ConditionState::NotEvaluated)
        );
        assert_eq!(
            view.work_use_permission.state(),
            rx_supervisor::use_assessment::WorkUseState::NotEvaluated
        );
        assert!(!view.work_use_permission.conditions().is_empty());
        assert!(
            view.work_use_permission
                .conditions()
                .iter()
                .all(|c| c.state == rx_supervisor::use_assessment::ConditionState::NotEvaluated)
        );
        println!(
            "{mode} after reopen: {}",
            serde_json::to_string(&view).unwrap()
        );
        assert!(s.history().unwrap().len() >= 4);
    }
}
#[test]
fn different_plan_same_name_pid_and_port_make_distinct_executions_of_one_registration() {
    let (dir, mut s, b, first_plan, uid) = setup();
    s.tick().unwrap();
    let first = s.state().unwrap().records[&n("observation")]
        .instance
        .clone()
        .unwrap();
    stop(&mut s);
    let (repo, _, _, r) = s.into_parts();
    drop(repo);
    let p = plan();
    assert_ne!(p.id, first_plan.id);
    let mut s = RegisteredSupervisor::open(
        SqliteRepository::open(dir.path().join("second-execution.db")).unwrap(),
        b.clone(),
        SoftwareOnly,
        p,
        [(program().id, program())].into(),
        &support(),
        r,
        uid.clone(),
    )
    .unwrap();
    s.tick().unwrap();
    let v = s.query().unwrap();
    assert_eq!(v.executions.len(), 2);
    let next = s.state().unwrap().records[&n("observation")]
        .instance
        .clone()
        .unwrap();
    assert_ne!(first, next);
    assert_ne!(uid, next);
    assert!(
        v.executions
            .iter()
            .all(|e| e.binding.registration == uid && e.last_observed.pid == Some(4242))
    );
    assert_eq!(b.0.borrow().starts.len(), 2);
    println!(
        "PID/name/port reuse: {}",
        serde_json::to_string(&v).unwrap()
    );
    stop(&mut s);
}
#[test]
fn retirement_blocks_new_assignment_but_keeps_running_history_and_unresolved_obligations() {
    let (_dir, mut s, b, _, _) = setup();
    s.tick().unwrap();
    let v = s.query().unwrap();
    s.retire(v.registration.revision).unwrap();
    assert_eq!(b.0.borrow().starts.len(), 1);
    assert!(b.0.borrow().stops.is_empty());
    let running = s.query().unwrap();
    assert_eq!(running.executions.len(), 1);
    assert_eq!(
        running.executions[0].last_observed.state,
        ExecutionState::Running
    );
    stop(&mut s);
    s.rearm_software().unwrap();
    let denied = s.tick().unwrap();
    assert_eq!(
        denied.state.records[&n("observation")].phase,
        Phase::StartFailed
    );
    assert_eq!(b.0.borrow().starts.len(), 1);
    println!("negative retired start: {:?}", denied.blocked);
    assert_eq!(s.query().unwrap().executions.len(), 1);
    let (_dir, mut s, b, _, _) = setup();
    s.tick().unwrap();
    b.0.borrow_mut().owned.clear();
    s.tick().unwrap();
    let old = s.query().unwrap();
    s.retire(old.registration.revision).unwrap();
    assert_eq!(
        s.query().unwrap().executions[0].last_observed.state,
        ExecutionState::Unknown
    );
    assert!(s.rearm_software().is_err());
    assert!(s.history().unwrap().len() >= 4);
}
#[test]
fn catalog_weakening_and_legacy_reopen_are_explained_and_leave_old_records_readable() {
    let (dir, mut s, _, p, uid) = setup();
    s.tick().unwrap();
    let (store, _, _, r) = s.into_parts();
    drop(store);
    let mut changed = program();
    changed.execution_requirements = None;
    let result = RegisteredSupervisor::open(
        SqliteRepository::open(dir.path().join("changed.db")).unwrap(),
        Fake::default(),
        SoftwareOnly,
        plan(),
        [(changed.id.clone(), changed)].into(),
        &support(),
        r,
        uid.clone(),
    );
    let error = result.err().unwrap();
    println!("negative undeclared catalog: {error}");
    let mut r = Registry::new(SqliteRepository::open(dir.path().join("registration.db")).unwrap());
    assert_eq!(r.query(&uid).unwrap().executions.len(), 1);
    let mut changed = program();
    changed.fixed_arguments.push("changed".into());
    let result = RegisteredSupervisor::open(
        SqliteRepository::open(dir.path().join("changed2.db")).unwrap(),
        Fake::default(),
        SoftwareOnly,
        p,
        [(changed.id.clone(), changed)].into(),
        &support(),
        r,
        uid,
    );
    assert!(
        result
            .err()
            .unwrap()
            .to_string()
            .contains("catalog declaration differs")
    );
    let path = dir.path().join("legacy.db");
    let mut old = program();
    old.execution_requirements = None;
    let p = plan();
    let mut legacy = Supervisor::open(
        SqliteRepository::open(&path).unwrap(),
        Fake::default(),
        SoftwareOnly,
        p.clone(),
        [(old.id.clone(), old)].into(),
        &support(),
    )
    .unwrap();
    legacy.tick().unwrap();
    let before = legacy.state().unwrap();
    drop(legacy);
    let result = Supervisor::open(
        SqliteRepository::open(&path).unwrap(),
        Fake::default(),
        SoftwareOnly,
        p,
        [(program().id, program())].into(),
        &support(),
    );
    let error = result.err().unwrap();
    println!("negative changed declaration reopen: {error}");
    assert!(error.to_string().contains("plan/catalog digest differs"));
    let mut repository = SqliteRepository::open(&path).unwrap();
    let rows = repository.snapshot().unwrap().1;
    assert_eq!(
        rows[0].document.value,
        serde_json::to_value(before).unwrap()
    );
}

struct FailCommit {
    inner: SqliteRepository,
    fail: Arc<AtomicBool>,
}
impl Repository for FailCommit {
    fn transact<T>(
        &mut self,
        f: impl FnOnce(&mut dyn Transaction) -> rx_ports::Result<T>,
    ) -> rx_ports::Result<T> {
        let fail = &self.fail;
        self.inner.transact(|tx| {
            let result = f(tx)?;
            if fail.swap(false, Ordering::SeqCst) {
                return Err(rx_ports::StoreError::Unavailable(
                    "injected commit failure".into(),
                ));
            }
            Ok(result)
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
fn failed_transaction_commits_neither_revision_nor_control_event_or_projection() {
    let dir = tempfile::tempdir().unwrap();
    let fail = Arc::new(AtomicBool::new(false));
    let mut r = Registry::new(FailCommit {
        inner: SqliteRepository::open(dir.path().join("registry.db")).unwrap(),
        fail: fail.clone(),
    });
    let v = r.register(declaration()).unwrap();
    let uid = v.registration.id;
    let history = r.history(&uid).unwrap();
    fail.store(true, Ordering::SeqCst);
    let error = r.retire(&uid, v.revision).unwrap_err();
    println!("negative atomic commit: {error}");
    assert_eq!(
        r.query(&uid).unwrap().registration.registration.state,
        RegistrationState::Accepted
    );
    assert_eq!(history, r.history(&uid).unwrap());
    let mut repository = r.into_repository();
    let (seq, rows) = repository.control_snapshot().unwrap();
    assert_eq!(seq, Counter(1));
    assert_eq!(rows[0].revision, Counter(1));
}
#[test]
fn failure_between_assignment_and_observation_is_unconfirmed_and_cannot_replay() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("registration.db");
    let fail = Arc::new(AtomicBool::new(false));
    let mut r = Registry::new(FailCommit {
        inner: SqliteRepository::open(&path).unwrap(),
        fail: fail.clone(),
    });
    let uid = r.register(declaration()).unwrap().registration.id;
    let b = Fake::default();
    b.0.borrow_mut().fail_after_spawn = Some(fail);
    let p = plan();
    let mut s = RegisteredSupervisor::open(
        SqliteRepository::open(dir.path().join("run.db")).unwrap(),
        b.clone(),
        SoftwareOnly,
        p.clone(),
        [(program().id, program())].into(),
        &support(),
        r,
        uid.clone(),
    )
    .unwrap();
    assert!(s.tick().is_err());
    assert_eq!(b.0.borrow().starts.len(), 1);
    assert_eq!(
        s.query().unwrap().executions[0].last_observed.state,
        ExecutionState::Assigned
    );
    let (repo, _, _, registry) = s.into_parts();
    drop(repo);
    drop(registry);
    let r = Registry::new(SqliteRepository::open(&path).unwrap());
    let fresh = Fake::default();
    let mut reopened = RegisteredSupervisor::open(
        SqliteRepository::open(dir.path().join("run.db")).unwrap(),
        fresh.clone(),
        SoftwareOnly,
        p,
        [(program().id, program())].into(),
        &support(),
        r,
        uid.clone(),
    )
    .unwrap();
    reopened.tick().unwrap();
    assert!(fresh.0.borrow().starts.is_empty());
    assert_eq!(
        reopened.query().unwrap().executions[0].last_observed.state,
        ExecutionState::Unknown
    );
    let (_, _, _, r) = reopened.into_parts();
    let mut another = RegisteredSupervisor::open(
        SqliteRepository::open(dir.path().join("another.db")).unwrap(),
        fresh.clone(),
        SoftwareOnly,
        plan(),
        [(program().id, program())].into(),
        &support(),
        r,
        uid,
    )
    .unwrap();
    let denied = another.tick().unwrap();
    println!("negative unresolved replay: {:?}", denied.blocked);
    assert_eq!(
        denied.state.records[&n("observation")].phase,
        Phase::StartFailed
    );
    assert!(fresh.0.borrow().starts.is_empty());
}

#[test]
fn missing_executable_does_not_deregister_the_component() {
    use sha2::{Digest as _, Sha256};
    let dir = tempfile::tempdir().unwrap();
    let executable = dir.path().join("observer");
    std::fs::write(&executable, b"temporary release fixture").unwrap();
    let mut program = program();
    program.executable = executable.clone();
    program.executable_sha256 =
        Digest::from_bytes(Sha256::digest(std::fs::read(&executable).unwrap()).into());
    let declaration = Declaration {
        label: n("missing-observer"),
        catalog: catalog_reference(&program).unwrap(),
    };
    let mut r = Registry::new(SqliteRepository::open(dir.path().join("registry.db")).unwrap());
    let accepted = r.register(declaration.clone()).unwrap();
    std::fs::rename(&executable, dir.path().join("retained-observer")).unwrap();
    let mut s = RegisteredSupervisor::open(
        SqliteRepository::open(dir.path().join("run.db")).unwrap(),
        rx_supervisor::process::OsProcesses::new(dir.path().join("logs")).unwrap(),
        SoftwareOnly,
        plan(),
        [(program.id.clone(), program)].into(),
        &support(),
        r,
        accepted.registration.id,
    )
    .unwrap();
    let report = s.tick().unwrap();
    assert_eq!(
        report.state.records[&n("observation")].phase,
        Phase::StartFailed
    );
    assert!(report.state.records[&n("observation")].pid.is_none());
    let view = s.query().unwrap();
    assert_eq!(view.registration.registration.declaration, declaration);
    assert_eq!(
        view.registration.registration.state,
        RegistrationState::Accepted
    );
    println!(
        "negative missing executable: {}",
        serde_json::to_string(&view).unwrap()
    );
}

#[test]
fn registration_never_replaces_lifecycle_authority() {
    struct Denied;
    impl LifecycleAuthority for Denied {
        fn may_start(&self, _: &Plan, _: &Process, _: &Launch) -> bool {
            false
        }
        fn may_stop(&self, _: &Plan, _: &Process, _: &Launch) -> bool {
            false
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let mut r = Registry::new(SqliteRepository::open(dir.path().join("registry.db")).unwrap());
    let uid = r.register(declaration()).unwrap().registration.id;
    let b = Fake::default();
    let mut s = RegisteredSupervisor::open(
        SqliteRepository::open(dir.path().join("run.db")).unwrap(),
        b.clone(),
        Denied,
        plan(),
        [(program().id, program())].into(),
        &support(),
        r,
        uid,
    )
    .unwrap();
    let report = s.tick().unwrap();
    assert!(!report.blocked.is_empty());
    assert!(b.0.borrow().starts.is_empty());
    assert!(s.query().unwrap().executions.is_empty());
    println!("negative authority: {:?}", report.blocked);
}

#[test]
fn historical_assignment_revision_survives_a_label_review_after_exit() {
    let (dir, mut managed, _, plan, component) = setup();
    managed.tick().unwrap();
    stop(&mut managed);
    let original = managed.query().unwrap().executions[0].binding.clone();
    let (store, _, _, mut registry) = managed.into_parts();
    drop(store);
    let accepted = registry.query(&component).unwrap().registration;
    let mut declaration = accepted.registration.declaration;
    declaration.label = n("reviewed-label");
    registry
        .update(&component, accepted.revision, declaration)
        .unwrap();
    let reopened = RegisteredSupervisor::open(
        SqliteRepository::open(dir.path().join("execution.db")).unwrap(),
        Fake::default(),
        SoftwareOnly,
        plan,
        [(program().id, program())].into(),
        &support(),
        registry,
        component,
    )
    .unwrap();
    assert_eq!(reopened.query().unwrap().executions[0].binding, original);
}
