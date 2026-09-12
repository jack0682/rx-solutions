use ed25519_dalek::{Signer, SigningKey};
use rx_domain::{canonical, intent::*, types::*};
use rx_package::*;
use rx_process_contract::{compile_input::CompileInput, model::ActionBinding};
use rx_process_package::{Recipe, assemble, compile_verified, directory, trust};
use std::collections::{BTreeMap, BTreeSet};
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn fixture() -> (CompileInput, Recipe, SigningKey, trust::Policy) {
    let source = serde_json::json!({"schema":"rx.process-source.v1","process":"test/package","entry":"main","conditions":{},"flows":[{"id":"main","root":"work","nodes":[{"id":"work","body":{"kind":"OPERATION","binding":"load"}}]}]});
    let action = ActionBinding {
        host: name("host/sim"),
        intent: Intent {
            kind: Kind::EnsureState,
            target: name("device/sim"),
            profile_digest: Digest::from_bytes([3; 32]),
            site_config_digest: Digest::from_bytes([4; 32]),
            calibration_digests: vec![],
            resource_set: vec![name("controller/sim")],
            execution_timeout_ms: Counter(1000),
            prepare_validity_ms: Counter(500),
            completion_rule: name("sim/done"),
            cancel_rule: name("sim/cancel"),
            body: Body::Predicate(PredicateGoal {
                predicate_id: name("loaded"),
                target: TypedValue::Boolean(true),
                settle_ms: Counter(0),
            }),
        },
    };
    let bindings = BTreeMap::from([(name("load"), action)]);
    let input = CompileInput {
        device_sources: BTreeMap::new(),
        schema: name("rx.process-compile-input.v1"),
        draft: Id::new("00000000-0000-4000-8000-000000000001").unwrap(),
        cell: name("cell/sim"),
        source_revision: Counter(2),
        binding_revision: Counter(1),
        source_document_digest: canonical::digest("RX-PROCESS-DRAFT-DOCUMENT-v1", &source).unwrap(),
        bindings_digest: canonical::digest("RX-DRAFT-COMPILE-BINDINGS-v1", &bindings).unwrap(),
        catalog_digest: Digest::from_bytes([5; 32]),
        source,
        bindings,
    };
    let contracts = ContractSet {
        base: Digest::from_bytes([1; 32]),
        cell: Digest::from_bytes([2; 32]),
        package_abi: name("rx.package-abi.v1"),
    };
    let target = Target {
        os: OperatingSystem::Linux,
        architecture: Architecture::Arm64,
        ros_distribution: None,
    };
    let recipe = Recipe {
        schema: name("rx.process-package-recipe.v1"),
        package: name("test/package"),
        version: "0.1.0".parse().unwrap(),
        publisher: name("test-only"),
        contracts: contracts.clone(),
        targets: vec![target.clone()],
        dependencies: vec![],
        assets: vec![],
    };
    let key = SigningKey::from_bytes(&[19; 32]);
    let policy = trust::Policy {
        additional_package_abis: Default::default(),
        schema: name("rx.package-verification-policy.v1"),
        contracts,
        target,
        keys: vec![trust::Key {
            id: name("test-key"),
            publisher: recipe.publisher.clone(),
            verifying_key: Digest::from_bytes(key.verifying_key().to_bytes()),
            kinds: [PackageKind::Process].into_iter().collect(),
            permissions: [
                Permission::ArtifactRead,
                Permission::OperationSubmit {
                    operation: name("load"),
                },
            ]
            .into_iter()
            .collect(),
        }],
        assets: vec![],
        dependencies: vec![],
    };
    (input, recipe, key, policy)
}
fn signature(candidate: &rx_process_package::Candidate, key: &SigningKey) -> SignatureEnvelope {
    SignatureEnvelope {
        key: name("test-key"),
        signature: key
            .sign(&candidate.signing_message(&name("test-key")).unwrap())
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    }
}
#[test]
fn deterministic_candidate_detached_signature_and_recompilation_preserve_input_identity() {
    let (input, recipe, key, policy) = fixture();
    let a = assemble(&input, &recipe).unwrap();
    let b = assemble(&input, &recipe).unwrap();
    assert_eq!(a.digest().unwrap(), b.digest().unwrap());
    assert_eq!(a.files(), b.files());
    assert_eq!(
        a.manifest()
            .permissions
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>(),
        policy.keys[0].permissions
    );
    let signed = signature(&a, &key);
    let verified = a.verify(&signed, &policy.load().unwrap()).unwrap();
    let compiled = compile_verified(&verified).unwrap();
    assert_eq!(compiled.package_digest, Some(verified.digest()));
    assert_eq!(compiled.bindings.len(), 1);
    assert!(a.manifest().files.iter().all(|f| !f.executable));
}
#[test]
fn signer_scope_key_alias_and_content_changes_do_not_gain_trust() {
    let (input, recipe, key, mut policy) = fixture();
    let candidate = assemble(&input, &recipe).unwrap();
    let mut signed = signature(&candidate, &key);
    let mut alias = policy.keys[0].clone();
    alias.id = name("alias");
    policy.keys.push(alias);
    signed.key = name("alias");
    assert!(candidate.verify(&signed, &policy.load().unwrap()).is_err());
    signed.key = name("test-key");
    policy.keys[0].permissions.clear();
    assert!(candidate.verify(&signed, &policy.load().unwrap()).is_err());
    let mut files = candidate.files().clone();
    files.insert(
        PackagePath::new("manifest.json").unwrap(),
        manifest_bytes(candidate.manifest()).unwrap(),
    );
    files
        .get_mut(&PackagePath::new(rx_process_package::SOURCE).unwrap())
        .unwrap()
        .push(b' ');
    assert!(rx_process_package::from_files(files).is_err());
}
#[test]
fn candidate_publication_does_not_overwrite_and_signed_directory_has_exact_inventory() {
    let (input, recipe, key, policy) = fixture();
    let candidate = assemble(&input, &recipe).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let unsigned = dir.path().join("candidate");
    directory::publish(&candidate, None, &unsigned).unwrap();
    let loaded = directory::candidate(&unsigned).unwrap();
    assert_eq!(candidate.digest().unwrap(), loaded.digest().unwrap());
    assert!(directory::publish(&candidate, None, &unsigned).is_err());
    let signed = dir.path().join("signed");
    directory::publish(&candidate, Some(&signature(&candidate, &key)), &signed).unwrap();
    let verified =
        rx_package::directory::verify_directory(&signed, &policy.load().unwrap()).unwrap();
    compile_verified(&verified).unwrap();
    std::fs::write(signed.join("extra.txt"), b"undeclared").unwrap();
    assert!(rx_package::directory::verify_directory(&signed, &policy.load().unwrap()).is_err());
}
#[test]
fn missing_program_artifacts_and_parallel_resource_conflicts_block_assembly() {
    let (mut input, mut recipe, _, _) = fixture();
    let artifact = ArtifactRef {
        sha256: Digest::from_bytes([8; 32]),
        schema_id: name("program/v1"),
        size_bytes: Counter(1),
    };
    input.bindings.get_mut(&name("load")).unwrap().intent.kind = Kind::FiniteAction;
    input.bindings.get_mut(&name("load")).unwrap().intent.body = Body::Program(ProgramGoal {
        program: artifact.clone(),
        parameter_set: artifact.clone(),
    });
    input.bindings_digest =
        canonical::digest("RX-DRAFT-COMPILE-BINDINGS-v1", &input.bindings).unwrap();
    assert!(assemble(&input, &recipe).is_err());
    recipe.assets.push(artifact);
    assert!(assemble(&input, &recipe).is_ok());
    input.source["flows"][0]["root"] = serde_json::json!("parallel");
    input.source["flows"][0]["nodes"].as_array_mut().unwrap().extend([serde_json::json!({"id":"second","body":{"kind":"OPERATION","binding":"load"}}),serde_json::json!({"id":"parallel","body":{"kind":"PARALLEL_ALL","children":["work","second"]}})]);
    input.source_document_digest =
        canonical::digest("RX-PROCESS-DRAFT-DOCUMENT-v1", &input.source).unwrap();
    assert!(assemble(&input, &recipe).is_err());
}
#[test]
fn policy_assets_are_read_and_dependency_files_are_verified_under_current_trust() {
    let (input, recipe, key, mut policy) = fixture();
    let candidate = assemble(&input, &recipe).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("dependency");
    rx_process_package::directory::publish(&candidate, Some(&signature(&candidate, &key)), &path)
        .unwrap();
    policy.dependencies.push(trust::DependencyInput {
        path,
        manifest_digest: candidate.digest().unwrap(),
    });
    assert_eq!(policy.load().unwrap().dependencies.len(), 1);
    policy.keys[0].kinds.clear();
    assert!(policy.load().is_err());
    policy.dependencies.clear();
    let file = directory.path().join("asset");
    std::fs::write(&file, b"asset").unwrap();
    policy.assets.push(trust::Asset {
        path: file.clone(),
        reference: ArtifactRef {
            sha256: content_digest(b"asset"),
            schema_id: name("test/asset"),
            size_bytes: Counter(5),
        },
    });
    assert!(policy.load().is_ok());
    std::fs::write(file, b"other").unwrap();
    assert!(policy.load().is_err());
}
#[test]
fn fixture_for_external_signing_demo_is_explicit_and_never_a_default_production_key() {
    let (input, recipe, key, policy) = fixture();
    let candidate = assemble(&input, &recipe).unwrap();
    let signed = signature(&candidate, &key);
    assert!(candidate.verify(&signed, &policy.load().unwrap()).is_ok());
    if let Ok(path) = std::env::var("RX_PACKAGE_TEST_FIXTURE") {
        let path = std::path::PathBuf::from(path);
        std::fs::create_dir(&path).unwrap();
        for (name, bytes) in [
            ("compile-input.json", canonical::bytes(&input).unwrap()),
            ("recipe.json", canonical::bytes(&recipe).unwrap()),
            ("policy.json", canonical::bytes(&policy).unwrap()),
            ("signature.json", canonical::bytes(&signed).unwrap()),
        ] {
            std::fs::write(path.join(name), bytes).unwrap();
        }
    }
}

#[test]
fn a_valid_signature_does_not_hide_inconsistent_packaged_compile_inputs() {
    let (input, recipe, key, policy) = fixture();
    let candidate = assemble(&input, &recipe).unwrap();
    let mut manifest = candidate.manifest().clone();
    let mut files = candidate.files().clone();
    let path = PackagePath::new(rx_process_package::BINDINGS).unwrap();
    let mut bindings = input.bindings;
    bindings.get_mut(&name("load")).unwrap().intent.target = name("changed-target");
    let bytes = canonical::bytes(&bindings).unwrap();
    let entry = manifest.files.iter_mut().find(|e| e.path == path).unwrap();
    entry.sha256 = content_digest(&bytes);
    entry.size_bytes = Counter(bytes.len() as u64);
    files.insert(path, bytes);
    let envelope = SignatureEnvelope {
        key: name("test-key"),
        signature: key
            .sign(&signing_message(&manifest, &name("test-key")).unwrap())
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    };
    let verified = verify_package(
        &manifest_bytes(&manifest).unwrap(),
        &canonical::bytes(&envelope).unwrap(),
        files,
        &policy.load().unwrap(),
    )
    .unwrap();
    assert!(compile_verified(&verified).is_err());
}
#[test]
fn external_signer_fixture_can_sign_a_caller_selected_local_test_candidate() {
    let (Some(input_path), Some(recipe_path), Some(output)) = (
        std::env::var_os("RX_PACKAGE_DEMO_INPUT"),
        std::env::var_os("RX_PACKAGE_DEMO_RECIPE"),
        std::env::var_os("RX_PACKAGE_DEMO_OUTPUT"),
    ) else {
        return;
    };
    let input: CompileInput = trust::read(std::path::Path::new(&input_path)).unwrap();
    let recipe: Recipe = trust::read(std::path::Path::new(&recipe_path)).unwrap();
    let candidate = assemble(&input, &recipe).unwrap();
    let key = SigningKey::from_bytes(&[19; 32]);
    let signature = signature(&candidate, &key);
    let policy = trust::Policy {
        additional_package_abis: Default::default(),
        schema: name("rx.package-verification-policy.v1"),
        contracts: recipe.contracts.clone(),
        target: recipe.targets[0].clone(),
        keys: vec![trust::Key {
            id: name("test-key"),
            publisher: recipe.publisher,
            verifying_key: Digest::from_bytes(key.verifying_key().to_bytes()),
            kinds: [PackageKind::Process].into_iter().collect(),
            permissions: candidate.manifest().permissions.iter().cloned().collect(),
        }],
        assets: vec![],
        dependencies: vec![],
    };
    let out = std::path::PathBuf::from(output);
    std::fs::create_dir(&out).unwrap();
    std::fs::write(
        out.join("signature.json"),
        canonical::bytes(&signature).unwrap(),
    )
    .unwrap();
    std::fs::write(out.join("policy.json"), canonical::bytes(&policy).unwrap()).unwrap();
    std::fs::write(
        out.join("expected-signing-message.bin"),
        candidate.signing_message(&name("test-key")).unwrap(),
    )
    .unwrap();
    assert!(
        candidate
            .verify(&signature, &policy.load().unwrap())
            .is_ok()
    );
}

fn review_request(
    package: &VerifiedPackage,
    policy: &VerificationPolicy,
) -> rx_process_contract::package_review::Request {
    rx_process_contract::package_review::Request {
        device_context_digest: None,
        schema: name("rx.process-review-request.v1"),
        id: Id::new("00000000-0000-4000-8000-000000000046").unwrap(),
        intake: Id::new("00000000-0000-4000-8000-000000000045").unwrap(),
        cell: name("cell/sim"),
        package_manifest: package.digest(),
        package_signature: content_digest(&canonical::bytes(package.signature()).unwrap()),
        configuration_digest: Digest::from_bytes([5; 32]),
        package_policy_fingerprint: policy.fingerprint().unwrap(),
        package_policy_file_digest: Digest::from_bytes([6; 32]),
        verification_authority_digest: Digest::from_bytes([7; 32]),
        binding_selections: BTreeMap::from([(name("load"), name("step/load"))]),
    }
}
#[test]
fn verification_report_uses_actual_signed_compilation_and_binds_request_policy() {
    let (input, recipe, key, policy) = fixture();
    let policy = policy.load().unwrap();
    let candidate = assemble(&input, &recipe).unwrap();
    let verified = candidate
        .verify(&signature(&candidate, &key), &policy)
        .unwrap();
    let request = review_request(&verified, &policy);
    let result = rx_process_package::review::verify_process(
        request.clone(),
        &verified,
        &policy,
        Digest::from_bytes([6; 32]),
    )
    .unwrap();
    assert!(result.report.issues.is_empty());
    let reference = result.report.resolved.as_ref().unwrap();
    let bytes = &result.files[&PackagePath::new("resolved.json").unwrap()];
    assert_eq!(content_digest(bytes), reference.sha256);
    let process: rx_process_contract::ResolvedProcess = canonical::decode_json(bytes).unwrap();
    rx_process_contract::source_link::verify(&input.validate().unwrap(), &process).unwrap();
    let key_id = name("test-key");
    assert_ne!(
        result.report.signing_message(&key_id).unwrap(),
        candidate.signing_message(&key_id).unwrap()
    );
    let mut wrong = request;
    wrong.package_signature = Digest::from_bytes([99; 32]);
    assert!(
        rx_process_package::review::verify_process(
            wrong,
            &verified,
            &policy,
            Digest::from_bytes([6; 32])
        )
        .is_err()
    );
}
#[test]
fn signed_but_inconsistent_process_produces_failure_report_without_resolved_artifact() {
    let (input, recipe, key, policy_doc) = fixture();
    let policy = policy_doc.load().unwrap();
    let candidate = assemble(&input, &recipe).unwrap();
    let mut files = candidate.files().clone();
    let changed = PackagePath::new("process/source.json").unwrap();
    files.insert(changed.clone(), b"{}".to_vec());
    let mut manifest = candidate.manifest().clone();
    let file = manifest
        .files
        .iter_mut()
        .find(|f| f.path == changed)
        .unwrap();
    file.sha256 = content_digest(&files[&changed]);
    file.size_bytes = Counter(2);
    let key_id = name("test-key");
    let envelope = SignatureEnvelope {
        key: key_id.clone(),
        signature: key
            .sign(&signing_message(&manifest, &key_id).unwrap())
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    };
    let verified = verify_package(
        &manifest_bytes(&manifest).unwrap(),
        &canonical::bytes(&envelope).unwrap(),
        files,
        &policy,
    )
    .unwrap();
    let report = rx_process_package::review::verify_process(
        review_request(&verified, &policy),
        &verified,
        &policy,
        Digest::from_bytes([6; 32]),
    )
    .unwrap();
    assert!(report.report.resolved.is_none());
    assert_eq!(report.report.issues.len(), 1);
    assert_eq!(report.files.len(), 1);
}

#[test]
fn device_provenance_survives_signed_package_and_actual_recompilation() {
    use rx_process_contract::compile_input::{
        BindingPlanRef, DeviceSource, bindings_digest, device_action_digest,
    };
    let (mut input, recipe, key, policy) = fixture();
    input.schema = name("rx.process-compile-input.v2");
    input.device_sources.insert(
        name("load"),
        DeviceSource {
            plan: BindingPlanRef {
                id: input.draft.clone(),
                revision: Counter(2),
                plan_digest: Digest::from_bytes([71; 32]),
            },
            binding: name("candidate/load"),
            step_digest: Digest::from_bytes([72; 32]),
            action_digest: device_action_digest(&input.bindings[&name("load")]).unwrap(),
        },
    );
    input.bindings_digest = bindings_digest(&input.bindings, &input.device_sources).unwrap();
    input.validate().unwrap();
    let candidate = assemble(&input, &recipe).unwrap();
    let stored: CompileInput = canonical::decode_json(
        candidate.files()[&PackagePath::new(rx_process_package::INPUT).unwrap()].as_slice(),
    )
    .unwrap();
    assert_eq!(stored.schema.as_str(), "rx.process-compile-input.v2");
    assert_eq!(
        stored.device_sources[&name("load")].plan,
        input.device_sources[&name("load")].plan
    );
    let verified = candidate
        .verify(&signature(&candidate, &key), &policy.load().unwrap())
        .unwrap();
    let compiled = compile_verified(&verified).unwrap();
    assert_eq!(
        compiled.bindings[&name("load")].intent.digest().unwrap(),
        input.bindings[&name("load")].intent.digest().unwrap()
    );
    assert_eq!(compiled.package_digest, Some(verified.digest()));
    for variant in 0..4 {
        let mut bad = input.clone();
        match variant {
            0 => {
                bad.device_sources
                    .get_mut(&name("load"))
                    .unwrap()
                    .plan
                    .revision = Counter(3)
            }
            1 => bad.schema = name("rx.process-compile-input.v1"),
            2 => bad.device_sources.clear(),
            _ => {
                bad.bindings.get_mut(&name("load")).unwrap().host = name("other/host");
                bad.bindings_digest = bindings_digest(&bad.bindings, &bad.device_sources).unwrap();
            }
        }
        assert!(bad.validate().is_err());
    }
}

#[test]
#[ignore = "exports a test-only public PROCESS signing key for mixed ABI policy integration"]
fn export_process_authoring_key() {
    let file = std::path::PathBuf::from(std::env::var("RX_MIXED_PROCESS_KEY").unwrap());
    assert!(!file.exists());
    let key = SigningKey::from_bytes(&[93; 32]);
    let value = trust::Key {
        id: name("test/process-key"),
        publisher: name("test/publisher"),
        verifying_key: Digest::from_bytes(key.verifying_key().to_bytes()),
        kinds: [PackageKind::Process].into(),
        permissions: [
            Permission::ArtifactRead,
            Permission::OperationSubmit {
                operation: name("load"),
            },
        ]
        .into(),
    };
    std::fs::write(file, canonical::bytes(&value).unwrap()).unwrap();
}
#[test]
#[ignore = "test-only external package signer for the isolated mixed-policy harness"]
fn sign_mixed_process_request() {
    let file = std::path::PathBuf::from(std::env::var("RX_MIXED_PROCESS_REQUEST").unwrap());
    let out = std::path::PathBuf::from(std::env::var("RX_MIXED_PROCESS_SIGNATURE").unwrap());
    assert!(!out.exists());
    let request: serde_json::Value = canonical::decode_json(&std::fs::read(file).unwrap()).unwrap();
    assert_eq!(request["schema"], "rx.package-signing-request.v1");
    assert_eq!(request["key"], "test/process-key");
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
    let key = SigningKey::from_bytes(&[93; 32]);
    let sig = SignatureEnvelope {
        key: name("test/process-key"),
        signature: key
            .sign(&bytes)
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    };
    std::fs::write(out, canonical::bytes(&sig).unwrap()).unwrap();
}

#[test]
#[ignore]
fn export_device_process_review_authority() {
    let path = std::env::var("RX_PROCESS_DEVICE_AUTHORITY").unwrap();
    let key = SigningKey::from_bytes(&[93; 32]);
    let authority = serde_json::json!({"schema":"rx.process-verification-authority.v1","keys":[{"id":"test/process-review","public_key":Digest::from_bytes(key.verifying_key().to_bytes()),"validators":[rx_process_package::review::validator_digest()]}]});
    use std::io::Write;
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .unwrap()
        .write_all(&canonical::bytes(&authority).unwrap())
        .unwrap();
}
#[test]
#[ignore]
fn sign_device_process_review() {
    let path = std::env::var("RX_PROCESS_DEVICE_REPORT").unwrap();
    let output = std::env::var("RX_PROCESS_DEVICE_SIGNATURE").unwrap();
    let report: rx_process_contract::package_review::Report =
        canonical::decode_json(&std::fs::read(path).unwrap()).unwrap();
    report.validate().unwrap();
    let key = SigningKey::from_bytes(&[93; 32]);
    let signature = SignatureEnvelope {
        key: name("test/process-review"),
        signature: key
            .sign(
                &report
                    .signing_message(&name("test/process-review"))
                    .unwrap(),
            )
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    };
    use std::io::Write;
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output)
        .unwrap()
        .write_all(&canonical::bytes(&signature).unwrap())
        .unwrap();
}
