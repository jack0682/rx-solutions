use ed25519_dalek::{Signer, SigningKey};
use rx_domain::types::*;
use rx_package::*;
use std::collections::BTreeMap;
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn package(bytes: Vec<u8>) -> VerifiedPackage {
    let key = SigningKey::from_bytes(&[19; 32]);
    let entry = PackagePath::new("source.json").unwrap();
    let contracts = || ContractSet {
        base: Digest::from_bytes([1; 32]),
        cell: Digest::from_bytes([2; 32]),
        package_abi: name("rx.package-abi.v1"),
    };
    let target = || Target {
        os: OperatingSystem::Linux,
        architecture: Architecture::Amd64,
        ros_distribution: None,
    };
    let manifest = Manifest {
        schema: name("rx.package.v1"),
        package: name("test/process"),
        version: "0.1.0".parse().unwrap(),
        publisher: name("test"),
        contracts: contracts(),
        targets: vec![target()],
        entry: EntryPoint::Process {
            source: entry.clone(),
        },
        permissions: vec![],
        dependencies: vec![],
        assets: vec![],
        files: vec![FileEntry {
            path: entry.clone(),
            sha256: content_digest(&bytes),
            size_bytes: Counter(bytes.len() as u64),
            executable: false,
        }],
    };
    let policy = VerificationPolicy {
        additional_package_abis: Default::default(),
        publishers: BTreeMap::from([(
            name("key"),
            TrustedPublisher {
                publisher: name("test"),
                verifying_key: key.verifying_key().to_bytes(),
                kinds: [PackageKind::Process].into_iter().collect(),
                permissions: Default::default(),
            },
        )]),
        contracts: contracts(),
        target: target(),
        dependencies: BTreeMap::new(),
        assets: BTreeMap::new(),
        max_files: 16,
        max_content_bytes: 1_048_576,
    };
    let signature = key.sign(&signing_message(&manifest, &name("key")).unwrap());
    let envelope = SignatureEnvelope {
        key: name("key"),
        signature: signature
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    };
    verify_package(
        &serde_json::to_vec(&manifest).unwrap(),
        &serde_json::to_vec(&envelope).unwrap(),
        BTreeMap::from([(entry, bytes)]),
        &policy,
    )
    .unwrap()
}
#[test]
fn verified_content_still_requires_process_semantic_validation() {
    let invalid = package(
        br#"{"schema":"rx.process-source.v1","arbitrary":"signed but not a process"}"#.to_vec(),
    );
    assert!(rx_process::compile_package(&invalid, BTreeMap::new()).is_err());
    let source = serde_json::json!({"schema":"rx.process-source.v1","process":"test/access","entry":"main","conditions":{},"flows":[{"id":"main","root":"access","nodes":[{"id":"access","body":{"kind":"INTERVENTION","procedure":{"sha256":"0303030303030303030303030303030303030303030303030303030303030303","schema_id":"rx.test.procedure.v1","size_bytes":"1"}}}]}]});
    let valid = package(serde_json::to_vec(&source).unwrap());
    let process = rx_process::compile_package(&valid, BTreeMap::new()).unwrap();
    assert_eq!(process.package_digest, Some(valid.digest()));
}
