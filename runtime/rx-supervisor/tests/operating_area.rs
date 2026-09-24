#![cfg(unix)]
#[allow(dead_code)] // Shared real-process fixture; this slice exercises a subset.
#[path = "support/work_fixture.rs"]
mod fixture;
#[path = "support/offline_signer.rs"]
mod issuer;
use fixture::*;
use issuer::ExternalSigner;
use rx_domain::{canonical, types::*};
use rx_package::operating_area as policy;
use rx_storage::mailbox::Mailbox;
use rx_supervisor::{decision::*, operating_area::OfflineWorkUseProvider, use_assessment::*};
use std::{process::Command, time::Duration};
fn challenge(dir: &std::path::Path) -> ChallengeInfo {
    let paths = std::fs::read_dir(dir)
        .unwrap()
        .map(|v| v.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("request-")
        })
        .collect::<Vec<_>>();
    assert_eq!(paths.len(), 1);
    canonical::decode_json(&std::fs::read(&paths[0]).unwrap()).unwrap()
}
fn publish(dir: &std::path::Path, signer: &ExternalSigner, info: ChallengeInfo) -> SignedDecision {
    let signed = signer.decision(Claim {
        schema: n("rx.external-decision.v1"),
        verdict: Verdict::Approve,
        challenge: info.clone(),
        decision: id(),
        ttl_ms: Counter(30_000),
    });
    let mailbox = Mailbox::open(dir).unwrap();
    let guard = mailbox.lock().unwrap();
    guard
        .publish_once(
            &format!("decision-{}.json", info.id),
            &canonical::bytes(&signed).unwrap(),
        )
        .unwrap();
    signed
}
#[test]
fn polling_keeps_original_nonce_and_deadline_and_context_failure_is_not_pending() {
    let mut p = Policy {
        authorities: [(
            n(policy::KEY_ID),
            Authority {
                issuer: n("test/issuer"),
                public_key: policy::PUBLIC_KEY,
                operating_area: n("test/diagnostics"),
                roles: [n(policy::ROLE)].into(),
                kinds: [Kind::WorkUse].into(),
                max_ttl_ms: Counter(150),
            },
        )]
        .into(),
    };
    let mut f = Fixture::new(Some(p.clone()), false);
    let dir = tempfile::tempdir().unwrap();
    let provider = OfflineWorkUseProvider::new(dir.path());
    let task = f.task(10, 4);
    provider.begin_attempt();
    assert!(f.manager().prepare_work(task.clone(), &provider).is_err());
    assert!(provider.pending());
    let original = challenge(dir.path());
    for _ in 0..2 {
        std::thread::sleep(Duration::from_millis(20));
        provider.begin_attempt();
        assert!(f.manager().prepare_work(task.clone(), &provider).is_err());
        assert_eq!(challenge(dir.path()), original);
    }
    std::thread::sleep(Duration::from_millis(170));
    provider.begin_attempt();
    let err = f.manager().prepare_work(task, &provider).unwrap_err();
    assert!(err.to_string().contains("original challenge lifetime"));
    assert!(!provider.pending());
    assert_eq!(challenge(dir.path()), original);
    p.authorities.values_mut().next().unwrap().max_ttl_ms = Counter(30_000);
    let mut f = Fixture::new(Some(p), false);
    let dir = tempfile::tempdir().unwrap();
    let provider = OfflineWorkUseProvider::new(dir.path());
    let task = f.task(10, 4);
    provider.begin_attempt();
    assert!(f.manager().prepare_work(task.clone(), &provider).is_err());
    assert!(provider.pending());
    std::fs::write(
        f.dir.path().join("counts.json"),
        r#"{"native_packages":"invalid","support_profiles":4}"#,
    )
    .unwrap();
    provider.begin_attempt();
    let err = f.manager().prepare_work(task, &provider).unwrap_err();
    assert!(err.to_string().contains("readiness"));
    assert!(!provider.pending());
}
#[test]
fn old_response_cannot_restore_a_live_request_after_receiver_restart() {
    let signer = ExternalSigner::new();
    let mut f = Fixture::new(Some(signer.policy()), false);
    let first = tempfile::tempdir().unwrap();
    let provider = OfflineWorkUseProvider::new(first.path());
    let task = f.task(10, 4);
    assert!(f.manager().prepare_work(task.clone(), &provider).is_err());
    let old = challenge(first.path());
    let signed = publish(first.path(), &signer, old.clone());
    let prepared = f.manager().prepare_work(task.clone(), &provider).unwrap();
    assert_eq!(prepared.assessment().state(), WorkUseState::Verified);
    f.reopen();
    assert_eq!(
        prepared.decision().check_live().unwrap_err(),
        Failure::Epoch
    );
    let error = f
        .manager()
        .prepare_work(task.clone(), &provider)
        .unwrap_err();
    println!("old_manager_reopen_refusal={error}");
    assert!(error.to_string().contains("current-owned-ready"));
    assert!(!f.rows().iter().any(|r| r.key.as_str().starts_with("work/")));
    // F9 deliberately refuses unresolved execution after reopen. Do not weaken
    // that gate to manufacture a new challenge. An explicit new receiver tests
    // transport replay separately, while the old live proof already fails Epoch.
    let mut f = Fixture::new(Some(signer.policy()), false);
    let second = tempfile::tempdir().unwrap();
    let provider = OfflineWorkUseProvider::new(second.path());
    assert!(f.manager().prepare_work(task.clone(), &provider).is_err());
    let new = challenge(second.path());
    assert_ne!(new.id, old.id);
    assert_ne!(new.epoch, old.epoch);
    let m = Mailbox::open(second.path()).unwrap();
    m.lock()
        .unwrap()
        .publish_once(
            &format!("decision-{}.json", new.id),
            &canonical::bytes(&signed).unwrap(),
        )
        .unwrap();
    assert!(
        f.manager()
            .prepare_work(task, &provider)
            .unwrap_err()
            .to_string()
            .contains("context-mismatch")
    );
    assert_eq!(
        f.manager().query().unwrap().work_use_permission.state(),
        WorkUseState::NotEvaluated
    );
    assert!(!f.rows().iter().any(|r| r.key.as_str().starts_with("work/")));
}
#[test]
fn pending_publication_is_not_use_and_commit_lock_spans_the_physical_transaction() {
    let signer = ExternalSigner::new();
    let mut f = Fixture::new(Some(signer.policy()), false);
    let dir = tempfile::tempdir().unwrap();
    let provider = OfflineWorkUseProvider::new(dir.path());
    let task = f.task(10, 4);
    assert!(f.manager().prepare_work(task.clone(), &provider).is_err());
    let original = challenge(dir.path());
    let m = Mailbox::open(dir.path()).unwrap();
    let guard = m.lock().unwrap();
    provider.begin_attempt();
    assert!(f.manager().prepare_work(task.clone(), &provider).is_err());
    assert!(provider.pending());
    drop(guard);
    assert_eq!(challenge(dir.path()), original);
    publish(dir.path(), &signer, original);
    let prepared = f.manager().prepare_work(task.clone(), &provider).unwrap();
    let path = dir.path().join("exchange.lock");
    let path2 = path.clone();
    f.hook(task.operation.clone(),Box::new(move||{
        let output=Command::new("/usr/bin/python3").arg("-c").arg("import fcntl,sys\nf=open(sys.argv[1],'a+')\ntry:fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB)\nexcept BlockingIOError:print('PUBLICATION_REFUSED_DURING_COMMIT')\nelse:raise SystemExit('lock acquired during commit')").arg(path).output().unwrap();
        assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));assert!(String::from_utf8_lossy(&output.stdout).contains("PUBLICATION_REFUSED_DURING_COMMIT"));
        println!("cooperating_publication_excluded_through_commit=true");Ok(false)
    }));
    let report = provider
        .commit_checkpoint(&prepared, || f.manager().commit_work(&prepared))
        .unwrap();
    assert_eq!(
        report.current_permission,
        "NONE; HISTORICAL_WORK_RESULT_ONLY"
    );
    let output=Command::new("/usr/bin/python3").arg("-c").arg("import fcntl,sys\nf=open(sys.argv[1],'a+');fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB);print('PUBLICATION_AVAILABLE_AFTER_COMMIT')").arg(path2).output().unwrap();
    assert!(output.status.success());
    let guard = m.lock().unwrap();
    assert!(
        provider
            .commit_checkpoint::<()>(&prepared, || panic!("must refuse before use"))
            .unwrap_err()
            .to_string()
            .contains("mailbox-unavailable")
    );
    drop(guard);
}

#[test]
fn cached_provider_rechecks_signed_revocation_and_invalidates_retained_proof() {
    let signer = ExternalSigner::new();
    let mut f = Fixture::new(Some(signer.policy()), false);
    let dir = tempfile::tempdir().unwrap();
    let provider = OfflineWorkUseProvider::new(dir.path());
    let task = f.task(10, 4);
    assert!(f.manager().prepare_work(task.clone(), &provider).is_err());
    let info = challenge(dir.path());
    publish(dir.path(), &signer, info.clone());
    let prepared = f.manager().prepare_work(task.clone(), &provider).unwrap();
    let signed = signer.revocation(
        prepared
            .decision()
            .revocation_claim(n("test/changed-condition")),
    );
    Mailbox::open(dir.path())
        .unwrap()
        .lock()
        .unwrap()
        .publish_once(
            &format!("revocation-{}.json", info.id),
            &canonical::bytes(&signed).unwrap(),
        )
        .unwrap();
    assert!(
        f.manager()
            .prepare_work(task, &provider)
            .unwrap_err()
            .to_string()
            .contains("work-use/revoked")
    );
    assert_eq!(
        prepared.decision().check_live().unwrap_err(),
        Failure::Revoked
    );
    assert!(!f.rows().iter().any(|r| r.key.as_str().starts_with("work/")));
}
