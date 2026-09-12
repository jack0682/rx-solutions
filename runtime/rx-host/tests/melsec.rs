use rx_domain::{intent::*, types::*};
use rx_host::{melsec::*, native::*, simulation::ManualClock, *};
use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
#[path = "support/melsec_fixture.rs"]
mod fixture;
use fixture::*;

#[test]
fn ack_is_pending_and_only_later_independent_sensor_observation_captures() {
    let _test_serial = TEST_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let plc = Plc::start();
    let p = profile(&plc);
    let c = clock();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("native");
    let (_, mut d) = open(&path, &p, &c);
    warm(&d, &p, &c);
    let op = id();
    let invocation = id();
    assert!(matches!(
        d.submit(&op, &invocation, &intent(&p)),
        Err(HostError::NativeUnknown(_))
    ));
    let entry = d.entry(&op).unwrap().unwrap();
    assert!(entry.acknowledgement.is_some());
    assert!(entry.capture.is_none());
    assert!(d.lookup(&op, &invocation).unwrap().is_none());
    assert_eq!(plc.writes(), 1);
    assert!(matches!(
        d.submit(&id(), &id(), &intent(&p)),
        Err(HostError::Busy)
    ));
    plc.update(|s| s.flags |= COMPLETE);
    let capture = d.lookup(&op, &invocation).unwrap().unwrap();
    assert_eq!(
        capture.status_schema.as_str(),
        "rx.melsec.predicate-satisfied.v1"
    );
    assert!(d.entry(&op).unwrap().unwrap().completion_snapshot.is_some());
    assert_eq!(
        d.submit(&op, &invocation, &intent(&p))
            .unwrap()
            .device_session,
        capture.device_session
    );
    assert!(d.can_handover(&p.resources));
    assert!(d.shutdown_snapshot(&p.resources).unwrap().safe_to_drop);
    assert_eq!(plc.writes(), 1);
}

#[test]
fn already_satisfied_state_records_evidence_without_a_native_write() {
    let _test_serial = TEST_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let plc = Plc::start();
    plc.update(|s| s.flags |= COMPLETE);
    let p = profile(&plc);
    let c = clock();
    let root = tempfile::tempdir().unwrap();
    let (_, mut d) = open(&root.path().join("native"), &p, &c);
    warm(&d, &p, &c);
    let op = id();
    let inv = id();
    d.submit(&op, &inv, &intent(&p)).unwrap();
    assert_eq!(plc.writes(), 0);
    let record = d.entry(&op).unwrap().unwrap();
    assert!(record.capture.is_some());
    assert!(record.acknowledgement.is_none());
    assert!(matches!(
        d.submit(&op, &id(), &intent(&p)),
        Err(HostError::Conflict)
    ));
    let mut other = intent(&p);
    if let Body::Predicate(g) = &mut other.body {
        g.target = TypedValue::Boolean(false);
    }
    assert!(matches!(
        d.submit(&op, &inv, &other),
        Err(HostError::Conflict)
    ));
}

#[test]
fn lost_reply_and_new_process_never_resubmit_or_clear_pending() {
    let _test_serial = TEST_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let plc = Plc::start();
    plc.update(|s| s.lose_reply = true);
    let p = profile(&plc);
    let c = clock();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("native");
    let (identity, mut d) = open(&path, &p, &c);
    warm(&d, &p, &c);
    let op = id();
    let inv = id();
    assert!(d.submit(&op, &inv, &intent(&p)).is_err());
    assert_eq!(plc.writes(), 1);
    assert!(d.entry(&op).unwrap().unwrap().write_error.is_some());
    assert!(d.lookup(&op, &inv).unwrap().is_none());
    assert!(d.submit(&op, &inv, &intent(&p)).is_err());
    drop(d);
    plc.update(|s| {
        s.lose_reply = false;
        s.flags |= COMPLETE;
    });
    let mut d = Melsec::open(&path, &identity, p.clone(), c).unwrap();
    assert!(d.lookup(&op, &inv).unwrap().is_none());
    assert!(d.submit(&op, &inv, &intent(&p)).is_err());
    assert!(matches!(
        d.submit(&id(), &id(), &intent(&p)),
        Err(HostError::Busy)
    ));
    assert_eq!(plc.writes(), 1);
}

#[test]
fn captured_original_session_survives_restart_but_uncaptured_ack_does_not() {
    let _test_serial = TEST_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    for complete in [false, true] {
        let plc = Plc::start();
        let p = profile(&plc);
        let c = clock();
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("native");
        let (identity, mut d) = open(&path, &p, &c);
        warm(&d, &p, &c);
        let op = id();
        let inv = id();
        assert!(d.submit(&op, &inv, &intent(&p)).is_err());
        let session = d.entry(&op).unwrap().unwrap().device_session;
        plc.update(|s| s.flags |= COMPLETE);
        if complete {
            d.lookup(&op, &inv).unwrap().unwrap();
        }
        drop(d);
        let mut d = Melsec::open(&path, &identity, p.clone(), c.clone()).unwrap();
        let saved = d.lookup(&op, &inv).unwrap();
        assert_eq!(saved.is_some(), complete);
        if let Some(capture) = saved {
            assert_eq!(capture.device_session, session);
            warm(&d, &p, &c);
            assert_ne!(
                d.guard(&intent(&p), &c.now()).unwrap().device_session,
                session
            );
        }
        assert_eq!(plc.writes(), 1);
    }
}

#[test]
fn publication_generation_regression_stale_and_inconsistent_images_latch() {
    let _test_serial = TEST_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    for case in 0..4 {
        let plc = Plc::start();
        let p = profile(&plc);
        let c = clock();
        let root = tempfile::tempdir().unwrap();
        let (_, d) = open(&root.path().join("native"), &p, &c);
        warm(&d, &p, &c);
        plc.update(|s| match case {
            0 => s.epoch += 1,
            1 => {
                s.sequence = 1;
                s.freeze = true;
            }
            2 => s.freeze = true,
            _ => {
                s.freeze = true;
                s.flags ^= COMPLETE;
            }
        });
        if case == 2 {
            c.ticks.fetch_add(31_000_000, Ordering::SeqCst);
        }
        assert!(d.observe_sources(&p.cell, &[name("laser/ready")]).is_err());
        assert!(d.guard(&intent(&p), &c.now()).is_err());
        assert_eq!(plc.writes(), 0);
    }
}

#[test]
fn valid_support_does_not_imply_drop_permission_and_protection_does_not_release_chuck() {
    let _test_serial = TEST_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let plc = Plc::start();
    let p = profile(&plc);
    let c = clock();
    let root = tempfile::tempdir().unwrap();
    let (_, mut d) = open(&root.path().join("native"), &p, &c);
    warm(&d, &p, &c);
    plc.update(|s| s.flags &= !(1 << 5));
    assert!(d.can_handover(&p.resources));
    assert!(!d.shutdown_snapshot(&p.resources).unwrap().safe_to_drop);
    d.protection().react(ProtectionIncident::OwnerLost);
    assert!(d.submit(&id(), &id(), &intent(&p)).is_err());
    assert!(!d.shutdown_snapshot(&p.resources).unwrap().safe_to_drop);
    assert!(d.shutdown_snapshot(&[name("other/resource")]).is_err());
    assert_eq!(plc.writes(), 0);
}

#[test]
fn new_or_replaced_journal_and_mismatched_profile_are_rejected_without_native_io() {
    let _test_serial = TEST_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let plc = Plc::start();
    let p = profile(&plc);
    let c = clock();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("native");
    let (identity, d) = open(&path, &p, &c);
    assert!(Melsec::open(&path, &identity, p.clone(), c.clone()).is_err());
    drop(d);
    let mut wrong = identity.clone();
    wrong.journal = id();
    assert!(Melsec::open(&path, &wrong, p.clone(), c.clone()).is_err());
    let mut changed = p.clone();
    changed.site_config = digest(10);
    assert!(Melsec::open(&path, &identity, changed, c.clone()).is_err());
    std::fs::remove_file(path.join("native.sqlite3")).unwrap();
    assert!(Melsec::open(&path, &identity, p.clone(), c.clone()).is_err());
    std::fs::write(path.join("native.sqlite3"), []).unwrap();
    assert!(Melsec::open(&path, &identity, p, c).is_err());
    assert_eq!(plc.state.lock().unwrap().reads, 0);
    assert_eq!(plc.writes(), 0);
}

#[test]
fn profile_and_intent_scope_fail_before_native_writes() {
    let _test_serial = TEST_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let plc = Plc::start();
    let p = profile(&plc);
    let mut bad = p.clone();
    bad.transport.access.write_m.insert(201);
    assert!(bad.validate().is_err());
    let mut bad = p.clone();
    bad.predicates[0].completion_bit = bad.status.ready_bit;
    assert!(bad.validate().is_err());
    let mut bad = p.clone();
    bad.transport.endpoint = "192.0.2.1:1234".parse().unwrap();
    assert!(bad.validate().is_err());
    let c = clock();
    let root = tempfile::tempdir().unwrap();
    let (_, mut d) = open(&root.path().join("native"), &p, &c);
    warm(&d, &p, &c);
    let mut wrong = intent(&p);
    wrong.site_config_digest = digest(20);
    assert!(d.submit(&id(), &id(), &wrong).is_err());
    assert!(
        d.observe_sources(&name("other/cell"), &[name("laser/ready")])
            .is_err()
    );
    assert!(d.observe_sources(&p.cell, &[name("unmapped")]).is_err());
    assert_eq!(plc.writes(), 0);
}

fn binding(p: &Profile) -> Binding {
    Binding {
        host: name("host/laser"),
        platform: name("platform"),
        cell: p.cell.clone(),
        definition: ArtifactRef {
            sha256: digest(30),
            schema_id: name("rx.cell-definition.v1"),
            size_bytes: Counter(1),
        },
        envelope: ArtifactRef {
            sha256: digest(31),
            schema_id: name("rx.operating-envelope.v1"),
            size_bytes: Counter(1),
        },
        qualification: Id::new("55555555-5555-4555-8555-555555555555").unwrap(),
        qualification_revision: Counter(1),
        allowed_intents: vec![intent(p)],
        scope_ids: vec![name("scope/laser")],
        condition_ids: p.conditions.keys().cloned().collect(),
        environment: Environment::Simulation,
        purposes: [Purpose::Production].into(),
    }
}
fn host_request(
    host: &Host<Melsec<ManualClock>, ManualClock>,
    p: &Profile,
    grant: &StoredGrant,
    c: &ManualClock,
) -> Request {
    let i = intent(p);
    let op = id();
    let d = i.digest().unwrap();
    let b = binding(p);
    Request {
        operation: op.clone(),
        intent: i,
        digest: d,
        grant: grant.id.clone(),
        permit: Permit {
            id: id(),
            operation: op,
            digest: d,
            cell: p.cell.clone(),
            epoch: Counter(1),
            scopes: [(name("scope/laser"), Counter(1))].into(),
            envelope: b.envelope.sha256,
            qualification: b.qualification,
            qualification_revision: Counter(1),
            grant: grant.id.clone(),
            host_boot: host.boot_id().unwrap(),
            conditions: p.conditions.keys().cloned().collect(),
            expires_at: TimePoint {
                clock_id: c.clock_id.clone(),
                ticks_ns: Counter(c.now().ticks_ns.0 + 1_000_000_000),
            },
            purpose: Purpose::Production,
            parent: PermitParent::Mandate(id()),
            source_digest: None,
        },
    }
}
#[test]
fn actual_host_gate_journal_reconcile_evidence_and_stop_use_melsec_adapter() {
    let _test_serial = TEST_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let plc = Plc::start();
    let p = profile(&plc);
    let c = clock();
    let root = tempfile::tempdir().unwrap();
    let (_, d) = open(&root.path().join("native"), &p, &c);
    warm(&d, &p, &c);
    let host = Host::open(root.path().join("host.db"), d, c.clone(), vec![binding(&p)]).unwrap();
    let caller = Caller {
        peer: name("platform"),
        session: id(),
    };
    host.bind_platform(caller.clone()).unwrap();
    let grant = host
        .acquire_grant(
            &caller,
            id(),
            p.resources.clone(),
            Counter(1),
            Counter(10_000_000_000),
        )
        .unwrap();
    let request = host_request(&host, &p, &grant, &c);
    assert!(host.prepare(&caller, request.clone()).is_err());
    assert_eq!(plc.writes(), 0);
    host.arm(
        &caller,
        id(),
        &p.cell,
        Counter(1),
        &[(name("scope/laser"), Counter(1))].into(),
        &BTreeSet::new(),
    )
    .unwrap();
    let prepared = host.prepare(&caller, request.clone()).unwrap();
    assert_eq!(plc.writes(), 0);
    let invocation = prepared.invocation.unwrap();
    assert!(
        host.authorize(&caller, request.clone(), &invocation)
            .is_err()
    );
    assert_eq!(plc.writes(), 1);
    assert_eq!(
        host.receipt(&caller, &request.operation).unwrap().state,
        ReceiptState::SendEntered
    );
    assert!(
        host.evidence_after(&caller, Counter(0), 128)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        host.authorize(&caller, request.clone(), &invocation)
            .unwrap()
            .state,
        ReceiptState::SendEntered
    );
    assert_eq!(
        host.reconcile(&caller, &request.operation).unwrap().state,
        ReceiptState::SendEntered
    );
    plc.update(|s| s.flags |= COMPLETE);
    assert_eq!(
        host.reconcile(&caller, &request.operation).unwrap().state,
        ReceiptState::ResultCaptured
    );
    let evidence = host.evidence_after(&caller, Counter(0), 128).unwrap();
    assert_eq!(evidence.len(), 1);
    assert_eq!(
        evidence[0].1.capture.status_schema.as_str(),
        "rx.melsec.predicate-satisfied.v1"
    );
    let observations = host
        .handover_observations(&caller, &request.operation)
        .unwrap();
    assert_eq!(observations.len(), 3);
    host.request_service_stop();
    assert!(
        host.prepare(&caller, host_request(&host, &p, &grant, &c))
            .is_err()
    );
    let stop = host.service_stop_snapshot().unwrap();
    assert!(stop.safe_to_drop);
    assert_eq!(plc.writes(), 1);
}

struct Crash(&'static str);
impl NativeBoundary for Crash {
    fn after_entry_commit(&self) {
        if self.0 == "entry" {
            std::process::exit(86);
        }
    }
    fn after_write_before_ack_commit(&self) {
        if self.0 == "write" {
            std::process::exit(86);
        }
    }
    fn after_capture_commit(&self) {
        if self.0 == "capture" {
            std::process::exit(86);
        }
    }
}
#[test]
#[ignore = "executed only as an isolated child by the crash-boundary test"]
fn melsec_crash_child() {
    let _test_serial = TEST_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let config = std::env::var("RX_MELSEC_TEST_INPUT").unwrap();
    let (directory, p, identity, op, inv, stage): (
        std::path::PathBuf,
        Profile,
        Identity,
        Id,
        Id,
        String,
    ) = serde_json::from_slice(&std::fs::read(config).unwrap()).unwrap();
    let c = clock();
    let stage = match stage.as_str() {
        "entry" => "entry",
        "write" => "write",
        "capture" | "late-capture" => "capture",
        _ => panic!(),
    };
    let mut d =
        Melsec::with_boundary(&directory, &identity, p.clone(), c.clone(), Crash(stage)).unwrap();
    warm(&d, &p, &c);
    let _ = d.submit(&op, &inv, &intent(&p));
    let _ = d.lookup(&op, &inv);
    panic!("crash hook not reached");
}
#[test]
fn abrupt_exit_at_native_commit_and_write_boundaries_never_replays() {
    let _test_serial = TEST_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    for (stage, expected_writes, captured) in [
        ("entry", 0, false),
        ("write", 1, false),
        ("capture", 0, true),
        ("late-capture", 1, true),
    ] {
        let plc = Plc::start();
        if stage == "late-capture" {
            plc.update(|s| s.complete_on_write = true);
        } else if captured {
            plc.update(|s| s.flags |= COMPLETE);
        }
        let p = profile(&plc);
        let c = clock();
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("native");
        let identity = Melsec::<ManualClock>::initialize(&path, &p).unwrap();
        let op = id();
        let inv = id();
        let input = root.path().join("input.json");
        std::fs::write(
            &input,
            serde_json::to_vec(&(
                path.clone(),
                p.clone(),
                identity.clone(),
                op.clone(),
                inv.clone(),
                stage,
            ))
            .unwrap(),
        )
        .unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "melsec_crash_child", "--ignored", "--nocapture"])
            .env("RX_MELSEC_TEST_INPUT", &input)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(86),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut d = Melsec::open(&path, &identity, p.clone(), c).unwrap();
        assert_eq!(d.lookup(&op, &inv).unwrap().is_some(), captured);
        let result = d.submit(&op, &inv, &intent(&p));
        assert_eq!(result.is_ok(), captured);
        assert_eq!(plc.writes(), expected_writes);
        if !captured {
            assert!(matches!(
                d.submit(&id(), &id(), &intent(&p)),
                Err(HostError::Busy)
            ));
        }
    }
}

struct DropReadyAfterEntry(Arc<Mutex<PlcState>>);
impl NativeBoundary for DropReadyAfterEntry {
    fn after_entry_commit(&self) {
        self.0.lock().unwrap().flags &= !(1 << 1);
    }
}
#[test]
fn final_native_read_after_journal_commit_rechecks_ready_without_resend() {
    let _test_serial = TEST_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let plc = Plc::start();
    let p = profile(&plc);
    let c = clock();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("native");
    let identity = Melsec::<ManualClock>::initialize(&path, &p).unwrap();
    let mut d = Melsec::with_boundary(
        &path,
        &identity,
        p.clone(),
        c.clone(),
        DropReadyAfterEntry(plc.state.clone()),
    )
    .unwrap();
    warm(&d, &p, &c);
    let op = id();
    let inv = id();
    assert!(d.submit(&op, &inv, &intent(&p)).is_err());
    let entry = d.entry(&op).unwrap().unwrap();
    assert!(entry.capture.is_none());
    assert!(entry.acknowledgement.is_none());
    assert_eq!(
        entry.request_frame,
        p.transport.write_m_frame(200, true).unwrap()
    );
    plc.update(|s| s.flags |= READY);
    assert!(d.submit(&op, &inv, &intent(&p)).is_err());
    assert!(matches!(
        d.submit(&id(), &id(), &intent(&p)),
        Err(HostError::Busy)
    ));
    assert_eq!(plc.writes(), 0);
}

#[test]
fn corrupted_completion_record_is_not_adopted_as_historical_fact() {
    let _test_serial = TEST_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    use rx_ports::Repository;
    let plc = Plc::start();
    plc.update(|s| s.flags |= COMPLETE);
    let p = profile(&plc);
    let c = clock();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("native");
    let (identity, mut d) = open(&path, &p, &c);
    warm(&d, &p, &c);
    let op = id();
    let inv = id();
    d.submit(&op, &inv, &intent(&p)).unwrap();
    drop(d);
    let mut store = rx_storage::SqliteRepository::open(path.join("native.sqlite3")).unwrap();
    store
        .transact(|tx| {
            let key = rx_host::journal::key("melsec-operation", &op);
            let row = tx.get(&key)?.unwrap();
            let mut entry: Entry = rx_host::journal::decode(&row, "rx.melsec.entry.v1")?;
            entry.completion_snapshot.as_mut().unwrap().flags &= !COMPLETE;
            tx.put(
                &key,
                Some(row.revision),
                &rx_host::journal::doc("rx.melsec.entry.v1", &entry)?,
            )?;
            Ok(())
        })
        .unwrap();
    drop(store);
    assert!(Melsec::open(&path, &identity, p, c).is_err());
    assert_eq!(plc.writes(), 0);
}

struct ExpireGuardAfterEntry(Arc<Mutex<PlcState>>, Arc<AtomicU64>);
impl NativeBoundary for ExpireGuardAfterEntry {
    fn after_entry_commit(&self) {
        self.0.lock().unwrap().advance_clock_on_read = Some(self.1.clone());
    }
}
#[test]
fn final_read_within_transport_budget_but_beyond_guard_lifetime_cannot_write() {
    let _test_serial = TEST_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let plc = Plc::start();
    let mut p = profile(&plc);
    p.status.source_max_age_ms = 50;
    let c = clock();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("native");
    let identity = Melsec::<ManualClock>::initialize(&path, &p).unwrap();
    let mut d = Melsec::with_boundary(
        &path,
        &identity,
        p.clone(),
        c.clone(),
        ExpireGuardAfterEntry(plc.state.clone(), c.ticks.clone()),
    )
    .unwrap();
    warm(&d, &p, &c);
    let op = id();
    assert!(d.submit(&op, &id(), &intent(&p)).is_err());
    assert!(d.entry(&op).unwrap().unwrap().acknowledgement.is_none());
    assert_eq!(plc.writes(), 0);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_real_boottime_adapter_observation_write_completion_and_drop() {
    let _test_serial = TEST_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    use rx_host::service_clock::SystemClock;
    let plc = Plc::start();
    let mut p = profile(&plc);
    p.transport.exchange_timeout_ms = 50;
    p.status.source_max_age_ms = 50;
    p.status.guard_validity_ms = 50;
    let c = SystemClock::new().unwrap();
    let started = c.now();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("native");
    let identity = Melsec::<SystemClock>::initialize(&path, &p).unwrap();
    let mut d = Melsec::open(&path, &identity, p.clone(), c.clone()).unwrap();
    assert!(d.guard(&intent(&p), &c.now()).is_err());
    d.guard(&intent(&p), &c.now()).unwrap();
    let op = id();
    let inv = id();
    assert!(d.submit(&op, &inv, &intent(&p)).is_err());
    assert!(d.lookup(&op, &inv).unwrap().is_none());
    plc.update(|s| s.flags |= COMPLETE);
    let capture = d.lookup(&op, &inv).unwrap().unwrap();
    assert_eq!(capture.captured_at.clock_id, started.clock_id);
    assert!(capture.captured_at.ticks_ns > started.ticks_ns);
    assert!(d.shutdown_snapshot(&p.resources).unwrap().safe_to_drop);
    assert_eq!(plc.writes(), 1);
}
