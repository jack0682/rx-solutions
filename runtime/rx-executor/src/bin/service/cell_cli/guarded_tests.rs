use super::*;
use rx_executor::{
    cell_service,
    lifecycle::{StopPhase, StopReason, StopRecord},
    service,
};
use std::sync::atomic::{AtomicU64, Ordering};

fn id(v: u64) -> Id {
    Id::new(format!("00000000-0000-4000-8000-{v:012}")).unwrap()
}
fn report() -> cell_service::Report {
    cell_service::Report {
        status: cell_service::Status {
            phase: Phase::Stopped,
            session: id(1),
            run: None,
            attachment: None,
            completed_runs: Counter(0),
            active: None,
            detail: None,
        },
        last_run: None,
    }
}

#[test]
fn live_executor_status_never_claims_terminal_cleanup_or_p_admission() {
    let mut r = report();
    r.status.phase = Phase::Idle;
    assert_eq!(
        guarded_live_state(&r.status, false, false),
        GuardedState::Ready
    );
    r.status.phase = Phase::Arming;
    r.status.run = Some(id(2));
    assert_eq!(
        guarded_live_state(&r.status, false, false),
        GuardedState::Ready
    );
    assert_eq!(
        guarded_live_state(&r.status, true, false),
        GuardedState::Stopping
    );
    assert_eq!(
        guarded_live_state(&r.status, false, true),
        GuardedState::Attention
    );
    r.status.phase = Phase::Stopped;
    assert_eq!(
        guarded_live_state(&r.status, false, false),
        GuardedState::Stopping
    );
    r.status.phase = Phase::Attention;
    assert_eq!(
        guarded_live_state(&r.status, true, false),
        GuardedState::Attention
    );
}

#[test]
fn executor_final_state_preserves_attention_and_rejects_incomplete_stop() {
    let mut r = report();
    assert_eq!(
        guarded_final_state(&r, false),
        GuardedState::Stopped {
            reconciliation_required: false
        }
    );
    assert_eq!(guarded_final_state(&r, true), GuardedState::Attention);
    for phase in [Phase::Idle, Phase::Arming, Phase::Running, Phase::Attention] {
        r.status.phase = phase;
        assert_eq!(guarded_final_state(&r, false), GuardedState::Attention);
    }
    r = report();
    r.status.run = Some(id(2));
    r.status.attachment = Some(id(3));
    assert_eq!(guarded_final_state(&r, false), GuardedState::Attention);
    let stop = StopRecord::new(id(2), id(1), None, StopReason::Requested, None);
    r.last_run = Some(service::Report {
        phase: StopPhase::Pending,
        stop_id: stop.id.clone(),
        durability_fault: None,
        last_error: None,
        stop,
    });
    assert_eq!(guarded_final_state(&r, false), GuardedState::Attention);
    let last = r.last_run.as_mut().unwrap();
    last.phase = StopPhase::PauseObserved;
    last.stop.phase = StopPhase::PauseObserved;
    last.durability_fault = Some("stop commit unconfirmed".into());
    assert_eq!(guarded_final_state(&r, false), GuardedState::Attention);
    let last = r.last_run.as_mut().unwrap();
    last.durability_fault = None;
    let schema = Name::new(rx_process_contract::execution::CHECKPOINT_SCHEMA).unwrap();
    last.stop.observation = Some(rx_executor::journal::RunResponse {
        run: id(2),
        revision: Counter(2),
        recipe: Digest::from_bytes([1; 32]),
        state: rx_process_contract::execution::RunState::Paused,
        executor_session: Some(id(1)),
        checkpoint: rx_process_contract::execution::CheckpointView {
            run: id(2),
            revision: Counter(2),
            executor_schema: schema.clone(),
            payload: ArtifactRef {
                schema_id: schema,
                sha256: Digest::from_bytes([2; 32]),
                size_bytes: Counter(1),
            },
            activations: vec![],
        },
    });
    assert_eq!(
        guarded_final_state(&r, false),
        GuardedState::Stopped {
            reconciliation_required: true
        }
    );
}

#[test]
fn completed_run_retirement_keeps_the_resident_executor_protocol_ready() {
    let mut current = report().status;
    current.phase = Phase::Running;
    current.run = Some(id(2));
    current.attachment = Some(id(3));
    for phase in [
        service::Phase::GraphComplete,
        service::Phase::Stopping,
        service::Phase::Stopped,
    ] {
        current.active = Some(service::Status {
            phase,
            session: id(1),
            run: id(2),
            visit: Counter(2),
            sequence: Some(Counter(44)),
            admission: false,
        });
        assert_eq!(
            guarded_live_state(&current, false, false),
            GuardedState::Ready
        );
        assert_eq!(
            guarded_live_state(&current, true, false),
            GuardedState::Stopping
        );
        assert_eq!(
            guarded_live_state(&current, false, true),
            GuardedState::Attention
        );
    }
    current.phase = Phase::Idle;
    current.run = None;
    current.attachment = None;
    current.active = None;
    assert_eq!(
        guarded_live_state(&current, false, false),
        GuardedState::Ready
    );
}

struct TestClock(AtomicU64);
impl rx_service_status::Clock for TestClock {
    fn now(&self) -> rx_service_status::Result<TimePoint> {
        Ok(TimePoint {
            clock_id: "test/executor-status".into(),
            ticks_ns: Counter(self.0.fetch_add(1, Ordering::SeqCst)),
        })
    }
}
#[tokio::test]
async fn executor_report_failure_signals_existing_cooperative_shutdown() {
    let root = tempfile::tempdir().unwrap();
    let path = fs::canonicalize(root.path()).unwrap().join("guarded.json");
    let scope = GuardedScope::Executor {
        installation: id(1),
        cell: Name::new("cell/test").unwrap(),
        service_journal: id(2),
        configuration_digest: Digest::from_bytes([1; 32]),
    };
    let reporter = Reporter::with_clock(
        path.clone(),
        scope,
        id(3),
        std::process::id(),
        Arc::new(TestClock(AtomicU64::new(1))),
    )
    .unwrap();
    let (stop, mut shutdown) = tokio::sync::watch::channel(false);
    let mut heartbeat = guarded_status::Heartbeat::start(reporter, move || {
        stop.send_replace(true);
    })
    .unwrap();
    heartbeat.publish(GuardedState::Ready).await.unwrap();
    fs::write(&path, b"foreign writer").unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), shutdown.changed())
        .await
        .unwrap()
        .unwrap();
    assert!(*shutdown.borrow());
    assert!(heartbeat.failure().is_some());
    assert!(
        heartbeat
            .finish(GuardedState::Stopped {
                reconciliation_required: false
            })
            .await
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), b"foreign writer");
}
