use ed25519_dalek::{Signer, SigningKey};
use rx_device_package::directory;
use rx_device_package::*;
use rx_domain::{canonical, types::*};
use rx_host::melsec::authoring::{Connection, PredicateSlot, StatusSemantics};
use rx_package::*;
use std::{collections::BTreeMap, path::Path};
fn n(v: &str) -> Name {
    Name::new(v).unwrap()
}
fn d(v: u8) -> Digest {
    Digest::from_bytes([v; 32])
}
fn id(v: u8) -> Id {
    Id::new(format!("00000000-0000-4000-8000-{v:012}")).unwrap()
}
fn artifact(data: &[u8], schema: &str) -> ArtifactRef {
    ArtifactRef {
        sha256: content_digest(data),
        schema_id: n(schema),
        size_bytes: Counter(data.len() as u64),
    }
}
const PUBLICATION: &[u8] = b"TEST ONLY atomic publication contract";
const PROGRAM: &[u8] = b"TEST ONLY PLC program evidence";
fn inputs() -> (Template, Site, Recipe) {
    let t = Template {
        schema: n("rx.melsec-template.v1"),
        id: n("test/melsec-chuck"),
        revision: Counter(1),
        controller_model: n("Q03UDVCPU"),
        resource_roles: vec![n("controller")],
        status: StatusSemantics {
            publication_contract: artifact(PUBLICATION, "rx.melsec.publication-contract.v1"),
            source_max_age_ms: 30,
            read_budget_ms: 50,
            guard_validity_ms: 30,
            valid_bit: 0,
            ready_bit: 1,
            queue_empty_bit: 2,
            control_bit: 3,
            support_bit: 4,
            drop_allowed_bit: 5,
        },
        conditions: [(n("ready"), 1)].into(),
        sources: [(n("chuck/closed"), 6)].into(),
        predicates: vec![PredicateSlot {
            predicate: n("chuck.closed"),
            command_slot: n("close-request"),
            completion_bit: 6,
            settle_ms: Counter(100),
        }],
    };
    let site = Site {
        schema: n("rx.melsec-site-binding.v1"),
        template_digest: t.digest().unwrap(),
        installation: id(1),
        cell: n("cell/one"),
        target: n("chuck/one"),
        site_config: d(10),
        calibrations: vec![],
        resources: [(n("controller"), n("resource/controller-one"))].into(),
        connection: Connection {
            environment: rx_melsec_mc::Environment::Simulation,
            endpoint: "127.0.0.1:5010".parse().unwrap(),
            route: rx_melsec_mc::Route {
                network: 0,
                pc: 255,
                module_io: 0x03ff,
                station: 0,
            },
            monitoring_timer: 1,
            connect_timeout_ms: 50,
            exchange_timeout_ms: 30,
            m_last: 1000,
            d_last: 1000,
        },
        status_first_d: 350,
        plc_program: artifact(PROGRAM, "rx.plc.program-evidence.v1"),
        command_addresses: [(n("close-request"), 200)].into(),
    };
    let recipe = Recipe {
        schema: n("rx.device-package-recipe.v1"),
        package: n("test/device"),
        version: semver::Version::new(1, 0, 0),
        publisher: n("test/publisher"),
        targets: vec![Target {
            os: OperatingSystem::Linux,
            architecture: Architecture::Arm64,
            ros_distribution: None,
        }],
    };
    (t, site, recipe)
}
fn policy(candidate: &Candidate) -> (VerificationPolicy, SigningKey) {
    let key = SigningKey::from_bytes(&[44; 32]);
    let manifest = candidate.manifest();
    (
        VerificationPolicy {
            additional_package_abis: Default::default(),
            publishers: [(
                n("test/key"),
                TrustedPublisher {
                    publisher: manifest.publisher.clone(),
                    verifying_key: key.verifying_key().to_bytes(),
                    kinds: [PackageKind::Device].into(),
                    permissions: manifest.permissions.iter().cloned().collect(),
                },
            )]
            .into(),
            contracts: contracts(),
            target: manifest.targets[0].clone(),
            dependencies: BTreeMap::new(),
            assets: manifest
                .assets
                .iter()
                .cloned()
                .map(|a| (a.sha256, a))
                .collect(),
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
fn one_template_resolves_two_sites_without_editing_the_shared_semantics() {
    let (t, s, r) = inputs();
    let a = assemble(&t, &s, &r).unwrap();
    let mut other = s.clone();
    other.installation = id(2);
    other.cell = n("cell/two");
    other.target = n("chuck/two");
    other
        .resources
        .insert(n("controller"), n("resource/controller-two"));
    other.command_addresses.insert(n("close-request"), 250);
    other.status_first_d = 450;
    let b = assemble(&t, &other, &r).unwrap();
    assert_ne!(a.digest().unwrap(), b.digest().unwrap());
    assert_eq!(s.template_digest, other.template_digest);
    let pa: rx_host::melsec::Profile =
        canonical::decode_json(a.files()[&PackagePath::new("profile.json").unwrap()].as_slice())
            .unwrap();
    let pb: rx_host::melsec::Profile =
        canonical::decode_json(b.files()[&PackagePath::new("profile.json").unwrap()].as_slice())
            .unwrap();
    assert_eq!(pa.predicates[0].predicate, pb.predicates[0].predicate);
    assert_eq!(pa.predicates[0].settle_ms, pb.predicates[0].settle_ms);
    assert_eq!(pb.predicates[0].command_m, 250);
    assert_eq!(pb.transport.access.write_m, [250].into());
    assert!(pb.transport.access.read_m.is_empty());
    assert_eq!(pb.transport.access.read_d[0].first, 450);
}
#[test]
fn candidate_roundtrip_external_signature_and_host_decoding_preserve_signed_provenance() {
    let (t, s, r) = inputs();
    let candidate = assemble(&t, &s, &r).unwrap();
    let root = tempfile::tempdir().unwrap();
    let out = root.path().join("candidate");
    directory::publish(&candidate, None, &out).unwrap();
    let loaded = directory::candidate(&out).unwrap();
    assert_eq!(candidate.digest().unwrap(), loaded.digest().unwrap());
    let (p, key) = policy(&candidate);
    let sig = signature(&candidate, &key);
    let verified = candidate.verify(&sig, &p).unwrap();
    let decoded = decode_verified(&verified).unwrap();
    assert_eq!(decoded.profile.installation, s.installation);
    let package = root.path().join("package");
    directory::publish(&candidate, Some(&sig), &package).unwrap();
    assert!(!package.join("candidate-recipe.json").exists());
    assert_eq!(
        rx_package::directory::verify_directory(&package, &p)
            .unwrap()
            .digest(),
        candidate.digest().unwrap()
    );
    assert!(directory::publish(&candidate, None, &out).is_err());
    assert!(directory::candidate(&package).is_err());
}
#[test]
fn template_hash_missing_extra_and_alias_bindings_are_rejected() {
    let (t, s, r) = inputs();
    for case in 0..6 {
        let mut site = s.clone();
        match case {
            0 => site.template_digest = d(99),
            1 => {
                site.command_addresses.clear();
            }
            2 => {
                site.command_addresses.insert(n("extra"), 200);
            }
            3 => {
                site.resources.clear();
            }
            4 => site.status_first_d = u32::MAX,
            _ => site.connection.endpoint = "192.0.2.1:5010".parse().unwrap(),
        }
        assert!(assemble(&t, &site, &r).is_err());
    }
    let mut bad = t.clone();
    bad.status.valid_bit = bad.status.ready_bit;
    assert!(bad.digest().is_err());
    let mut bad = t.clone();
    bad.predicates[0].completion_bit = 20;
    assert!(bad.digest().is_err());
    let mut two = t.clone();
    two.resource_roles.push(n("second"));
    let mut site = s;
    site.template_digest = two.digest().unwrap();
    site.resources
        .insert(n("second"), n("resource/controller-one"));
    assert!(assemble(&two, &site, &r).is_err());
}
#[test]
fn signed_but_inconsistent_assembly_cannot_pass_the_host_resolver() {
    let (t, s, r) = inputs();
    let c = assemble(&t, &s, &r).unwrap();
    let (policy, key) = policy(&c);
    let mut files = c.files().clone();
    let assembly = PackagePath::new("authoring/assembly.json").unwrap();
    let mut a: Assembly = canonical::decode_json(&files[&assembly]).unwrap();
    a.site.command_addresses.insert(n("close-request"), 299);
    files.insert(assembly.clone(), bytes(&a).unwrap());
    let mut manifest = c.manifest().clone();
    let e = manifest
        .files
        .iter_mut()
        .find(|f| f.path == assembly)
        .unwrap();
    e.sha256 = content_digest(&files[&assembly]);
    e.size_bytes = Counter(files[&assembly].len() as u64);
    let sig = SignatureEnvelope {
        key: n("test/key"),
        signature: key
            .sign(&signing_message(&manifest, &n("test/key")).unwrap())
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    };
    let verified = verify_package(
        &manifest_bytes(&manifest).unwrap(),
        &bytes(&sig).unwrap(),
        files,
        &policy,
    )
    .unwrap();
    assert!(decode_verified(&verified).is_err());
}
#[test]
fn candidate_changes_permissions_keys_and_missing_assets_do_not_bypass_verification() {
    let (t, s, r) = inputs();
    let c = assemble(&t, &s, &r).unwrap();
    let (mut p, key) = policy(&c);
    let sig = signature(&c, &key);
    p.assets.clear();
    assert!(c.verify(&sig, &p).is_err());
    let (mut p, _) = policy(&c);
    p.publishers
        .get_mut(&n("test/key"))
        .unwrap()
        .permissions
        .clear();
    assert!(c.verify(&sig, &p).is_err());
    let (mut p, _) = policy(&c);
    p.publishers.clear();
    assert!(c.verify(&sig, &p).is_err());
    let root = tempfile::tempdir().unwrap();
    let out = root.path().join("candidate");
    directory::publish(&c, None, &out).unwrap();
    let file = out.join("profile.json");
    std::fs::write(&file, b"{}").unwrap();
    assert!(directory::candidate(&out).is_err());
    let mut bad = r;
    bad.targets[0].os = OperatingSystem::Windows;
    assert!(assemble(&t, &s, &bad).is_err());
}
fn write(path: &Path, value: &impl serde::Serialize) {
    std::fs::write(path, bytes(value).unwrap()).unwrap();
}
#[test]
fn cli_assembles_requests_external_signature_seals_and_verifies_without_network() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path();
    let (t, s, r) = inputs();
    write(&dir.join("template.json"), &t);
    write(&dir.join("site.json"), &s);
    write(&dir.join("recipe.json"), &r);
    let run = |args: &[&str]| {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_rx-device-package"))
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    assert_eq!(
        run(&["template-digest", "template.json"])["template_digest"],
        serde_json::to_value(t.digest().unwrap()).unwrap()
    );
    assert_eq!(
        run(&[
            "assemble",
            "template.json",
            "site.json",
            "recipe.json",
            "candidate"
        ])["status"],
        "UNSIGNED_CANDIDATE"
    );
    run(&["request", "candidate", "test/key", "request.json"]);
    let request: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("request.json")).unwrap()).unwrap();
    let message = request["message_hex"]
        .as_str()
        .unwrap()
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect::<Vec<_>>();
    let c = directory::candidate(&dir.join("candidate")).unwrap();
    let (p, key) = policy(&c);
    assert_eq!(message, c.signing_message(&n("test/key")).unwrap());
    write(
        &dir.join("signature.json"),
        &SignatureEnvelope {
            key: n("test/key"),
            signature: key
                .sign(&message)
                .to_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
        },
    );
    std::fs::write(dir.join("publication.bin"), PUBLICATION).unwrap();
    std::fs::write(dir.join("program.bin"), PROGRAM).unwrap();
    let doc = rx_package::policy::Policy {
        additional_package_abis: Default::default(),
        schema: n("rx.package-verification-policy.v1"),
        contracts: p.contracts,
        target: p.target,
        keys: vec![rx_package::policy::Key {
            id: n("test/key"),
            publisher: r.publisher,
            verifying_key: Digest::from_bytes(key.verifying_key().to_bytes()),
            kinds: [PackageKind::Device].into(),
            permissions: c.manifest().permissions.iter().cloned().collect(),
        }],
        assets: vec![
            rx_package::policy::Asset {
                reference: t.status.publication_contract,
                path: dir.join("publication.bin"),
            },
            rx_package::policy::Asset {
                reference: s.plc_program,
                path: dir.join("program.bin"),
            },
        ],
        dependencies: vec![],
    };
    write(&dir.join("policy.json"), &doc);
    run(&[
        "seal",
        "candidate",
        "signature.json",
        "policy.json",
        "package",
    ]);
    assert_eq!(
        run(&["verify", "package", "policy.json"])["activation_authorized"],
        false
    );
    let inspect = run(&["inspect", "package", "policy.json"]);
    assert_eq!(inspect["profile"]["predicates"][0]["command_m"], 200);
    std::fs::write(dir.join("program.bin"), b"changed").unwrap();
    let failed = std::process::Command::new(env!("CARGO_BIN_EXE_rx-device-package"))
        .args(["verify", "package", "policy.json"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(!failed.status.success());
}

#[test]
fn semantically_unordered_inputs_produce_the_same_signing_message() {
    let (mut t, mut s, mut r) = inputs();
    t.resource_roles.push(n("aux"));
    t.predicates.push(PredicateSlot {
        predicate: n("door.closed"),
        command_slot: n("door-request"),
        completion_bit: 7,
        settle_ms: Counter(50),
    });
    s.template_digest = t.digest().unwrap();
    s.resources.insert(n("aux"), n("resource/aux"));
    s.command_addresses.insert(n("door-request"), 201);
    s.calibrations = vec![d(5), d(4)];
    r.targets.push(Target {
        os: OperatingSystem::Linux,
        architecture: Architecture::Amd64,
        ros_distribution: None,
    });
    let a = assemble(&t, &s, &r).unwrap();
    t.resource_roles.reverse();
    t.predicates.reverse();
    s.calibrations.reverse();
    r.targets.reverse();
    let b = assemble(&t, &s, &r).unwrap();
    assert_eq!(a.digest().unwrap(), b.digest().unwrap());
    assert_eq!(
        a.signing_message(&n("test/key")).unwrap(),
        b.signing_message(&n("test/key")).unwrap()
    );
}

#[test]
#[ignore = "exports explicitly simulated, unsigned inputs and public test policy for container verification"]
fn export_image_fixture() {
    let out = std::path::PathBuf::from(std::env::var("RX_DEVICE_TOOL_FIXTURE").unwrap());
    assert!(!out.exists());
    std::fs::create_dir(&out).unwrap();
    let (t, s, mut r) = inputs();
    r.targets[0].architecture = if std::env::consts::ARCH == "aarch64" {
        Architecture::Arm64
    } else {
        Architecture::Amd64
    };
    write(&out.join("template.json"), &t);
    write(&out.join("site.json"), &s);
    write(&out.join("recipe.json"), &r);
    std::fs::write(out.join("publication.bin"), PUBLICATION).unwrap();
    std::fs::write(out.join("program.bin"), PROGRAM).unwrap();
    let c = assemble(&t, &s, &r).unwrap();
    let (p, key) = policy(&c);
    let doc = rx_package::policy::Policy {
        additional_package_abis: Default::default(),
        schema: n("rx.package-verification-policy.v1"),
        contracts: p.contracts,
        target: p.target,
        keys: vec![rx_package::policy::Key {
            id: n("test/key"),
            publisher: r.publisher,
            verifying_key: Digest::from_bytes(key.verifying_key().to_bytes()),
            kinds: [PackageKind::Device].into(),
            permissions: c.manifest().permissions.iter().cloned().collect(),
        }],
        assets: vec![
            rx_package::policy::Asset {
                reference: t.status.publication_contract,
                path: "/data/fixture/publication.bin".into(),
            },
            rx_package::policy::Asset {
                reference: s.plc_program,
                path: "/data/fixture/program.bin".into(),
            },
        ],
        dependencies: vec![],
    };
    write(&out.join("policy.json"), &doc);
}

#[test]
#[ignore = "signs only with an embedded test key for isolated container verification"]
fn sign_image_request() {
    let input = std::path::PathBuf::from(std::env::var("RX_DEVICE_TOOL_REQUEST").unwrap());
    let output = std::path::PathBuf::from(std::env::var("RX_DEVICE_TOOL_SIGNATURE").unwrap());
    assert!(!output.exists());
    let request: serde_json::Value =
        serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
    assert_eq!(request["key"], "test/key");
    assert_eq!(request["schema"], "rx.package-signing-request.v1");
    let encoded = request["message_hex"].as_str().unwrap().as_bytes();
    assert_eq!(encoded.len() % 2, 0);
    let message = encoded
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        request["message_digest"],
        serde_json::to_value(content_digest(&message)).unwrap()
    );
    let signature = SigningKey::from_bytes(&[44; 32]).sign(&message);
    write(
        &output,
        &SignatureEnvelope {
            key: n("test/key"),
            signature: signature
                .to_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
        },
    );
}
