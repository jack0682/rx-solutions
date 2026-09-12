use rx_domain::{canonical, intent::*, types::*};
use rx_host::{service::config::*, simulation::*, *};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicU64},
    time::Duration,
};
pub fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
pub fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
pub fn pinned(root: &Path, label: &str, bytes: &[u8], secret: bool) -> PinnedFile {
    let path = root.join(label);
    std::fs::write(&path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            &path,
            std::fs::Permissions::from_mode(if secret { 0o600 } else { 0o644 }),
        )
        .unwrap();
    }
    PinnedFile {
        path,
        sha256: rx_package::content_digest(bytes),
    }
}
pub fn fixture() -> (tempfile::TempDir, PathBuf, ManualClock) {
    use rcgen::*;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut params = CertificateParams::new(vec![]).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params
        .distinguished_name
        .push(DnType::CommonName, "RX Host Test CA");
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let ca = CertifiedIssuer::self_signed(params, KeyPair::generate().unwrap()).unwrap();
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
    params
        .distinguished_name
        .push(DnType::CommonName, "RX Host Test Server");
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.use_authority_key_identifier_extension = true;
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let cert = params.signed_by(&key, &ca).unwrap();
    let client_key = KeyPair::generate().unwrap();
    let mut client_params = CertificateParams::new(vec!["platform".into()]).unwrap();
    client_params
        .distinguished_name
        .push(DnType::CommonName, "RX Host Test Client");
    client_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    client_params.use_authority_key_identifier_extension = true;
    client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    let client = client_params.signed_by(&client_key, &ca).unwrap();
    pinned(root, "client.pem", client.pem().as_bytes(), false);
    pinned(
        root,
        "client.key",
        client_key.serialize_pem().as_bytes(),
        true,
    );

    let bindings = vec![Binding {
        host: name("host/sim"),
        platform: name("platform"),
        cell: name("cell/sim"),
        definition: ArtifactRef {
            sha256: Digest::from_bytes([3; 32]),
            schema_id: name("rx.cell-definition.v1"),
            size_bytes: Counter(1),
        },
        envelope: ArtifactRef {
            sha256: Digest::from_bytes([4; 32]),
            schema_id: name("rx.operating-envelope.v1"),
            size_bytes: Counter(1),
        },
        qualification: id(),
        qualification_revision: Counter(1),
        allowed_intents: vec![Intent {
            kind: Kind::EnsureState,
            target: name("sim/chuck"),
            profile_digest: Digest::from_bytes([1; 32]),
            site_config_digest: Digest::from_bytes([2; 32]),
            calibration_digests: vec![],
            resource_set: vec![name("sim/controller")],
            execution_timeout_ms: Counter(1000),
            prepare_validity_ms: Counter(1000),
            completion_rule: name("sim/closed"),
            cancel_rule: name("sim/stop"),
            body: Body::Predicate(PredicateGoal {
                predicate_id: name("closed"),
                target: TypedValue::Boolean(true),
                settle_ms: Counter(0),
            }),
        }],
        scope_ids: vec![name("scope/main")],
        condition_ids: vec![name("sim/ready")],
        environment: Environment::Simulation,
        purposes: BTreeSet::from([Purpose::Production]),
    }];
    let runtime = root.join("runtime");
    std::fs::create_dir(&runtime).unwrap();
    let config = Configuration {
        schema: name("rx.host-startup.v1"),
        installation: id(),
        release_digest: Digest::from_bytes([8; 32]),
        host: name("host/sim"),
        bind: "127.0.0.1:0".parse().unwrap(),
        data_directory: root.join("data"),
        runtime_directory: runtime,
        bindings: pinned(
            root,
            "bindings.json",
            &canonical::bytes(&bindings).unwrap(),
            false,
        ),
        backend: Backend::FileSimulation,
        tls: TlsFiles {
            certificate: pinned(root, "server.pem", cert.pem().as_bytes(), false),
            key: pinned(root, "server.key", key.serialize_pem().as_bytes(), true),
            ca: pinned(root, "ca.pem", ca.pem().as_bytes(), false),
        },
        allowed_platform_certificates: BTreeMap::from([(
            rx_package::content_digest(client.der().as_ref()),
            name("platform"),
        )]),
        publisher: None,
        publication_drain_ms: Counter(0),
    };
    let path = root.join("startup.json");
    std::fs::write(&path, canonical::bytes(&config).unwrap()).unwrap();
    (
        dir,
        path,
        ManualClock {
            clock_id: "test/service-clock".into(),
            ticks: Arc::new(AtomicU64::new(1000)),
        },
    )
}
pub async fn status(path: &Path, phase: &str) -> serde_json::Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(bytes) = std::fs::read(path) {
                let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                if value["phase"] == phase {
                    return value;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
