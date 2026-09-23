use super::*;
use rx_supervisor::execution::{Application, Requirements};

#[test]
fn previous_application_observation_does_not_cover_a_new_prepared_instance() {
    let dir = tempfile::tempdir().unwrap();
    let mode = Arc::new(AtomicU8::new(0));
    let store = FaultStore {
        inner: SqliteRepository::open(dir.path().join("state.db")).unwrap(),
        mode: mode.clone(),
    };
    let mut recipe = program(Effect::NonActuating);
    recipe.execution_requirements = Some(Requirements(BTreeMap::new()));
    let mut p = plan();
    p.processes[0].restart_limit = Counter(1);
    let backend = Fake::default();
    let mut supervisor = Supervisor::open(
        store,
        backend.clone(),
        SoftwareOnly,
        p,
        [(recipe.id.clone(), recipe)].into(),
        &support(),
    )
    .unwrap();
    let first = supervisor.tick().unwrap();
    let old_instance = first.state.records[&name("main")].instance.clone().unwrap();
    assert!(matches!(
        first.execution_admission[&name("main")].application,
        Application::ReportedAtStart { .. }
    ));
    backend
        .0
        .lock()
        .unwrap()
        .exits
        .insert(old_instance.clone(), Some(1));
    supervisor.tick().unwrap();
    std::thread::sleep(Duration::from_millis(120));
    mode.store(1, Ordering::SeqCst); // New PREPARED commits; SPAWN_ENTERED fails before commit.
    assert!(supervisor.tick().is_err());
    let state = supervisor.state().unwrap();
    assert_eq!(state.records[&name("main")].phase, Phase::Prepared);
    assert_ne!(
        state.records[&name("main")].instance.as_ref(),
        Some(&old_instance)
    );
    assert!(
        matches!(
            supervisor.execution_admission().unwrap()[&name("main")].application,
            Application::Unconfirmed { .. }
        ),
        "previous instance receipt must not describe a new prepared attempt"
    );
    assert_eq!(backend.0.lock().unwrap().spawns.len(), 1);
    supervisor.tick().unwrap();
    assert!(matches!(
        supervisor.execution_admission().unwrap()[&name("main")].application,
        Application::ReportedAtStart { .. }
    ));
    assert_eq!(backend.0.lock().unwrap().spawns.len(), 2);
}
