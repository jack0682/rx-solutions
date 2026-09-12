#![cfg(unix)]

use rx_executor::service_owner::ServiceOwner;
use std::{fs, io, path::Path};

#[test]
fn one_root_cannot_register_two_services_with_different_run_journals() {
    let root = tempfile::tempdir().unwrap();
    let journal = root.path().join("run-a.sqlite3");
    let first = ServiceOwner::acquire(&journal).unwrap();
    for contender in [journal, root.path().join("run-b.sqlite3")] {
        let error = ServiceOwner::acquire(&contender).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("EXECUTOR_SERVICE_ROOT_UNAVAILABLE")
        );
    }
    // The root lock does not initialize or modify either run journal.
    assert!(!root.path().join("run-a.sqlite3").exists());
    assert!(!root.path().join("run-b.sqlite3").exists());
    drop(first);
    let _next = ServiceOwner::acquire(&root.path().join("run-b.sqlite3")).unwrap();
}

#[test]
fn different_service_roots_do_not_impose_a_global_cell_owner() {
    let root = tempfile::tempdir().unwrap();
    let _first = ServiceOwner::acquire(&root.path().join("a/run.sqlite3")).unwrap();
    let _second = ServiceOwner::acquire(&root.path().join("b/run.sqlite3")).unwrap();
}

#[test]
fn startup_failure_releases_ownership_without_removing_the_lock_file() {
    fn failed_start(journal: &Path) -> io::Result<()> {
        let _owner = ServiceOwner::acquire(journal)?;
        Err(io::Error::other("connection or startup failed"))
    }
    let root = tempfile::tempdir().unwrap();
    let journal = root.path().join("run.sqlite3");
    fs::write(&journal, b"existing immutable run journal").unwrap();
    assert!(failed_start(&journal).is_err());
    assert!(root.path().join(".rx-executor-service.lock").is_file());
    let _next = ServiceOwner::acquire(&journal).unwrap();
    assert_eq!(
        fs::read(journal).unwrap(),
        b"existing immutable run journal"
    );
}

#[test]
fn directory_alias_cannot_bypass_the_service_owner() {
    let root = tempfile::tempdir().unwrap();
    let actual = root.path().join("actual");
    fs::create_dir(&actual).unwrap();
    let alias = root.path().join("alias");
    std::os::unix::fs::symlink(&actual, &alias).unwrap();
    let _first = ServiceOwner::acquire(&actual.join("run.sqlite3")).unwrap();
    assert!(ServiceOwner::acquire(&alias.join("other-run.sqlite3")).is_err());
}

#[test]
fn ownership_symlink_is_rejected_without_touching_its_target() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("unrelated-file");
    fs::write(&target, b"must remain unchanged").unwrap();
    std::os::unix::fs::symlink(&target, root.path().join(".rx-executor-service.lock")).unwrap();
    assert!(ServiceOwner::acquire(&root.path().join("run.sqlite3")).is_err());
    assert_eq!(fs::read(target).unwrap(), b"must remain unchanged");
}

#[test]
fn ownership_special_files_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let socket_root = root.path().join("socket");
    let directory_root = root.path().join("directory");
    let fifo_root = root.path().join("fifo");
    for path in [&socket_root, &directory_root, &fifo_root] {
        fs::create_dir(path).unwrap();
    }
    let _socket =
        std::os::unix::net::UnixListener::bind(socket_root.join(".rx-executor-service.lock"))
            .unwrap();
    fs::create_dir(directory_root.join(".rx-executor-service.lock")).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(fifo_root.join(".rx-executor-service.lock"))
            .status()
            .unwrap()
            .success()
    );
    for path in [socket_root, directory_root, fifo_root] {
        assert!(ServiceOwner::acquire(&path.join("run.sqlite3")).is_err());
    }
}

#[test]
fn invalid_journal_path_cannot_overwrite_the_ownership_file() {
    let root = tempfile::tempdir().unwrap();
    assert!(ServiceOwner::acquire(Path::new("")).is_err());
    assert!(ServiceOwner::acquire(&root.path().join(".rx-executor-service.lock")).is_err());
}

#[cfg(all(target_os = "linux", feature = "test-harness"))]
mod entrypoint {
    use super::*;
    use rx_executor::clock::{Clock, LinuxBoottime};
    use std::{
        process::{Command, Output, Stdio},
        time::{Duration, Instant},
    };

    fn config(root: &Path, journal: &Path) -> std::path::PathBuf {
        let clock = LinuxBoottime::new().unwrap();
        let path = root.join("service.json");
        // The probe runs before TLS file reads or network entry. These files must stay absent.
        let missing = root.join("unused-test-only-credential");
        let value = serde_json::json!({
            "uri": "https://127.0.0.1:1",
            "server_name": "test.invalid",
            "ca": missing,
            "certificate": missing,
            "key": missing,
            "pin": {
                "principal": "executor/test",
                "peer_boot": "10000000-0000-4000-8000-000000000001",
                "installation": "10000000-0000-4000-8000-000000000002",
                "store_generation": "10000000-0000-4000-8000-000000000003",
                "release": "11".repeat(32),
                "clock_id": clock.now().unwrap().clock_id,
                "cell": "cell/test",
                "definition": "22".repeat(32)
            },
            "run": "10000000-0000-4000-8000-000000000004",
            "visit": "1",
            "journal": journal,
            "engine": root.join("unused-test-only-engine"),
            "engine_sha256": "33".repeat(32)
        });
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        path
    }

    fn invoke(config: &Path, probe: &Path) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rx-executor-service"))
            .arg(config)
            .env("RX_EXECUTOR_BEFORE_CONNECT_PROBE", probe)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if child.try_wait().unwrap().is_some() {
                return child.wait_with_output().unwrap();
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let output = child.wait_with_output().unwrap();
                panic!("executor startup exceeded test deadline: {output:?}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn competing_product_process_never_reaches_connect_and_failure_unlocks_root() {
        let root = tempfile::tempdir().unwrap();
        let journal = root.path().join("service/run.sqlite3");
        let config = config(root.path(), &journal);
        let probe = root.path().join("before-connect.log");
        let first_process_owner = ServiceOwner::acquire(&journal).unwrap();

        let rejected = invoke(&config, &probe);
        assert!(!rejected.status.success());
        assert!(
            String::from_utf8_lossy(&rejected.stderr).contains("EXECUTOR_SERVICE_ROOT_UNAVAILABLE")
        );
        assert!(!probe.exists(), "competing product process entered connect");
        assert!(
            !journal.exists(),
            "root admission must precede journal opening"
        );
        drop(first_process_owner);

        // Positive controls prove the same valid CLI input reaches the instrumented boundary.
        // Its intentional pre-connect failure must release the process's ownership each time.
        for expected in 1..=2 {
            let admitted = invoke(&config, &probe);
            assert!(!admitted.status.success());
            assert!(
                String::from_utf8_lossy(&admitted.stderr)
                    .contains("TEST_ONLY_EXECUTOR_BEFORE_CONNECT")
            );
            assert_eq!(
                fs::read_to_string(&probe).unwrap().lines().count(),
                expected
            );
            let _released = ServiceOwner::acquire(&journal).unwrap();
        }
        assert!(!journal.exists());
    }
}
