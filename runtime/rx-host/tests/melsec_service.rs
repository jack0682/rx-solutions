use ed25519_dalek::{Signer, SigningKey};
use rx_domain::{canonical, types::*};
use rx_host::{
    native::*,
    service::{self, config::*, device_package::*},
    *,
};
use rx_package::*;
use std::{collections::BTreeSet, path::Path, time::Duration};
#[path = "support/host_service_fixture.rs"]
#[allow(dead_code, unused_imports)]
mod host_fixture;
#[path = "support/melsec_fixture.rs"]
#[allow(dead_code)]
mod native_fixture;
use native_fixture::{Plc, TEST_SERIAL, digest, id, intent, name, profile};

fn package(root: &Path, p: &mut melsec::Profile) -> Backend {
    let package = root.join("device-package");
    std::fs::create_dir(&package).unwrap();
    let mut assets = Vec::new();
    for (label, schema, content) in [
        (
            "publication",
            "rx.melsec.publication-contract.v1",
            b"TEST ONLY: simulated atomic status source contract".as_slice(),
        ),
        (
            "program",
            "rx.plc.program-evidence.v1",
            b"TEST ONLY: independent simulated PLC implementation identity".as_slice(),
        ),
    ] {
        let file = host_fixture::pinned(root, &format!("{label}.bin"), content, false);
        let reference = ArtifactRef {
            sha256: file.sha256,
            schema_id: name(schema),
            size_bytes: Counter(content.len() as u64),
        };
        if label == "publication" {
            p.status.publication_contract = reference.sha256;
        } else {
            p.status.plc_program = reference.sha256;
        }
        assets.push(rx_package::policy::Asset {
            reference,
            path: file.path,
        });
    }
    let family = Family {
        schema: name("rx.melsec-family.v1"),
        family: name("mitsubishi/melsec-q"),
        controller_model: name("Q03UDVCPU"),
        environment: p.environment(),
    };
    let files = [
        ("family.json", canonical::bytes(&family).unwrap()),
        ("profile.json", canonical::bytes(p).unwrap()),
        ("adapter.json", canonical::bytes(&driver()).unwrap()),
    ];
    for (path, bytes) in &files {
        std::fs::write(package.join(path), bytes).unwrap();
    }
    let key = SigningKey::from_bytes(&[77; 32]);
    let key_id = name("test/device-publisher");
    let permissions = vec![
        Permission::ArtifactRead,
        Permission::NativeEndpoint {
            role: name("melsec-mc3e"),
        },
        Permission::ObservationRead {
            schema: name("rx.melsec.status-image.v1"),
        },
    ];
    let contracts = ContractSet {
        base: Digest::from_bytes(rx_host::rpc::base_manifest_hash().try_into().unwrap()),
        cell: Digest::from_bytes(rx_host::rpc::cell_manifest_hash().try_into().unwrap()),
        package_abi: name("rx.package-abi.v2"),
    };
    let target = Target {
        os: OperatingSystem::Linux,
        architecture: if std::env::consts::ARCH == "aarch64" {
            Architecture::Arm64
        } else {
            Architecture::Amd64
        },
        ros_distribution: None,
    };
    let manifest = Manifest {
        schema: name("rx.package.v2"),
        package: name("test/melsec-cell"),
        version: semver::Version::new(1, 0, 0),
        publisher: name("test/publisher"),
        contracts: contracts.clone(),
        targets: vec![target.clone()],
        entry: EntryPoint::DeviceReference {
            family: PackagePath::new("family.json").unwrap(),
            profiles: vec![PackagePath::new("profile.json").unwrap()],
            adapter: PackagePath::new("adapter.json").unwrap(),
        },
        permissions: permissions.clone(),
        dependencies: vec![],
        assets: assets.iter().map(|a| a.reference.clone()).collect(),
        files: files
            .iter()
            .map(|(path, data)| FileEntry {
                path: PackagePath::new(*path).unwrap(),
                sha256: content_digest(data),
                size_bytes: Counter(data.len() as u64),
                executable: false,
            })
            .collect(),
    };
    let bytes = manifest_bytes(&manifest).unwrap();
    std::fs::write(package.join("manifest.json"), &bytes).unwrap();
    let signature = key.sign(&signing_message(&manifest, &key_id).unwrap());
    let envelope = SignatureEnvelope {
        key: key_id.clone(),
        signature: signature
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    };
    std::fs::write(
        package.join("manifest.sig.json"),
        canonical::bytes(&envelope).unwrap(),
    )
    .unwrap();
    let policy = rx_package::policy::Policy {
        additional_package_abis: Default::default(),
        schema: name("rx.package-verification-policy.v1"),
        contracts,
        target,
        keys: vec![rx_package::policy::Key {
            id: key_id,
            publisher: name("test/publisher"),
            verifying_key: Digest::from_bytes(key.verifying_key().to_bytes()),
            kinds: [PackageKind::Device].into(),
            permissions: permissions.into_iter().collect(),
        }],
        assets,
        dependencies: vec![],
    };
    Backend::MelsecPackage {
        directory: package,
        manifest_digest: content_digest(&bytes),
        policy: host_fixture::pinned(
            root,
            "device-policy.json",
            &canonical::bytes(&policy).unwrap(),
            false,
        ),
    }
}
fn bind(loaded: &mut Loaded, p: &melsec::Profile) {
    let b = &mut loaded.bindings[0];
    b.cell = p.cell.clone();
    b.environment = p.environment();
    b.allowed_intents = vec![intent(p)];
    b.condition_ids = p.conditions.keys().cloned().collect();
    loaded.config.bindings = host_fixture::pinned(
        loaded.config.bindings.path.parent().unwrap(),
        "melsec-bindings.json",
        &canonical::bytes(&loaded.bindings).unwrap(),
        false,
    );
}
fn setup() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    simulation::ManualClock,
    Plc,
    melsec::Profile,
) {
    let (dir, file, c) = host_fixture::fixture();
    let plc = Plc::start();
    let mut p = profile(&plc);
    let mut loaded = Loaded::read(&file).unwrap();
    p.installation = loaded.config.installation.clone();
    loaded.config.backend = package(dir.path(), &mut p);
    bind(&mut loaded, &p);
    std::fs::write(&file, canonical::bytes(&loaded.config).unwrap()).unwrap();
    (dir, file, c, plc, p)
}

#[test]
fn signed_device_package_binds_release_profile_assets_and_exact_cell_without_io() {
    let _serial = TEST_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let (_dir, file, c, plc, p) = setup();
    let loaded = Loaded::read(&file).unwrap();
    let inspected = service::inspect(&loaded).unwrap();
    assert_eq!(inspected["activation_authorized"], false);
    assert_eq!(inspected["physical_qualification_verified"], false);
    service::initialize_with(&loaded, c.clone(), &service::Builtin).unwrap();
    let descriptor: serde_json::Value = canonical::decode_json(
        &std::fs::read(loaded.config.data_directory.join("installation.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(descriptor["native"]["kind"], "MELSEC");
    assert!(
        loaded
            .config
            .data_directory
            .join("native-melsec/native.sqlite3")
            .is_file()
    );
    assert_eq!(plc.state.lock().unwrap().reads, 0);
    assert_eq!(plc.writes(), 0);
    let mut adapter = service::AdapterFactory::open_passive(
        &service::Builtin,
        &loaded.config.backend,
        &loaded.config.data_directory,
        c.clone(),
    )
    .unwrap();
    assert_eq!(plc.state.lock().unwrap().reads, 0);
    assert!(adapter.guard(&intent(&p), &c.now()).is_err());
    adapter.guard(&intent(&p), &c.now()).unwrap();
    assert!(adapter.submit(&id(), &id(), &intent(&p)).is_err());
    assert_eq!(plc.writes(), 1);
    drop(adapter);
    std::fs::remove_file(
        loaded
            .config
            .data_directory
            .join("native-melsec/native.sqlite3"),
    )
    .unwrap();
    assert!(
        <service::Builtin as service::AdapterFactory<simulation::ManualClock>>::open_passive(
            &service::Builtin,
            &loaded.config.backend,
            &loaded.config.data_directory,
            c
        )
        .is_err()
    );
}

#[test]
fn signature_content_policy_asset_release_and_binding_tampering_fail_before_initialization() {
    let _serial = TEST_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    for case in 0..7 {
        let (dir, file, c, plc, _p) = setup();
        let mut loaded = Loaded::read(&file).unwrap();
        let Backend::MelsecPackage {
            directory, policy, ..
        } = &mut loaded.config.backend
        else {
            panic!()
        };
        match case {
            0 => {
                std::fs::write(directory.join("profile.json"), b"{}").unwrap();
            }
            1 => {
                let mut signature: SignatureEnvelope = canonical::decode_json(
                    &std::fs::read(directory.join("manifest.sig.json")).unwrap(),
                )
                .unwrap();
                signature.signature = "00".repeat(64);
                std::fs::write(
                    directory.join("manifest.sig.json"),
                    canonical::bytes(&signature).unwrap(),
                )
                .unwrap();
            }
            2 => {
                std::fs::write(&policy.path, b"{}").unwrap();
            }
            3 => {
                std::fs::write(dir.path().join("publication.bin"), b"changed").unwrap();
            }
            4 => {
                let mut doc: rx_package::policy::Policy =
                    canonical::decode_json(&policy.read(false).unwrap()).unwrap();
                doc.keys.clear();
                *policy = host_fixture::pinned(
                    dir.path(),
                    "revoked-policy.json",
                    &canonical::bytes(&doc).unwrap(),
                    false,
                );
            }
            5 => {
                loaded.bindings[0].cell = name("other/cell");
            }
            _ => {
                let mut doc: rx_package::policy::Policy =
                    canonical::decode_json(&policy.read(false).unwrap()).unwrap();
                doc.contracts.base = digest(90);
                *policy = host_fixture::pinned(
                    dir.path(),
                    "wrong-contract.json",
                    &canonical::bytes(&doc).unwrap(),
                    false,
                );
            }
        }
        assert!(
            service::initialize_with(&loaded, c, &service::Builtin).is_err(),
            "case {case}"
        );
        assert!(!loaded.config.data_directory.exists());
        assert_eq!(plc.state.lock().unwrap().reads, 0);
        assert_eq!(plc.writes(), 0);
    }
}

#[test]
fn physical_startup_qualification_id_does_not_authorize_arm() {
    let _serial = TEST_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let (dir, file, c) = host_fixture::fixture();
    let plc = Plc::start();
    let mut p = profile(&plc);
    p.transport.environment = rx_melsec_mc::Environment::Physical;
    let mut loaded = Loaded::read(&file).unwrap();
    p.installation = loaded.config.installation.clone();
    loaded.config.backend = package(dir.path(), &mut p);
    bind(&mut loaded, &p);
    std::fs::write(&file, canonical::bytes(&loaded.config).unwrap()).unwrap();
    let loaded = Loaded::read(&file).unwrap();
    service::initialize_with(&loaded, c.clone(), &service::Builtin).unwrap();
    let adapter = service::AdapterFactory::open_passive(
        &service::Builtin,
        &loaded.config.backend,
        &loaded.config.data_directory,
        c.clone(),
    )
    .unwrap();
    let b = loaded.bindings[0].clone();
    let host = Host::open(
        loaded.config.data_directory.join("host.db"),
        adapter,
        c,
        loaded.bindings,
    )
    .unwrap();
    let caller = Caller {
        peer: b.platform.clone(),
        session: id(),
    };
    host.bind_platform(caller.clone()).unwrap();
    let scopes = b
        .scope_ids
        .iter()
        .cloned()
        .map(|s| (s, Counter(1)))
        .collect();
    assert!(
        host.arm(
            &caller,
            id(),
            &b.cell,
            Counter(1),
            &scopes,
            &BTreeSet::new()
        )
        .is_err()
    );
    assert_eq!(plc.state.lock().unwrap().reads, 0);
    assert_eq!(plc.writes(), 0);
}

#[tokio::test]
async fn product_composition_initializes_starts_and_stops_the_signed_melsec_package() {
    let (_dir, file, c, plc, _p) = setup();
    let loaded = Loaded::read(&file).unwrap();
    service::initialize_with(&loaded, c.clone(), &service::Builtin).unwrap();
    let path = loaded.config.runtime_directory.join("host-status.json");
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(service::run_with(loaded, c, service::Builtin, async {
        let _ = stopped.await;
    }));
    let ready = host_fixture::status(&path, "SOFTWARE_READY_UNARMED").await;
    assert_eq!(ready["qualification_or_arm_restored"], false);
    assert_eq!(plc.writes(), 0);
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let done = host_fixture::status(&path, "STOPPED").await;
    assert_eq!(done["stop"]["safe_to_drop"], true);
    assert_eq!(plc.writes(), 0);
}

fn resign(directory: &Path) -> Digest {
    let mut m: Manifest =
        canonical::decode_json(&std::fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    for f in &mut m.files {
        let data = std::fs::read(directory.join(f.path.as_str())).unwrap();
        f.sha256 = content_digest(&data);
        f.size_bytes = Counter(data.len() as u64);
    }
    let bytes = manifest_bytes(&m).unwrap();
    std::fs::write(directory.join("manifest.json"), &bytes).unwrap();
    let key = SigningKey::from_bytes(&[77; 32]);
    let id = name("test/device-publisher");
    let signature = key.sign(&signing_message(&m, &id).unwrap());
    let envelope = SignatureEnvelope {
        key: id,
        signature: signature
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    };
    std::fs::write(
        directory.join("manifest.sig.json"),
        canonical::bytes(&envelope).unwrap(),
    )
    .unwrap();
    content_digest(&bytes)
}
#[test]
fn validly_signed_wrong_release_controller_or_installation_is_rejected() {
    let _serial = TEST_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    for case in 0..3 {
        let (_dir, file, c, plc, _) = setup();
        let mut loaded = Loaded::read(&file).unwrap();
        let Backend::MelsecPackage {
            directory,
            manifest_digest,
            ..
        } = &mut loaded.config.backend
        else {
            panic!()
        };
        if case == 0 {
            let mut d = driver();
            d.source_digest = digest(99);
            std::fs::write(
                directory.join("adapter.json"),
                canonical::bytes(&d).unwrap(),
            )
            .unwrap();
        }
        if case == 1 {
            let mut f: Family =
                canonical::decode_json(&std::fs::read(directory.join("family.json")).unwrap())
                    .unwrap();
            f.controller_model = name("unsupported-controller");
            std::fs::write(directory.join("family.json"), canonical::bytes(&f).unwrap()).unwrap();
        }
        if case == 2 {
            loaded.config.installation = id();
        }
        *manifest_digest = resign(directory);
        assert!(service::initialize_with(&loaded, c, &service::Builtin).is_err());
        assert!(!loaded.config.data_directory.exists());
        assert_eq!(plc.writes(), 0);
    }
}

#[test]
fn physical_metadata_requires_current_qualification_and_separate_arm_on_loopback() {
    let _serial = TEST_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    use rx_domain::{host_configuration as cfg, host_qualification as qual};
    let (dir, file, c) = host_fixture::fixture();
    let plc = Plc::start();
    let mut p = profile(&plc);
    p.transport.environment = rx_melsec_mc::Environment::Physical;
    let mut loaded = Loaded::read(&file).unwrap();
    p.installation = loaded.config.installation.clone();
    loaded.config.backend = package(dir.path(), &mut p);
    bind(&mut loaded, &p);
    std::fs::write(&file, canonical::bytes(&loaded.config).unwrap()).unwrap();
    let loaded = Loaded::read(&file).unwrap();
    service::initialize_with(&loaded, c.clone(), &service::Builtin).unwrap();
    let adapter = service::AdapterFactory::open_passive(
        &service::Builtin,
        &loaded.config.backend,
        &loaded.config.data_directory,
        c.clone(),
    )
    .unwrap();
    assert!(adapter.guard(&intent(&p), &c.now()).is_err());
    adapter.guard(&intent(&p), &c.now()).unwrap();
    let b = loaded.bindings[0].clone();
    let host = Host::open(
        loaded.config.data_directory.join("host.db"),
        adapter,
        c.clone(),
        loaded.bindings,
    )
    .unwrap();
    let who = Caller {
        peer: b.platform.clone(),
        session: id(),
    };
    host.bind_platform(who.clone()).unwrap();
    let snapshot = host.inspect_process_configuration(&who).unwrap().snapshot;
    let observed = &snapshot.cells[0];
    let fence = id();
    let block = id();
    let epoch = Counter(2);
    let scopes = observed
        .scopes
        .keys()
        .cloned()
        .map(|s| (s, epoch))
        .collect();
    let gate = host
        .fence(
            &who,
            fence.clone(),
            &b.cell,
            epoch,
            scopes,
            [block.clone()].into(),
        )
        .unwrap();
    let change = id();
    let configuration = cfg::Request {
        schema: name("rx.host-process-configuration-request.v1"),
        id: id(),
        change: change.clone(),
        preparation: Counter(1),
        plan_digest: digest(30),
        host: snapshot.host.clone(),
        expected_host_boot: snapshot.host_boot.clone(),
        expected_delivery_journal: snapshot.delivery_journal.clone(),
        binding_digest: snapshot.binding_digest,
        cells: vec![cfg::CellTarget {
            cell: b.cell.clone(),
            expected_context: None,
            before_configuration: digest(31),
            after_configuration: digest(32),
            recipe: ArtifactRef {
                sha256: digest(33),
                schema_id: name("rx.resolved-process.v1"),
                size_bytes: Counter(1),
            },
            definition: observed.definition,
            envelope: observed.envelope,
            environment: observed.environment.clone(),
            required_intents: vec![intent(&p).digest().unwrap()],
            required_conditions: b.condition_ids.clone(),
            epoch,
            scopes: gate.state.scopes.clone(),
            fence_request: fence.clone(),
        }],
    };
    let accepted = host
        .accept_process_configuration(&who, configuration)
        .unwrap();
    let context = accepted.snapshot.cells[0].applied.as_ref().unwrap();
    assert!(
        host.arm(
            &who,
            id(),
            &b.cell,
            epoch,
            &gate.state.scopes,
            &[block.clone()].into()
        )
        .is_err()
    );
    let qualification = id();
    let request = qual::Request {
        schema: name("rx.host-qualification-request.v1"),
        id: id(),
        host: snapshot.host,
        expected_host_boot: snapshot.host_boot,
        delivery_journal: snapshot.delivery_journal,
        binding_digest: snapshot.binding_digest,
        change,
        review: id(),
        review_revision: Counter(1),
        review_digest: digest(34),
        decision_revision: Counter(1),
        policy_digest: digest(35),
        application_digest: digest(36),
        cells: vec![qual::CellTarget {
            cell: b.cell.clone(),
            configuration: context.configuration,
            context_request: context.request.clone(),
            context_sequence: context.receipt_sequence,
            definition: observed.definition,
            envelope: observed.envelope,
            environment: observed.environment.clone(),
            qualification: qualification.clone(),
            qualification_revision: Counter(1),
            dependencies: vec![
                context.configuration,
                observed.definition,
                observed.envelope,
            ],
            limitations: ArtifactRef {
                sha256: digest(37),
                schema_id: name("test/physical-metadata-only"),
                size_bytes: Counter(1),
            },
            allowed_intents: vec![intent(&p).digest().unwrap()],
            purposes: vec![name("PRODUCTION")],
            epoch,
            scopes: gate.state.scopes.clone(),
            fence_request: fence,
            required_blocks: vec![block.clone()],
        }],
    };
    let result = host.accept_qualification(&who, request).unwrap();
    assert_eq!(
        result.receipt.as_ref().unwrap().status,
        qual::Status::Accepted
    );
    assert_eq!(plc.writes(), 0);
    host.arm(
        &who,
        id(),
        &b.cell,
        epoch,
        &gate.state.scopes,
        &[block].into(),
    )
    .unwrap();
    let grant = host
        .acquire_grant(
            &who,
            id(),
            p.resources.clone(),
            epoch,
            Counter(1_000_000_000),
        )
        .unwrap();
    let op = id();
    let i = intent(&p);
    let d = i.digest().unwrap();
    let request = Request {
        operation: op.clone(),
        intent: i,
        digest: d,
        grant: grant.id.clone(),
        permit: Permit {
            id: id(),
            operation: op.clone(),
            digest: d,
            cell: b.cell,
            epoch,
            scopes: gate.state.scopes,
            envelope: b.envelope.sha256,
            qualification,
            qualification_revision: Counter(1),
            grant: grant.id,
            host_boot: host.boot_id().unwrap(),
            conditions: b.condition_ids.into_iter().collect(),
            expires_at: TimePoint {
                clock_id: c.clock_id.clone(),
                ticks_ns: Counter(c.now().ticks_ns.0 + 500_000_000),
            },
            purpose: Purpose::Production,
            parent: PermitParent::Mandate(id()),
            source_digest: None,
        },
    };
    let prepared = host.prepare(&who, request.clone()).unwrap();
    assert_eq!(plc.writes(), 0);
    assert!(
        host.authorize(&who, request, prepared.invocation.as_ref().unwrap())
            .is_err()
    );
    assert_eq!(plc.writes(), 1);
    plc.update(|s| s.flags |= 1 << 6);
    assert_eq!(
        host.reconcile(&who, &op).unwrap().state,
        ReceiptState::ResultCaptured
    );
}

#[test]
#[ignore = "exports signed, test-only loopback configuration for the product image test"]
fn export_melsec_image_fixture() {
    let (dir, file, _c) = host_fixture::fixture();
    let plc = Plc::start();
    let mut p = profile(&plc);
    p.transport.endpoint = "127.0.0.1:5010".parse().unwrap();
    p.transport.exchange_timeout_ms = 50;
    p.status.source_max_age_ms = 50;
    p.status.guard_validity_ms = 50;
    let mut loaded = Loaded::read(&file).unwrap();
    p.installation = loaded.config.installation.clone();
    loaded.config.backend = package(dir.path(), &mut p);
    bind(&mut loaded, &p);
    let Backend::MelsecPackage {
        directory, policy, ..
    } = &mut loaded.config.backend
    else {
        panic!()
    };
    let mut verification: rx_package::policy::Policy =
        canonical::decode_json(&policy.read(false).unwrap()).unwrap();
    for asset in &mut verification.assets {
        asset.path = Path::new("/config").join(asset.path.file_name().unwrap());
    }
    let bytes = canonical::bytes(&verification).unwrap();
    std::fs::write(&policy.path, &bytes).unwrap();
    policy.sha256 = content_digest(&bytes);
    policy.path = "/config/device-policy.json".into();
    *directory = "/config/device-package".into();
    for pin in [
        &mut loaded.config.bindings,
        &mut loaded.config.tls.certificate,
        &mut loaded.config.tls.key,
        &mut loaded.config.tls.ca,
    ] {
        pin.path = Path::new("/config").join(pin.path.file_name().unwrap());
    }
    loaded.config.data_directory = "/data/host".into();
    loaded.config.runtime_directory = "/data/runtime".into();
    loaded.config.bind = "127.0.0.1:7444".parse().unwrap();
    std::fs::write(&file, canonical::bytes(&loaded.config).unwrap()).unwrap();
    let destination = std::path::PathBuf::from(std::env::var("RX_MELSEC_IMAGE_FIXTURE").unwrap());
    assert!(!destination.exists());
    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let next = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &next);
            } else {
                std::fs::copy(entry.path(), next).unwrap();
            }
        }
    }
    copy(dir.path(), &destination);
}
