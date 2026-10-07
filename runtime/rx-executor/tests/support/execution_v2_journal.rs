use super::*;
use rx_domain::{canonical, definition::Reference};
use rx_process_contract::execution_v2::{self as v2, executor as wire};
fn artifact(schema: &str) -> ArtifactRef {
    ArtifactRef {
        schema_id: name(schema),
        sha256: Digest::from_bytes([7; 32]),
        size_bytes: Counter(10),
    }
}
fn reference(n: u8) -> Reference {
    Reference {
        catalog: id(50),
        id: id(n),
        revision: Counter(1),
        digest: Digest::from_bytes([8; 32]),
    }
}
fn part() -> wire::Part {
    wire::Part {
        schema: name(wire::PART_SCHEMA),
        state: rx_process_contract::production::Part {
            id: id(70),
            run: scope().run,
            ordinal: Counter(1),
            revision: Counter(1),
            disposition: rx_process_contract::execution::PartDisposition::InProgress,
        },
        binding: wire::PartBinding {
            schema: name(wire::PART_BINDING_SCHEMA),
            run: scope().run,
            part: id(70),
            ordinal: Counter(1),
            slot_ordinal: Counter(1),
            slot: 0,
            object: reference(51),
            model: reference(52),
            object_values_digest: Digest::from_bytes([9; 32]),
            candidate: 0,
            publication: reference(53),
            policy: artifact(v2::POLICY_SCHEMA),
            configuration: artifact("rx.cell-configuration.v2"),
            report: artifact("rx.execution-report.v2"),
            parameters: [(name("pick"), artifact(v2::PARAMETER_SCHEMA))].into(),
        },
    }
}
fn begin() -> Body {
    Body::BeginExecutionPart(Box::new(execution_v2::Begin {
        cell: scope().cell,
        run: scope().run,
        ordinal: Counter(1),
        mandate: id(71),
        expected_budget: Counter(1),
        expected_cell: Counter(2),
    }))
}
fn submit() -> Body {
    Body::SubmitExecutionNode(Box::new(execution_v2::Submit {
        cell: scope().cell,
        node: name("node/a"),
        mandate: id(71),
        expected_cell: Counter(2),
        expected_run: Counter(4),
        part: part().binding,
        workflow_node: name("pick"),
        host: name("host"),
        intent_digest: Digest::from_bytes([10; 32]),
        epoch: Counter(1),
    }))
}
fn admission() -> wire::Admission {
    let p = part().binding;
    let s = v2::Selection {
        schema: name("rx.execution-selection.v2"),
        publication: p.publication.id.clone(),
        policy_digest: Digest::from_bytes([11; 32]),
        configuration_digest: p.configuration.sha256,
        run: p.run,
        part: p.part,
        ordinal: p.ordinal,
        slot_ordinal: p.slot_ordinal,
        object: p.object,
        object_values_digest: p.object_values_digest,
        candidate: p.candidate,
        slot: p.slot,
        report_digest: p.report.sha256,
        node: name("pick"),
        parameter: p.parameters[&name("pick")].clone(),
        intent_digest: Digest::from_bytes([10; 32]),
        authority_generation: Counter(1),
    };
    wire::Admission {
        schema: name(wire::ADMISSION_SCHEMA),
        binding: v2::OperationBinding {
            schema: name(v2::OPERATION_SCHEMA),
            operation: id(72),
            mandate: id(71),
            publication: p.publication,
            policy: p.policy,
            report: p.report,
            selection_digest: s.digest().unwrap(),
            selection: s,
        },
        operation: rx_domain::operation::Operation::admitted(id(72), Digest::from_bytes([10; 32])),
        activation: id(73),
        permit: id(74),
        host: name("host"),
    }
}
#[test]
fn v2_requests_reopen_original_keys_and_refuse_foreign_part_or_parameter_witness() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v2.sqlite3");
    let mut j = Journal::open(SqliteRepository::open(&path).unwrap(), scope()).unwrap();
    let l = Logical {
        node: name("production/part"),
        stage: Stage::BeginPart,
        ..logical()
    };
    let e = j.prepare(l.clone(), identity(), begin(), basis(1)).unwrap();
    j.enter(&e.key).unwrap();
    let p = part();
    p.validate().unwrap();
    j.observe(Observation {
        logical: l.clone(),
        target: ObservedTarget::Part {
            value: p.state.clone(),
        },
        basis: basis(2),
    })
    .unwrap();
    let mut wrong = p.clone();
    wrong.binding.part = id(75);
    wrong.state.id = id(75);
    assert!(
        j.reply(&e.key, Response::ExecutionPart(Box::new(wrong)))
            .is_err()
    );
    j.reply(&e.key, Response::ExecutionPart(Box::new(p)))
        .unwrap();
    drop(j);
    let mut j = Journal::open(SqliteRepository::open(&path).unwrap(), scope()).unwrap();
    assert_eq!(j.get(&l).unwrap().unwrap().key, e.key);
    let l = Logical {
        stage: Stage::SubmitOperation,
        ..logical()
    };
    let e = j
        .prepare(l.clone(), identity(), submit(), basis(3))
        .unwrap();
    j.enter(&e.key).unwrap();
    let a = admission();
    a.validate().unwrap();
    for variant in 0..4 {
        let mut wrong = a.clone();
        match variant {
            0 => wrong.binding.selection.run = id(80),
            1 => wrong.binding.selection.parameter.sha256 = Digest::from_bytes([33; 32]),
            2 => wrong.binding.selection.slot = 1,
            _ => wrong.binding.mandate = id(81),
        }
        wrong.binding.selection_digest = wrong.binding.selection.digest().unwrap();
        assert!(
            j.reply(&e.key, Response::ExecutionAdmission(Box::new(wrong)))
                .is_err()
        );
    }
    j.observe(Observation {
        logical: l.clone(),
        target: ObservedTarget::Operation {
            activation: a.activation.clone(),
            operation: a.binding.operation.clone(),
            intent_digest: a.binding.selection.intent_digest,
        },
        basis: basis(4),
    })
    .unwrap();
    let mut wrong = a.clone();
    wrong.activation = id(90);
    assert!(
        j.reply(&e.key, Response::ExecutionAdmission(Box::new(wrong)))
            .is_err()
    );
    j.reply(&e.key, Response::ExecutionAdmission(Box::new(a)))
        .unwrap();
    drop(j);
    let mut j = Journal::open(SqliteRepository::open(&path).unwrap(), scope()).unwrap();
    let old = j.get(&l).unwrap().unwrap();
    assert_eq!(old.key, e.key);
    assert!(matches!(old.resolution, Resolution::Reply { .. }));
    assert_eq!(
        canonical::bytes(&j.prepare(l, identity(), submit(), basis(5)).unwrap().body).unwrap(),
        canonical::bytes(&e.body).unwrap()
    );
}

#[test]
fn v2_journal_preserves_one_request_across_precommit_and_lost_reply_faults() {
    let dir = tempfile::tempdir().unwrap();
    let mode = Arc::new(AtomicU8::new(0));
    let repository = FaultRepo {
        inner: SqliteRepository::open(dir.path().join("fault.sqlite3")).unwrap(),
        mode: mode.clone(),
    };
    let mut j = Journal::open(repository, scope()).unwrap();
    let l = Logical {
        node: name("production/part"),
        stage: Stage::BeginPart,
        ..logical()
    };
    mode.store(1, Ordering::SeqCst);
    assert!(j.prepare(l.clone(), identity(), begin(), basis(1)).is_err());
    assert!(j.get(&l).unwrap().is_none());
    mode.store(2, Ordering::SeqCst);
    assert!(j.prepare(l.clone(), identity(), begin(), basis(1)).is_err());
    let entry = j.get(&l).unwrap().unwrap();
    mode.store(1, Ordering::SeqCst);
    assert!(j.enter(&entry.key).is_err());
    assert!(matches!(
        j.get(&l).unwrap().unwrap().send,
        SendState::Prepared
    ));
    mode.store(2, Ordering::SeqCst);
    assert!(j.enter(&entry.key).is_err());
    assert!(matches!(
        j.get(&l).unwrap().unwrap().send,
        SendState::EmitEntered
    ));
    mode.store(2, Ordering::SeqCst);
    assert!(
        j.reply(&entry.key, Response::ExecutionPart(Box::new(part())))
            .is_err()
    );
    let saved = j.get(&l).unwrap().unwrap();
    assert_eq!(saved.key, entry.key);
    assert!(matches!(saved.resolution, Resolution::Reply { .. }));
    assert_eq!(
        j.prepare(l, identity(), begin(), basis(9)).unwrap().key,
        entry.key
    );
}
