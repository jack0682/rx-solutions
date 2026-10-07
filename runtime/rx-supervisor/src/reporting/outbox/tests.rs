use super::*;
use crate::registration::{Binding, CatalogReference, Observation};
use rx_storage::SqliteRepository;
fn context() -> (
    tempfile::TempDir,
    Outbox<SqliteRepository>,
    Peer,
    VerifiedScope,
    Execution,
) {
    let dir = tempfile::tempdir().unwrap();
    let outbox = Outbox::new(SqliteRepository::open(dir.path().join("outbox.db")).unwrap());
    let peer = Peer {
        id: id(),
        principal: name("reporter"),
        peer_boot: id(),
        installation: id(),
        store_generation: id(),
        runtime_boot: id(),
        authentication_binding: Digest::from_bytes([1; 32]),
    };
    let scope = Scope {
        id: id(),
        component: id(),
        component_revision: Counter(1),
        source_registration: id(),
        source_revision: Counter(1),
        catalog: CatalogReference {
            program: name("status"),
            digest: Digest::from_bytes([2; 32]),
        },
        reporter_session: peer.id.clone(),
        issued_by: name("owner"),
        issued_at: TimePoint {
            clock_id: "test".into(),
            ticks_ns: Counter(1),
        },
        active: true,
        continuation: None,
    };
    let execution = Execution {
        binding: Binding {
            registration: scope.source_registration.clone(),
            registration_revision: scope.source_revision,
            catalog: scope.catalog.clone(),
            run: id(),
            selection: name("status"),
            instance: id(),
        },
        last_observed: Observation {
            state: ExecutionState::Running,
            pid: Some(5),
            exit_code: None,
            detail: "retained registry snapshot".into(),
            process_identity: None,
        },
    };
    (dir, outbox, peer, VerifiedScope(scope), execution)
}
fn head(scope: &VerifiedScope, execution: &Execution, receipt: Option<Receipt>) -> Head {
    Head {
        scope: scope.0.id.clone(),
        instance: execution.binding.instance.clone(),
        receipt,
    }
}
fn receipt(p: &Pending) -> Receipt {
    Receipt {
        id: id(),
        component: p.scope.component.clone(),
        component_revision: p.scope.component_revision,
        reporter: p.peer.clone(),
        report: p.report.clone(),
        accepted_at: TimePoint {
            clock_id: "test".into(),
            ticks_ns: Counter(2),
        },
        basis: Basis::ReportedRegistrySnapshot,
        execution_ownership: Ownership::NotEstablishedByReport,
        work_use_permission: WorkUse::NotEvaluated,
    }
}
#[test]
fn pending_bytes_survive_new_snapshot_and_database_reopen() {
    let (dir, mut outbox, peer, scope, mut execution) = context();
    outbox.observe(execution.clone()).unwrap();
    let pending = outbox
        .prepare(&peer, &scope, &head(&scope, &execution, None))
        .unwrap()
        .unwrap();
    let bytes = canonical::bytes(&pending).unwrap();
    execution.last_observed.state = ExecutionState::Exited;
    execution.last_observed.exit_code = Some(0);
    outbox.observe(execution.clone()).unwrap();
    outbox.into_repository().close().unwrap();
    let mut outbox = Outbox::new(SqliteRepository::open(dir.path().join("outbox.db")).unwrap());
    let replay = outbox
        .prepare(&peer, &scope, &head(&scope, &execution, None))
        .unwrap()
        .unwrap();
    assert_eq!(canonical::bytes(&replay).unwrap(), bytes);
    let accepted = receipt(&pending);
    outbox.acknowledge(&pending, accepted.clone()).unwrap();
    outbox.acknowledge(&pending, accepted.clone()).unwrap();
    let next = outbox
        .prepare(&peer, &scope, &head(&scope, &execution, Some(accepted)))
        .unwrap()
        .unwrap();
    assert_ne!(next.key, pending.key);
    assert_eq!(next.report.sequence, Counter(2));
    assert_eq!(next.report.state, ExecutionState::Exited);
}
#[test]
fn restart_needs_owner_continuation_and_preserves_unknown_prior_delivery() {
    for committed in [false, true] {
        let (_dir, mut outbox, peer, scope, execution) = context();
        outbox.observe(execution.clone()).unwrap();
        let old = outbox
            .prepare(&peer, &scope, &head(&scope, &execution, None))
            .unwrap()
            .unwrap();
        let next_peer = Peer {
            id: id(),
            peer_boot: id(),
            ..peer
        };
        let mut next_scope = scope.0.clone();
        next_scope.id = id();
        next_scope.reporter_session = next_peer.id.clone();
        let mut successor = VerifiedScope(next_scope);
        assert!(
            outbox
                .prepare(&next_peer, &successor, &head(&successor, &execution, None))
                .is_err()
        );
        successor.0.continuation = Some(Continuation {
            previous_scope: scope.0.id.clone(),
            root_scope: scope.0.id,
        });
        let prior = committed.then(|| receipt(&old));
        let next = outbox
            .prepare(&next_peer, &successor, &head(&successor, &execution, prior))
            .unwrap()
            .unwrap();
        assert_ne!(next.key, old.key);
        assert_ne!(next.peer.id, old.peer.id);
        assert_eq!(next.report.sequence, Counter(if committed { 2 } else { 1 }));
        let row = outbox
            .inspect(&execution.binding.instance)
            .unwrap()
            .unwrap();
        assert_eq!(row.unresolved_deliveries.0, if committed { 0 } else { 1 });
        let mut repository = outbox.into_repository();
        let events = repository.control_events_after(Counter(0), 100).unwrap();
        assert!(
            events
                .iter()
                .any(|e| e.document.value["pending"]["key"]
                    == serde_json::to_value(&old.key).unwrap())
        );
    }
}
#[test]
fn forged_ack_and_regressed_server_head_never_advance_sequence() {
    let (_dir, mut outbox, peer, scope, execution) = context();
    outbox.observe(execution.clone()).unwrap();
    let pending = outbox
        .prepare(&peer, &scope, &head(&scope, &execution, None))
        .unwrap()
        .unwrap();
    let mut forged = receipt(&pending);
    forged.report.source.instance = id();
    assert!(outbox.acknowledge(&pending, forged).is_err());
    assert_eq!(
        outbox
            .inspect(&execution.binding.instance)
            .unwrap()
            .unwrap()
            .pending,
        Some(pending.clone())
    );
    let accepted = receipt(&pending);
    outbox.acknowledge(&pending, accepted.clone()).unwrap();
    let before = outbox.repository.snapshot().unwrap();
    for _ in 0..3 {
        assert!(
            outbox
                .prepare(
                    &peer,
                    &scope,
                    &head(&scope, &execution, Some(accepted.clone()))
                )
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(outbox.repository.snapshot().unwrap(), before);
    assert!(
        outbox
            .prepare(&peer, &scope, &head(&scope, &execution, None))
            .is_err()
    );
}

#[path = "../../../tests/support/replacement_store.rs"]
mod faults;

#[test]
fn storage_rollback_and_lost_ack_commit_preserve_exact_request() {
    let (dir, outbox, peer, scope, execution) = context();
    outbox.into_repository().close().unwrap();
    let store = faults::SharedStore::new(dir.path().join("outbox.db"));
    let mut outbox = Outbox::new(store.clone());
    outbox.observe(execution.clone()).unwrap();
    store.fail_after_put("resident-report/outbox/");
    assert!(
        outbox
            .prepare(&peer, &scope, &head(&scope, &execution, None))
            .is_err()
    );
    assert!(
        outbox
            .inspect(&execution.binding.instance)
            .unwrap()
            .unwrap()
            .pending
            .is_none()
    );
    store.hook(key(&execution.binding.instance), Box::new(|| Ok(true)));
    assert!(
        outbox
            .prepare(&peer, &scope, &head(&scope, &execution, None))
            .is_err()
    );
    let persisted = outbox
        .inspect(&execution.binding.instance)
        .unwrap()
        .unwrap()
        .pending
        .unwrap();
    let recovered = outbox
        .prepare(&peer, &scope, &head(&scope, &execution, None))
        .unwrap()
        .unwrap();
    assert_eq!(persisted, recovered);
    let accepted = receipt(&persisted);
    store.hook(key(&execution.binding.instance), Box::new(|| Ok(true)));
    assert!(outbox.acknowledge(&persisted, accepted.clone()).is_err());
    outbox.acknowledge(&persisted, accepted).unwrap();
    assert_eq!(store.rows().len(), 1);
}

#[test]
fn pending_capacity_refuses_new_request_without_losing_existing_requests() {
    let (_dir, mut outbox, peer, scope, execution) = context();
    let mut first = None;
    for _ in 0..MAX_PENDING {
        let mut e = execution.clone();
        e.binding.instance = id();
        outbox.observe(e.clone()).unwrap();
        let pending = outbox
            .prepare(&peer, &scope, &head(&scope, &e, None))
            .unwrap()
            .unwrap();
        first.get_or_insert(pending);
    }
    outbox.observe(execution.clone()).unwrap();
    assert!(
        outbox
            .prepare(&peer, &scope, &head(&scope, &execution, None))
            .is_err()
    );
    let first = first.unwrap();
    assert_eq!(
        outbox
            .inspect(&first.report.source.instance)
            .unwrap()
            .unwrap()
            .pending,
        Some(first.clone())
    );
    outbox.acknowledge(&first, receipt(&first)).unwrap();
    assert!(
        outbox
            .prepare(&peer, &scope, &head(&scope, &execution, None))
            .unwrap()
            .is_some()
    );
}

#[test]
fn unsent_prior_run_is_discoverable_after_reopen_without_current_run_input() {
    let (dir, mut outbox, peer, scope, execution) = context();
    outbox.observe(execution.clone()).unwrap();
    outbox.into_repository().close().unwrap();
    let mut outbox = Outbox::new(SqliteRepository::open(dir.path().join("outbox.db")).unwrap());
    let (retained, total) = outbox.retained(128).unwrap();
    assert_eq!(retained, vec![execution.clone()]);
    assert_eq!(total, 1);
    let pending = outbox
        .prepare(&peer, &scope, &head(&scope, &execution, None))
        .unwrap()
        .unwrap();
    outbox.acknowledge(&pending, receipt(&pending)).unwrap();
    assert_eq!(outbox.retained(128).unwrap(), (vec![], 0));
}

#[test]
fn same_sequence_different_receipt_and_restored_store_require_reconciliation() {
    let (_dir, mut outbox, peer, scope, execution) = context();
    outbox.observe(execution.clone()).unwrap();
    let pending = outbox
        .prepare(&peer, &scope, &head(&scope, &execution, None))
        .unwrap()
        .unwrap();
    let accepted = receipt(&pending);
    outbox.acknowledge(&pending, accepted.clone()).unwrap();
    let mut different = accepted.clone();
    different.id = id();
    assert!(
        outbox
            .prepare(&peer, &scope, &head(&scope, &execution, Some(different)))
            .is_err()
    );
    let restored = Peer {
        id: id(),
        store_generation: id(),
        ..peer
    };
    let mut next = scope.0.clone();
    next.id = id();
    next.reporter_session = restored.id.clone();
    next.continuation = Some(Continuation {
        previous_scope: scope.0.id.clone(),
        root_scope: scope.0.id,
    });
    let next = VerifiedScope(next);
    assert!(
        outbox
            .prepare(&restored, &next, &head(&next, &execution, Some(accepted)))
            .is_err()
    );
}
