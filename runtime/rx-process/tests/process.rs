use rx_domain::{
    condition::Condition,
    intent::*,
    operation::{Conclusion, Operation, Outcome, ReleaseConditions},
    types::*,
};
use rx_process::{frontier::*, *};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
};
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id() -> Id {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    Id::new(format!(
        "00000000-0000-4000-8000-{:012x}",
        NEXT.fetch_add(1, Ordering::SeqCst)
    ))
    .unwrap()
}
fn artifact(value: u8) -> ArtifactRef {
    ArtifactRef {
        sha256: Digest::from_bytes([value; 32]),
        schema_id: name("example/artifact.v1"),
        size_bytes: Counter(1),
    }
}
fn binding(resource: &str) -> ActionBinding {
    ActionBinding {
        host: name("host/test"),
        intent: Intent {
            kind: Kind::FiniteAction,
            target: name(resource),
            profile_digest: Digest::from_bytes([1; 32]),
            site_config_digest: Digest::from_bytes([2; 32]),
            calibration_digests: vec![],
            resource_set: vec![name(resource)],
            execution_timeout_ms: Counter(1000),
            prepare_validity_ms: Counter(1000),
            completion_rule: name("complete"),
            cancel_rule: name("stop"),
            body: Body::Program(ProgramGoal {
                program: artifact(3),
                parameter_set: artifact(4),
            }),
        },
    }
}
fn node(id: &str, body: NodeBody) -> Node {
    Node { id: name(id), body }
}
fn source(body: NodeBody) -> ProcessSource {
    ProcessSource {
        schema: name("rx.process-source.v1"),
        process: name("example/tending"),
        entry: name("main"),
        flows: vec![Flow {
            id: name("main"),
            root: name("root"),
            nodes: vec![
                node("root", body),
                node(
                    "load",
                    NodeBody::Operation {
                        binding: name("load"),
                    },
                ),
                node(
                    "close",
                    NodeBody::Operation {
                        binding: name("close"),
                    },
                ),
            ],
        }],
        conditions: BTreeMap::from([(
            name("ready"),
            Condition::Eq {
                fact: name("ready"),
                schema: name("boolean/v1"),
                unit: name("unitless"),
                expected: TypedValue::Boolean(true),
            },
        )]),
    }
}
fn bindings() -> BTreeMap<Name, ActionBinding> {
    BTreeMap::from([
        (name("load"), binding("controller/robot")),
        (name("close"), binding("controller/clamp")),
    ])
}
fn view(process: &ResolvedProcess) -> ProgressView {
    ProgressView {
        run: id(),
        resolved_digest: resolved_digest(process).unwrap(),
        complete: true,
        operations: BTreeMap::new(),
        branches: BTreeMap::new(),
        waits: BTreeMap::new(),
        cleared_interventions: BTreeMap::new(),
    }
}
fn progress(binding: &ActionBinding, success: bool, released: bool) -> OperationProgress {
    let digest = binding.intent.digest().unwrap();
    let mut operation = Operation::admitted(id(), digest);
    operation.sent().unwrap();
    operation
        .conclude(Conclusion {
            outcome: if success {
                Outcome::Succeeded
            } else {
                Outcome::Failed
            },
            evidence_ids: vec![id()],
        })
        .unwrap();
    if released {
        operation
            .release(
                ReleaseConditions {
                    no_residual_native: true,
                    control_handover_confirmed: true,
                    support_handover_confirmed: true,
                },
                vec![id()],
            )
            .unwrap();
    }
    OperationProgress {
        operation,
        intent_digest: digest,
    }
}
fn leaves(node: &CompiledNode, out: &mut Vec<Name>) {
    match &node.body {
        CompiledBody::Operation { .. } => out.push(node.id.clone()),
        CompiledBody::Sequence { children } | CompiledBody::ParallelAll { children } => {
            for child in children {
                leaves(child, out)
            }
        }
        CompiledBody::Branch {
            when_true,
            when_false,
            ..
        } => {
            leaves(when_true, out);
            leaves(when_false, out)
        }
        _ => {}
    }
}
#[test]
fn sequence_waits_for_authoritative_completion_and_resource_handover() {
    let process = compile(
        &source(NodeBody::Sequence {
            children: vec![name("load"), name("close")],
        }),
        bindings(),
    )
    .unwrap();
    let mut state = view(&process);
    let mut operations = vec![];
    leaves(&process.root, &mut operations);
    assert_eq!(
        plan(&process, &state).unwrap().operations,
        vec![operations[0].clone()]
    );
    state.operations.insert(
        operations[0].clone(),
        progress(&process.bindings[&name("load")], true, false),
    );
    let waiting = plan(&process, &state).unwrap();
    assert!(waiting.operations.is_empty());
    assert_eq!(waiting.handovers, vec![operations[0].clone()]);
    state
        .operations
        .get_mut(&operations[0])
        .unwrap()
        .operation
        .release(
            ReleaseConditions {
                no_residual_native: true,
                control_handover_confirmed: true,
                support_handover_confirmed: true,
            },
            vec![id()],
        )
        .unwrap();
    assert_eq!(
        plan(&process, &state).unwrap().operations,
        vec![operations[1].clone()]
    );
}
#[test]
fn unknown_and_disputed_results_never_become_failure_or_a_new_attempt() {
    let process = compile(
        &source(NodeBody::Sequence {
            children: vec![name("load"), name("close")],
        }),
        bindings(),
    )
    .unwrap();
    let mut state = view(&process);
    let mut operations = vec![];
    leaves(&process.root, &mut operations);
    let digest = process.bindings[&name("load")].intent.digest().unwrap();
    let mut operation = Operation::admitted(id(), digest);
    operation.lose_continuity().unwrap();
    state.operations.insert(
        operations[0].clone(),
        OperationProgress {
            operation,
            intent_digest: digest,
        },
    );
    for _ in 0..5 {
        let next = plan(&process, &state).unwrap();
        assert_eq!(next.state, State::Blocked);
        assert!(next.operations.is_empty());
    }
    state.operations.insert(
        operations[0].clone(),
        progress(&process.bindings[&name("load")], true, true),
    );
    state
        .operations
        .get_mut(&operations[0])
        .unwrap()
        .operation
        .dispute()
        .unwrap();
    assert_eq!(plan(&process, &state).unwrap().state, State::Blocked);
}
#[test]
fn branches_require_a_committed_choice_and_reject_opposite_branch_history() {
    let process = compile(
        &source(NodeBody::Branch {
            condition: name("ready"),
            when_true: name("load"),
            when_false: name("close"),
        }),
        bindings(),
    )
    .unwrap();
    let mut state = view(&process);
    assert_eq!(
        plan(&process, &state).unwrap().decisions,
        vec![process.root.id.clone()]
    );
    state.branches.insert(
        process.root.id.clone(),
        BranchChoice {
            decision: id(),
            chosen: true,
            evidence_ids: vec![id()],
        },
    );
    let mut operations = vec![];
    leaves(&process.root, &mut operations);
    assert_eq!(
        plan(&process, &state).unwrap().operations,
        vec![operations[0].clone()]
    );
    state.operations.insert(
        operations[1].clone(),
        progress(&process.bindings[&name("close")], true, true),
    );
    assert!(plan(&process, &state).is_err());
}
#[test]
fn parallel_checks_resolved_resource_conflicts_and_suppresses_new_work_on_unknown() {
    let source = source(NodeBody::ParallelAll {
        children: vec![name("load"), name("close")],
    });
    let mut conflicting = bindings();
    conflicting.insert(name("close"), binding("controller/robot"));
    assert!(compile(&source, conflicting).is_err());
    let process = compile(&source, bindings()).unwrap();
    let mut state = view(&process);
    assert_eq!(plan(&process, &state).unwrap().operations.len(), 2);
    let mut operations = vec![];
    leaves(&process.root, &mut operations);
    let digest = process.bindings[&name("load")].intent.digest().unwrap();
    let mut operation = Operation::admitted(id(), digest);
    operation.lose_continuity().unwrap();
    state.operations.insert(
        operations[0].clone(),
        OperationProgress {
            operation,
            intent_digest: digest,
        },
    );
    let next = plan(&process, &state).unwrap();
    assert_eq!(next.state, State::Blocked);
    assert!(next.operations.is_empty());
}
#[test]
fn repeat_and_subflow_expand_into_distinct_stable_occurrences() {
    let mut source = source(NodeBody::Repeat {
        count: Counter(2),
        child: name("call"),
    });
    source.flows[0].nodes.truncate(1);
    source.flows[0].nodes.push(node(
        "call",
        NodeBody::Call {
            flow: name("supply"),
        },
    ));
    source.flows.push(Flow {
        id: name("supply"),
        root: name("op"),
        nodes: vec![node(
            "op",
            NodeBody::Operation {
                binding: name("load"),
            },
        )],
    });
    let first = compile(&source, bindings()).unwrap();
    let mut operations = vec![];
    leaves(&first.root, &mut operations);
    assert_eq!(operations.len(), 2);
    assert_ne!(operations[0], operations[1]);
    source.flows.reverse();
    for flow in &mut source.flows {
        flow.nodes.reverse();
    }
    let second = compile(&source, bindings()).unwrap();
    assert_eq!(
        resolved_digest(&first).unwrap(),
        resolved_digest(&second).unwrap()
    );
    let xml = rx_process::bt_xml::generate(&first).unwrap();
    assert!(xml.contains("BTCPP_format=\"4\""));
    assert_eq!(xml.matches("<RXOperation ").count(), 2);
    assert!(!xml.contains("RetryUntilSuccessful"));
    assert!(!xml.contains("<Script"));
}
#[test]
fn malformed_or_unbounded_source_and_partial_progress_are_rejected() {
    let mut source = source(NodeBody::Sequence {
        children: vec![name("load"), name("close")],
    });
    source.flows[0].nodes.push(node(
        "load",
        NodeBody::Operation {
            binding: name("load"),
        },
    ));
    assert!(compile(&source, bindings()).is_err());
    source.flows[0].nodes.pop();
    source.flows[0].nodes[0].body = NodeBody::Repeat {
        count: Counter(0),
        child: name("load"),
    };
    assert!(compile(&source, bindings()).is_err());
    source.flows[0].nodes[0].body = NodeBody::Sequence {
        children: vec![name("load"), name("close")],
    };
    let process = compile(&source, bindings()).unwrap();
    let mut state = view(&process);
    state.complete = false;
    assert!(plan(&process, &state).is_err());
    state.complete = true;
    state.resolved_digest = Digest::from_bytes([99; 32]);
    assert!(plan(&process, &state).is_err());
}

#[test]
fn waits_and_interventions_require_recorded_decisions_without_local_timeout_success() {
    let mut source = source(NodeBody::Sequence {
        children: vec![name("wait"), name("access")],
    });
    source.flows[0].nodes.truncate(1);
    source.flows[0].nodes.push(node(
        "wait",
        NodeBody::Wait {
            condition: name("ready"),
            timeout_ns: Counter(5000),
        },
    ));
    source.flows[0].nodes.push(node(
        "access",
        NodeBody::Intervention {
            procedure: artifact(8),
        },
    ));
    let process = compile(&source, bindings()).unwrap();
    let mut state = view(&process);
    let CompiledBody::Sequence { children } = &process.root.body else {
        panic!("sequence")
    };
    assert_eq!(
        plan(&process, &state).unwrap().waits,
        vec![children[0].id.clone()]
    );
    state.waits.insert(
        children[0].id.clone(),
        WaitProgress::Satisfied {
            decision: id(),
            evidence_ids: vec![id()],
        },
    );
    let blocked = plan(&process, &state).unwrap();
    assert_eq!(blocked.state, State::Blocked);
    assert_eq!(blocked.interventions, vec![children[1].id.clone()]);
    state
        .cleared_interventions
        .insert(children[1].id.clone(), id());
    assert_eq!(plan(&process, &state).unwrap().state, State::Completed);
    state.cleared_interventions.clear();
    state.waits.insert(
        children[0].id.clone(),
        WaitProgress::TimedOut { decision: id() },
    );
    assert_eq!(plan(&process, &state).unwrap().state, State::Failed);
}

#[test]
fn incomplete_history_and_recursive_subflows_do_not_create_new_work() {
    let mut source = source(NodeBody::Sequence {
        children: vec![name("load"), name("close")],
    });
    let process = compile(&source, bindings()).unwrap();
    let mut state = view(&process);
    let mut operations = vec![];
    leaves(&process.root, &mut operations);
    state.operations.insert(
        operations[1].clone(),
        progress(&process.bindings[&name("close")], true, true),
    );
    assert!(plan(&process, &state).is_err());
    source.flows[0].nodes.truncate(1);
    source.flows[0].nodes[0].body = NodeBody::Call { flow: name("main") };
    assert!(compile(&source, bindings()).is_err());
}

#[test]
fn source_link_rejects_reordered_children_and_swapped_branch_arms_with_valid_node_ids() {
    for branching in [false, true] {
        let source = source(if branching {
            NodeBody::Branch {
                condition: name("ready"),
                when_true: name("load"),
                when_false: name("close"),
            }
        } else {
            NodeBody::Sequence {
                children: vec![name("load"), name("close")],
            }
        });
        let mut process = compile(&source, bindings()).unwrap();
        rx_process_contract::source_link::verify(&source, &process).unwrap();
        match &mut process.root.body {
            CompiledBody::Sequence { children } => children.reverse(),
            CompiledBody::Branch {
                when_true,
                when_false,
                ..
            } => std::mem::swap(when_true, when_false),
            _ => panic!("control"),
        }
        rx_process_contract::validation::validate(&process).unwrap();
        assert!(rx_process_contract::source_link::verify(&source, &process).is_err());
    }
}
#[test]
fn source_link_checks_repeat_count_and_call_instantiation() {
    let mut repeated = source(NodeBody::Repeat {
        count: Counter(2),
        child: name("load"),
    });
    repeated.flows[0].nodes.retain(|n| n.id != name("close"));
    let mut process = compile(&repeated, bindings()).unwrap();
    rx_process_contract::source_link::verify(&repeated, &process).unwrap();
    let CompiledBody::Sequence { children } = &mut process.root.body else {
        panic!("repeat")
    };
    children.pop();
    rx_process_contract::validation::validate(&process).unwrap();
    assert!(rx_process_contract::source_link::verify(&repeated, &process).is_err());
    let mut called = source(NodeBody::Call {
        flow: name("other"),
    });
    called.flows[0].nodes.retain(|n| n.id == name("root"));
    called.flows.push(Flow {
        id: name("other"),
        root: name("work"),
        nodes: vec![node(
            "work",
            NodeBody::Operation {
                binding: name("load"),
            },
        )],
    });
    let mut process = compile(&called, bindings()).unwrap();
    rx_process_contract::source_link::verify(&called, &process).unwrap();
    let CompiledBody::Sequence { children } = &mut process.root.body else {
        panic!("call")
    };
    children[0].source.instantiation = vec!["wrong-call".into()];
    children[0].id = name(&format!(
        "node/{}",
        rx_domain::canonical::digest(
            "RX-PROCESS-NODE-v1",
            &(&process.process, &children[0].source)
        )
        .unwrap()
    ));
    rx_process_contract::validation::validate(&process).unwrap();
    assert!(rx_process_contract::source_link::verify(&called, &process).is_err());
}
