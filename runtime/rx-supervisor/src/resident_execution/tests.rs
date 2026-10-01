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
