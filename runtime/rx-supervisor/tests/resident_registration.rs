use rx_domain::types::*;
use rx_ports::{Document, Repository, Transaction};
use rx_solution_catalog::DeviceCatalog;
use rx_storage::SqliteRepository;
use rx_supervisor::{
    Error, Result, execution,
    model::*,
    process::{Backend, SpawnFailure},
    registered::RegisteredSupervisor,
    registration::*,
};
use std::{cell::RefCell, collections::BTreeMap, path::Path, rc::Rc};

fn n(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn catalog() -> DeviceCatalog {
    DeviceCatalog::decode(include_bytes!("../../../catalogs/device-support.v1.json")).unwrap()
}
fn program() -> Program {
    Program {
        id: n("test/status"),
        effect: Effect::NonActuating,
        executable: "/release/status".into(),
        executable_sha256: Digest::from_bytes([1; 32]),
        files: BTreeMap::new(),
        fixed_arguments: vec![],
        arguments: BTreeMap::new(),
        ready: ReadyProbe::AliveOnly,
        execution_requirements: Some(execution::Requirements(BTreeMap::new())),
        functional_readiness: None,
        decision_policy: None,
    }
}
fn plan() -> Plan {
    Plan {
        schema: n("rx.solutions-process-plan.v1"),
        id: id(),
        environment: Environment::Simulation,
        profiles: vec![],
        processes: ["first", "second"]
            .into_iter()
            .map(|s| Process {
                id: n(s),
                program: program().id,
                parameters: BTreeMap::new(),
                depends_on: vec![],
                startup_timeout_ms: Counter(1000),
                shutdown_timeout_ms: Counter(100),
                restart_limit: Counter(0),
                restart_backoff_ms: Counter(100),
            })
            .collect(),
    }
}
#[derive(Default)]
struct Effects {
    owned: BTreeMap<Id, Option<Option<i32>>>,
    starts: Vec<Name>,
    stops: Vec<(Id, bool)>,
    held: bool,
    guarded: Option<GuardedObservation>,
}
#[derive(Clone, Default)]
struct Fake(Rc<RefCell<Effects>>);
impl Backend for Fake {
    fn spawn(
        &mut self,
        l: &Launch,
        allow: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<u32, SpawnFailure> {
        if !allow() {
            return Err(SpawnFailure::NotStarted(Error::Invalid("denied".into())));
        }
        let mut e = self.0.borrow_mut();
        e.starts.push(l.selection.clone());
        e.owned.insert(l.instance.clone(), None);
        Ok(4242)
    }
    fn owns(&self, i: &Id) -> bool {
        self.0.borrow().owned.contains_key(i)
    }
    fn pid(&self, i: &Id) -> Option<u32> {
        self.owns(i).then_some(4242)
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
    fn guarded_status(&mut self, _: &Launch) -> Result<Option<GuardedObservation>> {
        Ok(self.0.borrow().guarded.clone())
    }
    fn terminate(&mut self, i: &Id, force: bool) -> Result<()> {
        let mut e = self.0.borrow_mut();
        e.stops.push((i.clone(), force));
        if !e.held {
            e.owned.insert(i.clone(), Some(Some(0)));
        }
        Ok(())
    }
}
type Resident = RegisteredSupervisor<SqliteRepository, Fake, GuardedServices, SqliteRepository>;
fn open(root: &Path, plan: Plan, program: Program, fake: Fake) -> Result<Resident> {
    RegisteredSupervisor::open_resident(
        SqliteRepository::open(root.join("supervisor.db"))?,
        fake,
        GuardedServices,
        plan,
        [(program.id.clone(), program)].into(),
        &catalog(),
        Registry::new(SqliteRepository::open(root.join("registration.db"))?),
    )
}
fn edit(root: &Path, key: &str, change: impl FnOnce(&mut serde_json::Value)) {
    let mut store = SqliteRepository::open(root.join("registration.db")).unwrap();
    store
        .transact(|tx| {
            let old = tx.get(&n(key))?.unwrap();
            let mut doc = old.document;
            change(&mut doc.value);
            let row = tx.put(&n(key), Some(old.revision), &doc)?;
            tx.append_control(
                &id(),
                &row,
                &Document {
                    schema: n("test/mutation"),
                    value: serde_json::json!({"test":true}),
                },
            )?;
            Ok(())
        })
        .unwrap();
}
fn registry_snapshot(root: &Path) -> Vec<rx_ports::Record> {
    SqliteRepository::open(root.join("registration.db"))
        .unwrap()
        .snapshot()
        .unwrap()
        .1
}
fn denied(root: &Path, plan: Plan, program: Program, reason: &str) {
    let before = registry_snapshot(root);
    let fake = Fake::default();
    let error = open(root, plan, program, fake.clone())
        .err()
        .expect("must refuse")
        .to_string();
    println!("negative resident: {error}");
    assert!(error.contains(reason), "{error}");
    assert!(fake.0.borrow().starts.is_empty());
    assert!(fake.0.borrow().stops.is_empty());
    assert_eq!(before, registry_snapshot(root));
}

#[test]
fn resident_reopen_keeps_distinct_ids_and_unknown_without_pid_adoption() {
    let root = tempfile::tempdir().unwrap();
    let plan = plan();
    let fake = Fake::default();
    let mut s = open(root.path(), plan.clone(), program(), fake).unwrap();
    let before = s.registrations().unwrap();
    assert_ne!(
        before[&n("first")].registration.registration.id,
        before[&n("second")].registration.registration.id
    );
    s.tick().unwrap();
    s.tick().unwrap();
    let running = s.state().unwrap();
    drop(s);
    let fresh = Fake::default();
    let mut s = open(root.path(), plan, program(), fresh.clone()).unwrap();
    let after = s.registrations().unwrap();
    for selection in [n("first"), n("second")] {
        assert_eq!(
            before[&selection].registration.registration,
            after[&selection].registration.registration
        );
        assert_eq!(
            after[&selection].executions[0].last_observed.state,
            ExecutionState::Unknown
        );
        assert_eq!(
            after[&selection].executions[0].binding.instance,
            *running.records[&selection].instance.as_ref().unwrap()
        );
        assert_eq!(after[&selection].executions[0].binding.selection, selection);
    }
    s.tick().unwrap();
    assert!(fresh.0.borrow().starts.is_empty());
    assert!(fresh.0.borrow().stops.is_empty());
    assert!(s.query().is_err()); // Resident mode does not inherit single-component judgments.
}
#[test]
fn resident_missing_selection_is_not_automatically_registered() {
    let root = tempfile::tempdir().unwrap();
    let plan = plan();
    drop(open(root.path(), plan.clone(), program(), Fake::default()).unwrap());
    edit(root.path(), "components/resident-selections", |v| {
        v.as_array_mut().unwrap().pop();
    });
    denied(root.path(), plan, program(), "mapping missing");
}
#[test]
fn resident_catalog_change_does_not_rewrite_accepted_history() {
    let root = tempfile::tempdir().unwrap();
    let plan = plan();
    drop(open(root.path(), plan.clone(), program(), Fake::default()).unwrap());
    let mut changed = program();
    changed.fixed_arguments.push("changed".into());
    denied(root.path(), plan, changed, "catalog digest changed");
}
#[test]
fn resident_retirement_does_not_register_a_replacement() {
    let root = tempfile::tempdir().unwrap();
    let plan = plan();
    let s = open(root.path(), plan.clone(), program(), Fake::default()).unwrap();
    let accepted = s
        .registrations()
        .unwrap()
        .remove(&n("first"))
        .unwrap()
        .registration;
    drop(s);
    let mut r = Registry::new(SqliteRepository::open(root.path().join("registration.db")).unwrap());
    r.retire(&accepted.registration.id, accepted.revision)
        .unwrap();
    drop(r);
    denied(root.path(), plan, program(), "registration retired");
}
#[test]
fn resident_duplicate_registration_mapping_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let plan = plan();
    drop(open(root.path(), plan.clone(), program(), Fake::default()).unwrap());
    edit(root.path(), "components/resident-selections", |v| {
        v[1]["registration"] = v[0]["registration"].clone()
    });
    denied(
        root.path(),
        plan,
        program(),
        "duplicate resident registration mapping",
    );
}
#[test]
fn resident_duplicate_selection_mapping_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let plan = plan();
    drop(open(root.path(), plan.clone(), program(), Fake::default()).unwrap());
    edit(root.path(), "components/resident-selections", |v| {
        v[1]["selection"] = v[0]["selection"].clone()
    });
    denied(
        root.path(),
        plan,
        program(),
        "duplicate resident selection mapping",
    );
}
#[test]
fn resident_cross_selection_assignment_cannot_satisfy_reconciliation() {
    let root = tempfile::tempdir().unwrap();
    let plan = plan();
    let mut s = open(root.path(), plan.clone(), program(), Fake::default()).unwrap();
    s.tick().unwrap();
    let views = s.registrations().unwrap();
    let first = &views[&n("first")].executions[0];
    let key = format!(
        "components/execution/{}/{}",
        first.binding.registration, first.binding.instance
    );
    drop(s);
    edit(root.path(), &key, |v| {
        v["binding"]["selection"] = serde_json::json!("second")
    });
    denied(
        root.path(),
        plan,
        program(),
        "another run/selection/catalog",
    );
}
#[test]
fn resident_swapped_index_cannot_adopt_another_components_execution() {
    let root = tempfile::tempdir().unwrap();
    let plan = plan();
    let mut s = open(root.path(), plan.clone(), program(), Fake::default()).unwrap();
    s.tick().unwrap();
    drop(s);
    edit(root.path(), "components/resident-selections", |v| {
        let first = v[0]["registration"].clone();
        v[0]["registration"] = v[1]["registration"].clone();
        v[1]["registration"] = first;
    });
    denied(root.path(), plan, program(), "no registration assignment");
}
#[test]
fn resident_legacy_store_cannot_be_adopted_by_creating_registration() {
    let root = tempfile::tempdir().unwrap();
    let plan = plan();
    let mut old = rx_supervisor::Supervisor::open(
        SqliteRepository::open(root.path().join("supervisor.db")).unwrap(),
        Fake::default(),
        GuardedServices,
        plan.clone(),
        [(program().id, program())].into(),
        &catalog(),
    )
    .unwrap();
    old.tick().unwrap();
    drop(old);
    denied(root.path(), plan, program(), "selection mapping missing");
}

#[test]
fn resident_guarded_legacy_retains_dependency_stop_order_and_never_forces() {
    let root = tempfile::tempdir().unwrap();
    let mut plan = plan();
    plan.processes[1].depends_on.push(n("first"));
    let mut program = program();
    program.effect = Effect::ProtocolGuardedService;
    program.execution_requirements = None;
    program.ready = ReadyProbe::GuardedStatus(GuardedStatusBinding::Host {
        path: "/run/test/status.json".into(),
        installation: id(),
        host: n("host/test"),
        installation_identity: Digest::from_bytes([2; 32]),
    });
    let fake = Fake::default();
    fake.0.borrow_mut().guarded = Some(GuardedObservation {
        state: GuardedState::Ready,
        sequence: Counter(1),
        observed_at: TimePoint {
            clock_id: "test".into(),
            ticks_ns: Counter(1),
        },
        payload_digest: Digest::from_bytes([3; 32]),
    });
    let mut s = open(root.path(), plan, program, fake.clone()).unwrap();
    for _ in 0..4 {
        s.tick().unwrap();
    }
    assert_eq!(fake.0.borrow().starts, vec![n("first"), n("second")]);
    for status in s.execution_admission().unwrap().values() {
        assert!(status.requested.is_none());
        assert!(matches!(
            status.application,
            execution::Application::LegacyNotDeclared
        ));
    }
    let state = s.state().unwrap();
    let first = state.records[&n("first")].instance.clone().unwrap();
    let second = state.records[&n("second")].instance.clone().unwrap();
    fake.0.borrow_mut().held = true;
    s.request_stop().unwrap();
    s.tick().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(110));
    s.tick().unwrap();
    assert_eq!(fake.0.borrow().stops, vec![(second.clone(), false)]);
    {
        let mut e = fake.0.borrow_mut();
        e.held = false;
        e.owned.insert(second, Some(Some(0)));
        e.guarded.as_mut().unwrap().state = GuardedState::Stopped {
            reconciliation_required: false,
        };
    }
    for _ in 0..3 {
        s.tick().unwrap();
    }
    let stopped = s.tick().unwrap();
    assert!(stopped.all_exited && stopped.guarded_shutdown_confirmed);
    assert_eq!(fake.0.borrow().stops.last(), Some(&(first, false)));
    assert!(fake.0.borrow().stops.iter().all(|(_, force)| !force));
}

struct FailIndex {
    prefix: &'static str,
    inner: SqliteRepository,
    fail: Rc<RefCell<bool>>,
}
impl Repository for FailIndex {
    fn transact<T>(
        &mut self,
        f: impl FnOnce(&mut dyn Transaction) -> rx_ports::Result<T>,
    ) -> rx_ports::Result<T> {
        self.inner.transact(|tx| {
            f(&mut IndexTransaction {
                inner: tx,
                hit: self.fail.clone(),
                prefix: self.prefix,
            })
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

struct IndexTransaction<'a> {
    prefix: &'static str,
    inner: &'a mut dyn rx_ports::Transaction,
    hit: Rc<RefCell<bool>>,
}
impl rx_ports::Transaction for IndexTransaction<'_> {
    fn put(
        &mut self,
        key: &Name,
        expected: Option<Counter>,
        doc: &Document,
    ) -> rx_ports::Result<rx_ports::Record> {
        if key.as_str().starts_with(self.prefix) {
            // Both registrations are staged, but the index has not been written.
            assert_eq!(self.inner.scan("components/registration/")?.len(), 2);
            *self.hit.borrow_mut() = true;
            return Err(rx_ports::StoreError::Unavailable(format!(
                "injected between registration and write to {}",
                self.prefix
            )));
        }
        self.inner.put(key, expected, doc)
    }
    fn get(&mut self, key: &Name) -> rx_ports::Result<Option<rx_ports::Record>> {
        self.inner.get(key)
    }
    fn scan(&mut self, prefix: &str) -> rx_ports::Result<Vec<rx_ports::Record>> {
        self.inner.scan(prefix)
    }
    fn control_head(&mut self) -> rx_ports::Result<Counter> {
        self.inner.control_head()
    }
    fn append_control(
        &mut self,
        event: &Id,
        entity: &rx_ports::Record,
        doc: &Document,
    ) -> rx_ports::Result<Counter> {
        self.inner.append_control(event, entity, doc)
    }
    fn outbox(&mut self, id: &Id) -> rx_ports::Result<Option<rx_ports::OutboxRecord>> {
        self.inner.outbox(id)
    }
    fn lookup(
        &mut self,
        scope: &rx_ports::RequestScope,
    ) -> rx_ports::Result<Option<rx_ports::SavedRequest>> {
        self.inner.lookup(scope)
    }
    fn remember(
        &mut self,
        scope: &rx_ports::RequestScope,
        request: &rx_ports::SavedRequest,
    ) -> rx_ports::Result<()> {
        self.inner.remember(scope, request)
    }
    fn append(&mut self, id: &Id, doc: &Document) -> rx_ports::Result<Counter> {
        self.inner.append(id, doc)
    }
    fn enqueue(&mut self, id: &Id, doc: &Document) -> rx_ports::Result<()> {
        self.inner.enqueue(id, doc)
    }
    fn transition_outbox(
        &mut self,
        id: &Id,
        expected: rx_ports::OutboxState,
        next: rx_ports::OutboxState,
    ) -> rx_ports::Result<()> {
        self.inner.transition_outbox(id, expected, next)
    }
}
#[test]
fn resident_failure_between_registration_and_index_rolls_back_every_record() {
    let root = tempfile::tempdir().unwrap();
    let hit = Rc::new(RefCell::new(false));
    let fake = Fake::default();
    let registry = Registry::new(FailIndex {
        prefix: "components/resident-selections",
        inner: SqliteRepository::open(root.path().join("registration.db")).unwrap(),
        fail: hit.clone(),
    });
    let result = RegisteredSupervisor::open_resident(
        SqliteRepository::open(root.path().join("supervisor.db")).unwrap(),
        fake.clone(),
        GuardedServices,
        plan(),
        [(program().id, program())].into(),
        &catalog(),
        registry,
    );
    let error = result.err().unwrap().to_string();
    assert!(error.contains("injected between"));
    assert!(*hit.borrow());
    assert!(fake.0.borrow().starts.is_empty());
    let mut store = SqliteRepository::open(root.path().join("registration.db")).unwrap();
    assert!(store.snapshot().unwrap().1.is_empty());
    assert_eq!(store.control_snapshot().unwrap().0, Counter(0));
    drop(store);
    let reopened = open(root.path(), plan(), program(), Fake::default()).unwrap();
    assert_eq!(reopened.registrations().unwrap().len(), 2);
}

#[test]
fn resident_failed_assignment_never_reaches_the_os_backend() {
    let root = tempfile::tempdir().unwrap();
    let hit = Rc::new(RefCell::new(false));
    let fake = Fake::default();
    let registry = Registry::new(FailIndex {
        prefix: "components/execution/",
        inner: SqliteRepository::open(root.path().join("registration.db")).unwrap(),
        fail: hit.clone(),
    });
    let mut resident = RegisteredSupervisor::open_resident(
        SqliteRepository::open(root.path().join("supervisor.db")).unwrap(),
        fake.clone(),
        GuardedServices,
        plan(),
        [(program().id, program())].into(),
        &catalog(),
        registry,
    )
    .unwrap();
    let result = resident.tick().unwrap();
    assert!(*hit.borrow());
    assert!(fake.0.borrow().starts.is_empty());
    assert!(
        result
            .state
            .records
            .values()
            .all(|r| r.phase == Phase::StartFailed && r.pid.is_none())
    );
    assert!(
        resident
            .registrations()
            .unwrap()
            .values()
            .all(|v| v.executions.is_empty())
    );
}
