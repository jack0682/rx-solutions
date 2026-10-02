#![cfg(unix)]
//! Shared model -> compiler -> real authorized Python Host -> independent file-device effects.
//! This test does not stand in for P enrollment, publication or an N-part runtime binding.
use rx_domain::{
    canonical,
    definition::{Definition, Reference},
    intent::*,
    types::*,
    workflow,
};
use rx_host::{
    python_skill::{Program, PythonSkill, ReleasePython},
    simulation::{FileDevice, ManualClock},
    *,
};
use rx_process::workflow::{Input, Parameter, Template};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, atomic::AtomicU64},
};
fn n(v: &str) -> Name {
    Name::new(v).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn read(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}
fn artifact(bytes: &[u8], schema: &str) -> ArtifactRef {
    ArtifactRef {
        sha256: rx_package::content_digest(bytes),
        schema_id: n(schema),
        size_bytes: Counter(bytes.len() as u64),
    }
}
fn pin(path: &Path) -> Digest {
    rx_package::content_digest(&std::fs::read(path).unwrap())
}
fn bind(value: Value, refs: &BTreeMap<String, Reference>) -> Value {
    match value {
        Value::Object(v) if v.len() == 1 && v.contains_key("$ref") => {
            serde_json::to_value(&refs[v["$ref"].as_str().unwrap()]).unwrap()
        }
        Value::Object(v) => Value::Object(v.into_iter().map(|(k, v)| (k, bind(v, refs))).collect()),
        Value::Array(v) => Value::Array(v.into_iter().map(|v| bind(v, refs)).collect()),
        value => value,
    }
}
fn report(root: &Path, part: &str) -> (workflow::Report, Reference) {
    let data = root.join("examples/definitions/0f-laser-simulation");
    let mut refs = BTreeMap::new();
    let mut all = BTreeMap::new();
    for file in ["cell.json", "m2-definitions.json"] {
        let package = read(&data.join(file));
        let catalog: Id = serde_json::from_value(package["catalog"].clone()).unwrap();
        for item in package["definitions"].as_array().unwrap() {
            let definition = Definition::new(
                catalog.clone(),
                serde_json::from_value(item["id"].clone()).unwrap(),
                Counter(1),
                item["label"].as_str().unwrap().into(),
                serde_json::from_value(bind(item["body"].clone(), &refs)).unwrap(),
            )
            .unwrap();
            refs.insert(
                item["key"].as_str().unwrap().to_owned(),
                definition.reference.clone(),
            );
            all.insert(definition.reference.clone(), definition);
        }
    }
    let package = read(&data.join("workflow.json"));
    let spec: workflow::Spec =
        serde_json::from_value(bind(package["spec"].clone(), &refs)).unwrap();
    let catalog: Id = serde_json::from_value(package["catalog"].clone()).unwrap();
    let model: Id = serde_json::from_value(package["id"].clone()).unwrap();
    let label = package["label"].as_str().unwrap();
    let reference = Reference {
        catalog: catalog.clone(),
        id: model.clone(),
        revision: Counter(1),
        digest: canonical::digest(
            "RX-WORKFLOW-MODEL-v1",
            &(&catalog, &model, Counter(1), label, &spec),
        )
        .unwrap(),
    };
    let request = workflow::Request {
        workflow: reference,
        contexts: BTreeMap::from([(n("part"), vec![refs[part].clone()])]),
        property_sets: vec![],
        overrides: BTreeMap::new(),
        inputs: BTreeMap::new(),
        slot_index: Counter(0),
    };
    let report = workflow::resolve(&spec, request, &all).unwrap();
    assert!(report.valid && report.concrete, "{:?}", report.violations);
    let receipt = Reference {
        catalog,
        id: id(),
        revision: Counter(1),
        digest: canonical::digest("RX-WORKFLOW-RESOLUTION-v1", &report).unwrap(),
    };
    (report, receipt)
}
fn run_scene(part: &str, expected_force: f64, expected_width: f64, duration: f64) {
    let temp = tempfile::tempdir().unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let prepare = r#"
import json,sys
from pathlib import Path
root=Path(sys.argv[1]); target=Path(sys.argv[2]);sys.path.insert(0,str(root/'deployment/local-skills'))
from python_environment import prepare
source=target/'source';source.mkdir();device=target/'device';device.mkdir()
original=root/'examples/process/laser-heat-treatment/simulation'
spec=json.loads((original/'skill.json').read_text());spec['simulation']['state_directory']=str(device)
(source/'skill.json').write_text(json.dumps(spec));(source/'skill.py').write_bytes((original/'skill.py').read_bytes())
built=prepare(source,[],target/'environment')
print(json.dumps({'python':str(Path(sys.executable).resolve()),'digest':built['environment_digest']}))
"#;
    let output = Command::new("python3")
        .args(["-c", prepare])
        .arg(root)
        .arg(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let ready: Value = serde_json::from_slice(&output.stdout).unwrap();
    let python = PathBuf::from(ready["python"].as_str().unwrap());
    let environment = temp.path().join("environment");
    let program = artifact(
        &std::fs::read(environment.join("environment.json")).unwrap(),
        "rx.python-environment.v1",
    );
    let (report, reference) = report(root, part);
    let mut templates = BTreeMap::new();
    for step in &report.steps {
        let skill = &step.skills[0];
        let workflow::Data::Text {
            value: implementation,
        } = &skill.implementation.value.data
        else {
            unreachable!()
        };
        let workflow::Data::Text { value: version } = &skill.version.value.data else {
            unreachable!()
        };
        let parameters = skill
            .parameters
            .iter()
            .map(|(parameter, property)| {
                let v = &step.properties[property];
                let kind = match v.value.data {
                    workflow::Data::Number { .. } => rx_domain::definition::ValueType::Number,
                    workflow::Data::Vector { .. } => rx_domain::definition::ValueType::Vector,
                    workflow::Data::Boolean { .. } => rx_domain::definition::ValueType::Boolean,
                    workflow::Data::Text { .. } => rx_domain::definition::ValueType::Text,
                };
                (
                    parameter.clone(),
                    Parameter {
                        unit: v.value.unit.clone(),
                        value_type: kind,
                        frame: v.frame.clone(),
                    },
                )
            })
            .collect();
        templates.insert(
            (
                implementation.clone(),
                version.clone(),
                skill.primitive.clone(),
            ),
            Template {
                implementation: implementation.clone(),
                version: version.clone(),
                primitive: skill.primitive.clone(),
                host: n("host/simulation"),
                parameters,
                intent: Intent {
                    kind: Kind::FiniteAction,
                    target: n("device/tending-simulation"),
                    profile_digest: Digest::from_bytes([1; 32]),
                    site_config_digest: Digest::from_bytes([2; 32]),
                    calibration_digests: vec![],
                    resource_set: vec![n("cell/simulation")],
                    execution_timeout_ms: Counter(30000),
                    prepare_validity_ms: Counter(1000),
                    completion_rule: n("rx.python.returned.v1"),
                    cancel_rule: n("sim/stop"),
                    body: Body::Program(ProgramGoal {
                        program: program.clone(),
                        parameter_set: program.clone(),
                    }),
                },
            },
        );
    }
    let compiled = rx_process::workflow::compile(&Input {
        schema: n("rx.workflow-compile-input.v1"),
        process: n("test/tending"),
        resolution: reference.clone(),
        report,
        templates: templates.into_values().collect(),
    })
    .unwrap();
    let intents: Vec<_> = compiled
        .trace
        .iter()
        .map(|trace| compiled.process.bindings[&trace.binding].intent.clone())
        .collect();
    let programs = compiled
        .trace
        .iter()
        .zip(&intents)
        .map(|(trace, intent)| Program {
            environment: environment.clone(),
            environment_digest: serde_json::from_value(ready["digest"].clone()).unwrap(),
            input: serde_json::from_slice(&compiled.assets[&trace.parameters.sha256]).unwrap(),
            intent: intent.clone(),
        })
        .collect();
    let clock = ManualClock {
        clock_id: "simulation/clock".into(),
        ticks: Arc::new(AtomicU64::new(1000)),
    };
    let journal = temp.path().join("python-journal");
    std::fs::create_dir(&journal).unwrap();
    let runner = root.join("deployment/local-skills/host_runner.py");
    let adapter = PythonSkill::open_library(
        ReleasePython {
            executable_digest: pin(&python),
            executable: python,
            runner_digest: pin(&runner),
            verifier_digest: pin(&runner.with_file_name("python_environment.py")),
            runner,
        },
        programs,
        journal,
        FileDevice::open(temp.path().join("support"), clock.clone()).unwrap(),
        clock.clone(),
    )
    .unwrap();
    let binding = Binding {
        host: n("host/simulation"),
        platform: n("platform"),
        cell: n("cell/simulation"),
        definition: program.clone(),
        envelope: program,
        qualification: id(),
        qualification_revision: Counter(1),
        allowed_intents: intents.clone(),
        scope_ids: vec![n("scope/cell")],
        condition_ids: vec![n("sim/ready")],
        environment: Environment::Simulation,
        purposes: [Purpose::Production].into_iter().collect(),
    };
    let host = Host::open(
        temp.path().join("host.db"),
        adapter,
        clock,
        vec![binding.clone()],
    )
    .unwrap();
    let caller = Caller {
        peer: n("platform"),
        session: id(),
    };
    host.bind_platform(caller.clone()).unwrap();
    let scopes = BTreeMap::from([(n("scope/cell"), Counter(1))]);
    host.arm(
        &caller,
        id(),
        &binding.cell,
        Counter(1),
        &scopes,
        &BTreeSet::new(),
    )
    .unwrap();
    let mut original = vec![];
    for (index, intent) in intents.into_iter().enumerate() {
        let grant = host
            .acquire_grant(
                &caller,
                id(),
                intent.resource_set.clone(),
                Counter(index as u64 + 1),
                Counter(60_000_000_000),
            )
            .unwrap();
        let operation = id();
        let digest = intent.digest().unwrap();
        let request = Request {
            operation: operation.clone(),
            intent,
            digest,
            grant: grant.id.clone(),
            permit: Permit {
                id: id(),
                operation: operation.clone(),
                digest,
                cell: binding.cell.clone(),
                epoch: Counter(1),
                scopes: scopes.clone(),
                envelope: binding.envelope.sha256,
                qualification: binding.qualification.clone(),
                qualification_revision: Counter(1),
                grant: grant.id,
                host_boot: host.boot_id().unwrap(),
                conditions: [n("sim/ready")].into_iter().collect(),
                expires_at: TimePoint {
                    clock_id: "simulation/clock".into(),
                    ticks_ns: Counter(10_000_000_000),
                },
                source_digest: None,
                purpose: Purpose::Production,
                parent: PermitParent::Mandate(id()),
            },
        };
        let prepared = host.prepare(&caller, request.clone()).unwrap();
        let effects_path = temp.path().join("device/effects.jsonl");
        let before_count = if effects_path.exists() {
            std::fs::read_to_string(&effects_path)
                .unwrap()
                .lines()
                .count()
        } else {
            0
        };
        assert_eq!(
            before_count, index,
            "prepare must not execute the next primitive"
        );

        let result = host
            .authorize(
                &caller,
                request.clone(),
                prepared.invocation.as_ref().unwrap(),
            )
            .unwrap();
        assert_eq!(result.state, ReceiptState::ResultCaptured);
        assert!(
            !host
                .handover_observations(&caller, &operation)
                .unwrap()
                .is_empty()
        );
        original.push((request, prepared.invocation.unwrap()));
    }
    let device = temp.path().join("device");
    let effects: Vec<Value> = std::fs::read_to_string(device.join("effects.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(effects.len(), 8);
    assert_eq!(
        effects
            .iter()
            .map(|e| e["node"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "pick",
            "load",
            "clamp",
            "close-door",
            "process",
            "open-door",
            "unload",
            "place"
        ]
    );
    assert_eq!(
        effects[0]["parameters"]["force"]["value"]["data"]["range"]["min"].as_f64(),
        Some(expected_force)
    );
    assert_eq!(
        effects[0]["parameters"]["width"]["value"]["data"]["range"]["min"].as_f64(),
        Some(expected_width)
    );
    let contact_z = if part == "part.ECC_51-14" {
        760.0
    } else {
        767.5
    };
    assert_eq!(
        effects[0]["device"]["motion"]["target"][2].as_f64(),
        Some(contact_z)
    );
    assert_eq!(
        effects[0]["device"]["motion"]["force"].as_f64(),
        Some(expected_force)
    );
    assert_eq!(effects[2]["device"]["clamped"], true);
    assert_eq!(effects[2]["device"]["holding"], false);
    assert_eq!(effects[3]["device"]["door_closed"], true);
    assert_eq!(effects[4]["device"]["process_done"], true);
    assert_eq!(effects[5]["device"]["door_closed"], false);
    assert_eq!(effects[6]["device"]["holding"], true);
    assert_eq!(effects[7]["device"]["holding"], false);
    assert_eq!(
        effects[7]["device"]["motion"]["target"][0].as_f64(),
        Some(800.0)
    );
    assert!(
        effects
            .iter()
            .all(|effect| effect["resolution"] == serde_json::to_value(&reference).unwrap())
    );
    let state = read(&device.join("state.json"));
    assert_eq!(state["phase"], "SUPPLY");
    assert_eq!(state["completed_slots"], json!([0]));
    assert!(state["observed_duration_s"].as_f64().unwrap() >= duration);
    assert_eq!(state["effects"], 8);
    for (request, invocation) in original {
        assert_eq!(
            host.authorize(&caller, request, &invocation).unwrap().state,
            ReceiptState::ResultCaptured
        );
    }
    assert_eq!(
        std::fs::read_to_string(device.join("effects.jsonl"))
            .unwrap()
            .lines()
            .count(),
        8
    );
}
#[test]
fn actual_host_library_consumes_part_a_parameters_and_waits_for_simulated_done() {
    run_scene("part.ECC_51-14", 25.0, 47.0, 5.0);
}
#[test]
fn same_simulation_code_consumes_changed_part_b_parameters_without_code_edits() {
    run_scene("part.ECC_99-14", 17.5, 77.0, 7.0);
}
