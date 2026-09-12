#![cfg(all(target_os = "linux", feature = "test-harness"))]

use rx_domain::types::*;
use rx_executor::{
    assignment_journal::{AssignmentJournal, Identity, Phase, Preparation},
    journal::{Basis, Scope},
    service_owner::ServiceOwner,
};
use rx_ports::Repository;
use rx_storage::SqliteRepository;
use sha2::{Digest as _, Sha256};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};

// Keep process-spawn fixtures separate from a test's temporarily open SQLite handles.
static SERIAL: Mutex<()> = Mutex::new(());
fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
struct Fixture {
    directory: tempfile::TempDir,
    config: PathBuf,
    root: PathBuf,
    probe: PathBuf,
    value: serde_json::Value,
}
fn id(v: u64) -> Id {
    Id::new(format!("00000000-0000-4000-8000-{v:012}")).unwrap()
}
fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("service");
    let config = directory.path().join("cell.json");
    let probe = directory.path().join("before-connect.log");
    let material = directory.path().join("test-only-credential");
    let bytes = b"TEST ONLY: not a usable TLS identity";
    fs::write(&material, bytes).unwrap();
    fs::set_permissions(&material, fs::Permissions::from_mode(0o600)).unwrap();
    let pin = serde_json::json!({
        "path": material,
        "sha256": Digest::from_bytes(Sha256::digest(bytes).into()),
    });
    let engine = directory.path().join("test-only-engine");
    let engine_bytes = b"TEST ONLY: the pre-connect probe prevents process launch";
    fs::write(&engine, engine_bytes).unwrap();
    fs::set_permissions(&engine, fs::Permissions::from_mode(0o500)).unwrap();
    let value = serde_json::json!({
        "schema": "rx.executor-cell-service.v1",
        "service_root": root,
        "expected_service": {
            "journal": id(1),
            "scope": {
                "installation": id(2), "store_generation": id(3),
                "principal": "executor/test", "release": "44".repeat(32),
                "cell": "cell/test", "definition": "55".repeat(32)
            }
        },
        "platform": {
            "uri": "https://127.0.0.1:1", "server_name": "test.invalid",
            "ca": pin, "certificate": pin, "key": pin
        },
        "engine": {
            "path": engine,
            "sha256": Digest::from_bytes(Sha256::digest(engine_bytes).into())
        }
    });
    fs::write(&config, serde_json::to_vec(&value).unwrap()).unwrap();
    Fixture {
        directory,
        config,
        root,
        probe,
        value,
    }
}
impl Fixture {
    fn write(&self, value: &serde_json::Value) {
        fs::write(&self.config, serde_json::to_vec(value).unwrap()).unwrap();
    }
    fn identity(&self) -> Identity {
        serde_json::from_value(self.value["expected_service"].clone()).unwrap()
    }
    fn invoke(&self, operation: &str, partial_init: bool) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rx-executor-service"));
        command
            .args(["cell", operation])
            .arg(&self.config)
            .env("RX_EXECUTOR_BEFORE_CONNECT_PROBE", &self.probe)
            .env_remove("RX_EXECUTOR_CELL_INIT_FAIL_AFTER_MANIFEST")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if partial_init {
            command.env("RX_EXECUTOR_CELL_INIT_FAIL_AFTER_MANIFEST", "1");
        }
        bounded_output(command)
    }
    fn init(&self) {
        let output = self.invoke("init", false);
        assert!(output.status.success(), "{output:?}");
        assert!(!self.probe.exists(), "init must not enter connection code");
    }
    fn refuse_before_connect(&self) {
        let before = probe_count(&self.probe);
        let output = self.invoke("run", false);
        assert!(!output.status.success(), "{output:?}");
        assert_eq!(probe_count(&self.probe), before, "{output:?}");
    }
    fn reach_probe(&self) {
        let before = probe_count(&self.probe);
        let output = self.invoke("run", false);
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("TEST_ONLY_EXECUTOR_BEFORE_CONNECT"),
            "{output:?}"
        );
        assert_eq!(probe_count(&self.probe), before + 1);
    }
}
fn probe_count(path: &Path) -> usize {
    fs::read_to_string(path).map_or(0, |value| value.lines().count())
}
fn bounded_output(mut command: Command) -> Output {
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!("cell CLI fixture exceeded deadline: {output:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn init_pins_expected_identity_and_normalized_defaults_without_peer_registration() {
    let _serial = serial();
    let fixture = fixture();
    fixture.refuse_before_connect();
    assert!(!fixture.root.join("cell-installation.json").exists());
    assert!(!fixture.root.join("assignment.sqlite3").exists());
    fixture.init();
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.root.join("cell-installation.json")).unwrap())
            .unwrap();
    assert_eq!(
        manifest["expected_service"],
        fixture.value["expected_service"]
    );
    assert_eq!(
        manifest["service_root"],
        serde_json::json!(fs::canonicalize(&fixture.root).unwrap())
    );
    let mut journal = AssignmentJournal::open_file_required(
        &fixture.root.join("assignment.sqlite3"),
        fixture.identity(),
    )
    .unwrap();
    assert!(journal.current().unwrap().is_none());
    drop(journal);
    assert!(!fixture.invoke("init", false).status.success());
    fixture.reach_probe();
    let mut explicit = fixture.value.clone();
    explicit["options"] = serde_json::json!({
        "poll_ms": 50, "communication_grace_ms": 5000, "stop_timeout_ms": 10000
    });
    fixture.write(&explicit);
    fixture.reach_probe();
}

#[test]
fn duplicate_and_root_alias_are_refused_before_session_open_then_cleanup_allows_reopen() {
    let _serial = serial();
    let fixture = fixture();
    fixture.init();
    let owner = ServiceOwner::acquire(&fixture.root.join("assignment.sqlite3")).unwrap();
    fixture.refuse_before_connect();
    let alias = fixture.directory.path().join("alias");
    std::os::unix::fs::symlink(&fixture.root, &alias).unwrap();
    let mut value = fixture.value.clone();
    value["service_root"] = serde_json::json!(alias);
    fixture.write(&value);
    fixture.refuse_before_connect();
    drop(owner);
    fixture.reach_probe();
    let _released = ServiceOwner::acquire(&fixture.root.join("assignment.sqlite3")).unwrap();
}

#[test]
fn manifest_only_partial_installation_is_not_overwritten_or_connected() {
    let _serial = serial();
    let fixture = fixture();
    let output = fixture.invoke("init", true);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("TEST_ONLY_EXECUTOR_CELL_INIT_AFTER_MANIFEST")
    );
    let manifest = fixture.root.join("cell-installation.json");
    let original = fs::read(&manifest).unwrap();
    assert!(!fixture.root.join("assignment.sqlite3").exists());
    assert!(!fixture.invoke("init", false).status.success());
    fixture.refuse_before_connect();
    assert_eq!(fs::read(manifest).unwrap(), original);
    assert!(!fixture.root.join("assignment.sqlite3").exists());
}

#[test]
fn missing_manifest_empty_journal_and_changed_header_are_refused_before_connect() {
    let _serial = serial();
    for damage in 0..3 {
        let fixture = fixture();
        fixture.init();
        let path = fixture.root.join("assignment.sqlite3");
        match damage {
            0 => fs::remove_file(fixture.root.join("cell-installation.json")).unwrap(),
            1 => fs::write(&path, []).unwrap(),
            _ => {
                let mut repository = SqliteRepository::open(&path).unwrap();
                repository
                    .transact(|tx| {
                        let key = Name::new("attachment/header").unwrap();
                        let mut row = tx.get(&key)?.unwrap();
                        row.document.value["journal"] = serde_json::json!(id(999));
                        tx.put(&key, Some(row.revision), &row.document)?;
                        Ok(())
                    })
                    .unwrap();
            }
        }
        fixture.refuse_before_connect();
        assert!(!fixture.invoke("init", false).status.success());
        if damage == 0 {
            assert!(!fixture.root.join("cell-installation.json").exists());
        }
        if damage == 1 {
            assert_eq!(fs::metadata(path).unwrap().len(), 0);
        }
    }
}

#[test]
fn initialized_preparing_or_attached_run_file_loss_is_detected_before_new_peer_boot_registration() {
    let _serial = serial();
    for attached in [false, true] {
        let fixture = fixture();
        fixture.init();
        let identity = fixture.identity();
        let mut assignments = AssignmentJournal::open_file_required(
            &fixture.root.join("assignment.sqlite3"),
            identity.clone(),
        )
        .unwrap();
        let scope = identity.scope;
        let preparation = Preparation {
            id: id(100),
            run_journal: id(101),
            executor_session: id(102),
            epoch: Counter(1),
            scope: Scope {
                installation: scope.installation,
                store_generation: scope.store_generation,
                principal: scope.principal,
                release: scope.release,
                cell: scope.cell,
                definition: scope.definition,
                run: id(103),
                resolved_digest: Digest::from_bytes([6; 32]),
            },
            basis: Basis {
                runtime_boot: id(104),
                sequence: Counter(1),
                run_revision: Counter(1),
                cell_revision: Counter(1),
                checked_at: TimePoint {
                    clock_id: "test-clock".into(),
                    ticks_ns: Counter(1),
                },
            },
        };
        assignments.prepare(preparation.clone()).unwrap();
        let run = assignments
            .initialize_run_file(&fixture.root, &preparation.id)
            .unwrap();
        if attached {
            drop(assignments.attach(run).unwrap());
        } else {
            drop(run);
        }
        let path = assignments
            .run_file(&fixture.root, &preparation.id)
            .unwrap();
        drop(assignments);
        fs::remove_file(&path).unwrap();
        fixture.refuse_before_connect();
        assert!(!path.exists());
        let mut assignments = AssignmentJournal::open_file_required(
            &fixture.root.join("assignment.sqlite3"),
            fixture.identity(),
        )
        .unwrap();
        assert_eq!(
            assignments.current().unwrap().unwrap().value.phase,
            if attached {
                Phase::Attached
            } else {
                Phase::Preparing
            }
        );
    }
}

#[test]
fn changed_configuration_and_pinned_bytes_cannot_reach_session_open() {
    let _serial = serial();
    for damage in 0..4 {
        let fixture = fixture();
        fixture.init();
        let mut value = fixture.value.clone();
        match damage {
            0 => {
                value["expected_service"]["journal"] = serde_json::json!(id(999));
                fixture.write(&value);
            }
            1 => {
                value["options"] = serde_json::json!({ "poll_ms": 100, "communication_grace_ms": 5000, "stop_timeout_ms": 10000 });
                fixture.write(&value);
            }
            2 => {
                fs::write(
                    value["platform"]["ca"]["path"].as_str().unwrap(),
                    b"changed credential bytes",
                )
                .unwrap();
            }
            _ => {
                let engine = Path::new(value["engine"]["path"].as_str().unwrap());
                fs::set_permissions(engine, fs::Permissions::from_mode(0o700)).unwrap();
                fs::write(engine, b"changed engine bytes").unwrap();
            }
        }
        fixture.refuse_before_connect();
    }
}

#[test]
fn copied_installation_cannot_silently_change_its_root() {
    let _serial = serial();
    let fixture = fixture();
    fixture.init();
    let relocated = fixture.directory.path().join("relocated");
    fs::create_dir(&relocated).unwrap();
    for file in ["cell-installation.json", "assignment.sqlite3"] {
        fs::copy(fixture.root.join(file), relocated.join(file)).unwrap();
    }
    let mut value = fixture.value.clone();
    value["service_root"] = serde_json::json!(relocated);
    fixture.write(&value);
    fixture.refuse_before_connect();
}

#[test]
fn cell_configuration_rejects_run_visit_program_arguments_and_invalid_timing() {
    let _serial = serial();
    let fixture = fixture();
    for field in ["run", "visit", "peer_boot", "clock_id", "argv", "launch"] {
        let mut value = fixture.value.clone();
        value[field] = serde_json::json!("untrusted override");
        fixture.write(&value);
        assert!(!fixture.invoke("init", false).status.success());
        assert!(!fixture.root.exists());
    }
    for (field, invalid) in [
        ("poll_ms", 0),
        ("poll_ms", 1001),
        ("communication_grace_ms", 60001),
        ("stop_timeout_ms", 60001),
    ] {
        let mut value = fixture.value.clone();
        value["options"] = serde_json::json!({ "poll_ms": 50, "communication_grace_ms": 5000, "stop_timeout_ms": 10000 });
        value["options"][field] = serde_json::json!(invalid);
        fixture.write(&value);
        assert!(!fixture.invoke("init", false).status.success());
        assert!(!fixture.root.exists());
    }
    assert!(!fixture.probe.exists());
}

#[test]
fn deployment_file_symlinks_and_private_key_permissions_are_rejected() {
    let _serial = serial();
    for symlink in [false, true] {
        let fixture = fixture();
        let mut value = fixture.value.clone();
        let credential = Path::new(value["platform"]["key"]["path"].as_str().unwrap());
        if symlink {
            let alias = fixture.directory.path().join("credential-alias");
            std::os::unix::fs::symlink(credential, &alias).unwrap();
            value["platform"]["key"]["path"] = serde_json::json!(alias);
            fixture.write(&value);
        } else {
            fs::set_permissions(credential, fs::Permissions::from_mode(0o644)).unwrap();
        }
        assert!(!fixture.invoke("init", false).status.success());
        assert!(!fixture.probe.exists());
        assert!(!fixture.root.join("cell-installation.json").exists());
    }
}
