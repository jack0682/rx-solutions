//! Test-only local capability simulation; the separate Linux scene proves the actual OS path.
use super::*;
use crate::{
    model::*,
    process::{Backend, SpawnFailure},
    registered::RegisteredSupervisor,
    registration::Registry,
};
use rx_domain::component::{Declaration, Registration, RegistrationState};
use rx_storage::SqliteRepository;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn n(s: &str) -> Name {
    Name::new(s).unwrap()
}
struct TestClock(Arc<AtomicU64>);
impl Clock for TestClock {
    fn now(&self) -> Result<TimePoint> {
        Ok(TimePoint {
            clock_id: "unit-clock".into(),
            ticks_ns: Counter(self.0.load(Ordering::SeqCst)),
        })
    }
}
struct Fake {
    calls: Arc<AtomicUsize>,
    done: Arc<AtomicBool>,
    owned: BTreeSet<Id>,
}
impl Backend for Fake {
    fn spawn(
        &mut self,
        l: &Launch,
        a: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<u32, SpawnFailure> {
        assert!(a());
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.owned.insert(l.instance.clone());
        Ok(4242)
    }
    fn pid(&self, id: &Id) -> Option<u32> {
        self.owned.contains(id).then_some(4242)
    }
    fn owns(&self, id: &Id) -> bool {
        self.owned.contains(id)
    }
    fn forget_exited(&mut self, id: &Id) -> Result<()> {
        self.owned.remove(id);
        Ok(())
    }
    fn exited(&mut self, id: &Id) -> Result<Option<Option<i32>>> {
        Ok((self.owned.contains(id) && self.done.load(Ordering::SeqCst)).then_some(Some(0)))
    }
    fn ready(&mut self, _: &Launch) -> Result<bool> {
        Ok(true)
    }
    fn terminate(&mut self, _: &Id, _: bool) -> Result<()> {
        self.done.store(true, Ordering::SeqCst);
        Ok(())
    }
}
fn prepared(registry: &mut Registry<SqliteRepository>) -> Prepared {
    let program = Program {
        id: n("unit-program"),
        effect: Effect::NonActuating,
        executable: "/unit/not-executed".into(),
        executable_sha256: Digest::from_bytes([1; 32]),
        files: BTreeMap::new(),
        fixed_arguments: vec![],
        arguments: BTreeMap::new(),
        ready: ReadyProbe::AliveOnly,
        execution_requirements: Some(crate::execution::Requirements(
            [(n("none"), crate::execution::Requirement::NotRequired)].into(),
        )),
        functional_readiness: None,
        decision_policy: None,
    };
    let reference = crate::registered::catalog_reference(&program).unwrap();
    let component = id();
    let node = data::Node {
        origin: None,
        registration: Registration {
            id: component.clone(),
            declaration: Declaration {
                label: n("unit"),
                catalog: reference.clone(),
            },
            state: RegistrationState::Accepted,
        },
        revision: Counter(1),
        instance: id(),
        selection: data::Selection {
            component,
            expected_revision: Counter(1),
            parameters: BTreeMap::new(),
            depends_on: vec![],
            startup_timeout_ms: Counter(1000),
            shutdown_timeout_ms: Counter(1000),
        },
    };
    let intent = data::Intent {
        id: id(),
        run: id(),
        owner: n("author"),
        supervisor: n("supervisor"),
        environment: data::Environment::Simulation,
        profiles: vec![],
        nodes: [(n("main"), node)].into(),
        created_at: TimePoint {
            clock_id: "unit-clock".into(),
            ticks_ns: Counter(1),
        },
    };
    let peer = data::Peer {
        registry: registry.platform_binding().unwrap(),
        id: id(),
        principal: n("supervisor"),
        peer_boot: id(),
        installation: id(),
        store_generation: id(),
        runtime_boot: id(),
        authentication_binding: Digest::from_bytes([2; 32]),
        enrollment: Digest::from_bytes([3; 32]),
    };
    let catalog = Catalog {
        release: Digest::from_bytes([4; 32]),
        programs: [(program.id.clone(), program)].into(),
        support: rx_solution_catalog::DeviceCatalog::decode(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../catalogs/device-support.v1.json"
        )))
        .unwrap(),
    };
    Prepared::build(intent, &peer, catalog, registry).unwrap()
}
fn grant(p: &Prepared) -> data::Grant {
    data::Grant {
        id: id(),
        assignment: p.intent.id.clone(),
        intent_digest: p.intent.digest().unwrap(),
        preparation_digest: p.offer.digest().unwrap(),
        peer: p.offer.peer.clone(),
        issued_at: TimePoint {
            clock_id: "unit-clock".into(),
            ticks_ns: Counter(1),
        },
        valid_until: TimePoint {
            clock_id: "unit-clock".into(),
            ticks_ns: Counter(1_000_000_000),
        },
    }
}
fn duplicate(p: &Prepared) -> Prepared {
    Prepared {
        intent: p.intent.clone(),
        offer: p.offer.clone(),
        plan: p.plan.clone(),
        catalog: Catalog {
            release: p.catalog.release,
            programs: p.catalog.programs.clone(),
            support: p.catalog.support.clone(),
        },
    }
}
fn database(path: &Path) -> SqliteRepository {
    SqliteRepository::open(path).unwrap()
}
#[test]
fn one_shot_owner_and_consumed_record_prevent_replay_and_history_rewrite() {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::new(database(&dir.path().join("registry.db")));
    let p = prepared(&mut registry);
    let same = duplicate(&p);
    let g = grant(&p);
    let clock: Arc<dyn Clock> = Arc::new(TestClock(Arc::new(AtomicU64::new(1))));
    let first = LiveGrant::new(g.clone(), &p, clock.clone()).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let done = Arc::new(AtomicBool::new(false));
    let backend = Fake {
        calls: calls.clone(),
        done: done.clone(),
        owned: BTreeSet::new(),
    };
    let mut manager = RegisteredSupervisor::open_platform(
        database(&dir.path().join("run.db")),
        backend,
        p,
        first,
        registry,
    )
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    manager.tick().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    manager.request_stop().unwrap();
    for _ in 0..4 {
        manager.tick().unwrap();
    }
    let observed = manager.platform_observation().unwrap();
    assert!(
        observed
            .nodes
            .values()
            .all(|v| v.state == data::ObservationState::Exited)
    );
    let (_store, backend, _authority, mut registry) = manager.into_parts();
    let component = same.intent.nodes[&n("main")].registration.id.clone();
    let before = registry.history(&component).unwrap();
    // Simulate an invalid second mint inside the test module: public Client refuses a second live token.
    let second = LiveGrant::new(g, &same, clock).unwrap();
    let result = RegisteredSupervisor::open_platform(
        database(&dir.path().join("duplicate.db")),
        backend,
        same,
        second,
        registry,
    );
    assert!(result.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let mut restored = Registry::new(database(&dir.path().join("registry.db")));
    assert_eq!(restored.history(&component).unwrap(), before);
    let catalog = restored
        .query(&component)
        .unwrap()
        .registration
        .registration
        .declaration
        .catalog;
    assert!(
        restored
            .register(Declaration {
                label: n("forbidden"),
                catalog
            })
            .is_err()
    );
}
#[test]
fn clock_expiry_wrong_registry_and_changed_launch_do_not_enter_the_backend() {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::new(database(&dir.path().join("registry.db")));
    let p = prepared(&mut registry);
    let ticks = Arc::new(AtomicU64::new(1));
    let clock: Arc<dyn Clock> = Arc::new(TestClock(ticks.clone()));
    let live = LiveGrant::new(grant(&p), &p, clock).unwrap();
    let authority = Authority(live.lease.clone());
    let process = &p.plan.processes[0];
    let launch = process
        .launch(
            &p.catalog.programs[&process.program],
            p.intent.nodes[&process.id].instance.clone(),
        )
        .unwrap();
    assert!(authority.may_start(&p.plan, process, &launch));
    let mut forged = launch.clone();
    forged.arguments.push("unapproved".into());
    assert!(!authority.may_start(&p.plan, process, &forged));
    ticks.store(1_000_000_000, Ordering::SeqCst);
    assert!(!authority.may_start(&p.plan, process, &launch));
    assert!(authority.may_stop(&p.plan, process, &launch));
    let mut another = Registry::new(database(&dir.path().join("another.db")));
    assert!(
        another
            .platform_preflight(&p.intent, &p.offer.peer)
            .is_err()
    );
}

fn recovery_fixture(
    stopped: bool,
) -> (
    tempfile::TempDir,
    Registry<SqliteRepository>,
    Prepared,
    data::Grant,
) {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::new(database(&dir.path().join("registration.db")));
    let prepared = prepared(&mut registry);
    let original = duplicate(&prepared);
    let grant = grant(&prepared);
    let clock: Arc<dyn Clock> = Arc::new(TestClock(Arc::new(AtomicU64::new(1))));
    let live = LiveGrant::new(grant.clone(), &prepared, clock).unwrap();
    let run = registry
        .platform_run_directory(&prepared.intent.id)
        .unwrap();
    std::fs::create_dir_all(&run).unwrap();
    let backend = Fake {
        calls: Arc::new(AtomicUsize::new(0)),
        done: Arc::new(AtomicBool::new(false)),
        owned: BTreeSet::new(),
    };
    let mut manager = RegisteredSupervisor::open_platform(
        database(&run.join("supervisor.db")),
        backend,
        prepared,
        live,
        registry,
    )
    .unwrap();
    manager.tick().unwrap();
    if stopped {
        manager.request_stop().unwrap();
        for _ in 0..4 {
            manager.tick().unwrap();
        }
    }
    let (store, _, _, registry) = manager.into_parts();
    store.close().unwrap();
    (dir, registry, original, grant)
}
fn original_assignment(prepared: &Prepared, grant: &data::Grant) -> data::Assignment {
    data::Assignment {
        intent: prepared.intent.clone(),
        phase: data::Phase::Running,
        content: Some(data::ContentReceipt {
            preparation: prepared.offer.clone(),
            basis: data::ContentBasis::EnrolledSupervisorVerifiedRelease,
            recorded_at: grant.issued_at.clone(),
        }),
        grant: Some(grant.clone()),
        observed: None,
        stop_requested: false,
    }
}
#[test]
fn source_recovery_reads_original_terminal_evidence_without_rewriting_history() {
    let (_dir, mut registry, prepared, grant) = recovery_fixture(true);
    let original = original_assignment(&prepared, &grant);
    let component = &prepared.intent.nodes[&n("main")].registration.id;
    let before = registry.history(component).unwrap();
    let mut peer = grant.peer.clone();
    peer.id = id();
    peer.peer_boot = id();
    peer.runtime_boot = id();
    let clock = TestClock(Arc::new(AtomicU64::new(1_000_000_000)));
    let result = recovery::inspect(
        original.clone(),
        &peer,
        duplicate(&prepared).catalog,
        &mut registry,
        &clock,
    )
    .unwrap();
    let value = serde_json::to_value(result).unwrap();
    assert_eq!(
        value["nodes"]["main"]["finding"]["basis"],
        "RECORDED_DIRECT_CHILD_EXIT"
    );
    assert_eq!(value["nodes"]["main"]["recorded_outcome"], "EXITED");
    assert_eq!(value["start_window_expired"], true);
    assert_eq!(value["original_peer"]["id"], grant.peer.id.to_string());
    assert_eq!(value["current_peer"]["id"], peer.id.to_string());
    assert!(
        value["operating_permission"]
            .as_str()
            .unwrap()
            .starts_with("NOT_GRANTED")
    );
    assert!(
        !value["nodes"]["main"]["residuals"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(registry.history(component).unwrap(), before);
    let run = registry
        .platform_run_directory(&original.intent.id)
        .unwrap();
    let mut store = database(&run.join("supervisor.db"));
    let (_, state) = crate::supervisor::persisted_state(&mut store).unwrap();
    assert_eq!(state.records[&n("main")].phase, Phase::Exited);
    store.close().unwrap();
    // No new live token is issued and the old P result was not changed by a source query.
    assert_eq!(original.phase, data::Phase::Running);
    assert!(original.observed.is_none());
}
#[test]
fn source_recovery_keeps_cold_running_without_birth_unverifiable() {
    let (_dir, mut registry, prepared, grant) = recovery_fixture(false);
    let component = &prepared.intent.nodes[&n("main")].registration.id;
    let before = registry.history(component).unwrap();
    let result = recovery::inspect(
        original_assignment(&prepared, &grant),
        &grant.peer,
        duplicate(&prepared).catalog,
        &mut registry,
        &TestClock(Arc::new(AtomicU64::new(1))),
    )
    .unwrap();
    let value = serde_json::to_value(result).unwrap();
    assert_eq!(value["start_window_expired"], false);
    assert_eq!(value["nodes"]["main"]["recorded_outcome"], "RUNNING");
    assert_eq!(
        value["nodes"]["main"]["finding"]["outcome"]["state"],
        "UNVERIFIABLE"
    );
    assert_eq!(registry.history(component).unwrap(), before);
    let mut store = database(
        &registry
            .platform_run_directory(&prepared.intent.id)
            .unwrap()
            .join("supervisor.db"),
    );
    let (_, state) = crate::supervisor::persisted_state(&mut store).unwrap();
    assert!(matches!(
        state.records[&n("main")].phase,
        Phase::Starting | Phase::ProcessReady
    ));
}
#[test]
fn source_recovery_refuses_changed_content_peer_journal_and_consumption() {
    let (dir, mut registry, prepared, grant) = recovery_fixture(true);
    let clock = TestClock(Arc::new(AtomicU64::new(1_000_000_000)));
    let original = original_assignment(&prepared, &grant);
    let mut content = duplicate(&prepared).catalog;
    content
        .programs
        .get_mut(&n("unit-program"))
        .unwrap()
        .fixed_arguments
        .push("changed".into());
    assert!(
        recovery::inspect(
            original.clone(),
            &grant.peer,
            content,
            &mut registry,
            &clock
        )
        .is_err()
    );
    let mut peer = grant.peer.clone();
    peer.registry = Digest::from_bytes([99; 32]);
    assert!(
        recovery::inspect(
            original.clone(),
            &peer,
            duplicate(&prepared).catalog,
            &mut registry,
            &clock
        )
        .is_err()
    );
    let mut another = Registry::new(database(&dir.path().join("another.db")));
    assert!(
        recovery::inspect(
            original.clone(),
            &grant.peer,
            duplicate(&prepared).catalog,
            &mut another,
            &clock
        )
        .is_err()
    );
    let mut changed = original.clone();
    changed.grant.as_mut().unwrap().id = id();
    assert!(
        recovery::inspect(
            changed,
            &grant.peer,
            duplicate(&prepared).catalog,
            &mut registry,
            &clock
        )
        .is_err()
    );
    let path = registry
        .platform_run_directory(&prepared.intent.id)
        .unwrap()
        .join("supervisor.db");
    let mut store = database(&path);
    // An already-held original manager store is not silently read through another file or manager.
    assert!(
        recovery::inspect(
            original.clone(),
            &grant.peer,
            duplicate(&prepared).catalog,
            &mut registry,
            &clock
        )
        .is_err()
    );
    use rx_ports::Repository;
    store
        .transact(|tx| {
            let mut row = tx.get(&n("supervisor/state"))?.unwrap();
            row.document.value["records"]["main"]["instance"] = serde_json::to_value(id()).unwrap();
            tx.put(&row.key, Some(row.revision), &row.document)?;
            Ok(())
        })
        .unwrap();
    store.close().unwrap();
    assert!(
        recovery::inspect(
            original,
            &grant.peer,
            duplicate(&prepared).catalog,
            &mut registry,
            &clock
        )
        .is_err()
    );
}

#[test]
fn original_consumed_execution_remains_investigable_after_a_new_assignment() {
    let (_dir, mut registry, prepared, first_grant) = recovery_fixture(true);
    let original = original_assignment(&prepared, &first_grant);
    let mut next_intent = prepared.intent.clone();
    next_intent.id = id();
    next_intent.run = id();
    for node in next_intent.nodes.values_mut() {
        node.instance = id();
    }
    let next = Prepared::build(
        next_intent,
        &prepared.offer.peer,
        duplicate(&prepared).catalog,
        &mut registry,
    )
    .unwrap();
    let next_run = registry.platform_run_directory(&next.intent.id).unwrap();
    std::fs::create_dir_all(&next_run).unwrap();
    let live = LiveGrant::new(
        grant(&next),
        &next,
        Arc::new(TestClock(Arc::new(AtomicU64::new(1)))),
    )
    .unwrap();
    let backend = Fake {
        calls: Arc::new(AtomicUsize::new(0)),
        done: Arc::new(AtomicBool::new(false)),
        owned: BTreeSet::new(),
    };
    let mut manager = RegisteredSupervisor::open_platform(
        database(&next_run.join("supervisor.db")),
        backend,
        next,
        live,
        registry,
    )
    .unwrap();
    manager.tick().unwrap();
    manager.request_stop().unwrap();
    for _ in 0..4 {
        manager.tick().unwrap();
    }
    let (store, _, _, mut registry) = manager.into_parts();
    store.close().unwrap();
    let component = &prepared.intent.nodes[&n("main")].registration.id;
    let before = registry.history(component).unwrap();
    let result = recovery::inspect(
        original,
        &first_grant.peer,
        duplicate(&prepared).catalog,
        &mut registry,
        &TestClock(Arc::new(AtomicU64::new(1_000_000_000))),
    )
    .unwrap();
    let value = serde_json::to_value(result).unwrap();
    assert_eq!(value["assignment"], prepared.intent.id.to_string());
    assert_eq!(
        value["nodes"]["main"]["finding"]["basis"],
        "RECORDED_DIRECT_CHILD_EXIT"
    );
    assert_eq!(registry.history(component).unwrap(), before);
}
