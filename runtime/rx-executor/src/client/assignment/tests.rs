//! Controlled-clock payload tests; channels are lazy and no RPC/server is started.
use super::*;
use assignment::{AttemptStatus, Candidate, Cardinality, PendingAttempt, View};
use std::{collections::BTreeMap, sync::Mutex};

struct TestClock(Mutex<Option<TimePoint>>);
impl TestClock {
    fn set(&self, now: Option<TimePoint>) {
        *self.0.lock().unwrap() = now;
    }
}
impl crate::clock::Clock for TestClock {
    fn now(&self) -> Result<TimePoint, Error> {
        self.0
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| invalid("test clock unavailable"))
    }
}

fn uid(value: u32) -> Id {
    Id::new(format!("{value:08x}-0000-4000-8000-000000000000")).unwrap()
}
fn name(value: &str) -> Name {
    Name::new(value).unwrap()
}
fn time(ticks: u64) -> TimePoint {
    TimePoint {
        clock_id: "test/assignment".into(),
        ticks_ns: Counter(ticks),
    }
}
fn artifact(value: u8, schema: &str) -> ArtifactRef {
    ArtifactRef {
        sha256: Digest::from_bytes([value; 32]),
        schema_id: name(schema),
        size_bytes: Counter(1),
    }
}
fn view() -> View {
    let definition = artifact(1, "rx.test.v1");
    View {
        schema: name(assignment::SCHEMA),
        installation: uid(1),
        store_generation: uid(2),
        runtime_boot: uid(3),
        sequence: Counter(10),
        caller_session: uid(4),
        cell: name("cell/a"),
        executor: name("executor/a"),
        definition: definition.clone(),
        cell_revision: Counter(3),
        cell_epoch: Counter(2),
        scope_epochs: BTreeMap::from([(name("zone/shared"), Counter(2))]),
        checked_at: time(1000),
        valid_until: time(100_001_000),
        cardinality: Cardinality::Single,
        candidates: vec![Candidate {
            run: uid(5),
            revision: Counter(7),
            state: RunState::Executing,
            purpose: Some(Purpose::Production),
            definition,
            resolved: artifact(2, "rx.resolved-process.v1"),
            executor_session: Some(uid(4)),
            mandate: Some(uid(6)),
            pending_attempt: None,
            configuration_current: true,
        }],
    }
}

fn client() -> (Client, Arc<TestClock>) {
    let raw = view();
    let clock = Arc::new(TestClock(Mutex::new(Some(time(1000)))));
    // connect_lazy establishes no transport; tests enter the same acceptance path
    // called after the authenticated RPC response in assignment_view.
    let channel = Channel::from_static("http://127.0.0.1:1").connect_lazy();
    let client = Client {
        pin: PeerPin {
            principal: raw.executor,
            peer_boot: uid(7),
            installation: raw.installation,
            store_generation: raw.store_generation,
            release: Digest::from_bytes([3; 32]),
            clock_id: raw.checked_at.clock_id,
            cell: raw.cell,
            definition: raw.definition.sha256,
        },
        clock: clock.clone(),
        session: base::Session {
            session_id: raw.caller_session.to_string(),
            ..Default::default()
        },
        read: wire::executor_read_service_client::ExecutorReadServiceClient::new(channel.clone()),
        workflow: base::workflow_service_client::WorkflowServiceClient::new(channel.clone()),
        plans: rx_protocol::executor_plan::executor_plan_service_client::ExecutorPlanServiceClient::new(channel.clone()),
        cells: cell::cell_service_client::CellServiceClient::new(channel.clone()),
        operations: base::operation_service_client::OperationServiceClient::new(channel.clone()),
        production: rx_protocol::production::production_service_client::ProductionServiceClient::new(channel.clone()),
        assignments: rx_protocol::assignment::executor_assignment_service_client::ExecutorAssignmentServiceClient::new(channel),
        runtime_boot: None,
        last_sequence: Counter(0),
        last_checked: Counter(0),
        process: None,
        max_payload: MAX_BYTES,
    };
    (client, clock)
}
fn payload(bytes: Vec<u8>) -> wire::ReadPayload {
    wire::ReadPayload {
        reference: Some(base::ArtifactRef {
            sha256: Sha256::digest(&bytes).to_vec(),
            schema_id: assignment::SCHEMA.into(),
            size_bytes: bytes.len() as u64,
        }),
        payload: bytes,
    }
}
fn read(client: &mut Client, raw: &View) -> Result<ValidatedAssignment, Error> {
    client.accept_assignment(
        payload(canonical::bytes(raw).unwrap()),
        raw.checked_at.clone(),
        Instant::now(),
    )
}

#[tokio::test]
async fn cardinalities_retain_old_sessions_expired_attempts_and_restricted_work() {
    let (mut client, _) = client();
    let mut none = view();
    none.cardinality = Cardinality::None;
    none.candidates.clear();
    let validated = read(&mut client, &none).unwrap();
    assert_eq!(validated.data().cardinality, Cardinality::None);
    assert!(validated.data().candidates.is_empty());

    for state in [
        RunState::Executing,
        RunState::Paused,
        RunState::RecoveryRequired,
    ] {
        let mut single = view();
        let candidate = &mut single.candidates[0];
        candidate.state = state;
        candidate.executor_session = Some(uid(8));
        candidate.configuration_current = false;
        candidate.definition = artifact(9, "rx.test.v1");
        let validated = read(&mut client, &single).unwrap();
        assert_eq!(
            canonical::bytes(validated.data()).unwrap(),
            canonical::bytes(&single).unwrap()
        );

        for status in [AttemptStatus::Pending, AttemptStatus::Arming] {
            let mut ambiguous = single.clone();
            let mut pending = view().candidates.remove(0);
            pending.run = uid(9);
            pending.state = RunState::Prepared;
            pending.executor_session = None;
            pending.mandate = None;
            pending.pending_attempt = Some(PendingAttempt {
                id: uid(10),
                status,
                executor_session: uid(8),
                valid_until: time(900),
            });
            ambiguous.cardinality = Cardinality::Ambiguous;
            ambiguous.candidates.push(pending.clone());
            let validated = read(&mut client, &ambiguous).unwrap();
            assert_eq!(
                canonical::bytes(validated.data()).unwrap(),
                canonical::bytes(&ambiguous).unwrap()
            );
            // A lone expired old-session pending attempt is still SINGLE.
            let mut single_pending = view();
            single_pending.candidates = vec![pending];
            let validated = read(&mut client, &single_pending).unwrap();
            assert_eq!(validated.data().cardinality, Cardinality::Single);
            assert!(validated.data().candidates[0].pending_attempt.is_some());
        }
    }
}

#[tokio::test]
async fn forged_identity_is_rejected_even_with_a_matching_payload_hash() {
    let cases: &[fn(&mut View)] = &[
        |v| v.schema = name("rx.execution-snapshot.v1"),
        |v| v.installation = uid(99),
        |v| v.store_generation = uid(99),
        |v| v.caller_session = uid(99),
        |v| v.cell = name("cell/other"),
        |v| v.executor = name("executor/other"),
        |v| {
            v.definition = artifact(99, "rx.test.v1");
            v.candidates[0].definition = v.definition.clone();
        },
        |v| {
            v.checked_at.clock_id = "test/other-clock".into();
            v.valid_until.clock_id = "test/other-clock".into();
        },
    ];
    for (index, change) in cases.iter().enumerate() {
        let (mut client, _) = client();
        let mut forged = view();
        change(&mut forged);
        assert!(
            matches!(read(&mut client, &forged), Err(Error::Invalid(_))),
            "identity {index}"
        );
        assert!(client.runtime_boot.is_none());
        assert_eq!(client.last_sequence, Counter(0));
    }
}

#[tokio::test]
async fn forged_cut_cardinality_and_duplicate_witnesses_fail_closed() {
    let cases: &[fn(&mut View)] = &[
        |v| v.sequence = Counter(0),
        |v| v.cell_revision = Counter(0),
        |v| v.cell_epoch = Counter(0),
        |v| v.scope_epochs.clear(),
        |v| *v.scope_epochs.values_mut().next().unwrap() = Counter(0),
        |v| v.definition.size_bytes = Counter(0),
        |v| v.cardinality = Cardinality::None,
        |v| v.candidates.clear(),
        |v| v.cardinality = Cardinality::Ambiguous,
        |v| {
            v.cardinality = Cardinality::Ambiguous;
            v.candidates.push(v.candidates[0].clone());
        },
        |v| v.candidates[0].revision = Counter(0),
        |v| v.candidates[0].state = RunState::Prepared,
        |v| v.candidates[0].state = RunState::Completed,
        |v| v.candidates[0].state = RunState::Abandoned,
        |v| v.candidates[0].definition = artifact(99, "rx.test.v1"),
        |v| v.candidates[0].resolved.size_bytes = Counter(0),
    ];
    for (index, change) in cases.iter().enumerate() {
        let (mut client, _) = client();
        let mut forged = view();
        change(&mut forged);
        assert!(
            matches!(read(&mut client, &forged), Err(Error::Invalid(_))),
            "cut {index}"
        );
    }
}

#[tokio::test]
async fn assignment_shares_runtime_sequence_and_time_regression_guards() {
    let (mut client, clock) = client();
    // These are the same fields updated by snapshot() and production_view().
    client.runtime_boot = Some(view().runtime_boot);
    client.last_sequence = Counter(10);
    client.last_checked = Counter(1000);
    for change in [
        (|v: &mut View| v.runtime_boot = uid(99)) as fn(&mut View),
        |v| v.sequence = Counter(9),
        |v| {
            v.checked_at = time(999);
            v.valid_until = time(100_000_999);
        },
    ] {
        let mut forged = view();
        change(&mut forged);
        assert!(matches!(read(&mut client, &forged), Err(Error::Invalid(_))));
        assert_eq!(client.last_sequence, Counter(10));
        assert_eq!(client.last_checked, Counter(1000));
        assert_eq!(client.runtime_boot, Some(view().runtime_boot));
    }
    assert!(read(&mut client, &view()).is_ok());
    let mut next = view();
    next.sequence = Counter(11);
    next.checked_at = time(1001);
    next.valid_until = time(100_001_001);
    clock.set(Some(time(1001)));
    read(&mut client, &next).unwrap();
    assert_eq!(client.last_sequence, Counter(11));
    assert_eq!(client.last_checked, Counter(1001));
}

#[tokio::test]
async fn payload_digest_size_schema_and_strict_json_are_independent_checks() {
    let (mut client, _) = client();
    let good = payload(canonical::bytes(&view()).unwrap());
    let cases: &[fn(&mut wire::ReadPayload)] = &[
        |p| p.reference = None,
        |p| p.reference.as_mut().unwrap().sha256 = vec![1; 31],
        |p| p.reference.as_mut().unwrap().sha256 = vec![1; 32],
        |p| p.reference.as_mut().unwrap().size_bytes += 1,
        |p| p.reference.as_mut().unwrap().schema_id = "rx.execution-snapshot.v1".into(),
        |p| p.payload.push(b' '),
    ];
    for (index, change) in cases.iter().enumerate() {
        let mut forged = good.clone();
        change(&mut forged);
        assert!(
            matches!(
                client.accept_assignment(forged, time(1000), Instant::now()),
                Err(Error::Invalid(_))
            ),
            "payload {index}"
        );
    }
    let bytes = canonical::bytes(&view()).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    for bytes in [
        b"{}".to_vec(),
        format!("{{\"schema\":\"{}\",{}", assignment::SCHEMA, &text[1..]).into_bytes(),
        format!("{{\"admission_allowed\":true,{}", &text[1..]).into_bytes(),
        text.replace("\"sequence\":\"10\"", "\"sequence\":10")
            .into_bytes(),
    ] {
        assert!(matches!(
            client.accept_assignment(payload(bytes), time(1000), Instant::now()),
            Err(Error::Invalid(_))
        ));
    }
    // Valid JSON with a correct digest remains too large for this binding.
    let mut oversized = canonical::bytes(&view()).unwrap();
    oversized.resize(assignment::MAX_PAYLOAD + 1, b' ');
    assert!(matches!(
        client.accept_assignment(payload(oversized), time(1000), Instant::now()),
        Err(Error::Invalid(_))
    ));
    client.max_payload = good.payload.len() - 1;
    assert!(matches!(
        client.accept_assignment(good, time(1000), Instant::now()),
        Err(Error::Invalid(_))
    ));
}

#[tokio::test]
async fn ttl_and_clock_interval_are_checked_before_accepting_a_view() {
    for until in [999, 1000, 100_001_001] {
        let (mut client, _) = client();
        let mut forged = view();
        forged.valid_until = time(until);
        assert!(matches!(read(&mut client, &forged), Err(Error::Invalid(_))));
    }
    let (mut client, clock) = client();
    let good = payload(canonical::bytes(&view()).unwrap());
    for (sent, received) in [
        (time(1001), time(1001)), // P checked before the request.
        (time(998), time(999)),   // P checked after receipt.
        (time(1000), time(999)),  // The trusted clock regressed in flight.
        (
            TimePoint {
                clock_id: "test/other".into(),
                ticks_ns: Counter(1000),
            },
            time(1000),
        ),
        (
            time(1000),
            TimePoint {
                clock_id: "test/other".into(),
                ticks_ns: Counter(1000),
            },
        ),
    ] {
        clock.set(Some(received));
        assert!(matches!(
            client.accept_assignment(good.clone(), sent, Instant::now()),
            Err(Error::Invalid(_))
        ));
    }
    assert!(client.runtime_boot.is_none());
}

#[tokio::test]
async fn expired_response_does_not_advance_the_shared_cut() {
    for locally_elapsed in [false, true] {
        let (mut client, clock) = client();
        let raw = view();
        let started = if locally_elapsed {
            Instant::now() - Duration::from_millis(200)
        } else {
            clock.set(Some(raw.valid_until.clone()));
            Instant::now()
        };
        assert!(matches!(
            client.accept_assignment(
                payload(canonical::bytes(&raw).unwrap()),
                time(1000),
                started
            ),
            Err(Error::Expired)
        ));
        assert!(client.runtime_boot.is_none());
        assert_eq!(client.last_sequence, Counter(0));
        assert_eq!(client.last_checked, Counter(0));
        clock.set(Some(time(1000)));
        assert!(read(&mut client, &raw).is_ok());
    }
}

#[tokio::test]
async fn accepted_read_expires_or_loses_currency_when_the_trusted_clock_changes() {
    let (mut client, clock) = client();
    let mut validated = read(&mut client, &view()).unwrap();
    assert!(validated.is_current());
    assert!(validated.expires_at() > Instant::now());
    for now in [
        Some(time(100_001_000)),
        Some(time(999)),
        Some(TimePoint {
            clock_id: "test/other".into(),
            ticks_ns: Counter(1000),
        }),
        None,
    ] {
        clock.set(now);
        assert!(!validated.is_current());
    }
    clock.set(Some(time(1000)));
    validated.deadline = Instant::now() - Duration::from_millis(1);
    assert!(!validated.is_current());
}
