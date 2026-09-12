use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

struct TestClock(AtomicU64);
impl rx_service_status::Clock for TestClock {
    fn now(&self) -> rx_service_status::Result<TimePoint> {
        Ok(TimePoint {
            clock_id: "test/host-status".into(),
            ticks_ns: Counter(self.0.fetch_add(1, Ordering::SeqCst)),
        })
    }
}
struct FlakyClock {
    healthy: AtomicBool,
    ticks: AtomicU64,
}
impl rx_service_status::Clock for FlakyClock {
    fn now(&self) -> rx_service_status::Result<TimePoint> {
        if !self.healthy.load(Ordering::SeqCst) {
            return Err(rx_service_status::Error::Invalid(
                "test clock temporarily unavailable".into(),
            ));
        }
        Ok(TimePoint {
            clock_id: "test/host-status".into(),
            ticks_ns: Counter(self.ticks.fetch_add(1, Ordering::SeqCst)),
        })
    }
}
fn id(v: u64) -> Id {
    Id::new(format!("00000000-0000-4000-8000-{v:012}")).unwrap()
}
fn view() -> Status {
    Status {
        schema: name("rx.host-service-status.v1"),
        phase: name("STOPPED"),
        installation: id(1),
        host: name("host/test"),
        instance: id(2),
        host_boot: id(3),
        endpoint: "https://127.0.0.1:1234".into(),
        clock_id: "test/host".into(),
        admission_open: false,
        publication: "DISABLED".into(),
        pending_evidence: Some(Counter(0)),
        qualification_or_arm_restored: false,
        stop: Some(crate::gate::StopSnapshot {
            host_boot: id(3),
            admission_open: false,
            pending_operations: vec![],
            retained_evidence: Counter(0),
            native: None,
            safe_to_drop: true,
            physical_shutdown_assessed: false,
        }),
    }
}

#[test]
fn guarded_host_stopped_requires_final_adapter_proof_and_closed_admission() {
    let mut v = view();
    assert_eq!(
        guarded_final_state(&v, false),
        GuardedState::Stopped {
            reconciliation_required: false
        }
    );
    assert_eq!(guarded_final_state(&v, true), GuardedState::Attention);
    v.stop.as_mut().unwrap().safe_to_drop = false;
    assert_eq!(guarded_final_state(&v, false), GuardedState::Attention);
    v = view();
    v.admission_open = true;
    assert_eq!(guarded_final_state(&v, false), GuardedState::Attention);
    v = view();
    v.stop = None;
    assert_eq!(guarded_final_state(&v, false), GuardedState::Attention);
    v = view();
    v.stop.as_mut().unwrap().admission_open = true;
    assert_eq!(guarded_final_state(&v, false), GuardedState::Attention);
    for unknown in [false, true] {
        v = view();
        v.pending_evidence = if unknown { None } else { Some(Counter(1)) };
        assert_eq!(
            guarded_final_state(&v, false),
            GuardedState::Stopped {
                reconciliation_required: true
            }
        );
    }
    v = view();
    v.stop.as_mut().unwrap().pending_operations.push(id(4));
    assert_eq!(
        guarded_final_state(&v, false),
        GuardedState::Stopped {
            reconciliation_required: true
        }
    );
}

#[cfg(unix)]
#[tokio::test]
async fn unchanged_host_status_heartbeats_and_stays_terminal_after_finish() {
    let root = tempfile::tempdir().unwrap();
    let path = fs::canonicalize(root.path()).unwrap().join("guarded.json");
    let scope = GuardedScope::Host {
        installation: id(1),
        host: name("host/test"),
        installation_identity: Digest::from_bytes([1; 32]),
    };
    let reporter = Reporter::with_clock(
        path.clone(),
        scope,
        id(2),
        std::process::id(),
        Arc::new(TestClock(AtomicU64::new(1))),
    )
    .unwrap();
    let mut heartbeat =
        guarded_status::Heartbeat::start(reporter, || panic!("unexpected reporter failure"))
            .unwrap();
    heartbeat.publish(GuardedState::Ready).await.unwrap();
    let first: serde_json::Value = canonical::decode_json(&fs::read(&path).unwrap()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let next: serde_json::Value =
                canonical::decode_json(&fs::read(&path).unwrap()).unwrap();
            if next["sequence"].as_str().unwrap().parse::<u64>().unwrap()
                >= first["sequence"].as_str().unwrap().parse::<u64>().unwrap() + 2
            {
                assert_eq!(next["state"]["kind"], "READY");
                assert_ne!(first["observed_at"], next["observed_at"]);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    heartbeat
        .finish(GuardedState::Stopped {
            reconciliation_required: false,
        })
        .await
        .unwrap();
    let final_bytes = fs::read(&path).unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(fs::read(&path).unwrap(), final_bytes);
}

#[cfg(unix)]
#[tokio::test]
async fn host_report_failure_requests_stop_and_cannot_become_stopped_later() {
    let root = tempfile::tempdir().unwrap();
    let path = fs::canonicalize(root.path()).unwrap().join("guarded.json");
    let scope = GuardedScope::Host {
        installation: id(1),
        host: name("host/test"),
        installation_identity: Digest::from_bytes([1; 32]),
    };
    let clock = Arc::new(FlakyClock {
        healthy: AtomicBool::new(true),
        ticks: AtomicU64::new(1),
    });
    let reporter = Reporter::with_clock(
        path.clone(),
        scope,
        id(2),
        std::process::id(),
        clock.clone(),
    )
    .unwrap();
    let stopped = Arc::new(AtomicBool::new(false));
    let callback = stopped.clone();
    let mut heartbeat = guarded_status::Heartbeat::start(reporter, move || {
        callback.store(true, Ordering::SeqCst);
    })
    .unwrap();
    clock.healthy.store(false, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(2), async {
        while !stopped.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(heartbeat.failure().is_some());
    clock.healthy.store(true, Ordering::SeqCst);
    assert!(
        heartbeat
            .finish(GuardedState::Stopped {
                reconciliation_required: false
            })
            .await
            .is_err()
    );
    let final_view: serde_json::Value = canonical::decode_json(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(final_view["state"]["kind"], "ATTENTION");
}
