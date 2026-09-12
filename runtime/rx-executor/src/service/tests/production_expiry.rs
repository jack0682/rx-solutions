use super::*;
use crate::{
    client::ValidatedProduction,
    journal::{Journal, Scope},
    worker::Coordination,
};
use rx_domain::{
    budget::{BudgetUnit, Consumption, RunBudget},
    types::*,
};
use rx_ports::{OutboxRecord, Record, Repository, StoredEvent, Transaction};
use rx_process_contract::{
    execution::{CHECKPOINT_SCHEMA, CheckpointView, PartDisposition, Purpose, Run, RunSnapshot},
    production,
};
use rx_storage::SqliteRepository;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const WINDOW_NS: u64 = 100_000_000;
fn id(value: u64) -> Id {
    Id::new(format!("00000000-0000-4000-8000-{value:012}")).unwrap()
}
fn name(value: &str) -> Name {
    Name::new(value).unwrap()
}
fn artifact(schema: &str, byte: u8) -> ArtifactRef {
    ArtifactRef {
        schema_id: name(schema),
        sha256: Digest::from_bytes([byte; 32]),
        size_bytes: Counter(1),
    }
}
struct ManualClock(AtomicU64);
impl Clock for ManualClock {
    fn now(&self) -> Result<TimePoint, Error> {
        Ok(TimePoint {
            clock_id: "test/production-expiry".into(),
            ticks_ns: Counter(self.0.load(Ordering::SeqCst)),
        })
    }
}
struct ExpiryRepository {
    inner: SqliteRepository,
    clock: Arc<ManualClock>,
    expire_after_commit: Arc<AtomicBool>,
    expire_at: u64,
}
impl Repository for ExpiryRepository {
    fn transact<T>(
        &mut self,
        operation: impl FnOnce(&mut dyn Transaction) -> rx_ports::Result<T>,
    ) -> rx_ports::Result<T> {
        let result = self.inner.transact(operation)?;
        if self.expire_after_commit.swap(false, Ordering::SeqCst) {
            self.clock.0.store(self.expire_at, Ordering::SeqCst);
        }
        Ok(result)
    }
    fn pending_outbox_after(
        &mut self,
        after: Option<&Id>,
        limit: usize,
    ) -> rx_ports::Result<Vec<OutboxRecord>> {
        self.inner.pending_outbox_after(after, limit)
    }
    fn control_events_after(
        &mut self,
        after: Counter,
        limit: usize,
    ) -> rx_ports::Result<Vec<StoredEvent>> {
        self.inner.control_events_after(after, limit)
    }
    fn control_snapshot(&mut self) -> rx_ports::Result<(Counter, Vec<Record>)> {
        self.inner.control_snapshot()
    }
    fn journal_head(&mut self) -> rx_ports::Result<Counter> {
        self.inner.journal_head()
    }
    fn pending_outbox(&mut self, limit: usize) -> rx_ports::Result<Vec<OutboxRecord>> {
        self.inner.pending_outbox(limit)
    }
    fn snapshot(&mut self) -> rx_ports::Result<(Counter, Vec<Record>)> {
        self.inner.snapshot()
    }
    fn events_after(&mut self, after: Counter, limit: usize) -> rx_ports::Result<Vec<StoredEvent>> {
        self.inner.events_after(after, limit)
    }
}
fn completed() -> production::View {
    let run = id(1);
    let session = id(2);
    let part = id(3);
    let mut budget = RunBudget::new(BudgetUnit::PartAttempt, Counter(1)).unwrap();
    budget
        .consume(Consumption::PartAttempt(part.clone()))
        .unwrap();
    let resolved = artifact("rx.resolved-process.v1", 1);
    production::View {
        schema: name(production::SCHEMA),
        installation: id(4),
        store_generation: id(5),
        runtime_boot: id(6),
        sequence: Counter(1),
        caller_session: session.clone(),
        definition: artifact("rx.cell-definition.v1", 2),
        resolved: resolved.clone(),
        cell_revision: Counter(1),
        cell_epoch: Counter(1),
        scope_epochs: [(name("scope/a"), Counter(1))].into(),
        checked_at: TimePoint {
            clock_id: "test/production-expiry".into(),
            ticks_ns: Counter(1000),
        },
        valid_until: TimePoint {
            clock_id: "test/production-expiry".into(),
            ticks_ns: Counter(1000 + WINDOW_NS),
        },
        run: RunSnapshot {
            revision: Counter(2),
            run: Run {
                id: run.clone(),
                cell: name("cell/a"),
                recipe_digest: resolved.sha256,
                envelope_digest: Digest::from_bytes([3; 32]),
                purpose: Some(Purpose::Production),
                state: RunState::Completed,
                budget: Some(budget),
                executor_session: Some(session),
                mandate: Some(id(7)),
                part_ids: vec![part.clone()],
                pending_attempt: None,
            },
            checkpoint: CheckpointView {
                run: run.clone(),
                revision: Counter(2),
                executor_schema: name(CHECKPOINT_SCHEMA),
                payload: artifact(CHECKPOINT_SCHEMA, 4),
                activations: vec![],
            },
        },
        parts: vec![production::Part {
            id: part,
            run,
            ordinal: Counter(1),
            revision: Counter(2),
            disposition: PartDisposition::ConfirmedCompleted,
        }],
        admission_allowed: false,
    }
}
fn status(view: &production::View) -> Status {
    Status {
        phase: Phase::GraphComplete,
        session: view.caller_session.clone(),
        run: view.run.run.id.clone(),
        visit: Counter(1),
        sequence: Some(view.sequence),
        admission: true,
    }
}

#[tokio::test]
async fn journal_observation_expiry_retries_completed_read_instead_of_stopping_context() {
    let root = tempfile::tempdir().unwrap();
    let raw = completed();
    let clock = Arc::new(ManualClock(AtomicU64::new(raw.checked_at.ticks_ns.0)));
    let advance = Arc::new(AtomicBool::new(false));
    let principal = name("executor/a");
    let release = Digest::from_bytes([8; 32]);
    let journal = Journal::open(
        ExpiryRepository {
            inner: SqliteRepository::open(root.path().join("requests.sqlite3")).unwrap(),
            clock: clock.clone(),
            expire_after_commit: advance.clone(),
            expire_at: raw.valid_until.ticks_ns.0,
        },
        Scope {
            installation: raw.installation.clone(),
            store_generation: raw.store_generation.clone(),
            principal: principal.clone(),
            release,
            cell: raw.run.run.cell.clone(),
            definition: raw.definition.sha256,
            run: raw.run.run.id.clone(),
            resolved_digest: raw.resolved.sha256,
        },
    )
    .unwrap();
    let (client, view) = ValidatedProduction::with_test_client(
        raw.clone(),
        clock.clone(),
        principal.clone(),
        release,
    );
    let mut worker = Worker::new(client, journal).unwrap();
    assert!(view.is_current());
    advance.store(true, Ordering::SeqCst);
    // This is the real coordinator: its first journal.observe commit advances the shared clock.
    let result = worker
        .coordinate(&view, Some(Counter(1)), true)
        .await
        .unwrap();
    assert!(matches!(result, Coordination::Waiting));
    assert!(!advance.load(Ordering::SeqCst));
    assert_eq!(clock.now().unwrap().ticks_ns, raw.valid_until.ticks_ns);
    let mut current = status(&raw);
    let result = waiting_production(&mut current, true, &view);
    assert!(matches!(result, Err(Error::Expired)));
    assert!(!current.admission);
    assert!(transient(&Error::Expired));
    assert!(worker.journal().stop_record().unwrap().is_none());

    // A fresh read still retires the completed visit first, then finishes the Run. Keep every
    // view at the real 100ms bound; retry if the test host itself exhausts a view while recording.
    let retry_deadline = Instant::now() + Duration::from_secs(3);
    let mut sequence = raw.sequence.0;
    loop {
        sequence += 1;
        let mut next = raw.clone();
        next.sequence = Counter(sequence);
        next.checked_at = clock.now().unwrap();
        next.valid_until.ticks_ns = Counter(next.checked_at.ticks_ns.0 + WINDOW_NS);
        let (_, fresh) =
            ValidatedProduction::with_test_client(next, clock.clone(), principal.clone(), release);
        match worker
            .coordinate(&fresh, Some(Counter(1)), true)
            .await
            .unwrap()
        {
            Coordination::Retire(_) => {
                assert!(matches!(
                    worker.coordinate(&fresh, None, false).await.unwrap(),
                    Coordination::Finished
                ));
                break;
            }
            Coordination::Waiting => {
                assert!(matches!(
                    waiting_production(&mut current, true, &fresh),
                    Err(Error::Expired)
                ));
                assert!(
                    Instant::now() < retry_deadline,
                    "fresh view could not survive local observation"
                );
            }
            _ => panic!("completed visit must retire or await a fresh read"),
        }
    }
    assert!(worker.journal().stop_record().unwrap().is_none());
}

#[tokio::test]
async fn expired_waiting_cannot_republish_an_old_admission_flag() {
    let mut raw = completed();
    raw.run.run.state = RunState::Executing;
    raw.parts[0].disposition = PartDisposition::InProgress;
    raw.admission_allowed = true;
    let clock = Arc::new(ManualClock(AtomicU64::new(raw.valid_until.ticks_ns.0)));
    let (_, view) = ValidatedProduction::with_test_client(
        raw.clone(),
        clock,
        name("executor/a"),
        Digest::from_bytes([8; 32]),
    );
    let mut current = status(&raw);
    assert!(matches!(
        waiting_production(&mut current, true, &view),
        Err(Error::Expired)
    ));
    assert!(!current.admission);
}
