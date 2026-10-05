#![cfg(target_os = "linux")]
//! Actual child/channel/capture with synthetic bindings; not qualification or Run evidence.
use rx_domain::{canonical, definition::Reference, intent::*, types::*};
use rx_host::{
    Clock, NativeAdapter,
    external_process::{self, External, profile::*},
    native::{NativeDispatch, NativeSubmission},
    service_clock::SystemClock,
};
use rx_process_contract::execution_v2::{self as v2, host_inputs::BoundInput};
use std::{
    collections::BTreeMap,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};
fn n(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn d(v: u8) -> Digest {
    Digest::from_bytes([v; 32])
}
fn artifact(schema: &str, raw: &[u8]) -> ArtifactRef {
    ArtifactRef {
        schema_id: n(schema),
        sha256: rx_package::content_digest(raw),
        size_bytes: Counter(raw.len() as u64),
    }
}
fn pin(path: &Path) -> FilePin {
    let path = path.canonicalize().unwrap();
    let raw = std::fs::read(&path).unwrap();
    FilePin {
        path,
        sha256: rx_package::content_digest(&raw),
        size_bytes: Counter(raw.len() as u64),
    }
}
#[test]
fn external_owned_entry_completion_and_passive_source_match_original_invocation() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("native");
    let session = external_process::initialize(&state).unwrap();
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../deployment/external-adapters/rx_external_adapter.py")
        .canonicalize()
        .unwrap();
    let python = Command::new("python3")
        .args([
            "-c",
            "import os,sys;print(os.path.realpath(sys.executable))",
        ])
        .output()
        .unwrap();
    assert!(python.status.success());
    let python = String::from_utf8(python.stdout).unwrap();
    let script = root.path().join("adapter.py");
    let effects = root.path().join("effect.json");
    std::fs::write(&script,format!(r#"import importlib.util,json
from pathlib import Path
spec=importlib.util.spec_from_file_location('sdk',{sdk:?})
sdk=importlib.util.module_from_spec(spec);spec.loader.exec_module(sdk)
class Adapter:
 def execute(self,envelope,correlation):
  Path({effects:?}).write_text(json.dumps({{'envelope':envelope,'correlation':correlation}}))
  return {{'status_schema':'fixture/completed','status':0}}
 def observe(self,sources): return {{s:sdk.sample({{'boolean':True}}) for s in sources}}
 def custody(self): return {{k:True for k in ('no_pending_commands','control_available','support_stable','safe_to_drop')}}
sdk.serve(Adapter())
"#,sdk=sdk.to_str().unwrap(),effects=effects.to_str().unwrap())).unwrap();
    let program = Program {
        schema: n(PROGRAM_SCHEMA),
        executable: pin(Path::new(python.trim())),
        arguments: vec![
            "-I".into(),
            "-S".into(),
            "-B".into(),
            script.to_str().unwrap().into(),
        ],
        dependencies: vec![pin(&script), pin(&sdk)],
    };
    let profile = Profile {
        schema: n(PROFILE_SCHEMA),
        protocol: n(PROTOCOL),
        program: program.reference().unwrap(),
        commands: BTreeMap::from([(
            n("echo"),
            v2::NodeContract {
                implementation: "fixture.echo".into(),
                version: "1".into(),
                primitive: n("echo"),
                parameters: BTreeMap::new(),
            },
        )]),
        observations: BTreeMap::from([(
            n("ready"),
            Observation {
                schema: n("boolean/v1"),
                unit: n("unitless"),
                value_type: ValueKind::Boolean,
                maximum_age_ns: Counter(1_000_000_000),
                maximum_uncertainty_ns: Counter(0),
            },
        )]),
        conditions: BTreeMap::from([(n("fixture/ready"), n("ready"))]),
    };
    let envelope = serde_json::json!({"schema":v2::PARAMETER_SCHEMA,"inputs":artifact("rx.execution-inputs.v2",b"{}"),"templates_digest":d(3),"candidate":0,"slot":0,"node":"echo","task":"fixture/echo","primitive":"echo","values":{},"done":{"observation":"ready","equals":{"unit":"unitless","data":{"kind":"BOOLEAN","value":true}}},"on_failure":"STOP","on_unknown":"HOLD_AND_RECONCILE"});
    let parameters = canonical::bytes(&envelope).unwrap();
    let parameter = artifact(v2::PARAMETER_SCHEMA, &parameters);
    let intent = Intent {
        kind: Kind::FiniteAction,
        target: n("fixture/echo"),
        profile_digest: profile.digest().unwrap(),
        site_config_digest: d(2),
        calibration_digests: vec![],
        resource_set: vec![n("fixture/resource")],
        execution_timeout_ms: Counter(2000),
        prepare_validity_ms: Counter(1000),
        completion_rule: n("fixture/completed"),
        cancel_rule: n("fixture/stop"),
        body: Body::Program(ProgramGoal {
            program: profile.program.clone(),
            parameter_set: parameter.clone(),
        }),
    };
    let publication = Reference {
        catalog: id(),
        id: id(),
        revision: Counter(1),
        digest: d(5),
    };
    let report = artifact("rx.execution-report.v2", b"{}");
    let operation = id();
    let invocation = id();
    let selection = v2::Selection {
        schema: n("rx.execution-selection.v2"),
        publication: publication.id.clone(),
        policy_digest: d(4),
        configuration_digest: d(6),
        run: id(),
        part: id(),
        ordinal: Counter(1),
        slot_ordinal: Counter(1),
        object: Reference {
            catalog: publication.catalog.clone(),
            id: id(),
            revision: Counter(1),
            digest: d(7),
        },
        object_values_digest: d(8),
        candidate: 0,
        slot: 0,
        report_digest: report.sha256,
        node: n("echo"),
        parameter,
        intent_digest: intent.digest().unwrap(),
        authority_generation: Counter(1),
    };
    let input = BoundInput {
        binding: v2::OperationBinding {
            schema: n(v2::OPERATION_SCHEMA),
            operation: operation.clone(),
            mandate: id(),
            publication,
            policy: artifact(v2::POLICY_SCHEMA, b"{}"),
            report,
            selection_digest: selection.digest().unwrap(),
            selection,
        },
        parameters,
    };
    let clock = SystemClock::new().unwrap();
    let mut adapter = External::open(profile, program, state, clock.clone()).unwrap();
    let samples = adapter
        .observe_sources(&n("cell/fixture"), &[n("ready")])
        .unwrap();
    assert!(matches!(samples[0].value, TypedValue::Boolean(true)));
    assert!(!effects.exists());
    let now = clock.now();
    let context = NativeDispatch {
        execution: Some(input.clone()),
        device_session: session.clone(),
        expires_at: TimePoint {
            clock_id: now.clock_id.clone(),
            ticks_ns: Counter(now.ticks_ns.0 + 1_000_000_000),
        },
    };
    let NativeSubmission::Entered(entry) = adapter
        .begin_with_context(&operation, &invocation, &intent, &context)
        .unwrap()
    else {
        panic!("entry expected")
    };
    assert_eq!(entry.operation, operation);
    assert_eq!(entry.invocation, invocation);
    entry.validate().unwrap();
    assert!(!adapter.can_handover(&intent.resource_set));
    let deadline = Instant::now() + Duration::from_secs(3);
    let completed = loop {
        let ready = adapter.completed().unwrap();
        if !ready.is_empty() {
            break ready;
        }
        assert!(Instant::now() < deadline, "completion missing");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(completed[0].invocation, invocation);
    assert_eq!(completed[0].capture.device_session, session);
    let effect: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&effects).unwrap()).unwrap();
    assert_eq!(effect["envelope"], envelope);
    assert_eq!(effect["correlation"]["operation"], operation.as_str());
    // Only the existing Host writer's post-commit acknowledgement clears this pending custody.
    assert!(!adapter.can_handover(&intent.resource_set));
    adapter.acknowledge_completion(&operation);
    assert!(adapter.can_handover(&intent.resource_set));
    let handover = adapter.handover_snapshot(&intent.resource_set).unwrap();
    assert!(handover.no_pending_commands && handover.control_available);
}
