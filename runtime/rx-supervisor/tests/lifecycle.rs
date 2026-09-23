use rx_domain::types::*;
use rx_ports::{Repository, Transaction};
use rx_solution_catalog::DeviceCatalog;
use rx_storage::SqliteRepository;
use rx_supervisor::{
    Error, Result, Supervisor,
    model::*,
    process::{Backend, SpawnFailure},
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::Duration,
};

#[path = "support/execution_retry.rs"]
mod execution_retry;
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn support() -> DeviceCatalog {
    DeviceCatalog::decode(include_bytes!("../../../catalogs/device-support.v1.json")).unwrap()
}
fn program(effect: Effect) -> Program {
    Program {
        functional_readiness: None,
        decision_policy: None,
        execution_requirements: None,
        id: name("test/service"),
        effect,
        executable: "/release/test-service".into(),
        executable_sha256: Digest::from_bytes([1; 32]),
        files: BTreeMap::new(),
        fixed_arguments: vec![],
        arguments: BTreeMap::new(),
        ready: ReadyProbe::AliveOnly,
    }
}
fn plan() -> Plan {
    Plan {
        schema: name("rx.solutions-process-plan.v1"),
        id: id(),
        environment: Environment::Simulation,
        profiles: vec![name("SIM-JTC-6DOF")],
        processes: vec![Process {
            id: name("main"),
            program: name("test/service"),
            parameters: BTreeMap::new(),
            depends_on: vec![],
            startup_timeout_ms: Counter(100),
            shutdown_timeout_ms: Counter(100),
            restart_limit: Counter(0),
            restart_backoff_ms: Counter(100),
        }],
    }
}
#[derive(Default)]
struct Effects {
    spawns: Vec<Id>,
    stops: Vec<(Id, bool)>,
    exits: BTreeMap<Id, Option<i32>>,
    uncertain: bool,
    stop_ignored: bool,
}
#[derive(Clone, Default)]
struct Fake(Arc<Mutex<Effects>>);
impl Backend for Fake {
    fn spawn(
        &mut self,
        l: &Launch,
        authorize: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<u32, SpawnFailure> {
        if !authorize() {
            return Err(SpawnFailure::NotStarted(Error::Invalid(
                "authority denied".into(),
            )));
        }
        let mut effects = self.0.lock().unwrap();
        effects.spawns.push(l.instance.clone());
        if effects.uncertain {
            return Err(SpawnFailure::Uncertain(Error::Reconciliation(
                "unknown spawn".into(),
            )));
        }
        Ok(effects.spawns.len() as u32 + 100)
    }
    fn pid(&self, id: &Id) -> Option<u32> {
        self.0
            .lock()
            .unwrap()
            .spawns
            .iter()
            .position(|i| i == id)
            .map(|i| i as u32 + 101)
    }
    fn owns(&self, id: &Id) -> bool {
        self.pid(id).is_some()
    }
    fn forget_exited(&mut self, id: &Id) -> Result<()> {
        assert!(self.0.lock().unwrap().exits.contains_key(id));
        Ok(())
    }
    fn exited(&mut self, id: &Id) -> Result<Option<Option<i32>>> {
        Ok(self.0.lock().unwrap().exits.get(id).cloned())
    }
    fn ready(&mut self, _: &Launch) -> Result<bool> {
        Ok(true)
    }
    fn terminate(&mut self, id: &Id, force: bool) -> Result<()> {
        let mut e = self.0.lock().unwrap();
        e.stops.push((id.clone(), force));
        if !e.stop_ignored {
            e.exits.insert(id.clone(), Some(0));
        }
        Ok(())
    }
}
#[derive(Clone)]
struct Authority {
    start: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}
impl LifecycleAuthority for Authority {
    fn may_start(&self, _: &Plan, _: &Process, _: &Launch) -> bool {
        self.start.load(Ordering::SeqCst)
    }
    fn may_stop(&self, _: &Plan, _: &Process, _: &Launch) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
}
struct FaultStore {
    inner: SqliteRepository,
    mode: Arc<AtomicU8>,
}
impl Repository for FaultStore {
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
    fn transact<T>(
        &mut self,
        operation: impl FnOnce(&mut dyn Transaction) -> rx_ports::Result<T>,
    ) -> rx_ports::Result<T> {
        let mode = self.mode.clone();
        let mut after = false;
        let result = self.inner.transact(|tx| {
            let before = tx.get(&name("supervisor/state"))?.map(|r| r.document.value);
            let result = operation(tx)?;
            let current = tx.get(&name("supervisor/state"))?.map(|r| r.document.value);
            let setting = mode.load(Ordering::SeqCst);
            if before != current {
                let phase = current
                    .as_ref()
                    .and_then(|v| v["records"]["main"]["phase"].as_str())
                    .unwrap_or("");
                let triggered = match setting {
                    1 | 2 => phase == "SPAWN_ENTERED",
                    3 | 4 => phase == "STARTING",
                    5 => current
                        .as_ref()
                        .is_some_and(|v| v["stop_requested"] == true),
                    6 => phase == "EXITED",
                    _ => false,
                };
                if triggered {
                    mode.store(0, Ordering::SeqCst);
                    if setting == 2 || setting == 4 {
                        after = true;
                    } else {
                        return Err(rx_ports::StoreError::Unavailable(
                            "injected before commit".into(),
                        ));
                    }
                }
            }
            Ok(result)
        })?;
        if after {
            Err(rx_ports::StoreError::Unavailable(
                "injected lost commit response".into(),
            ))
        } else {
            Ok(result)
        }
    }
}
fn setup(
    mode: Arc<AtomicU8>,
    effect: Effect,
) -> (
    tempfile::TempDir,
    Supervisor<FaultStore, Fake, Authority>,
    Fake,
    Authority,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = FaultStore {
        inner: SqliteRepository::open(dir.path().join("state.db")).unwrap(),
        mode,
    };
    let backend = Fake::default();
    let authority = Authority {
        start: Arc::new(AtomicBool::new(true)),
        stop: Arc::new(AtomicBool::new(true)),
    };
    let p = program(effect);
    let supervisor = Supervisor::open(
        store,
        backend.clone(),
        authority.clone(),
        plan(),
        [(p.id.clone(), p)].into_iter().collect(),
        &support(),
    )
    .unwrap();
    (dir, supervisor, backend, authority)
}
#[test]
fn launch_commit_boundaries_never_replay_an_uncertain_spawn() {
    for fault in [1, 2, 3, 4] {
        let mode = Arc::new(AtomicU8::new(0));
        let (_dir, mut s, b, _) = setup(mode.clone(), Effect::NonActuating);
        mode.store(fault, Ordering::SeqCst);
        assert!(s.tick().is_err());
        if fault == 2 {
            assert_eq!(b.0.lock().unwrap().spawns.len(), 0);
            assert_eq!(
                s.tick().unwrap().state.records[&name("main")].phase,
                Phase::Unknown
            );
            s.tick().unwrap();
            assert_eq!(b.0.lock().unwrap().spawns.len(), 0);
        } else {
            s.tick().unwrap();
            s.tick().unwrap();
            assert_eq!(b.0.lock().unwrap().spawns.len(), 1);
            assert_eq!(
                s.state().unwrap().records[&name("main")].phase,
                Phase::ProcessReady
            );
        }
    }
}
#[test]
fn supervisor_restart_keeps_live_or_uncertain_processes_unknown_and_never_signals_saved_pids() {
    let mode = Arc::new(AtomicU8::new(0));
    let (_dir, mut s, b, a) = setup(mode, Effect::NonActuating);
    s.tick().unwrap();
    s.tick().unwrap();
    let state = s.state().unwrap();
    let (repo, _, _) = s.into_parts();
    let mut p = plan();
    p.id = state.plan;
    let program = program(Effect::NonActuating);
    let mut recovered = Supervisor::open(
        repo,
        Fake::default(),
        a,
        p,
        [(program.id.clone(), program)].into_iter().collect(),
        &support(),
    )
    .unwrap();
    assert_eq!(
        recovered.tick().unwrap().state.records[&name("main")].phase,
        Phase::Unknown
    );
    recovered.request_stop().unwrap();
    assert!(!recovered.tick().unwrap().all_exited);
    assert_eq!(b.0.lock().unwrap().spawns.len(), 1);
    assert!(b.0.lock().unwrap().stops.is_empty());
}
#[test]
fn stop_authority_is_required_and_control_processes_are_never_force_killed() {
    let (_dir, mut s, b, a) = setup(
        Arc::new(AtomicU8::new(0)),
        Effect::RequiresPlatformAuthority,
    );
    s.tick().unwrap();
    s.tick().unwrap();
    a.stop.store(false, Ordering::SeqCst);
    s.request_stop().unwrap();
    assert!(!s.tick().unwrap().blocked.is_empty());
    assert!(b.0.lock().unwrap().stops.is_empty());
    a.stop.store(true, Ordering::SeqCst);
    b.0.lock().unwrap().stop_ignored = true;
    s.tick().unwrap();
    std::thread::sleep(Duration::from_millis(120));
    let status = s.tick().unwrap();
    assert!(!status.all_exited);
    assert!(
        status
            .blocked
            .iter()
            .any(|v| v.contains("forced termination not allowed"))
    );
    assert!(b.0.lock().unwrap().stops.iter().all(|(_, force)| !*force));
}
#[test]
fn failed_stop_persistence_still_prevents_new_processes_and_exit_observation_can_be_retried() {
    let mode = Arc::new(AtomicU8::new(0));
    let (_dir, mut s, b, _) = setup(mode.clone(), Effect::NonActuating);
    mode.store(5, Ordering::SeqCst);
    assert!(s.request_stop().is_err());
    s.tick().unwrap();
    assert!(b.0.lock().unwrap().spawns.is_empty());
    s.request_stop().unwrap();
    assert!(s.tick().unwrap().all_exited);
    s.rearm_software().unwrap();
    s.tick().unwrap();
    s.tick().unwrap();
    s.request_stop().unwrap();
    s.tick().unwrap();
    mode.store(6, Ordering::SeqCst);
    assert!(s.tick().is_err());
    assert!(s.tick().unwrap().all_exited);
}
#[test]
fn release_program_change_and_unsafe_plan_choices_are_rejected() {
    let mut p = plan();
    let program = program(Effect::NonActuating);
    let programs: BTreeMap<_, _> = [(program.id.clone(), program.clone())]
        .into_iter()
        .collect();
    let catalog = support();
    let digest = p.validate(&programs, &catalog).unwrap();
    let mut changed = programs.clone();
    changed
        .get_mut(&program.id)
        .unwrap()
        .fixed_arguments
        .push("changed".into());
    assert_ne!(p.validate(&changed, &catalog).unwrap(), digest);
    p.processes[0].depends_on.push(name("main"));
    assert!(p.validate(&programs, &catalog).is_err());
    p.processes[0].depends_on.clear();
    p.processes[0]
        .parameters
        .insert(name("executable"), "/bin/sh".into());
    assert!(p.validate(&programs, &catalog).is_err());
    p.processes[0].parameters.clear();
    p.profiles.push(name("not-supported"));
    assert!(p.validate(&programs, &catalog).is_err());
    p.profiles.pop();
    changed.get_mut(&program.id).unwrap().effect = Effect::RequiresPlatformAuthority;
    p.processes[0].restart_limit = Counter(1);
    assert!(p.validate(&changed, &catalog).is_err());
}
#[test]
fn uncertain_backend_result_is_not_a_known_start_failure() {
    let (_dir, mut s, b, _) = setup(Arc::new(AtomicU8::new(0)), Effect::NonActuating);
    b.0.lock().unwrap().uncertain = true;
    assert_eq!(
        s.tick().unwrap().state.records[&name("main")].phase,
        Phase::Unknown
    );
    s.tick().unwrap();
    assert_eq!(b.0.lock().unwrap().spawns.len(), 1);
}
#[test]
fn software_restart_budget_persists_until_an_explicit_clean_reactivation() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = plan();
    p.processes[0].restart_limit = Counter(1);
    let program = program(Effect::NonActuating);
    let b = Fake::default();
    let mut s = Supervisor::open(
        SqliteRepository::open(dir.path().join("state.db")).unwrap(),
        b.clone(),
        SoftwareOnly,
        p,
        [(program.id.clone(), program)].into_iter().collect(),
        &support(),
    )
    .unwrap();
    for _ in 0..2 {
        s.tick().unwrap();
        let instance = s.state().unwrap().records[&name("main")]
            .instance
            .clone()
            .unwrap();
        b.0.lock().unwrap().exits.insert(instance, Some(1));
        s.tick().unwrap();
        std::thread::sleep(Duration::from_millis(120));
    }
    assert!(!s.tick().unwrap().blocked.is_empty());
    assert_eq!(b.0.lock().unwrap().spawns.len(), 2);
    assert!(s.rearm_software().is_err());
    s.request_stop().unwrap();
    assert!(s.tick().unwrap().all_exited);
    s.rearm_software().unwrap();
    s.tick().unwrap();
    assert_eq!(b.0.lock().unwrap().spawns.len(), 3);
}

#[test]
fn dependencies_start_after_readiness_and_stop_before_their_provider() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = plan();
    let mut child = p.processes[0].clone();
    child.id = name("child");
    child.depends_on = vec![name("main")];
    p.processes.insert(0, child);
    let program = program(Effect::NonActuating);
    let b = Fake::default();
    let mut s = Supervisor::open(
        SqliteRepository::open(dir.path().join("state.db")).unwrap(),
        b.clone(),
        SoftwareOnly,
        p,
        [(program.id.clone(), program)].into_iter().collect(),
        &support(),
    )
    .unwrap();
    s.tick().unwrap();
    assert_eq!(b.0.lock().unwrap().spawns.len(), 1);
    s.tick().unwrap();
    assert_eq!(b.0.lock().unwrap().spawns.len(), 1);
    s.tick().unwrap();
    s.tick().unwrap();
    let before = s.state().unwrap();
    let child = before.records[&name("child")].instance.clone().unwrap();
    let parent = before.records[&name("main")].instance.clone().unwrap();
    s.request_stop().unwrap();
    for _ in 0..4 {
        s.tick().unwrap();
    }
    assert!(s.tick().unwrap().all_exited);
    assert_eq!(
        b.0.lock()
            .unwrap()
            .stops
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>(),
        vec![child, parent]
    );
}
#[test]
fn default_authority_does_not_start_control_owners_and_other_stores_are_not_claimed() {
    let dir = tempfile::tempdir().unwrap();
    let program = program(Effect::RequiresPlatformAuthority);
    let programs: BTreeMap<_, _> = [(program.id.clone(), program)].into_iter().collect();
    let backend = Fake::default();
    let mut s = Supervisor::open(
        SqliteRepository::open(dir.path().join("state.db")).unwrap(),
        backend.clone(),
        SoftwareOnly,
        plan(),
        programs.clone(),
        &support(),
    )
    .unwrap();
    assert!(!s.tick().unwrap().blocked.is_empty());
    assert!(backend.0.lock().unwrap().spawns.is_empty());
    let foreign = dir.path().join("foreign.db");
    let mut store = SqliteRepository::open(&foreign).unwrap();
    store
        .transact(|tx| {
            tx.put(
                &name("other/record"),
                None,
                &rx_ports::Document {
                    schema: name("rx.test.v1"),
                    value: serde_json::json!({"preserved":true}),
                },
            )?;
            Ok(())
        })
        .unwrap();
    assert!(
        Supervisor::open(
            store,
            Fake::default(),
            SoftwareOnly,
            plan(),
            programs,
            &support()
        )
        .is_err()
    );
    let rows = SqliteRepository::open(foreign)
        .unwrap()
        .snapshot()
        .unwrap()
        .1;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].document.value["preserved"], true);
}
