#![cfg(unix)]
//! Package/registry refusal checks, not native execution or a Run acceptance test.
use ed25519_dalek::{Signer, SigningKey};
use rx_device_package::{Candidate, Device, Recipe, contracts, decode_verified_any, external};
use rx_domain::{canonical, intent::*, types::*};
use rx_host::{
    external_process::profile::*,
    service::{
        config::PinnedFile,
        external_package::{Assembly, Registry},
    },
};
use rx_package::*;
use rx_process_contract::{
    execution_v2::{NodeContract, TemplateCatalog, TemplateDeclaration},
    native_outcome::*,
};
use std::collections::BTreeMap;
fn n(v: &str) -> Name {
    Name::new(v).unwrap()
}
fn id() -> Id {
    Id::new("00000000-0000-4000-8000-000000000001").unwrap()
}
fn source() -> (Assembly, Recipe) {
    let program = Program {
        schema: n(PROGRAM_SCHEMA),
        executable: FilePin {
            path: "/installed/echo".into(),
            sha256: Digest::from_bytes([1; 32]),
            size_bytes: Counter(1),
        },
        arguments: vec![],
        dependencies: vec![],
    };
    let contract = NodeContract {
        implementation: "fixture.echo".into(),
        version: "1.0.0".into(),
        primitive: n("echo"),
        parameters: BTreeMap::new(),
    };
    let profile = Profile {
        schema: n(PROFILE_SCHEMA),
        protocol: n(PROTOCOL),
        program: program.reference().unwrap(),
        commands: BTreeMap::from([(n("echo"), contract.clone())]),
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
    let input = ArtifactRef {
        schema_id: n("rx.workflow-parameters.v2"),
        sha256: content_digest(b"{}"),
        size_bytes: Counter(2),
    };
    let intent = Intent {
        kind: Kind::FiniteAction,
        target: n("fixture/echo"),
        profile_digest: profile.digest().unwrap(),
        site_config_digest: Digest::from_bytes([2; 32]),
        calibration_digests: vec![],
        resource_set: vec![n("fixture/resource")],
        execution_timeout_ms: Counter(1000),
        prepare_validity_ms: Counter(500),
        completion_rule: n("fixture/completed"),
        cancel_rule: n("fixture/stop"),
        body: Body::Program(ProgramGoal {
            program: profile.program.clone(),
            parameter_set: input,
        }),
    };
    let catalog = TemplateCatalog {
        schema: n("rx.execution-template-catalog.v2"),
        installation: id(),
        cell: n("cell/fixture"),
        environment: rx_process_contract::device_catalog::Environment::Simulation,
        documents: BTreeMap::new(),
        templates: BTreeMap::from([(
            n("echo"),
            TemplateDeclaration {
                action: rx_process_contract::model::ActionBinding {
                    host: n("host/fixture"),
                    intent,
                },
                contract,
            },
        )]),
    };
    let outcomes = NativeOutcomeTable {
        schema: n("rx.native-outcome-table.v1"),
        profile_digest: profile.digest().unwrap(),
        completion_rule: n("fixture/completed"),
        cases: vec![NativeOutcomeCase {
            status_schema: n("fixture/completed"),
            statuses: vec![Integer(0)],
            conclusion: NativeConclusion::Succeeded,
        }],
    };
    let recipe = Recipe {
        schema: n("rx.device-package-recipe.v1"),
        package: n("fixture/adapter"),
        version: semver::Version::new(1, 0, 0),
        publisher: n("fixture"),
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
    (
        Assembly {
            schema: n("rx.external-process-assembly.v1"),
            profile,
            program,
            catalog,
            outcomes,
        },
        recipe,
    )
}
fn signed() -> (Candidate, VerificationPolicy, SignatureEnvelope) {
    let (source, recipe) = source();
    let candidate = external::assemble(&source, &recipe).unwrap();
    let key = SigningKey::from_bytes(&[73; 32]);
    let m = candidate.manifest();
    let policy = VerificationPolicy {
        additional_package_abis: Default::default(),
        publishers: BTreeMap::from([(
            n("fixture/key"),
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
    };
    let signature = SignatureEnvelope {
        key: n("fixture/key"),
        signature: key
            .sign(&candidate.signing_message(&n("fixture/key")).unwrap())
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    };
    (candidate, policy, signature)
}
#[test]
fn external_package_verifies_but_bad_signature_and_tampered_program_do_not() {
    let (candidate, policy, signature) = signed();
    assert!(matches!(
        decode_verified_any(&candidate.verify(&signature, &policy).unwrap()).unwrap(),
        Device::External(_)
    ));
    let unsigned = SignatureEnvelope {
        key: signature.key.clone(),
        signature: "00".repeat(64),
    };
    assert!(candidate.verify(&unsigned, &policy).is_err());
    let mut files = candidate.files().clone();
    files
        .get_mut(&PackagePath::new("program.json").unwrap())
        .unwrap()
        .push(b' ');
    assert!(
        verify_package(
            &manifest_bytes(candidate.manifest()).unwrap(),
            &canonical::bytes(&signature).unwrap(),
            files,
            &policy
        )
        .is_err()
    );
}
#[test]
fn external_registry_has_no_fallback_for_unregistered_identity() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("registry.json");
    let raw = canonical::bytes(&Registry {
        schema: n("rx.external-adapter-registry.v1"),
        entries: BTreeMap::new(),
    })
    .unwrap();
    std::fs::write(&file, &raw).unwrap();
    let error = Registry::selected(
        &PinnedFile {
            path: file,
            sha256: content_digest(&raw),
        },
        &n("missing"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("UNREGISTERED_ADAPTER"));
}
#[test]
fn template_cannot_declare_an_unregistered_command() {
    let (mut source, recipe) = source();
    source
        .catalog
        .templates
        .get_mut(&n("echo"))
        .unwrap()
        .contract
        .primitive = n("other");
    assert!(external::assemble(&source, &recipe).is_err());
}
#[test]
fn changed_installed_program_fails_closure_check() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("program");
    std::fs::write(&path, b"original").unwrap();
    let pin = FilePin {
        path: path.clone(),
        sha256: content_digest(b"original"),
        size_bytes: Counter(8),
    };
    pin.verify().unwrap();
    std::fs::write(path, b"modified").unwrap();
    assert!(pin.verify().is_err());
}
