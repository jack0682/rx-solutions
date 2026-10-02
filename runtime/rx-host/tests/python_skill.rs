#![cfg(unix)]
use rx_domain::{intent::*, types::*};
use rx_host::{
    python_skill::{Program, PythonSkill, ReleasePython},
    simulation::{FileDevice, ManualClock},
    *,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    process::Command,
    sync::{Arc, atomic::AtomicU64},
};
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn hash(p: &std::path::Path) -> Digest {
    rx_package::content_digest(&std::fs::read(p).unwrap())
}
fn reference(p: &std::path::Path, schema: &str) -> ArtifactRef {
    ArtifactRef {
        sha256: hash(p),
        size_bytes: Counter(std::fs::metadata(p).unwrap().len()),
        schema_id: name(schema),
    }
}
struct Fixture {
    _directory: tempfile::TempDir,
    effects: PathBuf,
    host: Host<PythonSkill<FileDevice<ManualClock>, ManualClock>, ManualClock>,
    caller: Caller,
    binding: Binding,
    intent: Intent,
    intents: Vec<Intent>,
}
fn fixture(delay: bool) -> Fixture {
    fixture_programs(delay, 1)
}
fn fixture_programs(delay: bool, count: usize) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let script = r#"
import json,sys
from pathlib import Path
root=Path(sys.argv[1]);out=Path(sys.argv[2])
sys.path.insert(0,str(root/'tools'))
from test_python_environment import wheel
from python_environment import prepare
source=out/'source';source.mkdir()
(source/'skill.json').write_text('{"name":"sdk-call","version":"1.0.0"}')
(source/'skill.py').write_text('from rx_fixture_sdk import record\nimport time\ndef main(inputs):\n value=record(inputs["path"])\n '+('time.sleep(5)\n ' if sys.argv[3]=='delay' else '')+'return {"value":value}\n')
sdk=b'import os\ndef record(path):\n with open(path,"a") as f:\n  f.write("effect\\n");f.flush();os.fsync(f.fileno())\n return 7\n'
built=prepare(source,[wheel(out,{'rx_fixture_sdk/__init__.py':sdk})],out/'env')
input_data=json.dumps({'path':str(out/'effects')},sort_keys=True,separators=(',',':')).encode()
(out/'input.json').write_bytes(input_data)
print(json.dumps({'environment':built,'python':str(Path(sys.executable).resolve())}))
"#;
    let result = Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(root)
        .arg(directory.path())
        .arg(if delay { "delay" } else { "normal" })
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let setup: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    let executable = PathBuf::from(setup["python"].as_str().unwrap());
    let environment = directory.path().join("env");
    let input_path = directory.path().join("input.json");
    let intent = Intent {
        kind: Kind::FiniteAction,
        target: name("sim/python-sdk"),
        profile_digest: Digest::from_bytes([1; 32]),
        site_config_digest: Digest::from_bytes([2; 32]),
        calibration_digests: vec![],
        resource_set: vec![name("sim/controller")],
        execution_timeout_ms: Counter(if delay { 1000 } else { 5000 }),
        prepare_validity_ms: Counter(1000),
        completion_rule: name("rx.python.returned.v1"),
        cancel_rule: name("sim/stop"),
        body: Body::Program(ProgramGoal {
            program: reference(
                &environment.join("environment.json"),
                "rx.python-environment.v1",
            ),
            parameter_set: reference(&input_path, "rx.python-input.v1"),
        }),
    };
    let c = ManualClock {
        clock_id: "simulation/boottime".into(),
        ticks: Arc::new(AtomicU64::new(1000)),
    };
    let state = directory.path().join("python-journal");
    std::fs::create_dir(&state).unwrap();
    let runner = root.join("deployment/local-skills/host_runner.py");
    let release = ReleasePython {
        executable_digest: hash(&executable),
        executable,
        runner_digest: hash(&runner),
        verifier_digest: hash(&runner.with_file_name("python_environment.py")),
        runner,
    };
    let mut programs = vec![];
    let mut intents = vec![];
    for index in 0..count {
        let input = if index == 0 {
            serde_json::from_slice(&std::fs::read(&input_path).unwrap()).unwrap()
        } else {
            serde_json::json!({"path": directory.path().join(format!("effects-{index}"))})
        };
        let mut selected = intent.clone();
        let Body::Program(goal) = &mut selected.body else {
            unreachable!()
        };
        let bytes = rx_domain::canonical::bytes(&input).unwrap();
        goal.parameter_set.sha256 = rx_package::content_digest(&bytes);
        goal.parameter_set.size_bytes = Counter(bytes.len() as u64);
        intents.push(selected.clone());
        programs.push(Program {
            environment: environment.clone(),
            environment_digest: serde_json::from_value(
                setup["environment"]["environment_digest"].clone(),
            )
            .unwrap(),
            input,
            intent: selected,
        });
    }
    let support = FileDevice::open(directory.path().join("support"), c.clone()).unwrap();
    let adapter = if count == 1 {
        PythonSkill::open(release, programs.pop().unwrap(), state, support, c.clone())
    } else {
        PythonSkill::open_library(release, programs, state, support, c.clone())
    }
    .unwrap();
    let artifact = |n| ArtifactRef {
        sha256: Digest::from_bytes([n; 32]),
        schema_id: name("fixture/v1"),
        size_bytes: Counter(1),
    };
    let binding = Binding {
        host: name("host/sim"),
        platform: name("platform"),
        cell: name("cell/sim"),
        definition: artifact(3),
        envelope: artifact(4),
        qualification: id(),
        qualification_revision: Counter(1),
        allowed_intents: intents.clone(),
        scope_ids: vec![name("scope/main")],
        condition_ids: vec![name("sim/ready")],
        environment: Environment::Simulation,
        purposes: [Purpose::Production].into_iter().collect(),
    };
    let host = Host::open(
        directory.path().join("host.db"),
        adapter,
        c,
        vec![binding.clone()],
    )
    .unwrap();
    let caller = Caller {
        peer: name("platform"),
        session: id(),
    };
    let effects = directory.path().join("effects");
    Fixture {
        _directory: directory,
        effects,
        host,
        caller,
        binding,
        intent,
        intents,
    }
}
fn request(f: &Fixture) -> Request {
    request_for(f, &f.intent, 1)
}
fn request_for(f: &Fixture, intent: &Intent, fence: u64) -> Request {
    let scopes = BTreeMap::from([(name("scope/main"), Counter(1))]);
    f.host.bind_platform(f.caller.clone()).unwrap();
    f.host
        .arm(
            &f.caller,
            id(),
            &f.binding.cell,
            Counter(1),
            &scopes,
            &BTreeSet::new(),
        )
        .unwrap();
    let grant = f
        .host
        .acquire_grant(
            &f.caller,
            id(),
            intent.resource_set.clone(),
            Counter(fence),
            Counter(10_000_000_000),
        )
        .unwrap();
    let operation = id();
    let digest = intent.digest().unwrap();
    Request {
        operation: operation.clone(),
        intent: intent.clone(),
        digest,
        grant: grant.id.clone(),
        permit: Permit {
            id: id(),
            operation,
            digest,
            cell: f.binding.cell.clone(),
            epoch: Counter(1),
            scopes,
            envelope: f.binding.envelope.sha256,
            qualification: f.binding.qualification.clone(),
            qualification_revision: Counter(1),
            grant: grant.id,
            host_boot: f.host.boot_id().unwrap(),
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
#[test]
fn sdk_runs_only_after_host_authorization_and_duplicates_are_receipt_reads() {
    let f = fixture(false);
    let request = request(&f);
    let prepared = f.host.prepare(&f.caller, request.clone()).unwrap();
    assert!(!f.effects.exists());
    let mut foreign = f.caller.clone();
    foreign.session = id();
    assert!(
        f.host
            .authorize(
                &foreign,
                request.clone(),
                prepared.invocation.as_ref().unwrap()
            )
            .is_err()
    );
    assert!(!f.effects.exists());
    let result = f
        .host
        .authorize(
            &f.caller,
            request.clone(),
            prepared.invocation.as_ref().unwrap(),
        )
        .unwrap();
    assert_eq!(result.state, ReceiptState::ResultCaptured);
    let repeated = f
        .host
        .authorize(
            &f.caller,
            request.clone(),
            prepared.invocation.as_ref().unwrap(),
        )
        .unwrap();
    assert_eq!(result.journal_seq, repeated.journal_seq);
    assert_eq!(
        f.host
            .reconcile(&f.caller, &request.operation)
            .unwrap()
            .state,
        ReceiptState::ResultCaptured
    );
    assert_eq!(std::fs::read_to_string(&f.effects).unwrap(), "effect\n");
    assert!(
        !f.host
            .handover_observations(&f.caller, &request.operation)
            .unwrap()
            .is_empty()
    );
}
#[test]
fn sdk_timeout_keeps_send_entered_and_refuses_handover_without_reexecution() {
    let f = fixture(true);
    let request = request(&f);
    let prepared = f.host.prepare(&f.caller, request.clone()).unwrap();
    assert!(
        f.host
            .authorize(
                &f.caller,
                request.clone(),
                prepared.invocation.as_ref().unwrap()
            )
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(&f.effects).unwrap(), "effect\n");
    assert_eq!(
        f.host
            .reconcile(&f.caller, &request.operation)
            .unwrap()
            .state,
        ReceiptState::SendEntered
    );
    assert!(
        f.host
            .handover_observations(&f.caller, &request.operation)
            .is_err()
    );
    f.host
        .authorize(&f.caller, request, prepared.invocation.as_ref().unwrap())
        .unwrap();
    assert_eq!(std::fs::read_to_string(&f.effects).unwrap(), "effect\n");
}

#[test]
fn altered_python_environment_is_refused_before_sdk_effect() {
    let f = fixture(false);
    let request = request(&f);
    let prepared = f.host.prepare(&f.caller, request.clone()).unwrap();
    std::fs::write(
        f._directory.path().join("env/skill.py"),
        "def main(inputs): return {}\n",
    )
    .unwrap();
    assert!(
        f.host
            .authorize(&f.caller, request, prepared.invocation.as_ref().unwrap())
            .is_err()
    );
    assert!(!f.effects.exists());
}

#[test]
fn library_dispatches_distinct_pinned_inputs_and_preserves_original_lookup() {
    let f = fixture_programs(false, 2);
    let first = request_for(&f, &f.intents[0], 1);
    let pending = f.host.prepare(&f.caller, first.clone()).unwrap();
    assert!(!f.effects.exists());
    f.host
        .authorize(
            &f.caller,
            first.clone(),
            pending.invocation.as_ref().unwrap(),
        )
        .unwrap();
    let second = request_for(&f, &f.intents[1], 2);
    let pending = f.host.prepare(&f.caller, second.clone()).unwrap();
    f.host
        .authorize(
            &f.caller,
            second.clone(),
            pending.invocation.as_ref().unwrap(),
        )
        .unwrap();
    assert_eq!(std::fs::read_to_string(&f.effects).unwrap(), "effect\n");
    assert_eq!(
        std::fs::read_to_string(f._directory.path().join("effects-1")).unwrap(),
        "effect\n"
    );
    assert_eq!(
        f.host.reconcile(&f.caller, &first.operation).unwrap().state,
        ReceiptState::ResultCaptured
    );
    assert_eq!(
        f.host
            .reconcile(&f.caller, &second.operation)
            .unwrap()
            .state,
        ReceiptState::ResultCaptured
    );
    assert_eq!(std::fs::read_to_string(&f.effects).unwrap(), "effect\n");
}
#[test]
fn one_uncertain_library_program_blocks_the_shared_resource_for_other_programs() {
    let f = fixture_programs(true, 2);
    let first = request_for(&f, &f.intents[0], 1);
    let pending = f.host.prepare(&f.caller, first.clone()).unwrap();
    assert!(
        f.host
            .authorize(
                &f.caller,
                first.clone(),
                pending.invocation.as_ref().unwrap()
            )
            .is_err()
    );
    assert!(
        f.host
            .acquire_grant(
                &f.caller,
                id(),
                f.intents[1].resource_set.clone(),
                Counter(2),
                Counter(10_000_000_000)
            )
            .is_err()
    );
    assert!(!f._directory.path().join("effects-1").exists());
    assert!(
        f.host
            .handover_observations(&f.caller, &first.operation)
            .is_err()
    );
}
