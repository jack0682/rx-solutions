//! Executes real compiler/frontier code with synthetic operation inputs, never a P/Host client.
use rx_domain::{
    canonical,
    operation::{Conclusion, Disposition, Knowledge, Operation, Outcome, ReleaseConditions},
    types::{Id, Name},
};
use rx_process::{frontier::*, *};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/process/n1-handover")
}
fn inputs() -> (ProcessSource, BTreeMap<Name, ActionBinding>) {
    let read = |name| std::fs::read(fixture().join(name)).unwrap();
    (
        canonical::decode_json(&read("source.json")).unwrap(),
        canonical::decode_json(&read("bindings.json")).unwrap(),
    )
}
fn id(n: usize) -> Id {
    Id::new(format!("00000000-0000-4000-8000-{n:012x}")).unwrap()
}
fn view(p: &ResolvedProcess) -> ProgressView {
    ProgressView {
        run: id(1),
        resolved_digest: resolved_digest(p).unwrap(),
        complete: true, // Complete synthetic snapshot; not a completed Run.
        operations: BTreeMap::new(),
        branches: BTreeMap::new(),
        waits: BTreeMap::new(),
        cleared_interventions: BTreeMap::new(),
    }
}
fn leaves(p: &ResolvedProcess) -> &[CompiledNode] {
    let CompiledBody::Sequence { children } = &p.root.body else {
        panic!("N1 command prefix must be sequential")
    };
    children
}
fn admitted(p: &ResolvedProcess, node: &CompiledNode, n: usize) -> OperationProgress {
    let CompiledBody::Operation { binding } = &node.body else {
        panic!("expected a command operation")
    };
    let digest = p.bindings[binding].intent.digest().unwrap();
    OperationProgress {
        operation: Operation::admitted(id(n), digest),
        intent_digest: digest,
    }
}
fn snapshot(p: &ResolvedProcess, v: &ProgressView, step: &str) -> Value {
    json!({"step":step,"input_origin":"SYNTHETIC_NOT_A_VALIDATED_P_SNAPSHOT",
        "progress":v,"observed_frontier":plan(p,v).unwrap()})
}
fn success(op: &mut Operation, evidence: usize) {
    op.conclude(Conclusion {
        outcome: Outcome::Succeeded,
        evidence_ids: vec![id(evidence)],
    })
    .unwrap();
}
fn release(op: &mut Operation, evidence: usize) {
    // Assumed command-handover evidence only. No sensor/MaterialState claim is established.
    op.release(
        ReleaseConditions {
            no_residual_native: true,
            control_handover_confirmed: true,
            support_handover_confirmed: true,
        },
        vec![id(evidence)],
    )
    .unwrap();
}

#[test]
fn n1_command_handover_and_lost_response_trace() {
    let (source, bindings) = inputs();
    let p = compile(&source, bindings).unwrap();
    let mut v = view(&p);
    let nodes = leaves(&p);
    let mut normal = vec![snapshot(&p, &v, "command-prefix-candidate")];
    let mut before_release = None;
    for (i, node) in nodes.iter().enumerate() {
        let ready = plan(&p, &v).unwrap();
        assert_eq!(ready.operations, vec![node.id.clone()]);
        if node.source.node.as_str() == "sender-release" {
            before_release = Some(v.clone());
        }
        v.operations
            .insert(node.id.clone(), admitted(&p, node, 10 + i));
        normal.push(snapshot(&p, &v, "synthetic-operation-admitted"));
        v.operations
            .get_mut(&node.id)
            .unwrap()
            .operation
            .sent()
            .unwrap();
        normal.push(snapshot(&p, &v, "synthetic-send-entered"));
        success(
            &mut v.operations.get_mut(&node.id).unwrap().operation,
            100 + i,
        );
        let held = plan(&p, &v).unwrap();
        assert_eq!(held.state, State::Running);
        assert_eq!(held.handovers, vec![node.id.clone()]);
        assert!(
            held.operations.is_empty(),
            "success alone must not admit the successor"
        );
        normal.push(snapshot(
            &p,
            &v,
            "synthetic-succeeded-command-resource-held",
        ));
        release(
            &mut v.operations.get_mut(&node.id).unwrap().operation,
            200 + i,
        );
        normal.push(snapshot(&p, &v, "synthetic-command-resource-released"));
    }
    assert_eq!(plan(&p, &v).unwrap().state, State::Completed);
    // Completed is only the command frontier. This test supplies neither retained occupancy
    // observations nor ordinary receipt evidence, and cannot conclude N1 service completion.
    let mut lost = before_release.expect("sender-release is present");
    let node = nodes
        .iter()
        .find(|n| n.source.node.as_str() == "sender-release")
        .unwrap();
    let mut progress = admitted(&p, node, 12);
    progress.operation.sent().unwrap();
    lost.operations.insert(node.id.clone(), progress);
    let mut uncertain = vec![snapshot(&p, &lost, "synthetic-release-send-entered")];
    lost.operations
        .get_mut(&node.id)
        .unwrap()
        .operation
        .lose_continuity()
        .unwrap();
    let saved = canonical::bytes(&lost).unwrap();
    for _ in 0..10 {
        let blocked = plan(&p, &lost).unwrap();
        assert_eq!(blocked.state, State::Blocked);
        assert!(
            blocked.operations.is_empty(),
            "unknown release must not admit withdraw or retry"
        );
        assert_eq!(blocked.pending_operations, vec![id(12)]);
        assert_eq!(canonical::bytes(&lost).unwrap(), saved);
    }
    let original = &lost.operations[&node.id].operation;
    assert_eq!(original.outcome(), Outcome::None);
    assert_eq!(original.knowledge(), Knowledge::Unknown);
    assert_eq!(original.disposition(), Disposition::Quarantined);
    uncertain.push(snapshot(
        &p,
        &lost,
        "synthetic-response-loss-after-ten-planning-reads",
    ));
    lost.operations
        .get_mut(&node.id)
        .unwrap()
        .operation
        .conclude(Conclusion {
            outcome: Outcome::Unresolved,
            evidence_ids: vec![id(300)],
        })
        .unwrap(); // Assumed responsible investigation/abandonment; never a timeout decision.
    let unresolved = plan(&p, &lost).unwrap();
    assert_eq!(unresolved.state, State::Blocked);
    assert!(unresolved.operations.is_empty());
    assert_eq!(unresolved.pending_operations, vec![id(12)]);
    uncertain.push(snapshot(
        &p,
        &lost,
        "synthetic-investigation-concludes-unresolved",
    ));
    let boundary: Value =
        serde_json::from_slice(&std::fs::read(fixture().join("boundary.json")).unwrap()).unwrap();
    let trace = json!({"schema":"rx.example-n1-planning-trace.v1",
        "scope":"REAL_COMPILER_AND_FRONTIER_WITH_SYNTHETIC_OPERATION_INPUTS",
        "p_or_host_execution":false,"source_digest":p.source_digest,
        "resolved_digest":resolved_digest(&p).unwrap(),"command_path":normal,
        "lost_response":uncertain,"outside_graph":boundary["outside_graph"]});
    if let Some(path) = std::env::var_os("N1_TRACE_OUTPUT") {
        std::fs::write(path, serde_json::to_string_pretty(&trace).unwrap() + "\n").unwrap();
    }
}

#[test]
fn n1_cross_host_support_collision_is_not_hidden_by_controller_names() {
    let (mut source, mut bindings) = inputs();
    let flow = &mut source.flows[0];
    let names = [
        Name::new("receiver-hold").unwrap(),
        Name::new("sender-release").unwrap(),
    ];
    flow.nodes
        .retain(|n| n.id == flow.root || names.contains(&n.id));
    flow.nodes[0].body = NodeBody::ParallelAll {
        children: names.to_vec(),
    };
    for name in &names {
        let b = bindings.get_mut(name).unwrap();
        // Isolate the shared support token from BAY; targets and Hosts stay distinct.
        b.intent
            .resource_set
            .retain(|r| r == &b.intent.target || r.as_str() == "PKG-01/HG-01");
    }
    assert_ne!(bindings[&names[0]].host, bindings[&names[1]].host);
    assert!(
        compile(&source, bindings.clone())
            .unwrap_err()
            .reason
            .contains("parallel branches share")
    );
    for name in &names {
        bindings
            .get_mut(name)
            .unwrap()
            .intent
            .resource_set
            .retain(|r| r.as_str() != "PKG-01/HG-01");
    }
    // Negative control: the compiler cannot discover an omitted physical alias/resource.
    // This intentionally incomplete graph is compilable, not safe or eligible for dispatch.
    assert!(compile(&source, bindings).is_ok());
}
