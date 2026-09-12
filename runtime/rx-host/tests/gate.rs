use rx_domain::{intent::*, types::*};
use rx_host::{native::*, simulation::*, *};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn clock() -> ManualClock {
    ManualClock {
        clock_id: "simulation/boottime".into(),
        ticks: Arc::new(AtomicU64::new(1000)),
    }
}
fn intent() -> Intent {
    Intent {
        kind: Kind::EnsureState,
        target: name("sim/chuck"),
        profile_digest: Digest::from_bytes([1; 32]),
        site_config_digest: Digest::from_bytes([2; 32]),
        calibration_digests: vec![],
        resource_set: vec![name("sim/controller")],
        execution_timeout_ms: Counter(5000),
        prepare_validity_ms: Counter(1000),
        completion_rule: name("sim/closed"),
        cancel_rule: name("sim/stop"),
        body: Body::Predicate(PredicateGoal {
            predicate_id: name("chuck.closed"),
            target: TypedValue::Boolean(true),
            settle_ms: Counter(0),
        }),
    }
}
fn binding() -> Binding {
    Binding {
        host: name("host/sim"),
        platform: name("platform"),
        cell: name("cell/sim"),
        definition: ArtifactRef {
            sha256: Digest::from_bytes([3; 32]),
            schema_id: name("rx.cell-definition.v1"),
            size_bytes: Counter(1),
        },
        envelope: ArtifactRef {
            sha256: Digest::from_bytes([4; 32]),
            schema_id: name("rx.operating-envelope.v1"),
            size_bytes: Counter(1),
        },
        qualification: Id::new("55555555-5555-4555-8555-555555555555").unwrap(),
        qualification_revision: Counter(1),
        allowed_intents: vec![intent()],
        scope_ids: vec![name("scope/main")],
        condition_ids: vec![name("sim/ready")],
        environment: Environment::Simulation,
        purposes: [Purpose::Production, Purpose::Setup].into_iter().collect(),
    }
}
fn scopes(epoch: u64) -> BTreeMap<Name, Counter> {
    [(name("scope/main"), Counter(epoch))].into_iter().collect()
}
fn initialize<N: NativeAdapter, H: BoundaryHook>(
    host: &Host<N, ManualClock, H>,
    caller: &Caller,
    epoch: u64,
) -> StoredGrant {
    host.bind_platform(caller.clone()).unwrap();
    host.arm(
        caller,
        id(),
        &name("cell/sim"),
        Counter(epoch),
        &scopes(epoch),
        &BTreeSet::new(),
    )
    .unwrap();
    host.acquire_grant(
        caller,
        id(),
        vec![name("sim/controller")],
        Counter(epoch),
        Counter(10_000_000_000),
    )
    .unwrap()
}
fn request<N: NativeAdapter, H: BoundaryHook>(
    host: &Host<N, ManualClock, H>,
    grant: &StoredGrant,
    epoch: u64,
) -> Request {
    let intent = intent();
    let digest = intent.digest().unwrap();
    let operation = id();
    Request {
        operation: operation.clone(),
        intent,
        digest,
        grant: grant.id.clone(),
        permit: Permit {
            id: id(),
            operation,
            digest,
            cell: name("cell/sim"),
            epoch: Counter(epoch),
            scopes: scopes(epoch),
            envelope: binding().envelope.sha256,
            qualification: binding().qualification,
            qualification_revision: Counter(1),
            grant: grant.id.clone(),
            host_boot: host.boot_id().unwrap(),
            conditions: [name("sim/ready")].into_iter().collect(),
            expires_at: TimePoint {
                clock_id: "simulation/boottime".into(),
                ticks_ns: Counter(1_000_000_000),
            },
            source_digest: None,
            purpose: Purpose::Production,
            parent: PermitParent::Mandate(id()),
        },
    }
}
fn caller() -> Caller {
    Caller {
        peer: name("platform"),
        session: id(),
    }
}

#[test]
fn prepare_does_not_call_device_and_authorize_duplicates_are_receipt_lookup() {
    let directory = tempfile::tempdir().unwrap();
    let c = clock();
    let host = Host::open(
        directory.path().join("host.db"),
        FileDevice::open(directory.path().join("device"), c.clone()).unwrap(),
        c,
        vec![binding()],
    )
    .unwrap();
    let caller = caller();
    let grant = initialize(&host, &caller, 1);
    let request = request(&host, &grant, 1);
    let prepared = host.prepare(&caller, request.clone()).unwrap();
    assert!(
        FileDevice::effects(directory.path().join("device"))
            .unwrap()
            .is_empty()
    );
    let result = host
        .authorize(
            &caller,
            request.clone(),
            prepared.invocation.as_ref().unwrap(),
        )
        .unwrap();
    assert_eq!(result.state, ReceiptState::ResultCaptured);
    let repeated = host
        .authorize(&caller, request, prepared.invocation.as_ref().unwrap())
        .unwrap();
    assert_eq!(result.journal_seq, repeated.journal_seq);
    assert_eq!(
        FileDevice::effects(directory.path().join("device"))
            .unwrap()
            .len(),
        1
    );
    let journals = host.journals().unwrap();
    assert_ne!(journals.delivery_journal, journals.evidence_journal);
    let evidence = host.evidence_after(&caller, Counter(0), 128).unwrap();
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].0, Counter(1));
    assert!(result.journal_seq.0 > evidence[0].0.0);
}

#[test]
fn altered_permit_and_unapproved_intent_are_rejected_before_effect() {
    let directory = tempfile::tempdir().unwrap();
    let c = clock();
    let host = Host::open(
        directory.path().join("host.db"),
        FileDevice::open(directory.path().join("device"), c.clone()).unwrap(),
        c,
        vec![binding()],
    )
    .unwrap();
    let caller = caller();
    let grant = initialize(&host, &caller, 1);
    let request = request(&host, &grant, 1);
    let prepared = host.prepare(&caller, request.clone()).unwrap();
    let mut changed = request.clone();
    changed.permit.purpose = Purpose::Setup;
    assert!(matches!(
        host.authorize(&caller, changed, prepared.invocation.as_ref().unwrap()),
        Err(HostError::Conflict)
    ));
    let mut changed = request;
    changed.intent.target = name("sim/other");
    assert!(host.prepare(&caller, changed).is_err());
    assert!(
        FileDevice::effects(directory.path().join("device"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn same_grant_fence_requires_same_request_identity() {
    let directory = tempfile::tempdir().unwrap();
    let c = clock();
    let host = Host::open(
        directory.path().join("host.db"),
        FileDevice::open(directory.path().join("device"), c.clone()).unwrap(),
        c,
        vec![binding()],
    )
    .unwrap();
    let caller = caller();
    host.bind_platform(caller.clone()).unwrap();
    let key = id();
    let resources = vec![name("sim/controller")];
    let first = host
        .acquire_grant(
            &caller,
            key.clone(),
            resources.clone(),
            Counter(1),
            Counter(100),
        )
        .unwrap();
    let duplicate = host
        .acquire_grant(&caller, key, resources.clone(), Counter(1), Counter(100))
        .unwrap();
    assert_eq!(first.id, duplicate.id);
    assert!(
        host.acquire_grant(&caller, id(), resources, Counter(1), Counter(100))
            .is_err()
    );
}

#[test]
fn renewal_replay_does_not_extend_expiry_and_expired_grant_stays_expired() {
    let directory = tempfile::tempdir().unwrap();
    let c = clock();
    let host = Host::open(
        directory.path().join("host.db"),
        FileDevice::open(directory.path().join("device"), c.clone()).unwrap(),
        c.clone(),
        vec![binding()],
    )
    .unwrap();
    let caller = caller();
    host.bind_platform(caller.clone()).unwrap();
    let grant = host
        .acquire_grant(
            &caller,
            id(),
            vec![name("sim/controller")],
            Counter(1),
            Counter(100),
        )
        .unwrap();
    c.ticks.store(1050, Ordering::SeqCst);
    let renewed = host.renew_grant(&caller, &grant.id, Counter(1)).unwrap();
    c.ticks.store(1080, Ordering::SeqCst);
    assert_eq!(
        host.renew_grant(&caller, &grant.id, Counter(1))
            .unwrap()
            .expires_at,
        renewed.expires_at
    );
    c.ticks.store(1200, Ordering::SeqCst);
    assert!(host.renew_grant(&caller, &grant.id, Counter(2)).is_err());
}

#[test]
fn late_fence_replay_cannot_remove_a_new_arm_or_rewind_epoch() {
    let directory = tempfile::tempdir().unwrap();
    let c = clock();
    let host = Host::open(
        directory.path().join("host.db"),
        FileDevice::open(directory.path().join("device"), c.clone()).unwrap(),
        c,
        vec![binding()],
    )
    .unwrap();
    let caller = caller();
    host.bind_platform(caller.clone()).unwrap();
    let fence = id();
    let block = id();
    let first = host
        .fence(
            &caller,
            fence.clone(),
            &name("cell/sim"),
            Counter(2),
            scopes(2),
            [block.clone()].into_iter().collect(),
        )
        .unwrap();
    let arm = id();
    host.arm(
        &caller,
        arm.clone(),
        &name("cell/sim"),
        Counter(2),
        &scopes(2),
        &[block.clone()].into_iter().collect(),
    )
    .unwrap();
    host.arm(
        &caller,
        arm,
        &name("cell/sim"),
        Counter(2),
        &scopes(2),
        &[block.clone()].into_iter().collect(),
    )
    .unwrap();
    let repeated = host
        .fence(
            &caller,
            fence,
            &name("cell/sim"),
            Counter(2),
            scopes(2),
            [block].into_iter().collect(),
        )
        .unwrap();
    assert_eq!(first.sequence, repeated.sequence);
    let grant = host
        .acquire_grant(
            &caller,
            id(),
            vec![name("sim/controller")],
            Counter(2),
            Counter(10_000_000_000),
        )
        .unwrap();
    assert!(host.prepare(&caller, request(&host, &grant, 2)).is_ok());
    assert!(
        host.fence(
            &caller,
            id(),
            &name("cell/sim"),
            Counter(1),
            scopes(1),
            BTreeSet::new()
        )
        .is_err()
    );
}

#[test]
fn prepared_rebind_retires_old_permit_permanently() {
    let directory = tempfile::tempdir().unwrap();
    let c = clock();
    let host = Host::open(
        directory.path().join("host.db"),
        FileDevice::open(directory.path().join("device"), c.clone()).unwrap(),
        c,
        vec![binding()],
    )
    .unwrap();
    let caller = caller();
    let grant = initialize(&host, &caller, 1);
    let old = request(&host, &grant, 1);
    let first = host.prepare(&caller, old.clone()).unwrap();
    let mut new = old.clone();
    new.permit.id = id();
    let rebound = host.prepare(&caller, new.clone()).unwrap();
    assert_eq!(first.invocation, rebound.invocation);
    assert!(host.prepare(&caller, old.clone()).is_err());
    assert!(
        host.authorize(&caller, old, first.invocation.as_ref().unwrap())
            .is_err()
    );
    host.authorize(&caller, new, rebound.invocation.as_ref().unwrap())
        .unwrap();
    assert_eq!(
        FileDevice::effects(directory.path().join("device"))
            .unwrap()
            .len(),
        1
    );
}

struct ExpireAfterCommit(ManualClock);
impl BoundaryHook for ExpireAfterCommit {
    fn after_send_commit(&self) {
        self.0.ticks.store(2_000_000_000, Ordering::SeqCst);
    }
}
#[test]
fn expiry_during_commit_does_not_enter_native_and_is_not_retried() {
    let directory = tempfile::tempdir().unwrap();
    let c = clock();
    let host = Host::with_hooks(
        directory.path().join("host.db"),
        FileDevice::open(directory.path().join("device"), c.clone()).unwrap(),
        c.clone(),
        vec![binding()],
        ExpireAfterCommit(c),
    )
    .unwrap();
    let caller = caller();
    let grant = initialize(&host, &caller, 1);
    let request = request(&host, &grant, 1);
    let prepared = host.prepare(&caller, request.clone()).unwrap();
    assert!(
        host.authorize(
            &caller,
            request.clone(),
            prepared.invocation.as_ref().unwrap()
        )
        .is_err()
    );
    assert_eq!(
        host.receipt(&caller, &request.operation).unwrap().state,
        ReceiptState::SendEntered
    );
    host.authorize(&caller, request, prepared.invocation.as_ref().unwrap())
        .unwrap();
    assert!(
        FileDevice::effects(directory.path().join("device"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn cancellation_arriving_before_prepare_creates_permanent_tombstone() {
    let directory = tempfile::tempdir().unwrap();
    let c = clock();
    let host = Host::open(
        directory.path().join("host.db"),
        FileDevice::open(directory.path().join("device"), c.clone()).unwrap(),
        c,
        vec![binding()],
    )
    .unwrap();
    let caller = caller();
    let grant = initialize(&host, &caller, 1);
    let request = request(&host, &grant, 1);
    let first = host
        .void_before_send(&caller, request.operation.clone(), request.digest)
        .unwrap();
    let again = host
        .void_before_send(&caller, request.operation.clone(), request.digest)
        .unwrap();
    assert_eq!(first.sequence, again.sequence);
    assert!(matches!(
        host.prepare(&caller, request.clone()),
        Err(HostError::Voided)
    ));
    let receipt = host.receipt_view(&caller, &request.operation).unwrap();
    assert_eq!(
        receipt.stage,
        rx_protocol::base::ReceiptStage::NotDispatched as i32
    );
    assert!(receipt.invocation_id.is_none());
    assert!(receipt.operation_revision.is_none());
    assert!(
        FileDevice::effects(directory.path().join("device"))
            .unwrap()
            .is_empty()
    );
}

struct BlockingDevice {
    device: FileDevice,
    entered: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}
impl NativeAdapter for BlockingDevice {
    fn environment(&self) -> Environment {
        self.device.environment()
    }
    fn protection(&self) -> Arc<dyn LocalProtection> {
        self.device.protection()
    }
    fn guard(&self, intent: &Intent, now: &TimePoint) -> Result<Guard> {
        self.device.guard(intent, now)
    }
    fn can_handover(&self, r: &[Name]) -> bool {
        self.device.can_handover(r)
    }
    fn submit(&mut self, op: &Id, inv: &Id, intent: &Intent) -> Result<NativeCapture> {
        self.entered.send(()).unwrap();
        self.release.recv().unwrap();
        self.device.submit(op, inv, intent)
    }
    fn lookup(&mut self, op: &Id, inv: &Id) -> Result<Option<NativeCapture>> {
        self.device.lookup(op, inv)
    }
}
struct ReleaseGuard(Option<std::sync::mpsc::Sender<()>>);
impl Drop for ReleaseGuard {
    fn drop(&mut self) {
        if let Some(s) = self.0.take() {
            let _ = s.send(());
        }
    }
}
#[test]
fn native_submission_serializes_fence_while_protection_is_independent() {
    let directory = tempfile::tempdir().unwrap();
    let c = clock();
    let device = FileDevice::open(directory.path().join("device"), c.clone()).unwrap();
    let protection = device.protection.clone();
    let (entered, wait) = std::sync::mpsc::channel();
    let (release, gate) = std::sync::mpsc::channel();
    let mut release = ReleaseGuard(Some(release));
    let native = BlockingDevice {
        device,
        entered,
        release: gate,
    };
    let host =
        Arc::new(Host::open(directory.path().join("host.db"), native, c, vec![binding()]).unwrap());
    let caller = caller();
    let grant = initialize(&host, &caller, 1);
    let request = request(&host, &grant, 1);
    let prepared = host.prepare(&caller, request.clone()).unwrap();
    let send_host = host.clone();
    let send_caller = caller.clone();
    let send = std::thread::spawn(move || {
        send_host.authorize(&send_caller, request, prepared.invocation.as_ref().unwrap())
    });
    wait.recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let (attempted, seen) = std::sync::mpsc::channel();
    let (finished, done) = std::sync::mpsc::channel();
    let fence_host = host.clone();
    let fence_caller = caller.clone();
    let fence = std::thread::spawn(move || {
        attempted.send(()).unwrap();
        let receipt = fence_host
            .fence(
                &fence_caller,
                id(),
                &name("cell/sim"),
                Counter(2),
                scopes(2),
                BTreeSet::new(),
            )
            .unwrap();
        finished.send(()).unwrap();
        receipt
    });
    seen.recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    host.protection().react(ProtectionIncident::StoreFault);
    assert_eq!(protection.0.load(Ordering::SeqCst), 1);
    assert!(done.try_recv().is_err());
    release.0.take().unwrap().send(()).unwrap();
    let sent = send.join().unwrap().unwrap();
    let fenced = fence.join().unwrap();
    assert!(fenced.sequence > sent.journal_seq);
}

#[test]
fn simulated_device_has_an_independent_exclusive_owner() {
    let directory = tempfile::tempdir().unwrap();
    let c = clock();
    let first = FileDevice::open(directory.path(), c.clone()).unwrap();
    assert!(matches!(
        FileDevice::open(directory.path(), c.clone()),
        Err(HostError::Busy)
    ));
    drop(first);
    assert!(FileDevice::open(directory.path(), c).is_ok());
}

#[test]
fn evidence_publication_cursor_is_durable_scoped_and_never_accepts_a_foreign_ack() {
    use rx_host::publication::*;
    let directory = tempfile::tempdir().unwrap();
    let c = clock();
    let host = Host::open(
        directory.path().join("host.db"),
        FileDevice::open(directory.path().join("device"), c.clone()).unwrap(),
        c.clone(),
        vec![binding()],
    )
    .unwrap();
    let caller = caller();
    let grant = initialize(&host, &caller, 1);
    for _ in 0..3 {
        let request = request(&host, &grant, 1);
        let prepared = host.prepare(&caller, request.clone()).unwrap();
        host.authorize(&caller, request, prepared.invocation.as_ref().unwrap())
            .unwrap();
    }
    let destination = Destination {
        platform: name("platform"),
        installation: id(),
        store_generation: id(),
    };
    let chunk = host.publication_chunk(&destination).unwrap();
    assert_eq!(chunk.first, Counter(1));
    assert_eq!(chunk.tail, Counter(3));
    assert_eq!(chunk.records.len(), 3);
    let ack = |through, sequence| PublicationAck {
        installation: destination.installation.clone(),
        store_generation: destination.store_generation.clone(),
        view_id: CONTROL_VIEW.into(),
        journal: chunk.journal.clone(),
        through: Counter(through),
        platform_sequence: Counter(sequence),
    };
    assert!(
        host.acknowledge_publication(&destination, ack(4, 10))
            .is_err()
    );
    let mut wrong = ack(1, 10);
    wrong.store_generation = id();
    assert!(host.acknowledge_publication(&destination, wrong).is_err());
    let cursor = host
        .acknowledge_publication(&destination, ack(2, 10))
        .unwrap();
    assert_eq!(
        host.acknowledge_publication(&destination, ack(1, 9))
            .unwrap(),
        cursor
    );
    assert!(
        host.acknowledge_publication(&destination, ack(1, 11))
            .is_err()
    );
    let chunk = host.publication_chunk(&destination).unwrap();
    assert_eq!(chunk.first, Counter(3));
    assert_eq!(chunk.records.len(), 1);
    drop(host);
    let host = Host::open(
        directory.path().join("host.db"),
        FileDevice::open(directory.path().join("device"), c.clone()).unwrap(),
        c,
        vec![binding()],
    )
    .unwrap();
    assert_eq!(host.publication_chunk(&destination).unwrap().cursor, cursor);
    let restored = Destination {
        store_generation: id(),
        ..destination
    };
    assert_eq!(host.publication_chunk(&restored).unwrap().first, Counter(1));
    assert_eq!(
        FileDevice::effects(directory.path().join("device"))
            .unwrap()
            .len(),
        3
    );
}

fn configuration_request<N: NativeAdapter, H: BoundaryHook>(
    host: &Host<N, ManualClock, H>,
    caller: &Caller,
) -> rx_domain::host_configuration::Request {
    use rx_domain::host_configuration::{CellTarget, Request};
    let snapshot = host.inspect_process_configuration(caller).unwrap().snapshot;
    let targets = snapshot
        .cells
        .iter()
        .map(|c| {
            let epoch = Counter(c.epoch.0 + 1);
            let scopes = c
                .scopes
                .keys()
                .cloned()
                .map(|k| (k, Counter(epoch.0)))
                .collect();
            let fence = id();
            let receipt = host
                .fence(
                    caller,
                    fence.clone(),
                    &c.cell,
                    epoch,
                    scopes,
                    BTreeSet::from([id()]),
                )
                .unwrap();
            CellTarget {
                cell: c.cell.clone(),
                expected_context: c.applied.as_ref().map(|a| a.configuration),
                before_configuration: Digest::from_bytes([6; 32]),
                after_configuration: Digest::from_bytes([7; 32]),
                recipe: ArtifactRef {
                    sha256: Digest::from_bytes([8; 32]),
                    schema_id: name("rx.resolved-process.v1"),
                    size_bytes: Counter(1),
                },
                definition: c.definition,
                envelope: c.envelope,
                environment: c.environment.clone(),
                required_intents: vec![intent().digest().unwrap()],
                required_conditions: binding().condition_ids,
                epoch,
                scopes: receipt.state.scopes,
                fence_request: fence,
            }
        })
        .collect();
    Request {
        schema: name("rx.host-process-configuration-request.v1"),
        id: id(),
        change: id(),
        preparation: Counter(1),
        plan_digest: Digest::from_bytes([9; 32]),
        host: snapshot.host,
        expected_host_boot: snapshot.host_boot,
        expected_delivery_journal: snapshot.delivery_journal,
        binding_digest: snapshot.binding_digest,
        cells: targets,
    }
}
#[test]
fn configuration_context_is_durable_unqualified_and_same_request_never_reapplies() {
    use rx_domain::host_configuration::{Effect, Status};
    let directory = tempfile::tempdir().unwrap();
    let c = clock();
    let caller = caller();
    let host = Host::open(
        directory.path().join("h.db"),
        FileDevice::open(directory.path().join("device"), c.clone()).unwrap(),
        c.clone(),
        vec![binding()],
    )
    .unwrap();
    host.bind_platform(caller.clone()).unwrap();
    let old_arm = id();
    host.arm(
        &caller,
        old_arm.clone(),
        &binding().cell,
        Counter(1),
        &scopes(1),
        &BTreeSet::new(),
    )
    .unwrap();
    let request = configuration_request(&host, &caller);
    let first = host
        .accept_process_configuration(&caller, request.clone())
        .unwrap();
    first.validate().unwrap();
    let receipt = first.receipt.unwrap();
    assert_eq!(receipt.status, Status::AppliedUnqualified);
    assert_eq!(receipt.effect, Effect::Installed);
    assert!(first.context_matches_current_host);
    assert!(
        host.arm(
            &caller,
            old_arm,
            &binding().cell,
            Counter(1),
            &scopes(1),
            &BTreeSet::new()
        )
        .is_err()
    );
    let mut duplicate = request.clone();
    duplicate.id = id();
    duplicate.cells[0].expected_context = Some(duplicate.cells[0].after_configuration);
    assert!(matches!(
        host.accept_process_configuration(&caller, duplicate),
        Err(HostError::Store(rx_ports::StoreError::KeyConflict))
    ));
    let repeated = host
        .accept_process_configuration(&caller, request.clone())
        .unwrap()
        .receipt
        .unwrap();
    assert_eq!(repeated.sequence, receipt.sequence);
    assert!(
        FileDevice::effects(directory.path().join("device"))
            .unwrap()
            .is_empty()
    );
    let state = host.inspect_cell(&caller, &binding().cell).unwrap().state;
    assert!(matches!(
        host.arm(
            &caller,
            id(),
            &binding().cell,
            state.epoch,
            &state.scopes,
            &state.blocked
        ),
        Err(HostError::Guard)
    ));
    drop(host);
    let host = Host::open(
        directory.path().join("h.db"),
        FileDevice::open(directory.path().join("device"), c.clone()).unwrap(),
        c,
        vec![binding()],
    )
    .unwrap();
    let next_caller = Caller {
        peer: name("platform"),
        session: id(),
    };
    host.bind_platform(next_caller.clone()).unwrap();
    let historical = host
        .lookup_process_configuration(&next_caller, &request.id)
        .unwrap();
    assert_eq!(historical.receipt.unwrap().sequence, receipt.sequence);
    assert!(!historical.context_matches_current_host);
    let state = host
        .inspect_cell(&next_caller, &binding().cell)
        .unwrap()
        .state;
    assert!(
        host.arm(
            &next_caller,
            id(),
            &binding().cell,
            state.epoch,
            &state.scopes,
            &state.blocked
        )
        .is_err()
    );
    let confirm = configuration_request(&host, &next_caller);
    let receipt = host
        .accept_process_configuration(&next_caller, confirm)
        .unwrap()
        .receipt
        .unwrap();
    assert_eq!(receipt.effect, Effect::AlreadyPresent);
}
#[test]
fn configuration_scope_binding_and_fence_are_not_caller_success_claims() {
    let directory = tempfile::tempdir().unwrap();
    let c = clock();
    let host = Host::open(
        directory.path().join("h.db"),
        FileDevice::open(directory.path().join("device"), c.clone()).unwrap(),
        c,
        vec![binding()],
    )
    .unwrap();
    let who = caller();
    host.bind_platform(who.clone()).unwrap();
    let good = configuration_request(&host, &who);
    let mut changed = good.clone();
    changed.cells[0].fence_request = id();
    assert!(host.accept_process_configuration(&who, changed).is_err());
    let mut changed = good.clone();
    changed.binding_digest = Digest::from_bytes([99; 32]);
    assert!(host.accept_process_configuration(&who, changed).is_err());
    let mut changed = good.clone();
    changed.cells[0].definition = Digest::from_bytes([99; 32]);
    let value = host
        .accept_process_configuration(&who, changed.clone())
        .unwrap();
    assert_eq!(
        value.receipt.unwrap().status,
        rx_domain::host_configuration::Status::NotApplied
    );
    assert!(value.snapshot.cells[0].applied.is_none());
    assert!(host.accept_process_configuration(&who, good).is_err());
    let new_caller = caller();
    host.bind_platform(new_caller).unwrap();
    assert!(host.accept_process_configuration(&who, changed).is_err());
}
#[test]
fn configuration_does_not_treat_prepared_native_work_as_quiescent() {
    let directory = tempfile::tempdir().unwrap();
    let c = clock();
    let host = Host::open(
        directory.path().join("h.db"),
        FileDevice::open(directory.path().join("device"), c.clone()).unwrap(),
        c,
        vec![binding()],
    )
    .unwrap();
    let who = caller();
    let grant = initialize(&host, &who, 1);
    let request = request(&host, &grant, 1);
    host.prepare(&who, request.clone()).unwrap();
    let change = configuration_request(&host, &who);
    let value = host.accept_process_configuration(&who, change).unwrap();
    let receipt = value.receipt.unwrap();
    assert_eq!(
        receipt.status,
        rx_domain::host_configuration::Status::NotApplied
    );
    assert_eq!(receipt.reason, Some(name("HOST_WORK_UNRESOLVED")));
    assert!(value.snapshot.cells[0].applied.is_none());
    assert_eq!(
        host.receipt(&who, &request.operation).unwrap().state,
        ReceiptState::Prepared
    );
}
#[test]
fn configuration_commit_failure_has_no_receipt_context_or_consumed_slot() {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Fault(Arc<AtomicBool>);
    impl BoundaryHook for Fault {
        fn before_configuration_commit(&self) -> std::result::Result<(), String> {
            if self.0.swap(false, Ordering::SeqCst) {
                Err("injected rollback".into())
            } else {
                Ok(())
            }
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let c = clock();
    let fault = Arc::new(AtomicBool::new(true));
    let host = Host::with_hooks(
        directory.path().join("h.db"),
        FileDevice::open(directory.path().join("device"), c.clone()).unwrap(),
        c,
        vec![binding()],
        Fault(fault),
    )
    .unwrap();
    let who = caller();
    host.bind_platform(who.clone()).unwrap();
    let request = configuration_request(&host, &who);
    let before = host.journals().unwrap().delivery_seq;
    assert!(
        host.accept_process_configuration(&who, request.clone())
            .is_err()
    );
    let state = host
        .lookup_process_configuration(&who, &request.id)
        .unwrap();
    assert!(state.receipt.is_none() && state.snapshot.cells[0].applied.is_none());
    assert_eq!(host.journals().unwrap().delivery_seq, before);
    let result = host.accept_process_configuration(&who, request).unwrap();
    assert_eq!(
        result.receipt.unwrap().status,
        rx_domain::host_configuration::Status::AppliedUnqualified
    );
}
#[test]
fn configuration_missing_quiescence_does_not_create_applied_context() {
    struct NoQuiescence;
    impl NativeAdapter for NoQuiescence {
        fn environment(&self) -> Environment {
            Environment::Simulation
        }
        fn protection(&self) -> Arc<dyn LocalProtection> {
            Arc::new(ProtectionCounter(AtomicU64::new(0)))
        }
        fn guard(&self, _: &Intent, _: &TimePoint) -> Result<Guard> {
            Err(HostError::Guard)
        }
        fn can_handover(&self, _: &[Name]) -> bool {
            true
        }
        fn submit(&mut self, _: &Id, _: &Id, _: &Intent) -> Result<NativeCapture> {
            panic!("no native entry")
        }
        fn lookup(&mut self, _: &Id, _: &Id) -> Result<Option<NativeCapture>> {
            panic!("no native lookup")
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let host = Host::open(
        directory.path().join("h.db"),
        NoQuiescence,
        clock(),
        vec![binding()],
    )
    .unwrap();
    let who = caller();
    host.bind_platform(who.clone()).unwrap();
    let request = configuration_request(&host, &who);
    let result = host.accept_process_configuration(&who, request).unwrap();
    assert_eq!(
        result.receipt.unwrap().reason,
        Some(name("QUIESCENCE_UNAVAILABLE"))
    );
    assert!(result.snapshot.cells[0].applied.is_none());
}

#[test]
fn configuration_cohort_is_complete_and_a_rejected_member_prevents_all_context_changes() {
    let directory = tempfile::tempdir().unwrap();
    let c = clock();
    let mut second = binding();
    second.cell = name("cell/second");
    let host = Host::open(
        directory.path().join("h.db"),
        FileDevice::open(directory.path().join("device"), c.clone()).unwrap(),
        c,
        vec![binding(), second],
    )
    .unwrap();
    let who = caller();
    host.bind_platform(who.clone()).unwrap();
    let request = configuration_request(&host, &who);
    let mut partial = request.clone();
    partial.cells.pop();
    assert!(host.accept_process_configuration(&who, partial).is_err());
    let mut rejected = request;
    rejected.cells[1].required_intents = vec![Digest::from_bytes([99; 32])];
    let result = host.accept_process_configuration(&who, rejected).unwrap();
    assert_eq!(
        result.receipt.unwrap().status,
        rx_domain::host_configuration::Status::NotApplied
    );
    assert!(result.snapshot.cells.iter().all(|c| c.applied.is_none()));
    let next = configuration_request(&host, &who);
    let value = host.accept_process_configuration(&who, next).unwrap();
    assert_eq!(
        value.receipt.unwrap().status,
        rx_domain::host_configuration::Status::AppliedUnqualified
    );
    assert!(value.snapshot.cells.iter().all(|c| c.applied.is_some()));
}
#[test]
fn configuration_rejects_an_old_quiescence_observation() {
    struct Old;
    impl NativeAdapter for Old {
        fn environment(&self) -> Environment {
            Environment::Simulation
        }
        fn protection(&self) -> Arc<dyn LocalProtection> {
            Arc::new(ProtectionCounter(AtomicU64::new(0)))
        }
        fn guard(&self, _: &Intent, _: &TimePoint) -> Result<Guard> {
            Err(HostError::Guard)
        }
        fn can_handover(&self, _: &[Name]) -> bool {
            true
        }
        fn handover_snapshot(&self, _: &[Name]) -> Result<LocalHandover> {
            Ok(LocalHandover {
                device_session: id(),
                observed_at: TimePoint {
                    clock_id: "simulation/boottime".into(),
                    ticks_ns: Counter(0),
                },
                uncertainty_ns: Counter(0),
                no_pending_commands: true,
                control_available: true,
                support_stable: true,
            })
        }
        fn submit(&mut self, _: &Id, _: &Id, _: &Intent) -> Result<NativeCapture> {
            panic!("no native entry")
        }
        fn lookup(&mut self, _: &Id, _: &Id) -> Result<Option<NativeCapture>> {
            panic!("no native lookup")
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let c = clock();
    c.ticks.store(200_000_000, Ordering::SeqCst);
    let host = Host::open(directory.path().join("h.db"), Old, c, vec![binding()]).unwrap();
    let who = caller();
    host.bind_platform(who.clone()).unwrap();
    let request = configuration_request(&host, &who);
    let value = host.accept_process_configuration(&who, request).unwrap();
    assert_eq!(
        value.receipt.unwrap().status,
        rx_domain::host_configuration::Status::NotApplied
    );
    assert!(value.snapshot.cells[0].applied.is_none());
}

#[path = "support/qualification_tests.rs"]
mod qualification_tests;

#[test]
fn service_stop_latches_before_database_access_and_preserves_uncertain_work() {
    let dir = tempfile::tempdir().unwrap();
    let c = clock();
    let h = Host::open(
        dir.path().join("h.db"),
        FileDevice::open(dir.path().join("device"), c.clone()).unwrap(),
        c,
        vec![binding()],
    )
    .unwrap();
    let who = caller();
    let grant = initialize(&h, &who, 1);
    let request = request(&h, &grant, 1);
    let prepared = h.prepare(&who, request.clone()).unwrap();
    h.request_service_stop();
    assert!(
        h.authorize(&who, request.clone(), prepared.invocation.as_ref().unwrap())
            .is_err()
    );
    assert!(
        h.acquire_grant(
            &who,
            id(),
            vec![name("sim/controller")],
            Counter(2),
            Counter(1_000_000)
        )
        .is_err()
    );
    assert!(
        h.arm(
            &who,
            id(),
            &binding().cell,
            Counter(1),
            &scopes(1),
            &BTreeSet::new()
        )
        .is_err()
    );
    let kept = h.receipt(&who, &request.operation).unwrap();
    assert_eq!(kept.state, ReceiptState::Prepared);
    let snapshot = h.service_stop_snapshot().unwrap();
    assert!(
        !snapshot.admission_open && snapshot.safe_to_drop && !snapshot.physical_shutdown_assessed
    );
    assert_eq!(snapshot.pending_operations, vec![request.operation]);
    assert!(
        FileDevice::effects(dir.path().join("device"))
            .unwrap()
            .is_empty()
    );
}
#[test]
fn stable_handover_is_not_permission_to_drop_an_adapter() {
    struct Unsupported(FileDevice);
    impl NativeAdapter for Unsupported {
        fn environment(&self) -> Environment {
            Environment::Simulation
        }
        fn protection(&self) -> Arc<dyn LocalProtection> {
            self.0.protection()
        }
        fn guard(&self, i: &Intent, t: &TimePoint) -> Result<Guard> {
            self.0.guard(i, t)
        }
        fn can_handover(&self, r: &[Name]) -> bool {
            self.0.can_handover(r)
        }
        fn handover_snapshot(&self, r: &[Name]) -> Result<LocalHandover> {
            self.0.handover_snapshot(r)
        }
        fn submit(&mut self, o: &Id, v: &Id, i: &Intent) -> Result<NativeCapture> {
            self.0.submit(o, v, i)
        }
        fn lookup(&mut self, o: &Id, v: &Id) -> Result<Option<NativeCapture>> {
            self.0.lookup(o, v)
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let c = clock();
    let h = Host::open(
        dir.path().join("h.db"),
        Unsupported(FileDevice::open(dir.path().join("device"), c.clone()).unwrap()),
        c,
        vec![binding()],
    )
    .unwrap();
    h.request_service_stop();
    let snapshot = h.service_stop_snapshot().unwrap();
    assert!(!snapshot.safe_to_drop && snapshot.native.is_none());
}

#[test]
fn service_stop_latch_interrupts_the_pre_native_boundary_without_waiting_for_gate_lock() {
    struct Pause {
        entered: Arc<std::sync::Barrier>,
        resume: Arc<std::sync::Barrier>,
    }
    impl BoundaryHook for Pause {
        fn after_send_commit(&self) {
            self.entered.wait();
            self.resume.wait();
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let c = clock();
    let entered = Arc::new(std::sync::Barrier::new(2));
    let resume = Arc::new(std::sync::Barrier::new(2));
    let h = Arc::new(
        Host::with_hooks(
            dir.path().join("h.db"),
            FileDevice::open(dir.path().join("device"), c.clone()).unwrap(),
            c,
            vec![binding()],
            Pause {
                entered: entered.clone(),
                resume: resume.clone(),
            },
        )
        .unwrap(),
    );
    let who = caller();
    let grant = initialize(&h, &who, 1);
    let request = request(&h, &grant, 1);
    let prepared = h.prepare(&who, request.clone()).unwrap();
    let worker = h.clone();
    let caller = who.clone();
    let invoke = request.clone();
    let thread = std::thread::spawn(move || {
        worker.authorize(&caller, invoke, prepared.invocation.as_ref().unwrap())
    });
    entered.wait();
    let (sent, received) = std::sync::mpsc::channel();
    let stopper = h.clone();
    let stop = std::thread::spawn(move || {
        stopper.request_service_stop();
        sent.send(()).unwrap();
    });
    received
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    stop.join().unwrap();
    resume.wait();
    assert!(thread.join().unwrap().is_err());
    assert_eq!(
        h.receipt(&who, &request.operation).unwrap().state,
        ReceiptState::SendEntered
    );
    assert!(
        FileDevice::effects(dir.path().join("device"))
            .unwrap()
            .is_empty()
    );
}
