#![cfg(unix)]
#[path = "support/external_signer.rs"]
mod external_signer;
#[path = "support/work_fixture.rs"]
mod fixture;
use external_signer::ExternalSigner;
use fixture::*;
use rx_domain::types::*;
use rx_supervisor::{decision::*, use_assessment::*, work_use::*};
use std::{cell::Cell, time::Duration};
static CASES: std::sync::Mutex<()> = std::sync::Mutex::new(());
struct Issuer {
    signer: ExternalSigner,
    ttl: u64,
    calls: Cell<usize>,
}
impl Issuer {
    fn new() -> Self {
        Self {
            signer: ExternalSigner::new(),
            ttl: 30_000,
            calls: Cell::new(0),
        }
    }
    fn policy(&self) -> Policy {
        let mut p = self.signer.policy();
        for a in p.authorities.values_mut() {
            a.roles.insert(n("work/support-gap-report"));
        }
        p
    }
}
impl WorkUsePort for Issuer {
    fn assess(&self, r: &WorkUseRequest) -> WorkUseReply {
        self.calls.set(self.calls.get() + 1);
        let request = r.decision_request().expect("author-pinned test request");
        let challenge = request.challenge(&self.signer.key_id()).unwrap();
        let signed = self
            .signer
            .decision(challenge.claim(id(), Counter(self.ttl)));
        WorkUseReply::Verified(Box::new(challenge.verify(&signed).unwrap()))
    }
}
struct Cached(VerifiedDecision);
impl WorkUsePort for Cached {
    fn assess(&self, _: &WorkUseRequest) -> WorkUseReply {
        WorkUseReply::Verified(Box::new(self.0.clone()))
    }
}
fn work_rows(f: &mut Fixture) -> usize {
    f.rows()
        .iter()
        .filter(|r| r.key.as_str().starts_with("work/"))
        .count()
}
fn error(e: impl std::fmt::Display) -> String {
    let s = e.to_string();
    println!("named_refusal={s}");
    s
}
#[test]
fn derived_support_gap_is_actual_work_and_history_is_not_current_permission() {
    let _case = CASES.lock().unwrap_or_else(|p| p.into_inner());
    let issuer = Issuer::new();
    let installed = std::env::var("RX_WORK_INSTALLED").as_deref() == Ok("1");
    let mut f = Fixture::new(Some(issuer.policy()), installed);
    let (native, profiles) = f.observed_counts();
    let first = f.task(native + 2, profiles + 2);
    let prepared = f.manager().prepare_work(first.clone(), &issuer).unwrap();
    let report = f.manager().commit_work(&prepared).unwrap();
    assert_eq!(
        report.native_packages,
        Comparison {
            observed: Counter(native),
            required: Counter(native + 2),
            shortfall: Counter(2)
        }
    );
    assert_eq!(report.support_profiles.shortfall, Counter(2));
    assert_eq!(
        report.instance,
        f.manager().state().unwrap().records[&n("status")]
            .instance
            .clone()
            .unwrap()
    );
    assert!(report.observed_at.ticks_ns.0 > 0);
    assert!(report.current_permission.contains("HISTORICAL"));
    let second = f.task(native, profiles);
    let p2 = f.manager().prepare_work(second.clone(), &issuer).unwrap();
    let r2 = f.manager().commit_work(&p2).unwrap();
    assert_eq!(r2.native_packages.shortfall, Counter(0));
    assert_eq!(r2.support_profiles.shortfall, Counter(0));
    assert_eq!(r2.source_digest, report.source_digest);
    assert_ne!(r2.input_digest, report.input_digest);
    let third = f.task(native + 1, profiles + 1);
    assert!(
        error(
            f.manager()
                .prepare_work(third, &Cached(prepared.decision().clone()))
                .unwrap_err()
        )
        .contains("context")
    );
    assert_eq!(
        f.manager()
            .recorded_work(&n("status"), &first.operation)
            .unwrap(),
        report
    );
    assert_eq!(work_rows(&mut f), 4);
    println!(
        "work_derived_result={}",
        serde_json::json!({"installed_release":installed,"externally_observed_counts":[native,profiles],"first":report,"second":r2,"issuer_scope":"TEST_ONLY_NOT_OPERATING_APPROVAL"})
    );
}
#[test]
fn absent_authority_blocks_work_without_blocking_f5_diagnostics() {
    let _case = CASES.lock().unwrap_or_else(|p| p.into_inner());
    let mut f = Fixture::new(None, false);
    let task = f.task(12, 6);
    assert!(
        error(
            f.manager()
                .prepare_work(task, &NoWorkUseProvider)
                .unwrap_err()
        )
        .contains("author-policy-absent")
    );
    let view = f
        .manager()
        .assess_use(
            UseScope {
                operating_area: n("test/diagnostics"),
                role: n("diagnostics/support-summary"),
            },
            &NoWorkUseProvider,
        )
        .unwrap();
    assert_eq!(view.functional_readiness.state(), ConditionState::Satisfied);
    assert_eq!(view.work_use_permission.state(), WorkUseState::Unsupported);
    use rx_supervisor::registration::{Declaration, Registry, diagnostic::*};
    let catalog = catalog();
    let mut registry = Registry::new(
        rx_storage::SqliteRepository::open(f.dir.path().join("diagnostic.db")).unwrap(),
    );
    let component = registry
        .register(Declaration {
            label: n("diagnostic"),
            catalog: catalog.reference().unwrap(),
        })
        .unwrap()
        .registration
        .id;
    let mut consumer = Consumer::open(registry, component, catalog).unwrap();
    let binding = consumer
        .track(n("catalog-summary"), n("test/diagnostics"), None)
        .unwrap();
    let run = consumer
        .assign(&binding.id, &mut NoSource)
        .unwrap()
        .run
        .unwrap();
    consumer.begin(&binding.id, &run.id, &mut NoSource).unwrap();
    let result = consumer
        .finish(&binding.id, &run.id, &mut NoSource)
        .unwrap()
        .result
        .unwrap();
    let used = consumer
        .consume_result(&result.id, &mut NoSource)
        .unwrap()
        .result
        .unwrap();
    assert_eq!(used.purpose, Purpose::DiagnosticOnly);
    assert_eq!(
        used.work_use,
        rx_supervisor::registration::WorkUsePermission::Unsupported
    );
    assert_eq!(work_rows(&mut f), 0);
    println!(
        "diagnostic_access_retained={}",
        serde_json::to_string(&used).unwrap()
    );
}
#[test]
fn changed_report_and_registration_between_judgment_and_cut_refuse_commit() {
    let _case = CASES.lock().unwrap_or_else(|p| p.into_inner());
    let issuer = Issuer::new();
    let mut f = Fixture::new(Some(issuer.policy()), false);
    let task = f.task(12, 6);
    let prepared = f.manager().prepare_work(task.clone(), &issuer).unwrap();
    f.counts(10, 3);
    assert!(
        error(f.manager().commit_work(&prepared).unwrap_err()).contains("changed-since-judgment")
    );
    assert!(
        f.manager()
            .recorded_work(&n("status"), &task.operation)
            .is_err()
    );
    assert_eq!(work_rows(&mut f), 0);
    std::fs::write(
        f.dir.path().join("counts.json"),
        r#"{"native_packages":"unavailable","support_profiles":3}"#,
    )
    .unwrap();
    assert!(
        error(f.manager().commit_work(&prepared).unwrap_err()).contains("readiness-not-satisfied")
    );
    assert_eq!(work_rows(&mut f), 0);
    f.counts(10, 3);
    let fresh = f.manager().prepare_work(task, &issuer).unwrap();
    f.manager().retire(Counter(1)).unwrap();
    assert!(error(f.manager().commit_work(&fresh).unwrap_err()).contains("registration-changed"));
    assert_eq!(work_rows(&mut f), 0);
}
#[test]
fn expiry_and_revocation_before_cut_do_not_commit_or_spend() {
    let _case = CASES.lock().unwrap_or_else(|p| p.into_inner());
    let mut issuer = Issuer::new();
    issuer.ttl = 1000;
    let mut f = Fixture::new(Some(issuer.policy()), false);
    let task = f.task(12, 6);
    let p = f.manager().prepare_work(task, &issuer).unwrap();
    while p.decision().check_live().is_ok() {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(error(f.manager().commit_work(&p).unwrap_err()).contains("expired"));
    assert_eq!(work_rows(&mut f), 0);
    issuer.ttl = 30_000;
    let task = f.task(12, 6);
    let p = f.manager().prepare_work(task, &issuer).unwrap();
    let signed = issuer
        .signer
        .revocation(p.decision().revocation_claim(n("test/withdrawn")));
    p.decision().revoke(&signed).unwrap();
    assert!(error(f.manager().commit_work(&p).unwrap_err()).contains("revoked"));
    assert_eq!(work_rows(&mut f), 0);
}
#[test]
fn rollback_spends_nothing_and_response_loss_is_resolved_by_query_not_replay() {
    let _case = CASES.lock().unwrap_or_else(|p| p.into_inner());
    let issuer = Issuer::new();
    let mut f = Fixture::new(Some(issuer.policy()), false);
    let task = f.task(12, 6);
    let p = f.manager().prepare_work(task.clone(), &issuer).unwrap();
    f.hook(
        task.operation.clone(),
        Box::new(|| {
            Err(rx_ports::StoreError::Unavailable(
                "test rollback after staged result and consumption".into(),
            ))
        }),
    );
    assert!(error(f.manager().commit_work(&p).unwrap_err()).contains("rollback"));
    assert_eq!(work_rows(&mut f), 0);
    assert!(
        f.manager()
            .recorded_work(&n("status"), &task.operation)
            .is_err()
    );
    f.manager().commit_work(&p).unwrap();
    assert_eq!(work_rows(&mut f), 2);
    assert!(error(f.manager().commit_work(&p).unwrap_err()).contains("already-committed"));
    assert_eq!(work_rows(&mut f), 2);
    let lost = f.task(12, 6);
    let p = f.manager().prepare_work(lost.clone(), &issuer).unwrap();
    f.hook(lost.operation.clone(), Box::new(|| Ok(true)));
    assert!(error(f.manager().commit_work(&p).unwrap_err()).contains("response lost"));
    let recovered = f
        .manager()
        .recorded_work(&n("status"), &lost.operation)
        .unwrap();
    assert_eq!(recovered.task, lost);
    assert_eq!(work_rows(&mut f), 4);
    let calls = issuer.calls.get();
    assert!(
        error(f.manager().prepare_work(lost, &issuer).unwrap_err()).contains("already-committed")
    );
    assert_eq!(issuer.calls.get(), calls);
    assert_eq!(work_rows(&mut f), 4);
    println!(
        "response_loss_recovered={}",
        serde_json::to_string(&recovered).unwrap()
    );
}
#[test]
fn receiver_restart_keeps_history_but_invalidates_retained_permission() {
    let _case = CASES.lock().unwrap_or_else(|p| p.into_inner());
    let issuer = Issuer::new();
    let mut f = Fixture::new(Some(issuer.policy()), false);
    let task = f.task(12, 6);
    let p = f.manager().prepare_work(task.clone(), &issuer).unwrap();
    let report = f.manager().commit_work(&p).unwrap();
    let next = f.task(12, 6);
    let pending = f.manager().prepare_work(next, &issuer).unwrap();
    f.reopen();
    assert_eq!(pending.decision().check_live(), Err(Failure::Epoch));
    assert!(f.manager().commit_work(&pending).is_err());
    assert_eq!(
        f.manager()
            .recorded_work(&n("status"), &task.operation)
            .unwrap(),
        report
    );
    assert_eq!(work_rows(&mut f), 2);
}
#[test]
fn revocation_waits_for_whole_transaction_and_does_not_rewrite_completed_work() {
    let _case = CASES.lock().unwrap_or_else(|p| p.into_inner());
    for rollback in [false, true] {
        let issuer = Issuer::new();
        let mut f = Fixture::new(Some(issuer.policy()), false);
        let task = f.task(12, 6);
        let p = f.manager().prepare_work(task.clone(), &issuer).unwrap();
        let proof = p.decision().clone();
        let signed = issuer
            .signer
            .revocation(proof.revocation_claim(n("test/during-commit")));
        let (go, wait) = std::sync::mpsc::channel();
        let (began, start) = std::sync::mpsc::channel();
        let (done, completed) = std::sync::mpsc::channel();
        let completed = std::sync::Arc::new(std::sync::Mutex::new(completed));
        let pending = completed.clone();
        let thread = std::thread::spawn(move || {
            wait.recv_timeout(Duration::from_secs(3)).unwrap();
            began.send(()).unwrap();
            let result = proof.revoke(&signed);
            done.send(()).unwrap();
            result
        });
        f.hook(
            task.operation.clone(),
            Box::new(move || {
                go.send(()).unwrap();
                start.recv_timeout(Duration::from_secs(3)).unwrap();
                std::thread::sleep(Duration::from_millis(80));
                assert!(
                    matches!(
                        pending.lock().unwrap().try_recv(),
                        Err(std::sync::mpsc::TryRecvError::Empty)
                    ),
                    "revocation interleaved with transaction"
                );
                if rollback {
                    Err(rx_ports::StoreError::Unavailable(
                        "rollback while revocation waits".into(),
                    ))
                } else {
                    Ok(false)
                }
            }),
        );
        let committed = f.manager().commit_work(&p);
        completed
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        thread.join().unwrap().unwrap();
        assert_eq!(p.decision().check_live(), Err(Failure::Revoked));
        if rollback {
            assert!(committed.is_err());
            assert_eq!(work_rows(&mut f), 0);
            assert!(error(f.manager().commit_work(&p).unwrap_err()).contains("revoked"));
            println!("revocation_after_rollback_refused=true");
        } else {
            let report = committed.unwrap();
            assert_eq!(
                f.manager()
                    .recorded_work(&n("status"), &task.operation)
                    .unwrap(),
                report
            );
            assert!(f.manager().commit_work(&p).is_err());
            assert_eq!(work_rows(&mut f), 2);
        }
    }
    println!("revocation_serialized_through_commit=true");
}
#[test]
fn expiry_after_logical_cut_during_commit_io_preserves_inert_result() {
    let _case = CASES.lock().unwrap_or_else(|p| p.into_inner());
    let mut issuer = Issuer::new();
    issuer.ttl = 1000;
    let mut f = Fixture::new(Some(issuer.policy()), false);
    let task = f.task(12, 6);
    let p = f.manager().prepare_work(task.clone(), &issuer).unwrap();
    f.hook(
        task.operation.clone(),
        Box::new(|| {
            std::thread::sleep(Duration::from_millis(1200));
            Ok(false)
        }),
    );
    let report = f.manager().commit_work(&p).unwrap();
    assert_eq!(p.decision().check_live(), Err(Failure::Expired));
    assert_eq!(
        f.manager()
            .recorded_work(&n("status"), &task.operation)
            .unwrap(),
        report
    );
    assert_eq!(work_rows(&mut f), 2);
    assert!(report.current_permission.contains("NONE"));
    println!(
        "expired_during_post_cut_io_result_retained={}",
        serde_json::to_string(&report).unwrap()
    );
}

#[test]
fn explicit_external_denial_and_missing_judgment_response_cannot_prepare_work() {
    let _case = CASES.lock().unwrap_or_else(|p| p.into_inner());
    let issuer = Issuer::new();
    let mut f = Fixture::new(Some(issuer.policy()), false);
    struct Denied;
    impl WorkUsePort for Denied {
        fn assess(&self, _: &WorkUseRequest) -> WorkUseReply {
            WorkUseReply::Denied {
                decision_reference: n("test/external-denial"),
                conditions: [(
                    n("test/current-policy"),
                    "test area refuses this task".into(),
                )]
                .into(),
            }
        }
    }
    struct LostResponse;
    impl WorkUsePort for LostResponse {
        fn assess(&self, _: &WorkUseRequest) -> WorkUseReply {
            WorkUseReply::NotEvaluated {
                conditions: [(
                    n("test/response-lost"),
                    "no decision response obtained".into(),
                )]
                .into(),
            }
        }
    }
    let task = f.task(12, 6);
    assert!(
        error(f.manager().prepare_work(task.clone(), &Denied).unwrap_err())
            .contains("external-judgment-Denied")
    );
    assert!(
        error(
            f.manager()
                .prepare_work(task.clone(), &LostResponse)
                .unwrap_err()
        )
        .contains("test/response-lost")
    );
    assert!(
        error(
            f.manager()
                .prepare_work(task, &NoWorkUseProvider)
                .unwrap_err()
        )
        .contains("external-judgment-Unsupported")
    );
    assert_eq!(work_rows(&mut f), 0);
}
