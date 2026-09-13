use rx_domain::types::*;
use rx_solution_catalog::DeviceCatalog;
use rx_storage::SqliteRepository;
use rx_supervisor::{
    Error, Result, Supervisor,
    model::*,
    process::{Backend, SpawnFailure},
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

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
        id: n("test/guarded"),
        effect: Effect::ProtocolGuardedService,
        executable: "/release/daemon".into(),
        executable_sha256: Digest::from_bytes([1; 32]),
        files: BTreeMap::new(),
        fixed_arguments: vec![],
        arguments: BTreeMap::new(),
        ready: ReadyProbe::GuardedStatus(GuardedStatusBinding::Host {
            path: "/run/test/host.json".into(),
            installation: id(),
            host: n("host/a"),
            installation_identity: Digest::from_bytes([2; 32]),
        }),
    }
}
fn plan() -> Plan {
    Plan {
        schema: n("rx.solutions-process-plan.v1"),
        id: id(),
        environment: Environment::Simulation,
        profiles: vec![],
        processes: vec![Process {
            id: n("main"),
            program: n("test/guarded"),
            parameters: BTreeMap::new(),
            depends_on: vec![],
            startup_timeout_ms: Counter(1000),
            shutdown_timeout_ms: Counter(100),
            restart_limit: Counter(0),
            restart_backoff_ms: Counter(100),
        }],
    }
}
fn observation(state: GuardedState) -> GuardedObservation {
    GuardedObservation {
        state,
        sequence: Counter(1),
        observed_at: TimePoint {
            clock_id: "test-clock".into(),
            ticks_ns: Counter(100),
        },
        payload_digest: Digest::from_bytes([3; 32]),
    }
}
#[derive(Default)]
struct Effects {
    instance: Option<Id>,
    observation: Option<GuardedObservation>,
    code: Option<Option<i32>>,
    signals: Vec<bool>,
    held: bool,
    spawns: usize,
    fail_terminations: usize,
}
#[derive(Clone, Default)]
struct Fake(Arc<Mutex<Effects>>);
impl Backend for Fake {
    fn spawn(
        &mut self,
        launch: &Launch,
        allow: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<u32, SpawnFailure> {
        if !allow() {
            return Err(SpawnFailure::NotStarted(Error::Invalid("denied".into())));
        }
        let mut e = self.0.lock().unwrap();
        e.instance = Some(launch.instance.clone());
        e.spawns += 1;
        Ok(123)
    }
    fn pid(&self, instance: &Id) -> Option<u32> {
        self.owns(instance).then_some(123)
    }
    fn owns(&self, instance: &Id) -> bool {
        self.0.lock().unwrap().instance.as_ref() == Some(instance)
    }
    fn forget_exited(&mut self, _: &Id) -> Result<()> {
        self.0.lock().unwrap().instance = None;
        Ok(())
    }
    fn exited(&mut self, _: &Id) -> Result<Option<Option<i32>>> {
        Ok(self.0.lock().unwrap().code)
    }
    fn ready(&mut self, _: &Launch) -> Result<bool> {
        Ok(true)
    } // Deliberately insufficient for guarded readiness.
    fn guarded_status(&mut self, _: &Launch) -> Result<Option<GuardedObservation>> {
        Ok(self.0.lock().unwrap().observation.clone())
    }
    fn terminate(&mut self, _: &Id, force: bool) -> Result<()> {
        let mut e = self.0.lock().unwrap();
        e.signals.push(force);
        if e.fail_terminations > 0 {
            e.fail_terminations -= 1;
            return Err(Error::Reconciliation("first TERM delivery failed".into()));
        }
        if !e.held {
            e.code = Some(Some(0));
        }
        Ok(())
    }
}

#[test]
fn failed_first_term_retries_the_same_owned_service_without_waiting_for_timeout() {
    let root = tempfile::tempdir().unwrap();
    let fake = Fake::default();
    let program = program();
    fake.0.lock().unwrap().observation = Some(observation(GuardedState::Ready));
    let mut supervisor = Supervisor::open(
        SqliteRepository::open(root.path().join("s.db")).unwrap(),
        fake.clone(),
        GuardedServices,
        plan(),
        [(program.id.clone(), program)].into(),
        &catalog(),
    )
    .unwrap();
    supervisor.tick().unwrap();
    supervisor.tick().unwrap();
    supervisor.request_stop().unwrap();
    let instance = fake.0.lock().unwrap().instance.clone().unwrap();
    {
        let mut e = fake.0.lock().unwrap();
        e.fail_terminations = 1;
        e.observation = Some(observation(GuardedState::Stopped {
            reconciliation_required: false,
        }));
    }
    assert!(supervisor.tick().is_err());
    assert_eq!(
        supervisor.state().unwrap().records[&n("main")].phase,
        Phase::StopRequested
    );
    assert_eq!(fake.0.lock().unwrap().instance.as_ref(), Some(&instance));
    assert!(fake.0.lock().unwrap().code.is_none());
    supervisor.tick().unwrap();
    assert_eq!(fake.0.lock().unwrap().signals, vec![false, false]);
    let stopped = supervisor.tick().unwrap();
    assert!(stopped.all_exited && stopped.guarded_shutdown_confirmed);
    assert_eq!(fake.0.lock().unwrap().spawns, 1);
}

#[test]
fn guarded_recipe_cannot_use_alive_only_parameters_restart_or_physical_authority_fallback() {
    let mut p = plan();
    let mut program = program();
    assert!(
        p.validate(&[(program.id.clone(), program.clone())].into(), &catalog())
            .is_ok()
    );
    let launch = p.processes[0].launch(&program, id()).unwrap();
    assert!(!SoftwareOnly.may_start(&p, &p.processes[0], &launch));
    assert!(GuardedServices.may_start(&p, &p.processes[0], &launch));
    p.processes[0].restart_limit = Counter(1);
    assert!(
        p.validate(&[(program.id.clone(), program.clone())].into(), &catalog())
            .is_err()
    );
    p.processes[0].restart_limit = Counter(0);
    p.processes[0]
        .parameters
        .insert(n("executable"), "/bin/sh".into());
    assert!(
        p.validate(&[(program.id.clone(), program.clone())].into(), &catalog())
            .is_err()
    );
    p.processes[0].parameters.clear();
    program.ready = ReadyProbe::AliveOnly;
    assert!(
        p.validate(&[(program.id.clone(), program.clone())].into(), &catalog())
            .is_err()
    );
    program.effect = Effect::RequiresPlatformAuthority;
    let launch = p.processes[0].launch(&program, id()).unwrap();
    assert!(!GuardedServices.may_start(&p, &p.processes[0], &launch));
    assert!(!GuardedServices.may_stop(&p, &p.processes[0], &launch));
}

#[test]
fn guarded_paths_are_per_instance_and_scope_is_preserved() {
    let program = program();
    let p = plan();
    let first = p.processes[0].launch(&program, id()).unwrap();
    let second = p.processes[0].launch(&program, id()).unwrap();
    let (ReadyProbe::GuardedStatus(a), ReadyProbe::GuardedStatus(b)) = (first.ready, second.ready)
    else {
        panic!("guarded binding");
    };
    assert_eq!(a.scope(), b.scope());
    assert_ne!(a.path(), b.path());
    assert_eq!(a.path().parent(), Some(std::path::Path::new("/run/test")));
    assert!(a.path().to_string_lossy().contains(first.instance.as_str()));
}

#[test]
fn guarded_readiness_requires_protocol_and_exit_zero_requires_terminal_report() {
    for final_state in [
        None,
        Some(GuardedState::Ready),
        Some(GuardedState::Stopped {
            reconciliation_required: true,
        }),
    ] {
        let root = tempfile::tempdir().unwrap();
        let fake = Fake::default();
        let p = plan();
        let program = program();
        let mut supervisor = Supervisor::open(
            SqliteRepository::open(root.path().join("s.db")).unwrap(),
            fake.clone(),
            GuardedServices,
            p,
            [(program.id.clone(), program)].into(),
            &catalog(),
        )
        .unwrap();
        supervisor.tick().unwrap();
        assert_eq!(
            supervisor.tick().unwrap().state.records[&n("main")].phase,
            Phase::Starting
        );
        fake.0.lock().unwrap().observation = Some(observation(GuardedState::Ready));
        assert_eq!(
            supervisor.tick().unwrap().state.records[&n("main")].phase,
            Phase::ProcessReady
        );
        supervisor.request_stop().unwrap();
        fake.0.lock().unwrap().observation = final_state.map(observation);
        supervisor.tick().unwrap();
        let report = supervisor.tick().unwrap();
        assert!(report.all_exited);
        assert_eq!(
            report.guarded_shutdown_confirmed,
            matches!(final_state, Some(GuardedState::Stopped { .. }))
        );
        assert!(report.reconciliation_required);
        assert_eq!(fake.0.lock().unwrap().signals, vec![false]);
        assert!(!report.physical_shutdown_assessed);
    }
}

#[test]
fn lost_protocol_readiness_latches_cooperative_shutdown_instead_of_restart() {
    let root = tempfile::tempdir().unwrap();
    let fake = Fake::default();
    let p = plan();
    let program = program();
    fake.0.lock().unwrap().observation = Some(observation(GuardedState::Ready));
    let mut supervisor = Supervisor::open(
        SqliteRepository::open(root.path().join("s.db")).unwrap(),
        fake.clone(),
        GuardedServices,
        p,
        [(program.id.clone(), program)].into(),
        &catalog(),
    )
    .unwrap();
    supervisor.tick().unwrap();
    supervisor.tick().unwrap();
    fake.0.lock().unwrap().observation = None;
    assert!(supervisor.tick().unwrap().state.stop_requested);
    supervisor.tick().unwrap();
    supervisor.tick().unwrap();
    supervisor.tick().unwrap();
    assert_eq!(fake.0.lock().unwrap().spawns, 1);
    assert_eq!(fake.0.lock().unwrap().signals, vec![false]);
}

#[test]
fn nonzero_guarded_exit_is_unconfirmed_even_with_stopped_report() {
    let root = tempfile::tempdir().unwrap();
    let fake = Fake::default();
    let program = program();
    let mut supervisor = Supervisor::open(
        SqliteRepository::open(root.path().join("s.db")).unwrap(),
        fake.clone(),
        GuardedServices,
        plan(),
        [(program.id.clone(), program)].into(),
        &catalog(),
    )
    .unwrap();
    supervisor.tick().unwrap();
    {
        let mut e = fake.0.lock().unwrap();
        e.code = Some(Some(1));
        e.observation = Some(observation(GuardedState::Stopped {
            reconciliation_required: false,
        }));
    }
    let report = supervisor.tick().unwrap();
    assert!(report.state.stop_requested && report.all_exited);
    assert!(!report.guarded_shutdown_confirmed);
    assert!(matches!(
        report.state.records[&n("main")].guarded_exit,
        Some(GuardedExit::Unconfirmed { .. })
    ));
}

#[test]
fn held_guarded_service_is_not_forced_after_timeout() {
    let root = tempfile::tempdir().unwrap();
    let fake = Fake::default();
    let program = program();
    {
        let mut e = fake.0.lock().unwrap();
        e.held = true;
        e.observation = Some(observation(GuardedState::Ready));
    }
    let mut supervisor = Supervisor::open(
        SqliteRepository::open(root.path().join("s.db")).unwrap(),
        fake.clone(),
        GuardedServices,
        plan(),
        [(program.id.clone(), program)].into(),
        &catalog(),
    )
    .unwrap();
    supervisor.tick().unwrap();
    supervisor.tick().unwrap();
    supervisor.request_stop().unwrap();
    supervisor.tick().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(120));
    let report = supervisor.tick().unwrap();
    assert!(!report.all_exited);
    assert_eq!(fake.0.lock().unwrap().signals, vec![false]);
    assert!(
        report
            .blocked
            .iter()
            .any(|v| v.contains("forced termination not allowed"))
    );
}
