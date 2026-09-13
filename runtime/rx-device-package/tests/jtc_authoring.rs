use ed25519_dalek::{Signer, SigningKey};
use rx_device_package::directory;
use rx_device_package::{
    jtc::{ActionSlot, Site, Template},
    *,
};
use rx_domain::{canonical, types::*};
use rx_host::{
    Environment,
    ros_jtc::protocol::{Goal, Point, Tolerance},
    service::{self, AdapterFactory},
};
use rx_package::*;
use std::{collections::BTreeMap, path::Path};
#[path = "../../rx-host/tests/support/host_service_fixture.rs"]
#[allow(dead_code, unused_imports)]
mod host_fixture;
fn n(v: &str) -> Name {
    Name::new(v).unwrap()
}
fn d(v: u8) -> Digest {
    Digest::from_bytes([v; 32])
}
fn id(v: u8) -> Id {
    Id::new(format!("00000000-0000-4000-8000-{v:012}")).unwrap()
}
const CALIBRATION: &[u8] = b"TEST ONLY robot calibration; no hardware measurements";
const TOOL: &[u8] = b"TEST ONLY tool definition; no physical suitability";
fn asset(data: &[u8], schema: &str) -> ArtifactRef {
    ArtifactRef {
        sha256: content_digest(data),
        size_bytes: Counter(data.len() as u64),
        schema_id: n(schema),
    }
}
fn write(p: &Path, v: &impl serde::Serialize) {
    std::fs::write(p, canonical::bytes(v).unwrap()).unwrap();
}
fn inputs() -> (Template, Site, Recipe) {
    let t = Template {
        schema: n("rx.ros-jtc-template.v1"),
        id: n("test/sim-arm"),
        revision: Counter(1),
        catalog_sha256: rx_host::ros_jtc::protocol::catalog_digest(),
        support_id: n("SIM-JTC-6DOF"),
        controller: "arm_controller".into(),
        resource_roles: [n("controller")].into(),
        condition_roles: [n("ready")].into(),
        actions: vec![ActionSlot {
            id: n("supply"),
            joint_group: n("arm"),
            tool_role: n("gripper"),
            execution_timeout_ms: Counter(1000),
            prepare_validity_ms: Counter(1000),
        }],
        authority_max_age_ms: Counter(100),
    };
    let config = rx_host::ros_jtc::protocol::Configuration {
        schema: n("rx.ros-jtc-bridge.v1"),
        catalog_sha256: t.catalog_sha256,
        support_id: t.support_id.clone(),
        controller: t.controller.clone(),
        namespace: "/robot".into(),
        controller_manager: "/robot/controller_manager".into(),
        domain_id: 171,
        timeout_ms: 100,
        capacity: 32,
    };
    let joints = config.joints().unwrap();
    let tolerance = joints
        .iter()
        .map(|j| Tolerance {
            name: j.clone(),
            position: 0.1,
            velocity: 0.1,
            acceleration: 0.1,
        })
        .collect::<Vec<_>>();
    let goal = Goal {
        joints: joints.clone(),
        points: vec![Point {
            positions: vec![0.1; joints.len()],
            velocities: vec![],
            accelerations: vec![],
            time_ns: Counter(100_000_000),
        }],
        path_tolerance: tolerance.clone(),
        goal_tolerance: tolerance,
        goal_time_ns: Counter(100_000_000),
    };
    let s = Site {
        schema: n("rx.ros-jtc-site.v1"),
        template_digest: t.digest().unwrap(),
        installation: id(1),
        cell: n("cell/one"),
        target: n("robot/arm"),
        site_config: d(1),
        environment: Environment::Simulation,
        calibrations: vec![asset(CALIBRATION, "rx.robot-calibration.v1")],
        tools: [(n("gripper"), asset(TOOL, "rx.tool-definition.v1"))].into(),
        resources: [(n("controller"), n("robot/controller"))].into(),
        conditions: [(n("ready"), n("robot/ready"))].into(),
        namespace: config.namespace,
        controller_manager: config.controller_manager,
        domain_id: 171,
        timeout_ms: 100,
        capacity: 32,
        goals: [(n("supply"), goal)].into(),
    };
    let r = Recipe {
        schema: n("rx.device-package-recipe.v1"),
        package: n("test/jtc"),
        version: semver::Version::new(1, 0, 0),
        publisher: n("test/publisher"),
        targets: vec![Target {
            os: OperatingSystem::Linux,
            architecture: if std::env::consts::ARCH == "aarch64" {
                Architecture::Arm64
            } else {
                Architecture::Amd64
            },
            ros_distribution: Some(n("jazzy")),
        }],
    };
    (t, s, r)
}
fn key() -> SigningKey {
    SigningKey::from_bytes(&[51; 32])
}
fn policy(c: &Candidate) -> VerificationPolicy {
    VerificationPolicy {
        additional_package_abis: Default::default(),
        contracts: contracts(),
        target: c.manifest().targets[0].clone(),
        dependencies: BTreeMap::new(),
        assets: c
            .manifest()
            .assets
            .iter()
            .map(|a| (a.sha256, a.clone()))
            .collect(),
        max_files: 8,
        max_content_bytes: 2 * 1024 * 1024,
        publishers: [(
            n("test/key"),
            TrustedPublisher {
                publisher: c.manifest().publisher.clone(),
                verifying_key: key().verifying_key().to_bytes(),
                kinds: [PackageKind::Device].into(),
                permissions: c.manifest().permissions.iter().cloned().collect(),
            },
        )]
        .into(),
    }
}
fn signature(m: &Manifest) -> SignatureEnvelope {
    SignatureEnvelope {
        key: n("test/key"),
        signature: key()
            .sign(&rx_package::signing_message(m, &n("test/key")).unwrap())
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    }
}
fn published(dir: &Path) -> (Candidate, service::config::Backend) {
    let (t, s, r) = inputs();
    let c = jtc::assemble(&t, &s, &r).unwrap();
    let sig = signature(c.manifest());
    directory::publish(&c, Some(&sig), &dir.join("package")).unwrap();
    std::fs::write(dir.join("calibration.bin"), CALIBRATION).unwrap();
    std::fs::write(dir.join("tool.bin"), TOOL).unwrap();
    let p = policy(&c);
    let doc = rx_package::policy::Policy {
        additional_package_abis: Default::default(),
        schema: n("rx.package-verification-policy.v1"),
        contracts: p.contracts,
        target: p.target,
        dependencies: vec![],
        keys: vec![rx_package::policy::Key {
            id: n("test/key"),
            publisher: r.publisher,
            verifying_key: Digest::from_bytes(key().verifying_key().to_bytes()),
            kinds: [PackageKind::Device].into(),
            permissions: c.manifest().permissions.iter().cloned().collect(),
        }],
        assets: vec![
            rx_package::policy::Asset {
                reference: s.calibrations[0].clone(),
                path: dir.join("calibration.bin"),
            },
            rx_package::policy::Asset {
                reference: s.tools[&n("gripper")].clone(),
                path: dir.join("tool.bin"),
            },
        ],
    };
    write(&dir.join("policy.json"), &doc);
    let b = service::config::Backend::JtcPackage {
        directory: dir.join("package"),
        manifest_digest: c.digest().unwrap(),
        policy: service::config::PinnedFile {
            path: dir.join("policy.json"),
            sha256: content_digest(&canonical::bytes(&doc).unwrap()),
        },
    };
    (c, b)
}
#[test]
fn template_reuse_and_candidate_roundtrip_preserve_bound_actions_and_result_meanings() {
    let (t, s, r) = inputs();
    let c = jtc::assemble(&t, &s, &r).unwrap();
    let verified = c.verify(&signature(c.manifest()), &policy(&c)).unwrap();
    let Device::Jtc(device) = decode_verified_any(&verified).unwrap() else {
        panic!()
    };
    assert_eq!(
        device.operations[&n("supply")].profile_digest,
        device.profile.digest().unwrap()
    );
    assert_eq!(
        device.outcomes.profile_digest,
        device.profile.digest().unwrap()
    );
    let mut other = s.clone();
    other.cell = n("cell/two");
    other.installation = id(2);
    other.namespace = "/second".into();
    assert_ne!(
        c.digest().unwrap(),
        jtc::assemble(&t, &other, &r).unwrap().digest().unwrap()
    );
    let root = tempfile::tempdir().unwrap();
    directory::publish(&c, None, &root.path().join("candidate")).unwrap();
    assert_eq!(
        directory::candidate(&root.path().join("candidate"))
            .unwrap()
            .digest()
            .unwrap(),
        c.digest().unwrap()
    );
}
#[test]
fn signed_but_inconsistent_results_actions_model_and_release_are_rejected() {
    let (t, s, r) = inputs();
    let c = jtc::assemble(&t, &s, &r).unwrap();
    for (file, variant) in [
        ("outcomes.json", 0),
        ("operations.json", 1),
        ("family.json", 2),
        ("adapter.json", 3),
    ] {
        let mut files = c.files().clone();
        let path = PackagePath::new(file).unwrap();
        let mut value: serde_json::Value = canonical::decode_json(&files[&path]).unwrap();
        match variant {
            0 => value["cases"][0]["conclusion"] = serde_json::json!("FAILED"),
            1 => value["supply"]["execution_timeout_ms"] = serde_json::json!("2000"),
            2 => value["model"] = serde_json::json!("another/model"),
            _ => value["source_digest"] = serde_json::to_value(d(99)).unwrap(),
        };
        files.insert(path.clone(), canonical::bytes(&value).unwrap());
        let mut manifest = c.manifest().clone();
        let entry = manifest.files.iter_mut().find(|f| f.path == path).unwrap();
        entry.sha256 = content_digest(&files[&path]);
        entry.size_bytes = Counter(files[&path].len() as u64);
        let verified = verify_package(
            &manifest_bytes(&manifest).unwrap(),
            &canonical::bytes(&signature(&manifest)).unwrap(),
            files,
            &policy(&c),
        )
        .unwrap();
        assert!(decode_verified_any(&verified).is_err(), "{file}");
    }
}
#[test]
fn missing_alias_or_foreign_bindings_and_invalid_trajectory_are_rejected() {
    let (t, s, r) = inputs();
    for variant in 0..8 {
        let mut site = s.clone();
        match variant {
            0 => site.template_digest = d(99),
            1 => {
                site.goals.clear();
            }
            2 => {
                site.tools.clear();
            }
            3 => {
                site.conditions.clear();
            }
            4 => {
                site.resources.insert(n("extra"), n("robot/controller"));
            }
            5 => site.goals.get_mut(&n("supply")).unwrap().joints.reverse(),
            6 => {
                site.calibrations.clear();
            }
            _ => {
                site.goals.get_mut(&n("supply")).unwrap().points[0].time_ns = Counter(2_000_000_000)
            }
        }
        assert!(jtc::assemble(&t, &site, &r).is_err());
    }
    let mut wrong = t.clone();
    wrong.catalog_sha256 = d(88);
    assert!(wrong.digest().is_err());
    let mut wrong = r;
    wrong.targets[0].ros_distribution = None;
    assert!(jtc::assemble(&t, &s, &wrong).is_err());
}
#[test]
fn simulation_catalog_cannot_be_promoted_to_a_physical_site() {
    let (template, mut site, recipe) = inputs();
    site.environment = Environment::Physical;
    assert!(jtc::assemble(&template, &site, &recipe).is_err());
}
#[test]
fn host_package_checks_assets_and_exact_intents_but_cannot_create_control_authority() {
    let root = tempfile::tempdir().unwrap();
    let (_, backend) = published(root.path());
    let device = service::jtc_package::load(&backend).unwrap();
    let p = &device.profile;
    let mut bindings = vec![rx_host::Binding {
        host: n("host/jtc"),
        platform: n("platform"),
        cell: p.cell.clone(),
        definition: asset(b"cell", "rx.cell-definition.v1"),
        envelope: asset(b"envelope", "rx.operating-envelope.v1"),
        qualification: id(4),
        qualification_revision: Counter(1),
        allowed_intents: device.operations.values().cloned().collect(),
        scope_ids: vec![n("scope/jtc")],
        condition_ids: p.conditions.iter().cloned().collect(),
        environment: Environment::Simulation,
        purposes: [rx_host::Purpose::Production].into(),
    }];
    device.validate_bindings(&bindings).unwrap();
    bindings[0].allowed_intents[0].execution_timeout_ms = Counter(2000);
    assert!(device.validate_bindings(&bindings).is_err());
    type C = rx_host::simulation::ManualClock;
    let stage = root.path().join("stage");
    std::fs::create_dir(&stage).unwrap();
    let metadata = <service::Builtin as AdapterFactory<C>>::initialize_metadata(
        &service::Builtin,
        &backend,
        &stage,
    )
    .unwrap();
    assert!(matches!(
        metadata,
        Some(service::NativeInstallation::Jtc { .. })
    ));
    assert!(stage.join("native-jtc/native.sqlite3").is_file());
    let c = C {
        clock_id: "test/clock".into(),
        ticks: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1)),
    };
    let err = match <service::Builtin as AdapterFactory<C>>::open_passive(
        &service::Builtin,
        &backend,
        &stage,
        c,
    ) {
        Ok(_) => panic!("unexpected provider"),
        Err(e) => e,
    };
    assert!(
        err.to_string()
            .contains("JTC_CONTROL_PROVIDER_NOT_CONFIGURED")
    );
    std::fs::write(root.path().join("tool.bin"), b"changed").unwrap();
    assert!(service::jtc_package::load(&backend).is_err());
}
#[test]
fn actual_cli_assembles_requests_external_signature_seals_and_inspects_jtc() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path();
    let (t, s, r) = inputs();
    write(&dir.join("template.json"), &t);
    write(&dir.join("site.json"), &s);
    write(&dir.join("recipe.json"), &r);
    let run = |args: &[&str]| {
        let o = std::process::Command::new(env!("CARGO_BIN_EXE_rx-device-package"))
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        serde_json::from_slice::<serde_json::Value>(&o.stdout).unwrap()
    };
    run(&["template-digest", "template.json"]);
    run(&[
        "assemble",
        "template.json",
        "site.json",
        "recipe.json",
        "candidate",
    ]);
    run(&["request", "candidate", "test/key", "request.json"]);
    let req: serde_json::Value =
        canonical::decode_json(&std::fs::read(dir.join("request.json")).unwrap()).unwrap();
    let c = directory::candidate(&dir.join("candidate")).unwrap();
    let expected = c
        .signing_message(&n("test/key"))
        .unwrap()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    assert_eq!(req["message_hex"], expected);
    write(&dir.join("signature.json"), &signature(c.manifest()));
    let _ = published(dir);
    run(&[
        "seal",
        "candidate",
        "signature.json",
        "policy.json",
        "sealed",
    ]);
    let result = run(&["inspect", "sealed", "policy.json"]);
    assert_eq!(result["control_provider"], "NOT_CONFIGURED");
    assert_eq!(result["activation_authorized"], false);
    assert_eq!(
        result["outcomes"]["profile_digest"],
        result["profile_digest"]
    );
    assert_eq!(
        result["operations"]["supply"]["profile_digest"],
        result["profile_digest"]
    );
}

#[test]
fn unordered_template_actions_have_one_digest_and_joint_order_is_preserved() {
    let (mut t, mut s, r) = inputs();
    let mut action = t.actions[0].clone();
    action.id = n("return");
    t.actions.push(action);
    let mut goal = s.goals[&n("supply")].clone();
    goal.points[0].positions[0] = 0.2;
    s.goals.insert(n("return"), goal);
    s.template_digest = t.digest().unwrap();
    let a = jtc::assemble(&t, &s, &r).unwrap();
    t.actions.reverse();
    s.calibrations.reverse();
    let b = jtc::assemble(&t, &s, &r).unwrap();
    assert_eq!(a.digest().unwrap(), b.digest().unwrap());
    assert_eq!(
        a.signing_message(&n("test/key")).unwrap(),
        b.signing_message(&n("test/key")).unwrap()
    );
}
#[tokio::test]
async fn host_initializes_jtc_metadata_and_refuses_run_before_creating_a_controller_client() {
    let (root, file, clock) = host_fixture::fixture();
    let (_, backend) = published(root.path());
    let device = service::jtc_package::load(&backend).unwrap();
    let mut loaded = service::config::Loaded::read(&file).unwrap();
    loaded.config.installation = device.profile.installation.clone();
    loaded.config.backend = backend;
    let b = &mut loaded.bindings[0];
    b.cell = device.profile.cell.clone();
    b.allowed_intents = device.operations.values().cloned().collect();
    b.condition_ids = device.profile.conditions.iter().cloned().collect();
    loaded.config.bindings = host_fixture::pinned(
        root.path(),
        "bindings.json",
        &canonical::bytes(&loaded.bindings).unwrap(),
        false,
    );
    write(&file, &loaded.config);
    let loaded = service::config::Loaded::read(&file).unwrap();
    let inspection = service::inspect(&loaded).unwrap();
    assert_eq!(inspection["control_provider"], "NOT_CONFIGURED");
    assert_eq!(inspection["native_processes_started"], 0);
    service::initialize_with(&loaded, clock.clone(), &service::Builtin).unwrap();
    let metadata = std::fs::read(loaded.config.data_directory.join("installation.json")).unwrap();
    let value: serde_json::Value = canonical::decode_json(&metadata).unwrap();
    assert_eq!(value["native"]["kind"], "JTC");
    assert!(service::initialize_with(&loaded, clock.clone(), &service::Builtin).is_err());
    let status = loaded.config.runtime_directory.join("host-status.json");
    let result = service::run_with(
        loaded,
        clock,
        service::Builtin,
        std::future::pending::<()>(),
    )
    .await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("JTC_CONTROL_PROVIDER_NOT_CONFIGURED")
    );
    assert!(!status.exists());
}
#[test]
#[ignore = "exports only SIMULATION inputs and a public test policy for the image authoring test"]
fn export_image_fixture() {
    let out = std::path::PathBuf::from(std::env::var("RX_DEVICE_TOOL_FIXTURE").unwrap());
    assert!(!out.exists());
    std::fs::create_dir(&out).unwrap();
    let (t, s, r) = inputs();
    write(&out.join("template.json"), &t);
    write(&out.join("site.json"), &s);
    write(&out.join("recipe.json"), &r);
    let _ = published(&out);
    let mut p: rx_package::policy::Policy =
        canonical::decode_json(&std::fs::read(out.join("policy.json")).unwrap()).unwrap();
    for a in &mut p.assets {
        a.path = Path::new("/data/fixture").join(a.path.file_name().unwrap());
    }
    write(&out.join("policy.json"), &p);
    let (host_root, host_file, _) = host_fixture::fixture();
    let mut loaded = service::config::Loaded::read(&host_file).unwrap();
    let resolved = jtc::Assembly {
        schema: n("rx.ros-jtc-assembly.v1"),
        template: t,
        site: s.clone(),
    }
    .resolve()
    .unwrap();
    loaded.config.installation = s.installation;
    loaded.config.backend = service::config::Backend::JtcPackage {
        directory: "/data/package".into(),
        manifest_digest: d(0),
        policy: service::config::PinnedFile {
            path: "/data/fixture/policy.json".into(),
            sha256: content_digest(&canonical::bytes(&p).unwrap()),
        },
    };
    let b = &mut loaded.bindings[0];
    b.cell = s.cell;
    b.allowed_intents = resolved.operations.into_values().collect();
    b.condition_ids = resolved.profile.conditions.into_iter().collect();
    write(&out.join("bindings.json"), &loaded.bindings);
    loaded.config.bindings = service::config::PinnedFile {
        path: "/data/fixture/bindings.json".into(),
        sha256: content_digest(&canonical::bytes(&loaded.bindings).unwrap()),
    };
    for file in [
        &mut loaded.config.tls.certificate,
        &mut loaded.config.tls.key,
        &mut loaded.config.tls.ca,
    ] {
        let label = file.path.file_name().unwrap();
        std::fs::copy(&file.path, out.join(label)).unwrap();
        file.path = Path::new("/data/fixture").join(label);
    }
    loaded.config.data_directory = "/data/host-data".into();
    loaded.config.runtime_directory = "/data/runtime".into();
    write(&out.join("host-startup.json"), &loaded.config);
    drop(host_root);
}
#[test]
#[ignore = "test-only external signer for the isolated image fixture"]
fn sign_image_request() {
    let input = std::path::PathBuf::from(std::env::var("RX_DEVICE_TOOL_REQUEST").unwrap());
    let out = std::path::PathBuf::from(std::env::var("RX_DEVICE_TOOL_SIGNATURE").unwrap());
    assert!(!out.exists());
    let request: serde_json::Value =
        canonical::decode_json(&std::fs::read(input).unwrap()).unwrap();
    assert_eq!(request["schema"], "rx.package-signing-request.v1");
    assert_eq!(request["key"], "test/key");
    let text = request["message_hex"].as_str().unwrap();
    assert!(text.len().is_multiple_of(2));
    let bytes = text
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        serde_json::to_value(content_digest(&bytes)).unwrap(),
        request["message_digest"]
    );
    let signature = SignatureEnvelope {
        key: n("test/key"),
        signature: key()
            .sign(&bytes)
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    };
    write(&out, &signature);
}

fn review_request(c: &Candidate) -> rx_process_contract::device_review::Request {
    let data = &c.files()[&PackagePath::new("device-catalog.json").unwrap()];
    let catalog: rx_process_contract::device_catalog::Catalog =
        canonical::decode_json(data).unwrap();
    rx_process_contract::device_review::Request {
        schema: n("rx.device-review-request.v1"),
        id: id(31),
        intake: id(32),
        installation: catalog.installation,
        cell: catalog.cell,
        package_manifest: c.digest().unwrap(),
        package_signature: content_digest(&canonical::bytes(&signature(c.manifest())).unwrap()),
        catalog: ArtifactRef {
            sha256: content_digest(data),
            schema_id: n("rx.device-operation-catalog.v1"),
            size_bytes: Counter(data.len() as u64),
        },
        configuration_digest: d(33),
        package_policy_fingerprint: policy(c).fingerprint().unwrap(),
        package_policy_file_digest: d(34),
        verification_authority_digest: d(35),
    }
}
#[test]
fn device_software_report_uses_real_decoder_and_cannot_claim_physical_validation() {
    let (t, s, r) = inputs();
    let c = jtc::assemble(&t, &s, &r).unwrap();
    let p = policy(&c);
    let package = c.verify(&signature(c.manifest()), &p).unwrap();
    let request = review_request(&c);
    let report = review::verify(request.clone(), &package, &p, d(40)).unwrap();
    assert!(report.passed());
    assert_eq!(report.checks.len(), 3);
    let mut value = serde_json::to_value(&report).unwrap();
    value["scope"] = serde_json::json!("PHYSICAL_QUALIFICATION");
    assert!(serde_json::from_value::<rx_process_contract::device_review::Report>(value).is_err());
    let mut missing = report.clone();
    missing
        .checks
        .remove(&rx_process_contract::device_review::Check::DeviceSourceConsistency);
    assert!(missing.validate().is_err());
    let mut wrong = request;
    wrong.catalog.sha256 = d(99);
    assert!(review::verify(wrong, &package, &p, d(40)).is_err());
    let mut files = c.files().clone();
    let path = PackagePath::new("authoring/assembly.json").unwrap();
    let mut assembly: serde_json::Value = canonical::decode_json(&files[&path]).unwrap();
    assembly["site"]["site_config"] = serde_json::to_value(d(90)).unwrap();
    files.insert(path.clone(), canonical::bytes(&assembly).unwrap());
    let mut m = c.manifest().clone();
    let f = m.files.iter_mut().find(|v| v.path == path).unwrap();
    f.sha256 = content_digest(&files[&path]);
    f.size_bytes = Counter(files[&path].len() as u64);
    let sig = signature(&m);
    let package = verify_package(
        &manifest_bytes(&m).unwrap(),
        &canonical::bytes(&sig).unwrap(),
        files,
        &p,
    )
    .unwrap();
    let mut req = review_request(&c);
    req.package_manifest = package.digest();
    req.package_signature = content_digest(&canonical::bytes(&sig).unwrap());
    let failed = review::verify(req, &package, &p, d(40)).unwrap();
    assert!(!failed.passed());
    assert_eq!(
        failed.checks[&rx_process_contract::device_review::Check::DeviceSourceConsistency],
        rx_process_contract::device_review::ResultKind::Failed
    );
    assert!(!failed.issues.is_empty());
}
#[test]
fn actual_cli_generates_a_report_and_a_separate_signing_request() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path();
    let (c, _) = published(dir);
    let mut request = review_request(&c);
    let source: rx_package::policy::Policy =
        rx_package::policy::read(&dir.join("policy.json")).unwrap();
    request.package_policy_fingerprint = source_policy(&source).unwrap().fingerprint().unwrap();
    write(&dir.join("review-request.json"), &request);
    let call = |args: &[&str]| {
        let o = std::process::Command::new(env!("CARGO_BIN_EXE_rx-device-package"))
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        serde_json::from_slice::<serde_json::Value>(&o.stdout).unwrap()
    };
    let result = call(&[
        "review",
        "package",
        "policy.json",
        "review-request.json",
        "report",
    ]);
    assert_eq!(result["software_checks_passed"], true);
    assert_eq!(result["physical_validation"], "NOT_PERFORMED");
    assert_eq!(result["activation_authorized"], false);
    call(&[
        "review-signing-request",
        "report/verification.json",
        "test/device-verifier",
        "report-signing.json",
    ]);
    let report: rx_process_contract::device_review::Report =
        canonical::decode_json(&std::fs::read(dir.join("report/verification.json")).unwrap())
            .unwrap();
    let data: serde_json::Value =
        canonical::decode_json(&std::fs::read(dir.join("report-signing.json")).unwrap()).unwrap();
    assert_eq!(
        data["message_hex"],
        report
            .signing_message(&n("test/device-verifier"))
            .unwrap()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    assert!(!dir.join("report/verification.sig.json").exists());
}
#[test]
#[ignore = "external test-only device report signer for real API integration"]
fn sign_device_report() {
    let file = std::path::PathBuf::from(std::env::var("RX_DEVICE_REVIEW_REPORT").unwrap());
    let report: rx_process_contract::device_review::Report =
        canonical::decode_json(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(report.request.cell.as_str(), "cell/demo");
    let key = SigningKey::from_bytes(&[65; 32]);
    let id = n("test/device-verifier");
    let signature = SignatureEnvelope {
        key: id.clone(),
        signature: key
            .sign(&report.signing_message(&id).unwrap())
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    };
    write(
        &file.parent().unwrap().join("verification.sig.json"),
        &signature,
    );
}
#[test]
#[ignore = "exports a public device verifier trust fixture, never a production key"]
fn export_device_review_authority() {
    let path = std::path::PathBuf::from(std::env::var("RX_DEVICE_REVIEW_AUTHORITY").unwrap());
    assert!(!path.exists());
    let key = SigningKey::from_bytes(&[65; 32]);
    write(
        &path,
        &serde_json::json!({"schema":"rx.device-verification-authority.v1","keys":[{"id":"test/device-verifier","public_key":Digest::from_bytes(key.verifying_key().to_bytes()),"validators":[review::validator_digest()]}]}),
    );
}
