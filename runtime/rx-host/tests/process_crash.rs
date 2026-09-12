#![cfg(feature = "test-harness")]
use std::{
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn sigkill_at_both_journal_native_boundaries_never_replays_device_effect() {
    let binary = std::path::PathBuf::from(env!("CARGO_BIN_EXE_rx-host-crash-fixture"));
    for (point, effects, evidence, after) in [
        ("before-native", 0, 0, "SEND_ENTERED"),
        ("after-native", 1, 1, "RESULT_CAPTURED"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut child = ChildGuard(
            Command::new(&binary)
                .arg(directory.path())
                .arg(point)
                .stdout(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let end = Instant::now() + Duration::from_secs(10);
        while !directory.path().join("boundary-ready").exists() {
            assert!(Instant::now() < end, "boundary marker missing: {point}");
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "child exited before boundary"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        child.0.kill().unwrap();
        let exit = child.0.wait().unwrap();
        assert!(!exit.success());
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(exit.signal(), Some(9));
        }
        let output = Command::new(&binary)
            .arg(directory.path())
            .arg("recover")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: serde_json::Value =
            serde_json::from_slice(&std::fs::read(directory.path().join("result.json")).unwrap())
                .unwrap();
        assert_eq!(result["before"], "SEND_ENTERED");
        assert_eq!(result["after"], after);
        assert_eq!(result["effects"], effects);
        assert_eq!(result["evidence"], evidence);
    }
}

#[test]
fn configuration_commit_survives_sigkill_before_reply_without_restoring_arm() {
    let binary = std::path::PathBuf::from(env!("CARGO_BIN_EXE_rx-host-crash-fixture"));
    let directory = tempfile::tempdir().unwrap();
    let mut child = ChildGuard(
        Command::new(&binary)
            .arg(directory.path())
            .arg("configuration-commit")
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let end = Instant::now() + Duration::from_secs(10);
    while !directory.path().join("boundary-ready").exists() {
        assert!(Instant::now() < end && child.0.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(5));
    }
    child.0.kill().unwrap();
    assert!(!child.0.wait().unwrap().success());
    let output = Command::new(binary)
        .arg(directory.path())
        .arg("configuration-recover")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.path().join("configuration-result.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(value["receipt"]["status"], "APPLIED_UNQUALIFIED");
    assert_eq!(value["activation_authorized"], false);
    assert_eq!(value["context_matches_current_host"], false);
}

#[test]
fn qualification_commit_survives_sigkill_before_reply_without_restoring_authority() {
    let binary = std::path::PathBuf::from(env!("CARGO_BIN_EXE_rx-host-crash-fixture"));
    let dir = tempfile::tempdir().unwrap();
    let mut child = ChildGuard(
        Command::new(&binary)
            .arg(dir.path())
            .arg("qualification-commit")
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let end = Instant::now() + Duration::from_secs(10);
    while !dir.path().join("boundary-ready").exists() {
        assert!(Instant::now() < end && child.0.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(5));
    }
    child.0.kill().unwrap();
    assert!(!child.0.wait().unwrap().success());
    let out = Command::new(binary)
        .arg(dir.path())
        .arg("qualification-recover")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(
        &std::fs::read(dir.path().join("qualification-result.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(value["receipt"]["status"], "ACCEPTED");
    assert_eq!(value["receipt_matches_current_host"], false);
    assert_eq!(value["activation_authorized"], false);
}
