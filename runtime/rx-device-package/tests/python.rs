#![cfg(unix)]
use ed25519_dalek::{Signer, SigningKey};
use rx_device_package::{
    Candidate, Device, Recipe, contracts, decode_verified_any, directory, python,
};
use rx_domain::{canonical, intent::*, types::*};
use rx_host::service::python_skill::Registration;
use rx_package::*;
use std::collections::BTreeMap;
fn n(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id() -> Id {
    Id::new("00000000-0000-4000-8000-000000000001").unwrap()
}
fn artifact(raw: &[u8], schema: &str) -> ArtifactRef {
    ArtifactRef {
        sha256: content_digest(raw),
        schema_id: n(schema),
        size_bytes: Counter(raw.len() as u64),
    }
}
fn inputs() -> (Registration, Vec<u8>, Recipe) {
    // Metadata-only signing fixture; actual SDK execution has separate Host tests.
    let body = serde_json::json!({"schema":"rx.python-environment.v1","path":"/skills/prepared","python":{"platform":"linux","machine":std::env::consts::ARCH},"skill":{"name":"fixture","version":"1.0.0"}});
    let digest = content_digest(&canonical::bytes(&body).unwrap());
    let environment =
        canonical::bytes(&serde_json::json!({"environment_digest":digest,"environment":body}))
            .unwrap();
    let input = serde_json::json!({"value":5});
    let bytes = canonical::bytes(&input).unwrap();
    let registration = Registration {
        schema: n("rx.python-skill-registration.v1"),
        installation: id(),
        host: n("host/python"),
        cell: n("cell/python"),
        environment: "/skills/prepared".into(),
        environment_digest: digest,
        input,
        intent: Intent {
            kind: Kind::FiniteAction,
            target: n("device/python"),
            profile_digest: Digest::from_bytes([0; 32]),
            site_config_digest: Digest::from_bytes([1; 32]),
            calibration_digests: vec![],
            resource_set: vec![n("resource/python")],
            execution_timeout_ms: Counter(1000),
            prepare_validity_ms: Counter(500),
            completion_rule: n("rx.python.returned.v1"),
            cancel_rule: n("python/unknown"),
            body: Body::Program(ProgramGoal {
                program: artifact(&environment, "rx.python-environment.v1"),
                parameter_set: artifact(&bytes, "rx.python-input.v1"),
            }),
        },
    };
    let recipe = Recipe {
        schema: n("rx.device-package-recipe.v1"),
        package: n("test/python"),
        version: semver::Version::new(1, 0, 0),
        publisher: n("test"),
        targets: vec![Target {
            os: OperatingSystem::Linux,
            architecture: if std::env::consts::ARCH == "aarch64" {
                Architecture::Arm64
            } else {
                Architecture::Amd64
            },
            ros_distribution: None,
        }],
    };
    (registration, environment, recipe)
}
fn policy(c: &Candidate) -> (VerificationPolicy, SigningKey) {
    let key = SigningKey::from_bytes(&[71; 32]);
    let m = c.manifest();
    (
        VerificationPolicy {
            additional_package_abis: Default::default(),
            publishers: BTreeMap::from([(
                n("test/key"),
                TrustedPublisher {
                    publisher: m.publisher.clone(),
                    verifying_key: key.verifying_key().to_bytes(),
                    kinds: [PackageKind::Device].into(),
                    permissions: m.permissions.iter().cloned().collect(),
                },
            )]),
            contracts: contracts(),
            target: m.targets[0].clone(),
            assets: m.assets.iter().cloned().map(|a| (a.sha256, a)).collect(),
            dependencies: BTreeMap::new(),
            max_files: 8,
            max_content_bytes: 2 * 1024 * 1024,
        },
        key,
    )
}
fn signature(c: &Candidate, key: &SigningKey) -> SignatureEnvelope {
    SignatureEnvelope {
        key: n("test/key"),
        signature: key
            .sign(&c.signing_message(&n("test/key")).unwrap())
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    }
}
#[test]
fn python_package_roundtrip_preserves_generic_catalog_and_originals() {
    let (r, e, recipe) = inputs();
    let c = python::assemble(&r, &e, &recipe).unwrap();
    let (p, key) = policy(&c);
    let package = c.verify(&signature(&c, &key), &p).unwrap();
    let Device::Python(profile) = decode_verified_any(&package).unwrap() else {
        panic!("Python decoder not selected")
    };
    assert_ne!(profile.intent.profile_digest, r.intent.profile_digest);
    let catalog: rx_process_contract::device_catalog::Catalog = canonical::decode_json(
        package
            .file(&PackagePath::new("device-catalog.json").unwrap())
            .unwrap(),
    )
    .unwrap();
    catalog.validate().unwrap();
    assert_eq!(catalog.profile_digest, profile.intent.profile_digest);
    assert_eq!(catalog.operations.len(), 1);
    let cli_root = tempfile::tempdir().unwrap();
    let reg_file = cli_root.path().join("registration.json");
    let env_file = cli_root.path().join("environment.json");
    let recipe_file = cli_root.path().join("recipe.json");
    std::fs::write(&reg_file, canonical::bytes(&r).unwrap()).unwrap();
    std::fs::write(&env_file, &e).unwrap();
    std::fs::write(&recipe_file, canonical::bytes(&recipe).unwrap()).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rx-device-package"))
        .arg("python-assemble")
        .arg(&reg_file)
        .arg(&env_file)
        .arg(&recipe_file)
        .arg(cli_root.path().join("candidate"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["manifest_digest"], c.digest().unwrap().to_string());
    assert_eq!(value["activation_authorized"], false);
    let signed_root = cli_root.path().join("signed");
    directory::publish(&c, Some(&signature(&c, &key)), &signed_root).unwrap();
    let parameter_bytes = canonical::bytes(&r.input).unwrap();
    let assets = c
        .manifest()
        .assets
        .iter()
        .map(|a| {
            let file = cli_root.path().join(a.sha256.to_string());
            let bytes = if a.sha256 == content_digest(&e) {
                &e
            } else {
                &parameter_bytes
            };
            std::fs::write(&file, bytes).unwrap();
            rx_package::policy::Asset {
                reference: a.clone(),
                path: file,
            }
        })
        .collect();
    let mut local_policy = rx_package::policy::Policy {
        schema: n("rx.package-verification-policy.v1"),
        additional_package_abis: vec![],
        contracts: contracts(),
        target: c.manifest().targets[0].clone(),
        keys: vec![rx_package::policy::Key {
            id: n("test/key"),
            publisher: recipe.publisher.clone(),
            verifying_key: Digest::from_bytes(key.verifying_key().to_bytes()),
            kinds: [PackageKind::Device].into(),
            permissions: c.manifest().permissions.iter().cloned().collect(),
        }],
        assets,
        dependencies: vec![],
    };
    let policy_path = cli_root.path().join("policy.json");
    let raw = canonical::bytes(&local_policy).unwrap();
    std::fs::write(&policy_path, &raw).unwrap();
    let mut backend = rx_host::service::config::Backend::PythonSkillPackage {
        directory: signed_root,
        manifest_digest: c.digest().unwrap(),
        policy: rx_host::service::config::PinnedFile {
            path: policy_path.clone(),
            sha256: content_digest(&raw),
        },
    };
    assert_eq!(
        rx_host::service::python_package::load(&backend).unwrap().0,
        c.digest().unwrap()
    );
    local_policy.keys.clear();
    let raw = canonical::bytes(&local_policy).unwrap();
    std::fs::write(&policy_path, &raw).unwrap();
    if let rx_host::service::config::Backend::PythonSkillPackage { policy, .. } = &mut backend {
        policy.sha256 = content_digest(&raw);
    }
    assert!(rx_host::service::python_package::load(&backend).is_err());
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
fn signed_but_inconsistent_python_catalog_is_rejected() {
    let (r, e, recipe) = inputs();
    let c = python::assemble(&r, &e, &recipe).unwrap();
    let (p, key) = policy(&c);
    let mut files = c.files().clone();
    let path = PackagePath::new("operations.json").unwrap();
    files.insert(path.clone(), b"{}".to_vec());
    let mut manifest = c.manifest().clone();
    let entry = manifest.files.iter_mut().find(|f| f.path == path).unwrap();
    entry.sha256 = content_digest(b"{}");
    entry.size_bytes = Counter(2);
    let sig = SignatureEnvelope {
        key: n("test/key"),
        signature: key
            .sign(&signing_message(&manifest, &n("test/key")).unwrap())
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    };
    let signed = verify_package(
        &manifest_bytes(&manifest).unwrap(),
        &canonical::bytes(&sig).unwrap(),
        files,
        &p,
    )
    .unwrap();
    assert!(decode_verified_any(&signed).is_err());
}
#[test]
fn changed_input_environment_and_target_are_not_accepted() {
    let (mut r, e, mut recipe) = inputs();
    r.input = serde_json::json!({"value":6});
    assert!(python::assemble(&r, &e, &recipe).is_err());
    let (r, e, _) = inputs();
    recipe.targets[0].architecture = if recipe.targets[0].architecture == Architecture::Arm64 {
        Architecture::Amd64
    } else {
        Architecture::Arm64
    };
    assert!(python::assemble(&r, &e, &recipe).is_err());
    let mut altered: serde_json::Value = serde_json::from_slice(&e).unwrap();
    altered["environment"]["path"] = serde_json::json!("/elsewhere");
    assert!(python::assemble(&r, &canonical::bytes(&altered).unwrap(), &recipe).is_err());
}

#[test]
#[ignore = "explicit test-only external signing; no private key enters a runtime image"]
fn sign_python_fixture_message() {
    use std::io::Write;
    let input = std::env::var("RX_PYTHON_SIGN_REQUEST").unwrap();
    let output = std::path::PathBuf::from(std::env::var("RX_PYTHON_SIGN_OUTPUT").unwrap());
    let request: serde_json::Value =
        serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
    assert_eq!(request["key"], "test/key");
    let hex = request["message_hex"].as_str().unwrap();
    assert!(hex.len().is_multiple_of(2) && hex.len() < 4 * 1024 * 1024);
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect::<Vec<_>>();
    let key = SigningKey::from_bytes(&[71; 32]);
    let signature = SignatureEnvelope {
        key: n("test/key"),
        signature: key
            .sign(&bytes)
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    };
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)
        .unwrap();
    file.write_all(&canonical::bytes(&signature).unwrap())
        .unwrap();
    file.sync_all().unwrap();
    std::fs::write(output.with_extension("public.json"),canonical::bytes(&serde_json::json!({"verifying_key":Digest::from_bytes(key.verifying_key().to_bytes())})).unwrap()).unwrap();
}

#[test]
fn library_is_one_signed_package_with_complete_operations_and_original_reassembly() {
    use rx_host::service::python_library::{Entry, Library};
    let (first, environment, recipe) = inputs();
    let mut second = first.clone();
    second.input = serde_json::json!({"value": 8});
    let Body::Program(goal) = &mut second.intent.body else {
        unreachable!()
    };
    goal.parameter_set = artifact(
        &canonical::bytes(&second.input).unwrap(),
        "rx.python-input.v1",
    );
    let library = Library {
        schema: n("rx.python-skill-library.v1"),
        installation: first.installation.clone(),
        host: first.host.clone(),
        cell: first.cell.clone(),
        environment: first.environment.clone(),
        environment_digest: first.environment_digest,
        programs: BTreeMap::from([
            (
                n("first"),
                Entry {
                    input: first.input,
                    intent: first.intent,
                },
            ),
            (
                n("second"),
                Entry {
                    input: second.input,
                    intent: second.intent,
                },
            ),
        ]),
    };
    let candidate = python::assemble_library(&library, &environment, &recipe).unwrap();
    let cli = tempfile::tempdir().unwrap();
    let library_file = cli.path().join("library.json");
    let environment_file = cli.path().join("environment.json");
    let recipe_file = cli.path().join("recipe.json");
    std::fs::write(&library_file, canonical::bytes(&library).unwrap()).unwrap();
    std::fs::write(&environment_file, &environment).unwrap();
    std::fs::write(&recipe_file, canonical::bytes(&recipe).unwrap()).unwrap();
    let cli_output = cli.path().join("candidate");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_rx-device-package"))
        .arg("python-library-assemble")
        .arg(&library_file)
        .arg(&environment_file)
        .arg(&recipe_file)
        .arg(&cli_output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        directory::candidate(&cli_output).unwrap().digest().unwrap(),
        candidate.digest().unwrap()
    );

    assert_eq!(candidate.manifest().assets.len(), 3);
    let (verification, key) = policy(&candidate);
    let package = candidate
        .verify(&signature(&candidate, &key), &verification)
        .unwrap();
    let Device::PythonLibrary(profile) = decode_verified_any(&package).unwrap() else {
        panic!("library");
    };
    assert_eq!(profile.programs.len(), 2);
    assert!(
        profile
            .programs
            .values()
            .all(|entry| entry.intent.profile_digest == profile.profile_digest().unwrap())
    );
    let root = tempfile::tempdir().unwrap();
    directory::publish(&candidate, None, &root.path().join("candidate")).unwrap();
    assert_eq!(
        directory::candidate(&root.path().join("candidate"))
            .unwrap()
            .digest()
            .unwrap(),
        candidate.digest().unwrap()
    );
    let mut changed = library.clone();
    changed.programs.get_mut(&n("second")).unwrap().input = serde_json::json!({"value":9});
    assert!(python::assemble_library(&changed, &environment, &recipe).is_err());
    let mut duplicate = library.clone();
    duplicate
        .programs
        .insert(n("alias"), duplicate.programs[&n("first")].clone());
    assert!(python::assemble_library(&duplicate, &environment, &recipe).is_err());
    let mut files = candidate.files().clone();
    let file = PackagePath::new("operations.json").unwrap();
    files.insert(file.clone(), b"{}".to_vec());
    let mut manifest = candidate.manifest().clone();
    let entry = manifest
        .files
        .iter_mut()
        .find(|entry| entry.path == file)
        .unwrap();
    entry.sha256 = content_digest(b"{}");
    entry.size_bytes = Counter(2);
    let signature = SignatureEnvelope {
        key: n("test/key"),
        signature: key
            .sign(&signing_message(&manifest, &n("test/key")).unwrap())
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    };
    let inconsistent = verify_package(
        &manifest_bytes(&manifest).unwrap(),
        &canonical::bytes(&signature).unwrap(),
        files,
        &verification,
    )
    .unwrap();
    assert!(decode_verified_any(&inconsistent).is_err());
}

#[test]
fn execution_package_preserves_common_templates_and_python_only_pins() {
    use rx_host::{python_execution::Profile, service::python_execution_package::Assembly};
    use rx_process_contract::execution_v2 as v2;
    let (registration, environment, recipe) = inputs();
    let fixture: v2::Policy = canonical::decode_json(include_bytes!(
        "../../rx-host/tests/fixtures/execution-v2/policy.json"
    ))
    .unwrap();
    let Body::Program(goal) = &registration.intent.body else {
        unreachable!()
    };
    let assembly = Assembly {
        schema: n("rx.python-execution-assembly.v2"),
        profile: Profile {
            schema: n("rx.python-execution-profile.v2"),
            environment: registration.environment.clone(),
            environment_digest: registration.environment_digest,
            program: goal.program.clone(),
        },
        catalog: v2::TemplateCatalog {
            schema: n(v2::TEMPLATE_CATALOG_SCHEMA),
            installation: registration.installation.clone(),
            cell: registration.cell.clone(),
            environment: rx_process_contract::device_catalog::Environment::Simulation,
            documents: BTreeMap::new(),
            templates: BTreeMap::from([(
                n("node"),
                v2::TemplateDeclaration {
                    action: rx_process_contract::ActionBinding {
                        host: registration.host.clone(),
                        intent: registration.intent.clone(),
                    },
                    contract: fixture.node_contracts[&n("node")].clone(),
                },
            )]),
        },
    };
    let candidate = python::assemble_execution(&assembly, &environment, &recipe).unwrap();
    assert_eq!(candidate.files().len(), 8);
    let (verification, key) = policy(&candidate);
    let signed = candidate
        .verify(&signature(&candidate, &key), &verification)
        .unwrap();
    let Device::PythonExecution(checked) = decode_verified_any(&signed).unwrap() else {
        panic!("v2 profile")
    };
    assert_eq!(checked.templates.catalog().templates.len(), 1);
    assert_eq!(checked.profile.program, assembly.profile.program);
    assert_eq!(
        checked.templates.catalog().templates[&n("node")]
            .action
            .intent
            .profile_digest,
        checked.templates.catalog().documents[&n("profile")]
            .artifact
            .sha256
    );
    let profile: serde_json::Value = canonical::decode_json(
        signed
            .file(&PackagePath::new("profile.json").unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(profile.as_object().unwrap().len(), 4);
    assert!(profile.get("parameters").is_none());
    let root = tempfile::tempdir().unwrap();
    directory::publish(&candidate, None, &root.path().join("candidate")).unwrap();
    assert_eq!(
        directory::candidate(&root.path().join("candidate"))
            .unwrap()
            .digest()
            .unwrap(),
        candidate.digest().unwrap()
    );
    for (name, bytes) in [
        ("assembly.json", canonical::bytes(&assembly).unwrap()),
        ("environment.json", environment.clone()),
        ("recipe.json", canonical::bytes(&recipe).unwrap()),
    ] {
        std::fs::write(root.path().join(name), bytes).unwrap();
    }
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rx-device-package"))
        .arg("python-execution-assemble")
        .arg(root.path().join("assembly.json"))
        .arg(root.path().join("environment.json"))
        .arg(root.path().join("recipe.json"))
        .arg(root.path().join("cli-candidate"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        directory::candidate(&root.path().join("cli-candidate"))
            .unwrap()
            .digest()
            .unwrap(),
        candidate.digest().unwrap()
    );
    let mut changed = assembly.clone();
    changed.profile.program.sha256 = Digest::from_bytes([9; 32]);
    assert!(python::assemble_execution(&changed, &environment, &recipe).is_err());
    let mut changed = assembly;
    changed
        .catalog
        .templates
        .get_mut(&n("node"))
        .unwrap()
        .action
        .intent
        .completion_rule = n("unsupported");
    assert!(python::assemble_execution(&changed, &environment, &recipe).is_err());
}
