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
