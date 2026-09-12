use super::*;
use rx_domain::host_qualification as q;
fn target<N: NativeAdapter, H: BoundaryHook>(
    host: &Host<N, ManualClock, H>,
    who: &Caller,
) -> q::Request {
    let initial = host.inspect_process_configuration(who).unwrap();
    if initial.snapshot.cells.iter().any(|c| c.applied.is_none()) {
        let r = configuration_request(host, who);
        host.accept_process_configuration(who, r).unwrap();
    }
    let snapshot = host.inspect_process_configuration(who).unwrap().snapshot;
    let mut cells = Vec::new();
    let mut change = None;
    for c in &snapshot.cells {
        let context = c.applied.as_ref().unwrap();
        change = Some(context.change.clone());
        let epoch = Counter(c.epoch.0 + 1);
        let scopes = c
            .scopes
            .iter()
            .map(|(k, v)| (k.clone(), Counter(v.0 + 1)))
            .collect();
        let fence = id();
        let mut blocks = c.blocked.iter().cloned().collect::<BTreeSet<_>>();
        blocks.insert(id());
        let gate = host
            .fence(who, fence.clone(), &c.cell, epoch, scopes, blocks.clone())
            .unwrap();
        cells.push(q::CellTarget {
            cell: c.cell.clone(),
            configuration: context.configuration,
            context_request: context.request.clone(),
            context_sequence: context.receipt_sequence,
            definition: c.definition,
            envelope: c.envelope,
            environment: c.environment.clone(),
            qualification: id(),
            qualification_revision: Counter(1),
            dependencies: vec![context.configuration, c.definition, c.envelope],
            limitations: ArtifactRef {
                sha256: Digest::from_bytes([81; 32]),
                schema_id: name("test/limitations"),
                size_bytes: Counter(1),
            },
            allowed_intents: vec![intent().digest().unwrap()],
            purposes: vec![name("PRODUCTION")],
            epoch,
            scopes: gate.state.scopes,
            fence_request: fence,
            required_blocks: blocks.into_iter().collect(),
        });
    }
    q::Request {
        schema: name("rx.host-qualification-request.v1"),
        id: id(),
        host: snapshot.host,
        expected_host_boot: snapshot.host_boot,
        delivery_journal: snapshot.delivery_journal,
        binding_digest: snapshot.binding_digest,
        change: change.unwrap(),
        review: id(),
        review_revision: Counter(1),
        review_digest: Digest::from_bytes([82; 32]),
        decision_revision: Counter(1),
        policy_digest: Digest::from_bytes([83; 32]),
        application_digest: Digest::from_bytes([84; 32]),
        cells,
    }
}
#[test]
fn qualification_acceptance_preserves_blocks_requires_explicit_arm_and_rejects_old_qualification() {
    let dir = tempfile::tempdir().unwrap();
    let c = clock();
    let host = Host::open(
        dir.path().join("h.db"),
        FileDevice::open(dir.path().join("device"), c.clone()).unwrap(),
        c,
        vec![binding()],
    )
    .unwrap();
    let who = caller();
    host.bind_platform(who.clone()).unwrap();
    let q = target(&host, &who);
    let t = &q.cells[0];
    let before = host.inspect_cell(&who, &t.cell).unwrap().state;
    let first = host.accept_qualification(&who, q.clone()).unwrap();
    assert_eq!(first.receipt.as_ref().unwrap().status, q::Status::Accepted);
    assert!(first.receipt_matches_current_host && !first.activation_authorized);
    assert_eq!(
        host.inspect_cell(&who, &t.cell).unwrap().state.blocked,
        before.blocked
    );
    let grant = host
        .acquire_grant(
            &who,
            id(),
            vec![name("sim/controller")],
            t.epoch,
            Counter(10_000_000_000),
        )
        .unwrap();
    let mut operation = request(&host, &grant, t.epoch.0);
    operation.permit.qualification = t.qualification.clone();
    operation.permit.qualification_revision = t.qualification_revision;
    assert!(host.prepare(&who, operation.clone()).is_err());
    assert!(
        FileDevice::effects(dir.path().join("device"))
            .unwrap()
            .is_empty()
    );
    host.arm(&who, id(), &t.cell, t.epoch, &t.scopes, &before.blocked)
        .unwrap();
    let old = request(&host, &grant, t.epoch.0);
    assert!(host.prepare(&who, old).is_err());
    let mut wrong_purpose = operation.clone();
    wrong_purpose.permit.purpose = Purpose::Setup;
    assert!(host.prepare(&who, wrong_purpose).is_err());
    let replay = host.accept_qualification(&who, q.clone()).unwrap();
    assert_eq!(
        replay.receipt.as_ref().unwrap().sequence,
        first.receipt.as_ref().unwrap().sequence
    );
    let prepared = host.prepare(&who, operation.clone()).unwrap();
    host.authorize(&who, operation, prepared.invocation.as_ref().unwrap())
        .unwrap();
    assert_eq!(
        FileDevice::effects(dir.path().join("device"))
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn qualification_restart_keeps_receipt_and_never_restores_applicability_or_arm() {
    let dir = tempfile::tempdir().unwrap();
    let c = clock();
    let open = || {
        Host::open(
            dir.path().join("h.db"),
            FileDevice::open(dir.path().join("device"), c.clone()).unwrap(),
            c.clone(),
            vec![binding()],
        )
        .unwrap()
    };
    let h = open();
    let who = caller();
    h.bind_platform(who.clone()).unwrap();
    let q = target(&h, &who);
    let receipt = h
        .accept_qualification(&who, q.clone())
        .unwrap()
        .receipt
        .unwrap();
    drop(h);
    let h = open();
    let next = caller();
    h.bind_platform(next.clone()).unwrap();
    let restored = h.lookup_qualification(&next, &q.id).unwrap();
    assert_eq!(
        restored.receipt.as_ref().unwrap().sequence,
        receipt.sequence
    );
    assert!(!restored.receipt_matches_current_host);
    let t = &q.cells[0];
    assert!(
        h.arm(
            &next,
            id(),
            &t.cell,
            t.epoch,
            &t.scopes,
            &t.required_blocks.iter().cloned().collect()
        )
        .is_err()
    );
    let repeat = h.accept_qualification(&next, q).unwrap();
    assert!(!repeat.receipt_matches_current_host);
    assert!(
        FileDevice::effects(dir.path().join("device"))
            .unwrap()
            .is_empty()
    );
}
#[test]
fn qualification_same_key_same_review_and_changed_scope_cannot_replace_accepted_identity() {
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
    h.bind_platform(who.clone()).unwrap();
    let q = target(&h, &who);
    h.accept_qualification(&who, q.clone()).unwrap();
    let mut changed = q.clone();
    changed.cells[0].qualification = id();
    assert!(matches!(
        h.accept_qualification(&who, changed.clone()),
        Err(HostError::Conflict)
    ));
    changed.id = id();
    assert!(h.accept_qualification(&who, changed.clone()).is_err());
    changed.review = id();
    assert!(h.accept_qualification(&who, changed).is_err());
    let original = h.lookup_qualification(&who, &q.id).unwrap();
    assert_eq!(original.accepted[0].qualification, q.cells[0].qualification);
    let newer = target(&h, &who);
    assert!(
        !h.lookup_qualification(&who, &q.id)
            .unwrap()
            .receipt_matches_current_host
    );
    let mut expanded = newer;
    expanded.cells[0]
        .allowed_intents
        .push(Digest::from_bytes([91; 32]));
    let result = h.accept_qualification(&who, expanded).unwrap();
    assert_eq!(
        result.receipt.as_ref().unwrap().status,
        q::Status::NotAccepted
    );
    assert_eq!(result.accepted[0].qualification, q.cells[0].qualification);
}
#[test]
fn qualification_commit_failure_rolls_back_cohort_receipt_and_slot() {
    use std::sync::atomic::AtomicBool;
    struct Fail(Arc<AtomicBool>);
    impl BoundaryHook for Fail {
        fn before_qualification_commit(&self) -> std::result::Result<(), String> {
            if self.0.swap(false, Ordering::SeqCst) {
                Err("injected before commit".into())
            } else {
                Ok(())
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let c = clock();
    let h = Host::with_hooks(
        dir.path().join("h.db"),
        FileDevice::open(dir.path().join("device"), c.clone()).unwrap(),
        c,
        vec![binding()],
        Fail(Arc::new(AtomicBool::new(true))),
    )
    .unwrap();
    let who = caller();
    h.bind_platform(who.clone()).unwrap();
    let q = target(&h, &who);
    let before = h.journals().unwrap().delivery_seq;
    assert!(h.accept_qualification(&who, q.clone()).is_err());
    let absent = h.lookup_qualification(&who, &q.id).unwrap();
    assert!(absent.receipt.is_none() && absent.accepted.is_empty());
    assert_eq!(h.journals().unwrap().delivery_seq, before);
    assert_eq!(
        h.accept_qualification(&who, q)
            .unwrap()
            .receipt
            .unwrap()
            .status,
        q::Status::Accepted
    );
}
#[test]
fn qualification_full_cohort_is_atomic_and_a_wrong_member_cannot_be_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let c = clock();
    let a = binding();
    let mut b = a.clone();
    b.cell = name("cell/other");
    b.definition.sha256 = Digest::from_bytes([52; 32]);
    let h = Host::open(
        dir.path().join("h.db"),
        FileDevice::open(dir.path().join("device"), c.clone()).unwrap(),
        c,
        vec![a, b],
    )
    .unwrap();
    let who = caller();
    h.bind_platform(who.clone()).unwrap();
    let q = target(&h, &who);
    let mut partial = q.clone();
    partial.cells.pop();
    assert!(h.accept_qualification(&who, partial).is_err());
    let mut wrong = q.clone();
    wrong.cells[1].context_request = id();
    assert!(h.accept_qualification(&who, wrong).is_err());
    assert!(h.inspect_qualification(&who).unwrap().accepted.is_empty());
    let accepted = h.accept_qualification(&who, q).unwrap();
    assert_eq!(accepted.accepted.len(), 2);
    assert!(accepted.receipt_matches_current_host);
}
#[test]
fn qualification_revalidation_does_not_hide_prepared_native_work() {
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
    h.bind_platform(who.clone()).unwrap();
    let q = target(&h, &who);
    h.accept_qualification(&who, q.clone()).unwrap();
    let t = &q.cells[0];
    h.arm(
        &who,
        id(),
        &t.cell,
        t.epoch,
        &t.scopes,
        &t.required_blocks.iter().cloned().collect(),
    )
    .unwrap();
    let grant = h
        .acquire_grant(
            &who,
            id(),
            vec![name("sim/controller")],
            t.epoch,
            Counter(10_000_000_000),
        )
        .unwrap();
    let mut op = request(&h, &grant, t.epoch.0);
    op.permit.qualification = t.qualification.clone();
    h.prepare(&who, op).unwrap();
    let next = target(&h, &who);
    let refused = h.accept_qualification(&who, next).unwrap();
    assert_eq!(
        refused.receipt.as_ref().unwrap().status,
        q::Status::NotAccepted
    );
    assert_eq!(
        refused
            .receipt
            .as_ref()
            .unwrap()
            .reason
            .as_ref()
            .unwrap()
            .as_str(),
        "HOST_WORK_UNRESOLVED"
    );
    assert!(
        FileDevice::effects(dir.path().join("device"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn qualification_identity_cannot_be_reused_for_different_evidence_and_empty_scope_grants_no_operations()
 {
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
    h.bind_platform(who.clone()).unwrap();
    let first = target(&h, &who);
    h.accept_qualification(&who, first.clone()).unwrap();
    let mut second = target(&h, &who);
    second.cells[0].qualification = first.cells[0].qualification.clone();
    assert!(h.accept_qualification(&who, second.clone()).is_err());
    second.cells[0].qualification = id();
    second.cells[0].allowed_intents.clear();
    let accepted = h.accept_qualification(&who, second.clone()).unwrap();
    assert_eq!(
        accepted.receipt.as_ref().unwrap().status,
        q::Status::Accepted
    );
    let t = &second.cells[0];
    h.arm(
        &who,
        id(),
        &t.cell,
        t.epoch,
        &t.scopes,
        &t.required_blocks.iter().cloned().collect(),
    )
    .unwrap();
    let grant = h
        .acquire_grant(
            &who,
            id(),
            vec![name("sim/controller")],
            t.epoch,
            Counter(10_000_000_000),
        )
        .unwrap();
    let mut op = request(&h, &grant, t.epoch.0);
    op.permit.qualification = t.qualification.clone();
    assert!(h.prepare(&who, op).is_err());
    assert!(
        FileDevice::effects(dir.path().join("device"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn qualification_requires_supported_fresh_local_quiescence() {
    use std::sync::atomic::AtomicU8;
    struct Device {
        inner: FileDevice,
        mode: Arc<AtomicU8>,
    }
    impl NativeAdapter for Device {
        fn environment(&self) -> Environment {
            self.inner.environment()
        }
        fn protection(&self) -> Arc<dyn LocalProtection> {
            self.inner.protection()
        }
        fn guard(&self, i: &Intent, t: &TimePoint) -> Result<Guard> {
            let mut guard = self.inner.guard(i, t)?;
            if self.mode.load(Ordering::SeqCst) == 5 {
                guard.device_session = id();
            }
            Ok(guard)
        }
        fn can_handover(&self, r: &[Name]) -> bool {
            self.inner.can_handover(r)
        }
        fn handover_snapshot(&self, r: &[Name]) -> Result<LocalHandover> {
            let mut q = self.inner.handover_snapshot(r)?;
            match self.mode.load(Ordering::SeqCst) {
                1 => return Err(HostError::Guard),
                2 => q.observed_at.clock_id = "wrong/clock".into(),
                3 => q.support_stable = false,
                4 => q.no_pending_commands = false,
                _ => {}
            }
            Ok(q)
        }
        fn submit(&mut self, o: &Id, v: &Id, i: &Intent) -> Result<NativeCapture> {
            self.inner.submit(o, v, i)
        }
        fn lookup(&mut self, o: &Id, v: &Id) -> Result<Option<NativeCapture>> {
            self.inner.lookup(o, v)
        }
    }
    for mode in 1..=5 {
        let dir = tempfile::tempdir().unwrap();
        let c = clock();
        let state = Arc::new(AtomicU8::new(0));
        let h = Host::open(
            dir.path().join("h.db"),
            Device {
                inner: FileDevice::open(dir.path().join("device"), c.clone()).unwrap(),
                mode: state.clone(),
            },
            c,
            vec![binding()],
        )
        .unwrap();
        let who = caller();
        h.bind_platform(who.clone()).unwrap();
        let q = target(&h, &who);
        if mode == 5 {
            h.accept_qualification(&who, q.clone()).unwrap();
            let t = &q.cells[0];
            h.arm(
                &who,
                id(),
                &t.cell,
                t.epoch,
                &t.scopes,
                &t.required_blocks.iter().cloned().collect(),
            )
            .unwrap();
            let grant = h
                .acquire_grant(
                    &who,
                    id(),
                    vec![name("sim/controller")],
                    t.epoch,
                    Counter(10_000_000_000),
                )
                .unwrap();
            let mut op = request(&h, &grant, t.epoch.0);
            op.permit.qualification = t.qualification.clone();
            state.store(mode, Ordering::SeqCst);
            assert!(h.prepare(&who, op).is_err());
            assert!(
                FileDevice::effects(dir.path().join("device"))
                    .unwrap()
                    .is_empty()
            );
            continue;
        }
        state.store(mode, Ordering::SeqCst);
        let refused = h.accept_qualification(&who, q).unwrap();
        assert_eq!(
            refused.receipt.as_ref().unwrap().status,
            q::Status::NotAccepted
        );
        assert!(refused.accepted.is_empty() && !refused.receipt_matches_current_host);
        assert!(
            FileDevice::effects(dir.path().join("device"))
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn qualification_arm_rejects_partial_clearance_and_any_changed_cohort_member() {
    let dir = tempfile::tempdir().unwrap();
    let c = clock();
    let a = binding();
    let mut b = a.clone();
    b.cell = name("cell/other");
    b.definition.sha256 = Digest::from_bytes([53; 32]);
    let h = Host::open(
        dir.path().join("h.db"),
        FileDevice::open(dir.path().join("device"), c.clone()).unwrap(),
        c,
        vec![a, b],
    )
    .unwrap();
    let who = caller();
    h.bind_platform(who.clone()).unwrap();
    let q = target(&h, &who);
    h.accept_qualification(&who, q.clone()).unwrap();
    let t = &q.cells[0];
    let before = h.inspect_cell(&who, &t.cell).unwrap().state;
    let partial = before.blocked.iter().take(1).cloned().collect();
    assert!(before.blocked.len() > 1);
    assert!(
        h.arm(&who, id(), &t.cell, t.epoch, &t.scopes, &partial)
            .is_err()
    );
    assert_eq!(
        h.inspect_cell(&who, &t.cell).unwrap().state.blocked,
        before.blocked
    );
    let other = &q.cells[1];
    h.fence(
        &who,
        id(),
        &other.cell,
        Counter(other.epoch.0 + 1),
        other
            .scopes
            .iter()
            .map(|(k, v)| (k.clone(), Counter(v.0 + 1)))
            .collect(),
        BTreeSet::from([id()]),
    )
    .unwrap();
    assert!(
        h.arm(&who, id(), &t.cell, t.epoch, &t.scopes, &before.blocked)
            .is_err()
    );
    assert!(
        FileDevice::effects(dir.path().join("device"))
            .unwrap()
            .is_empty()
    );
}
