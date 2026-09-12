use rx_domain::{
    budget::{BudgetUnit, Consumption, RunBudget},
    canonical,
    types::*,
};
use rx_executor::{
    assignment_journal::{
        AssignmentJournal, Identity, Phase, Preparation, RecoveredFile, ServiceScope,
    },
    frame,
    journal::{self, Basis, Journal, Scope},
    lifecycle::{StopReason, StopRecord},
};
use rx_ports::*;
use rx_process_contract::{
    execution::{
        CHECKPOINT_SCHEMA, CheckpointView, PartDisposition, Purpose, Run, RunSnapshot, RunState,
    },
    production,
};
use rx_storage::SqliteRepository;
use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};

fn n(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id(v: u64) -> Id {
    Id::new(format!("00000000-0000-4000-8000-{v:012}")).unwrap()
}
fn d(v: u8) -> Digest {
    Digest::from_bytes([v; 32])
}
fn header() -> Identity {
    Identity {
        journal: id(1),
        scope: ServiceScope {
            installation: id(2),
            store_generation: id(3),
            principal: n("executor/a"),
            release: d(4),
            cell: n("cell/a"),
            definition: d(5),
        },
    }
}
fn prep(v: u64) -> Preparation {
    let h = header().scope;
    Preparation {
        id: id(100 + v),
        run_journal: id(200 + v),
        scope: Scope {
            installation: h.installation,
            store_generation: h.store_generation,
            principal: h.principal,
            release: h.release,
            cell: h.cell,
            definition: h.definition,
            run: id(300 + v),
            resolved_digest: d(6),
        },
        executor_session: id(7),
        epoch: Counter(1),
        basis: Basis {
            runtime_boot: id(8),
            sequence: Counter(1),
            run_revision: Counter(1),
            cell_revision: Counter(1),
            checked_at: TimePoint {
                clock_id: "test-clock".into(),
                ticks_ns: Counter(100),
            },
        },
    }
}
fn artifact(schema: &str, digest: Digest) -> ArtifactRef {
    ArtifactRef {
        schema_id: n(schema),
        sha256: digest,
        size_bytes: Counter(1),
    }
}
fn completion(p: &Preparation) -> production::View {
    let part = id(400);
    let mut budget = RunBudget::new(BudgetUnit::PartAttempt, Counter(1)).unwrap();
    budget
        .consume(Consumption::PartAttempt(part.clone()))
        .unwrap();
    production::View {
        schema: n(production::SCHEMA),
        installation: p.scope.installation.clone(),
        store_generation: p.scope.store_generation.clone(),
        runtime_boot: p.basis.runtime_boot.clone(),
        sequence: Counter(10),
        caller_session: p.executor_session.clone(),
        definition: artifact("rx.cell-definition.v1", p.scope.definition),
        resolved: artifact("rx.resolved-process.v1", p.scope.resolved_digest),
        cell_revision: Counter(3),
        cell_epoch: p.epoch,
        scope_epochs: [(n("scope/a"), Counter(1))].into(),
        checked_at: TimePoint {
            clock_id: "test-clock".into(),
            ticks_ns: Counter(200),
        },
        valid_until: TimePoint {
            clock_id: "test-clock".into(),
            ticks_ns: Counter(300),
        },
        run: RunSnapshot {
            revision: Counter(4),
            run: Run {
                id: p.scope.run.clone(),
                cell: p.scope.cell.clone(),
                recipe_digest: p.scope.resolved_digest,
                envelope_digest: d(9),
                purpose: Some(Purpose::Production),
                state: RunState::Completed,
                budget: Some(budget),
                executor_session: Some(p.executor_session.clone()),
                mandate: Some(id(11)),
                part_ids: vec![part.clone()],
                pending_attempt: None,
            },
            checkpoint: CheckpointView {
                run: p.scope.run.clone(),
                revision: Counter(4),
                executor_schema: n(CHECKPOINT_SCHEMA),
                payload: artifact(CHECKPOINT_SCHEMA, d(12)),
                activations: vec![],
            },
        },
        parts: vec![production::Part {
            id: part,
            run: p.scope.run.clone(),
            ordinal: Counter(1),
            revision: Counter(2),
            disposition: PartDisposition::ConfirmedCompleted,
        }],
        admission_allowed: false,
    }
}
fn init(path: &Path) -> AssignmentJournal<SqliteRepository> {
    AssignmentJournal::initialize_file(path, header()).unwrap()
}
fn open(path: &Path) -> AssignmentJournal<SqliteRepository> {
    AssignmentJournal::open_file_required(path, header()).unwrap()
}
fn attached(
    root: &Path,
) -> (
    AssignmentJournal<SqliteRepository>,
    Journal<SqliteRepository>,
    Preparation,
) {
    let mut journal = init(&root.join("service.sqlite3"));
    let p = prep(1);
    journal.prepare(p.clone()).unwrap();
    let run = journal.initialize_run_file(root, &p.id).unwrap();
    let (record, requests) = journal.attach(run).unwrap();
    assert_eq!(record.value.phase, Phase::Attached);
    (journal, requests, p)
}

struct FaultRepo {
    inner: SqliteRepository,
    mode: Arc<AtomicU8>,
}
impl Repository for FaultRepo {
    fn transact<T>(&mut self, f: impl FnOnce(&mut dyn Transaction) -> Result<T>) -> Result<T> {
        let mode = self.mode.swap(0, Ordering::SeqCst);
        let result = self.inner.transact(|tx| {
            let value = f(tx)?;
            if mode == 1 {
                Err(StoreError::Unavailable("before commit".into()))
            } else {
                Ok(value)
            }
        })?;
        if mode == 2 {
            Err(StoreError::Unavailable("commit reply lost".into()))
        } else {
            Ok(result)
        }
    }
    fn pending_outbox_after(&mut self, a: Option<&Id>, l: usize) -> Result<Vec<OutboxRecord>> {
        self.inner.pending_outbox_after(a, l)
    }
    fn control_events_after(&mut self, a: Counter, l: usize) -> Result<Vec<StoredEvent>> {
        self.inner.control_events_after(a, l)
    }
    fn control_snapshot(&mut self) -> Result<(Counter, Vec<Record>)> {
        self.inner.control_snapshot()
    }
    fn journal_head(&mut self) -> Result<Counter> {
        self.inner.journal_head()
    }
    fn pending_outbox(&mut self, l: usize) -> Result<Vec<OutboxRecord>> {
        self.inner.pending_outbox(l)
    }
    fn snapshot(&mut self) -> Result<(Counter, Vec<Record>)> {
        self.inner.snapshot()
    }
    fn events_after(&mut self, a: Counter, l: usize) -> Result<Vec<StoredEvent>> {
        self.inner.events_after(a, l)
    }
}
fn fault(path: &Path, mode: &Arc<AtomicU8>) -> FaultRepo {
    FaultRepo {
        inner: SqliteRepository::open(path).unwrap(),
        mode: mode.clone(),
    }
}

#[test]
fn required_open_never_initializes_missing_empty_or_foreign_service_metadata() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("service.sqlite3");
    assert!(AssignmentJournal::open_file_required(&path, header()).is_err());
    assert!(!path.exists());
    fs::write(&path, []).unwrap();
    assert!(AssignmentJournal::open_file_required(&path, header()).is_err());
    assert!(AssignmentJournal::initialize_file(&path, header()).is_err());
    assert_eq!(fs::metadata(&path).unwrap().len(), 0);
    fs::remove_file(&path).unwrap();
    drop(init(&path));
    assert!(AssignmentJournal::initialize_file(&path, header()).is_err());
    for changed in 0..7 {
        let mut h = header();
        match changed {
            0 => h.journal = id(99),
            1 => h.scope.installation = id(99),
            2 => h.scope.store_generation = id(99),
            3 => h.scope.principal = n("other"),
            4 => h.scope.release = d(99),
            5 => h.scope.cell = n("other"),
            _ => h.scope.definition = d(99),
        }
        assert!(AssignmentJournal::open_file_required(&path, h).is_err());
    }
    assert!(open(&path).current().unwrap().is_none());
}

#[test]
fn initialized_header_survives_a_lost_initialization_reply() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("service.sqlite3");
    let mode = Arc::new(AtomicU8::new(2));
    assert!(AssignmentJournal::initialize(fault(&path, &mode), header()).is_err());
    assert!(open(&path).current().unwrap().is_none());
}

#[test]
fn a_crash_before_header_commit_leaves_an_error_instead_of_reinitializing_the_file() {
    let root = tempfile::tempdir().unwrap();
    let service = root.path().join("partial-service.sqlite3");
    let mode = Arc::new(AtomicU8::new(1));
    assert!(AssignmentJournal::initialize(fault(&service, &mode), header()).is_err());
    assert!(AssignmentJournal::open_file_required(&service, header()).is_err());
    assert!(AssignmentJournal::initialize_file(&service, header()).is_err());

    let mut journal = init(&root.path().join("service.sqlite3"));
    let p = prep(1);
    journal.prepare(p.clone()).unwrap();
    let run_path = journal.run_file(root.path(), &p.id).unwrap();
    mode.store(1, Ordering::SeqCst);
    assert!(
        journal
            .initialize_run(&p.id, fault(&run_path, &mode))
            .is_err()
    );
    assert!(journal.open_run_file_required(root.path(), &p.id).is_err());
    assert!(journal.initialize_run_file(root.path(), &p.id).is_err());
    assert!(journal.recover_current_file(root.path()).is_err());
    assert_eq!(
        journal.current().unwrap().unwrap().value.phase,
        Phase::Preparing
    );
}

#[test]
fn preparing_rollback_and_lost_reply_preserve_exact_reservation_and_single_current() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("service.sqlite3");
    let mode = Arc::new(AtomicU8::new(0));
    let mut journal = AssignmentJournal::initialize(fault(&path, &mode), header()).unwrap();
    let p = prep(1);
    mode.store(1, Ordering::SeqCst);
    assert!(journal.prepare(p.clone()).is_err());
    assert!(journal.current().unwrap().is_none());
    mode.store(2, Ordering::SeqCst);
    assert!(journal.prepare(p.clone()).is_err());
    drop(journal);
    let mut journal = open(&path);
    let saved = journal.prepare(p.clone()).unwrap();
    assert_eq!(saved.revision, Counter(1));
    assert_eq!(saved.value.phase, Phase::Preparing);
    assert_eq!(
        canonical::bytes(&saved.value.preparation).unwrap(),
        canonical::bytes(&p).unwrap()
    );
    assert!(journal.prepare(prep(2)).is_err());
    let mut changed = p.clone();
    changed.basis.sequence = Counter(2);
    assert!(matches!(
        journal.prepare(changed),
        Err(StoreError::KeyConflict)
    ));
    let mut repository = journal.into_repository();
    assert_eq!(repository.events_after(Counter(0), 100).unwrap().len(), 1);
}

#[test]
fn preparing_missing_run_is_explicit_but_partial_or_attached_missing_run_is_an_error() {
    let root = tempfile::tempdir().unwrap();
    let mut journal = init(&root.path().join("service.sqlite3"));
    let p = prep(1);
    journal.prepare(p.clone()).unwrap();
    assert!(matches!(
        journal.recover_current_file(root.path()).unwrap(),
        Some(RecoveredFile::NeedsInitialization(_))
    ));
    let path = journal.run_file(root.path(), &p.id).unwrap();
    assert!(journal.open_run_file_required(root.path(), &p.id).is_err());
    assert!(!path.exists());
    fs::write(&path, []).unwrap();
    assert!(journal.recover_current_file(root.path()).is_err());
    assert!(journal.initialize_run_file(root.path(), &p.id).is_err());
    fs::remove_file(&path).unwrap();
    assert!(journal.creation_entered(&p.id).unwrap());
    assert!(journal.recover_current_file(root.path()).is_err());
    assert!(journal.initialize_run_file(root.path(), &p.id).is_err());
    assert!(!path.exists());

    let fresh = tempfile::tempdir().unwrap();
    let (mut journal, requests, p) = attached(fresh.path());
    let path = journal.run_file(fresh.path(), &p.id).unwrap();
    drop(requests);
    fs::remove_file(&path).unwrap();
    assert!(journal.recover_current_file(fresh.path()).is_err());
    assert!(journal.initialize_run_file(fresh.path(), &p.id).is_err());
    assert!(!path.exists());
}

#[test]
fn lost_run_initialization_reply_recovers_binding_without_reinitializing() {
    let root = tempfile::tempdir().unwrap();
    let mut journal = init(&root.path().join("service.sqlite3"));
    let p = prep(1);
    journal.prepare(p.clone()).unwrap();
    let mode = Arc::new(AtomicU8::new(2));
    let path = journal.run_file(root.path(), &p.id).unwrap();
    assert!(journal.initialize_run(&p.id, fault(&path, &mode)).is_err());
    assert_eq!(
        journal.current().unwrap().unwrap().value.phase,
        Phase::Preparing
    );
    assert!(journal.initialize_run_file(root.path(), &p.id).is_err());
    let run = journal.open_run_file_required(root.path(), &p.id).unwrap();
    let (record, requests) = journal.attach(run).unwrap();
    assert_eq!(record.value.phase, Phase::Attached);
    assert_eq!(requests.scope(), &p.scope);
}

#[test]
fn attach_rollback_and_lost_reply_are_recovered_without_a_second_transition() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("service.sqlite3");
    let mode = Arc::new(AtomicU8::new(0));
    let mut journal = AssignmentJournal::initialize(fault(&path, &mode), header()).unwrap();
    let p = prep(1);
    journal.prepare(p.clone()).unwrap();
    let run = journal.initialize_run_file(root.path(), &p.id).unwrap();
    mode.store(1, Ordering::SeqCst);
    assert!(journal.attach(run).is_err());
    assert_eq!(
        journal.current().unwrap().unwrap().value.phase,
        Phase::Preparing
    );
    let run = journal.open_run_file_required(root.path(), &p.id).unwrap();
    mode.store(2, Ordering::SeqCst);
    assert!(journal.attach(run).is_err());
    drop(journal);
    let mut journal = open(&path);
    let run = journal.open_run_file_required(root.path(), &p.id).unwrap();
    let (record, requests) = journal.attach(run).unwrap();
    assert_eq!(record.revision, Counter(2));
    drop(requests);
    assert_eq!(
        journal
            .into_repository()
            .events_after(Counter(0), 100)
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn unbound_legacy_journal_is_not_adopted_or_modified() {
    let root = tempfile::tempdir().unwrap();
    let mut journal = init(&root.path().join("service.sqlite3"));
    let p = prep(1);
    journal.prepare(p.clone()).unwrap();
    let path = journal.run_file(root.path(), &p.id).unwrap();
    drop(Journal::open(SqliteRepository::open(&path).unwrap(), p.scope.clone()).unwrap());
    assert!(journal.open_run_file_required(root.path(), &p.id).is_err());
    assert!(journal.initialize_run_file(root.path(), &p.id).is_err());
    let mut repository = SqliteRepository::open(path).unwrap();
    assert!(
        repository
            .transact(|tx| tx.get(&n("executor/attachment-binding")))
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_run_store_from_another_service_identity_cannot_be_attached() {
    let root = tempfile::tempdir().unwrap();
    let mut a = init(&root.path().join("a.sqlite3"));
    let mut other = header();
    other.journal = id(500);
    let mut b = AssignmentJournal::initialize_file(&root.path().join("b.sqlite3"), other).unwrap();
    let p = prep(1);
    a.prepare(p.clone()).unwrap();
    b.prepare(p.clone()).unwrap();
    let run = a.initialize_run_file(root.path(), &p.id).unwrap();
    assert!(b.attach(run).is_err());
    assert_eq!(b.current().unwrap().unwrap().value.phase, Phase::Preparing);
    assert!(b.open_run_file_required(root.path(), &p.id).is_err());
}

#[test]
fn changed_scope_or_run_journal_identity_is_never_repaired() {
    for key in ["executor/header", "executor/attachment-binding"] {
        let root = tempfile::tempdir().unwrap();
        let mut journal = init(&root.path().join("service.sqlite3"));
        let p = prep(1);
        journal.prepare(p.clone()).unwrap();
        let run = journal.initialize_run_file(root.path(), &p.id).unwrap();
        let mut repository = run.into_repository();
        repository
            .transact(|tx| {
                let mut row = tx.get(&n(key))?.unwrap();
                if key == "executor/header" {
                    row.document.value["resolved_digest"] = serde_json::json!(d(99));
                } else {
                    row.document.value["preparation"]["run_journal"] = serde_json::json!(id(99));
                }
                tx.put(&row.key, Some(row.revision), &row.document)?;
                Ok(())
            })
            .unwrap();
        assert!(journal.open_run_required(&p.id, repository).is_err());
        assert_eq!(
            journal.current().unwrap().unwrap().value.phase,
            Phase::Preparing
        );
    }
}

#[test]
fn completion_rejects_paused_recovery_abandoned_wrong_identity_and_incomplete_parts() {
    let root = tempfile::tempdir().unwrap();
    let (mut journal, requests, p) = attached(root.path());
    let mut run = journal
        .open_run_required(&p.id, requests.into_repository())
        .unwrap();
    for state in [
        RunState::Paused,
        RunState::RecoveryRequired,
        RunState::Abandoned,
    ] {
        let mut view = completion(&p);
        view.run.run.state = state;
        assert!(journal.close_completed(&mut run, view).is_err());
    }
    for changed in 0..7 {
        let mut view = completion(&p);
        match changed {
            0 => view.installation = id(99),
            1 => view.store_generation = id(99),
            2 => view.definition.sha256 = d(99),
            3 => view.run.run.cell = n("other"),
            4 => view.run.run.executor_session = Some(id(99)),
            5 => view.parts[0].disposition = PartDisposition::InProgress,
            _ => view.resolved.sha256 = d(99),
        }
        assert!(journal.close_completed(&mut run, view).is_err());
    }
    assert_eq!(
        journal.current().unwrap().unwrap().value.phase,
        Phase::Attached
    );
}

#[test]
fn preparing_cannot_close_and_closed_history_cannot_rebind_the_same_run() {
    let root = tempfile::tempdir().unwrap();
    let mut journal = init(&root.path().join("service.sqlite3"));
    let p = prep(1);
    journal.prepare(p.clone()).unwrap();
    let mut run = journal.initialize_run_file(root.path(), &p.id).unwrap();
    assert!(journal.close_completed(&mut run, completion(&p)).is_err());
    let (_, requests) = journal.attach(run).unwrap();
    let mut run = journal
        .open_run_required(&p.id, requests.into_repository())
        .unwrap();
    let closed = journal.close_completed(&mut run, completion(&p)).unwrap();
    assert_eq!(closed.revision, Counter(3));
    assert!(journal.current().unwrap().is_none());
    assert_eq!(
        journal
            .close_completed(&mut run, completion(&p))
            .unwrap()
            .revision,
        Counter(3)
    );
    let mut changed = completion(&p);
    changed.sequence = Counter(11);
    assert!(matches!(
        journal.close_completed(&mut run, changed),
        Err(StoreError::KeyConflict)
    ));
    assert!(journal.attach(run).is_err());
    assert_eq!(
        journal.prepare(p.clone()).unwrap().value.phase,
        Phase::Closed
    );
    let mut rebind = p.clone();
    rebind.id = id(999);
    assert!(matches!(
        journal.prepare(rebind),
        Err(StoreError::KeyConflict)
    ));
    let next = prep(2);
    assert_eq!(
        journal.prepare(next.clone()).unwrap().value.phase,
        Phase::Preparing
    );
    let mut old = journal.open_run_file_required(root.path(), &p.id).unwrap();
    journal.close_completed(&mut old, completion(&p)).unwrap();
    assert_eq!(
        journal.current().unwrap().unwrap().value.preparation.id,
        next.id
    );
}

#[test]
fn close_rollback_and_lost_reply_clear_current_atomically_and_preserve_stop_and_pending_requests() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("service.sqlite3");
    let mode = Arc::new(AtomicU8::new(0));
    let mut journal = AssignmentJournal::initialize(fault(&path, &mode), header()).unwrap();
    let p = prep(1);
    journal.prepare(p.clone()).unwrap();
    let run = journal.initialize_run_file(root.path(), &p.id).unwrap();
    let (_, mut requests) = journal.attach(run).unwrap();
    let logical = journal::Logical {
        visit: Counter(1),
        node: n("node/a"),
        stage: journal::Stage::ResolveActivation,
        control: None,
    };
    let context = frame::Identity {
        run: p.scope.run.clone(),
        executor_session: p.executor_session.clone(),
        resolved_digest: p.scope.resolved_digest,
        visit: Counter(1),
        epoch: p.epoch,
    };
    let pending = requests
        .prepare(
            logical.clone(),
            context.clone(),
            journal::Body::ResolveActivation {
                run: p.scope.run.clone(),
                node: n("node/a"),
                visit: Counter(1),
                expected_run: Counter(1),
            },
            p.basis.clone(),
        )
        .unwrap();
    requests.enter(&pending.key).unwrap();
    let stop = StopRecord::new(
        p.scope.run.clone(),
        p.executor_session.clone(),
        Some(context),
        StopReason::Requested,
        Some(p.basis.checked_at.clone()),
    );
    requests.ensure_stop(&stop).unwrap();
    let mut run = journal
        .open_run_required(&p.id, requests.into_repository())
        .unwrap();
    mode.store(1, Ordering::SeqCst);
    assert!(journal.close_completed(&mut run, completion(&p)).is_err());
    assert_eq!(
        journal.current().unwrap().unwrap().value.phase,
        Phase::Attached
    );
    mode.store(2, Ordering::SeqCst);
    assert!(journal.close_completed(&mut run, completion(&p)).is_err());
    drop(journal);
    let mut journal = open(&path);
    assert!(journal.current().unwrap().is_none());
    assert_eq!(
        journal
            .close_completed(&mut run, completion(&p))
            .unwrap()
            .revision,
        Counter(3)
    );
    let mut requests = Journal::open(run.into_repository(), p.scope).unwrap();
    let saved = requests.get(&logical).unwrap().unwrap();
    assert_eq!(saved.key, pending.key);
    assert_eq!(saved.send, journal::SendState::EmitEntered);
    assert!(matches!(saved.resolution, journal::Resolution::Pending));
    assert_eq!(
        canonical::bytes(&requests.stop_record().unwrap().unwrap().record).unwrap(),
        canonical::bytes(&stop).unwrap()
    );
    assert_eq!(
        journal
            .into_repository()
            .events_after(Counter(0), 100)
            .unwrap()
            .len(),
        4
    );
}

#[test]
fn malformed_current_pointer_fails_required_open_instead_of_hiding_the_attached_run() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("service.sqlite3");
    let mut journal = init(&path);
    journal.prepare(prep(1)).unwrap();
    let mut repository = journal.into_repository();
    repository
        .transact(|tx| {
            let mut row = tx.get(&n("attachment/state"))?.unwrap();
            row.document.value["current"] = serde_json::Value::Null;
            tx.put(&row.key, Some(row.revision), &row.document)?;
            Ok(())
        })
        .unwrap();
    assert!(AssignmentJournal::open_required(repository, header()).is_err());
}

#[cfg(unix)]
#[test]
fn required_run_file_does_not_follow_symlinks() {
    let root = tempfile::tempdir().unwrap();
    let mut journal = init(&root.path().join("service.sqlite3"));
    let p = prep(1);
    journal.prepare(p.clone()).unwrap();
    let target = root.path().join("unrelated");
    fs::write(&target, b"unchanged").unwrap();
    std::os::unix::fs::symlink(&target, journal.run_file(root.path(), &p.id).unwrap()).unwrap();
    assert!(journal.open_run_file_required(root.path(), &p.id).is_err());
    assert_eq!(fs::read(target).unwrap(), b"unchanged");
}

#[test]
fn initialized_preparing_run_file_loss_never_reuses_either_initializer() {
    for lost_reply in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let service = root.path().join("service.sqlite3");
        let mut journal = init(&service);
        let p = prep(1);
        journal.prepare(p.clone()).unwrap();
        let path = journal.run_file(root.path(), &p.id).unwrap();
        if lost_reply {
            let mode = Arc::new(AtomicU8::new(2));
            assert!(journal.initialize_run(&p.id, fault(&path, &mode)).is_err());
        } else {
            drop(journal.initialize_run_file(root.path(), &p.id).unwrap());
        }
        // Both a successful initializer and a lost header-commit response recover the original store.
        let Some(RecoveredFile::Existing { attachment, run }) =
            journal.recover_current_file(root.path()).unwrap()
        else {
            panic!("original run header must be recoverable");
        };
        assert_eq!(attachment.revision, Counter(1));
        assert_eq!(attachment.value.phase, Phase::Preparing);
        drop(run);
        drop(journal);
        fs::remove_file(&path).unwrap();

        let mut journal = open(&service);
        assert!(journal.creation_entered(&p.id).unwrap());
        assert!(journal.recover_current_file(root.path()).is_err());
        assert!(journal.initialize_run_file(root.path(), &p.id).is_err());
        assert!(!path.exists());
        let replacement = root.path().join("replacement.sqlite3");
        assert!(
            journal
                .initialize_run(&p.id, SqliteRepository::open(&replacement).unwrap())
                .is_err()
        );
        let mut replacement = SqliteRepository::open(replacement).unwrap();
        assert!(
            replacement
                .transact(|tx| tx.get(&n("executor/header")))
                .unwrap()
                .is_none()
        );
        assert_eq!(journal.current().unwrap().unwrap().revision, Counter(1));
    }
}

#[test]
fn lost_creation_marker_reply_stops_before_file_reservation_and_survives_reopen() {
    let root = tempfile::tempdir().unwrap();
    let service = root.path().join("service.sqlite3");
    let mode = Arc::new(AtomicU8::new(0));
    let mut journal = AssignmentJournal::initialize(fault(&service, &mode), header()).unwrap();
    let p = prep(1);
    journal.prepare(p.clone()).unwrap();
    let path = journal.run_file(root.path(), &p.id).unwrap();
    mode.store(2, Ordering::SeqCst);
    assert!(journal.initialize_run_file(root.path(), &p.id).is_err());
    assert!(
        !path.exists(),
        "a lost marker response must not reach create_new"
    );
    drop(journal);

    let mut journal = open(&service);
    assert!(journal.creation_entered(&p.id).unwrap());
    assert!(journal.recover_current_file(root.path()).is_err());
    assert!(journal.initialize_run_file(root.path(), &p.id).is_err());
    assert!(!path.exists());
    let current = journal.current().unwrap().unwrap();
    assert_eq!(current.revision, Counter(1));
    assert_eq!(current.value.phase, Phase::Preparing);
}

#[test]
fn creation_marker_committed_before_filesystem_failure_forbids_later_retry() {
    let root = tempfile::tempdir().unwrap();
    let service = root.path().join("service.sqlite3");
    let mut journal = init(&service);
    let p = prep(1);
    journal.prepare(p.clone()).unwrap();
    let run_root = root.path().join("not-created");
    assert!(journal.initialize_run_file(&run_root, &p.id).is_err());
    assert!(journal.creation_entered(&p.id).unwrap());
    assert!(!run_root.exists());
    drop(journal);

    fs::create_dir(&run_root).unwrap();
    let mut journal = open(&service);
    assert!(journal.recover_current_file(&run_root).is_err());
    assert!(journal.initialize_run_file(&run_root, &p.id).is_err());
    assert!(!journal.run_file(&run_root, &p.id).unwrap().exists());
}

#[test]
fn creation_marker_rollback_permits_the_first_initialization_without_changing_phase() {
    let root = tempfile::tempdir().unwrap();
    let service = root.path().join("service.sqlite3");
    let mode = Arc::new(AtomicU8::new(0));
    let mut journal = AssignmentJournal::initialize(fault(&service, &mode), header()).unwrap();
    let p = prep(1);
    journal.prepare(p.clone()).unwrap();
    let path = journal.run_file(root.path(), &p.id).unwrap();
    mode.store(1, Ordering::SeqCst);
    assert!(journal.initialize_run_file(root.path(), &p.id).is_err());
    assert!(!path.exists());
    assert!(!journal.creation_entered(&p.id).unwrap());
    assert!(matches!(
        journal.recover_current_file(root.path()).unwrap(),
        Some(RecoveredFile::NeedsInitialization(_))
    ));
    let run = journal.initialize_run_file(root.path(), &p.id).unwrap();
    assert!(journal.creation_entered(&p.id).unwrap());
    assert_eq!(journal.current().unwrap().unwrap().revision, Counter(1));
    let (record, requests) = journal.attach(run).unwrap();
    assert_eq!(record.revision, Counter(2));
    assert_eq!(requests.scope(), &p.scope);
}

#[test]
fn old_header_without_creation_history_is_not_silently_upgraded() {
    let root = tempfile::tempdir().unwrap();
    let mut repository = SqliteRepository::open(root.path().join("legacy.sqlite3")).unwrap();
    repository
        .transact(|tx| {
            tx.put(
                &n("attachment/header"),
                None,
                &Document {
                    schema: n("rx.executor-attachment-header.v1"),
                    value: serde_json::to_value(header()).unwrap(),
                },
            )?;
            tx.put(
                &n("attachment/state"),
                None,
                &Document {
                    schema: n("rx.executor-attachment-state.v1"),
                    value: serde_json::json!({ "current": null, "count": "0" }),
                },
            )?;
            Ok(())
        })
        .unwrap();
    assert!(AssignmentJournal::open_required(repository, header()).is_err());
}

#[test]
fn run_creation_marker_cannot_change_after_header_initialization() {
    let root = tempfile::tempdir().unwrap();
    let mut journal = init(&root.path().join("service.sqlite3"));
    let p = prep(1);
    journal.prepare(p.clone()).unwrap();
    drop(journal.initialize_run_file(root.path(), &p.id).unwrap());
    let mut repository = journal.into_repository();
    repository
        .transact(|tx| {
            let key = n(&format!("attachment/creation-entered/{}", p.id));
            let mut marker = tx.get(&key)?.unwrap();
            marker.document.value["run_journal"] = serde_json::json!(id(999));
            tx.put(&key, Some(marker.revision), &marker.document)?;
            Ok(())
        })
        .unwrap();
    assert!(AssignmentJournal::open_required(repository, header()).is_err());
}
