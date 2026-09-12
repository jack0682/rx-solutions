use rx_domain::types::*;
use rx_ports::{Document, Repository};
use rx_storage::SqliteRepository;
use rx_supervisor::{
    builtin::{Initializer, ServiceRole},
    initialization,
};
use std::{collections::BTreeMap, path::Path};
fn n(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn commands(root: &Path) -> Vec<Initializer> {
    [
        ServiceRole::Host,
        ServiceRole::Host,
        ServiceRole::Executor,
        ServiceRole::Executor,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, role)| Initializer {
        id: n(&format!("rx/service/service-{index}")),
        role,
        executable: "/release/initializer".into(),
        executable_sha256: Digest::from_bytes([1; 32]),
        files: BTreeMap::new(),
        arguments: vec![],
        data_directory: root.join(format!("data-{index}")),
        runtime_directory: None,
        installation: id(),
        principal: n(&format!("principal-{index}")),
        cells: [(n("cell/a"), Digest::from_bytes([2; 32]))].into(),
    })
    .collect()
}
#[test]
fn required_composition_initialization_rejects_missing_partial_changed_list_and_wrong_digest() {
    let root = tempfile::tempdir().unwrap();
    let mut repository = SqliteRepository::open(root.path().join("init.db")).unwrap();
    let expected = Digest::from_bytes([1; 32]);
    let commands = commands(root.path());
    assert!(initialization::require_complete(&mut repository, expected, &commands).is_err());
    let state = initialization::State {
        digest: expected,
        steps: commands
            .iter()
            .enumerate()
            .map(|(index, command)| initialization::Step {
                program: command.id.clone(),
                role: command.role,
                instance: id(),
                phase: if index < 3 {
                    initialization::Phase::Completed
                } else {
                    initialization::Phase::Entered
                },
                exit_code: if index < 3 { Some(0) } else { None },
                error: None,
            })
            .collect(),
    };
    repository
        .transact(|tx| {
            tx.put(
                &n("service-initialization/state"),
                None,
                &Document {
                    schema: n("rx.solutions-service-initialization.v1"),
                    value: serde_json::to_value(state).unwrap(),
                },
            )?;
            Ok(())
        })
        .unwrap();
    assert!(initialization::require_complete(&mut repository, expected, &commands).is_err());
    assert!(
        initialization::require_complete(&mut repository, Digest::from_bytes([2; 32]), &commands)
            .is_err()
    );
    assert!(initialization::require_complete(&mut repository, expected, &commands[..3]).is_err());
}
#[cfg(unix)]
#[tokio::test]
async fn four_named_initializer_successes_are_recorded_once_and_entered_is_never_replayed() {
    use sha2::{Digest as _, Sha256};
    let root = tempfile::tempdir().unwrap();
    let script = root.path().join("init.py");
    std::fs::write(&script, "from pathlib import Path\nimport sys\np=Path(sys.argv[1]);p.write_text(p.read_text()+'x' if p.exists() else 'x')\n").unwrap();
    let python = std::path::PathBuf::from("/usr/bin/python3");
    let hash = |p: &Path| Digest::from_bytes(Sha256::digest(std::fs::read(p).unwrap()).into());
    let mut commands = commands(root.path());
    for (index, command) in commands.iter_mut().enumerate() {
        command.executable = python.clone();
        command.executable_sha256 = hash(&python);
        command.files = BTreeMap::from([(script.clone(), hash(&script))]);
        command.arguments = vec![
            script.to_string_lossy().into(),
            root.path()
                .join(format!("count-{index}"))
                .to_string_lossy()
                .into(),
        ];
    }
    let expected = initialization::digest(Digest::from_bytes([1; 32]), &commands).unwrap();
    let mut repository = SqliteRepository::open(root.path().join("init.db")).unwrap();
    let state = initialization::run(
        &mut repository,
        expected,
        &commands,
        &root.path().join("logs"),
    )
    .await
    .unwrap();
    assert_eq!(state.steps.len(), 4);
    assert!(
        state
            .steps
            .iter()
            .all(|s| s.phase == initialization::Phase::Completed)
    );
    initialization::run(
        &mut repository,
        expected,
        &commands,
        &root.path().join("logs"),
    )
    .await
    .unwrap();
    for index in 0..4 {
        assert_eq!(
            std::fs::read_to_string(root.path().join(format!("count-{index}"))).unwrap(),
            "x"
        );
    }
    let mut changed = commands.clone();
    changed.swap(0, 1);
    assert!(initialization::require_complete(&mut repository, expected, &changed).is_err());
    let mut entered = SqliteRepository::open(root.path().join("entered.db")).unwrap();
    let state = initialization::State {
        digest: expected,
        steps: commands
            .iter()
            .map(|c| initialization::Step {
                program: c.id.clone(),
                role: c.role,
                instance: id(),
                phase: initialization::Phase::Entered,
                exit_code: None,
                error: None,
            })
            .collect(),
    };
    entered
        .transact(|tx| {
            tx.put(
                &n("service-initialization/state"),
                None,
                &Document {
                    schema: n("rx.solutions-service-initialization.v1"),
                    value: serde_json::to_value(state).unwrap(),
                },
            )?;
            Ok(())
        })
        .unwrap();
    assert!(
        initialization::run(
            &mut entered,
            expected,
            &commands,
            &root.path().join("entered-logs")
        )
        .await
        .is_err()
    );
    for index in 0..4 {
        assert_eq!(
            std::fs::read_to_string(root.path().join(format!("count-{index}"))).unwrap(),
            "x"
        );
    }
}
