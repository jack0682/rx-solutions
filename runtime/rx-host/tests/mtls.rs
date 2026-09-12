use rx_domain::{intent::*, types::*};
use rx_host::{
    rpc::{Configuration, RpcHost, TlsMaterial, base_manifest_hash, cell_manifest_hash},
    simulation::*,
    *,
};
use rx_protocol::{base, cell};
use sha2::Digest as _;
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicU64},
    time::Duration,
};
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity as TlsIdentity};

fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn intent() -> Intent {
    Intent {
        kind: Kind::EnsureState,
        target: name("sim/chuck"),
        profile_digest: Digest::from_bytes([1; 32]),
        site_config_digest: Digest::from_bytes([2; 32]),
        calibration_digests: vec![],
        resource_set: vec![name("sim/controller")],
        execution_timeout_ms: Counter(5000),
        prepare_validity_ms: Counter(1000),
        completion_rule: name("sim/closed"),
        cancel_rule: name("sim/stop"),
        body: Body::Predicate(PredicateGoal {
            predicate_id: name("chuck.closed"),
            target: TypedValue::Boolean(true),
            settle_ms: Counter(0),
        }),
    }
}
fn binding() -> Binding {
    Binding {
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
        qualification: Id::new("55555555-5555-4555-8555-555555555555").unwrap(),
        qualification_revision: Counter(1),
        allowed_intents: vec![intent()],
        scope_ids: vec![name("scope/main")],
        condition_ids: vec![name("sim/ready")],
        environment: Environment::Simulation,
        purposes: [Purpose::Production, Purpose::Setup].into_iter().collect(),
    }
}

struct Fixture {
    directory: tempfile::TempDir,
    address: String,
    ca: String,
    stranger: TlsIdentity,
    channel: Channel,
    installation: Id,
    release: Digest,
    stop: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<std::result::Result<(), tonic::transport::Error>>,
}
impl Fixture {
    async fn start() -> Self {
        use rcgen::*;
        let mut params = CertificateParams::new(vec![]).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let ca = CertifiedIssuer::self_signed(params, KeyPair::generate().unwrap()).unwrap();
        let leaf = |label: &str, usage: ExtendedKeyUsagePurpose| {
            let key = KeyPair::generate().unwrap();
            let mut params = CertificateParams::new(vec![label.into()]).unwrap();
            params.extended_key_usages = vec![usage];
            params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
            let certificate = params.signed_by(&key, &ca).unwrap();
            (certificate, key)
        };
        let (server, server_key) = leaf("localhost", ExtendedKeyUsagePurpose::ServerAuth);
        let (client, client_key) = leaf("platform", ExtendedKeyUsagePurpose::ClientAuth);
        let (stranger, stranger_key) = leaf("stranger", ExtendedKeyUsagePurpose::ClientAuth);
        let fingerprint = Digest::from_bytes(sha2::Sha256::digest(client.der().as_ref()).into());
        let identity = TlsIdentity::from_pem(client.pem(), client_key.serialize_pem());
        let stranger = TlsIdentity::from_pem(stranger.pem(), stranger_key.serialize_pem());
        let directory = tempfile::tempdir().unwrap();
        let clock = ManualClock {
            clock_id: "simulation/boottime".into(),
            ticks: Arc::new(AtomicU64::new(1000)),
        };
        let native = FileDevice::open(directory.path().join("device"), clock.clone()).unwrap();
        let host = Arc::new(
            Host::open(
                directory.path().join("host.db"),
                native,
                clock,
                vec![binding()],
            )
            .unwrap(),
        );
        let installation = id();
        let release = Digest::from_bytes([9; 32]);
        let server_adapter = RpcHost::new(
            host,
            Configuration {
                installation: installation.clone(),
                release_digest: release,
                allowed_certificates: BTreeMap::from([(fingerprint, name("platform"))]),
            },
        )
        .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("https://{}", listener.local_addr().unwrap());
        let (stop, stopping) = tokio::sync::oneshot::channel();
        let ca = ca.pem();
        let task = tokio::spawn(server_adapter.serve(
            listener,
            TlsMaterial {
                server_certificate_pem: server.pem().into_bytes(),
                server_key_pem: server_key.serialize_pem().into_bytes(),
                client_ca_pem: ca.clone().into_bytes(),
            },
            async {
                let _ = stopping.await;
            },
        ));
        let channel = connect(&address, &ca, Some(identity.clone()))
            .await
            .unwrap();
        Self {
            directory,
            address,
            ca,
            stranger,
            channel,
            installation,
            release,
            stop,
            task,
        }
    }
    fn hello(&self) -> base::PeerHello {
        base::PeerHello {
            peer_id: "platform".into(),
            role: base::Role::Platform as i32,
            boot_id: id().to_string(),
            installation_id: self.installation.to_string(),
            store_generation: id().to_string(),
            supported_versions: vec![base::Version {
                major: 1,
                minor: 0,
                schema_hash: base_manifest_hash(),
            }],
            release_digest: self.release.as_bytes().to_vec(),
            journal_id: None,
            last_seq: None,
            shared_clock_id: "simulation/boottime".into(),
        }
    }
    async fn session(&self) -> base::Session {
        base::session_service_client::SessionServiceClient::new(self.channel.clone())
            .open(self.hello())
            .await
            .unwrap()
            .into_inner()
    }
    async fn cell_open(&self, session: &base::Session) {
        cell::cell_service_client::CellServiceClient::new(self.channel.clone())
            .open(cell::CellHello {
                base_manifest_hash: base_manifest_hash(),
                cell_manifest_hash: cell_manifest_hash(),
                peer_id: "platform".into(),
                base_session_id: session.session_id.clone(),
                cell_definition_digest: binding().definition.sha256.as_bytes().to_vec(),
                shared_clock_id: "simulation/boottime".into(),
            })
            .await
            .unwrap();
    }
    async fn finish(self) {
        let _ = self.stop.send(());
        drop(self.channel);
        tokio::time::timeout(Duration::from_secs(5), self.task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}
async fn connect(
    address: &str,
    ca: &str,
    identity: Option<TlsIdentity>,
) -> std::result::Result<Channel, tonic::transport::Error> {
    let mut tls = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(ca))
        .domain_name("localhost");
    if let Some(identity) = identity {
        tls = tls.identity(identity);
    }
    Channel::from_shared(address.to_string())
        .unwrap()
        .tls_config(tls)?
        .timeout(Duration::from_secs(3))
        .connect()
        .await
}
fn context(session: &base::Session) -> base::CallContext {
    base::CallContext {
        session_id: session.session_id.clone(),
        call_id: id().to_string(),
        request_key: Some(id().to_string()),
        expected_revision: None,
    }
}
fn cell_call(context: base::CallContext) -> cell::CellCall {
    cell::CellCall {
        context: Some(context),
        cell_id: "cell/sim".into(),
        expected_cell_revision: None,
    }
}
async fn prepare(
    f: &Fixture,
    session: &base::Session,
) -> (cell::HostPrepareRequest, base::Receipt) {
    let mut base_host = base::host_service_client::HostServiceClient::new(f.channel.clone());
    let grant = base_host
        .acquire_grant(base::GrantRequest {
            context: Some(context(session)),
            resource_set: vec!["sim/controller".into()],
            fence: 1,
            owner_id: "platform".into(),
            requested_ttl_ms: 1000,
        })
        .await
        .unwrap()
        .into_inner();
    let mut cell_host =
        cell::cell_host_service_client::CellHostServiceClient::new(f.channel.clone());
    let inspect = cell_host
        .inspect(cell_call(context(session)))
        .await
        .unwrap()
        .into_inner();
    cell_host
        .arm_cell(cell::HostArmCellRequest {
            call: Some(cell_call(context(session))),
            attempt_id: id().to_string(),
            target: inspect.cell.clone(),
            clearance_ids: vec![],
            block_ids_to_clear: vec![],
        })
        .await
        .unwrap();
    let operation = id();
    let intent = intent();
    let digest = intent.digest().unwrap();
    let proto_intent =
        rx_protocol::json::from_slice(&rx_domain::canonical::bytes(&intent).unwrap()).unwrap();
    let call = context(session);
    let expires = base::TimePoint {
        clock_id: "simulation/boottime".into(),
        ticks_ns: 500_001_000,
    };
    let permit = cell::DispatchPermit {
        permit_id: id().to_string(),
        operation_id: operation.to_string(),
        intent_digest: digest.as_bytes().to_vec(),
        cell: inspect.cell.clone(),
        envelope_digest: binding().envelope.sha256.as_bytes().to_vec(),
        qualification_id: binding().qualification.to_string(),
        qualification_revision: 1,
        purpose: cell::Purpose::Production as i32,
        parent: Some(cell::PermitParent {
            value: Some(cell::permit_parent::Value::MandateId(id().to_string())),
        }),
        grant: Some(grant.clone()),
        host_boot_id: inspect.host_boot_id,
        issued_at: Some(base::TimePoint {
            clock_id: "simulation/boottime".into(),
            ticks_ns: 1000,
        }),
        expires_at: Some(expires.clone()),
        conditions: vec![cell::ConditionEvaluation {
            condition_id: "sim/ready".into(),
            condition_revision: 1,
            verdict: cell::Verdict::Pass as i32,
            reason: cell::CellReason::None as i32,
            evidence_ids: vec![id().to_string()],
            evaluated_at: Some(base::TimePoint {
                clock_id: "simulation/boottime".into(),
                ticks_ns: 1000,
            }),
            valid_until: Some(expires),
            cell: inspect.cell,
        }],
        state: cell::PermitState::Issued as i32,
    };
    let request = cell::HostPrepareRequest {
        call: Some(cell_call(call.clone())),
        base_request: Some(base::PrepareOperation {
            context: Some(call),
            operation_id: operation.to_string(),
            intent: Some(proto_intent),
            intent_digest: digest.as_bytes().to_vec(),
            grant: Some(grant),
        }),
        permit: Some(permit),
    };
    let receipt = cell_host
        .prepare(request.clone())
        .await
        .unwrap()
        .into_inner();
    (request, receipt)
}

#[tokio::test]
async fn mtls_negotiation_prepare_authorize_and_evidence_roundtrip() {
    let f = Fixture::start().await;
    let session = f.session().await;
    f.cell_open(&session).await;
    let (prepared, receipt) = prepare(&f, &session).await;
    assert_eq!(receipt.stage, base::ReceiptStage::HostPrepared as i32);
    assert_eq!(
        FileDevice::effects(f.directory.path().join("device"))
            .unwrap()
            .len(),
        0
    );
    let base = prepared.base_request.unwrap();
    let call_context = context(&session);
    let authorize = cell::HostAuthorizeRequest {
        call: Some(cell_call(call_context.clone())),
        base_request: Some(base::AuthorizeDispatch {
            context: Some(call_context),
            operation_id: base.operation_id.clone(),
            invocation_id: receipt.invocation_id.unwrap(),
            intent_digest: base.intent_digest,
            grant: base.grant,
        }),
        permit: prepared.permit,
    };
    let mut client = cell::cell_host_service_client::CellHostServiceClient::new(f.channel.clone());
    let result = client
        .authorize(authorize.clone())
        .await
        .unwrap()
        .into_inner();
    let duplicate = client.authorize(authorize).await.unwrap().into_inner();
    assert_eq!(result.journal_seq, duplicate.journal_seq);
    assert_eq!(
        result.host_state,
        Some(base::HostReceiptState::ResultCaptured as i32)
    );
    assert!(result.operation_revision.is_none());
    assert_ne!(result.stage, base::ReceiptStage::ResultRecorded as i32);
    let evidence = base::host_service_client::HostServiceClient::new(f.channel.clone())
        .reconcile(base::OperationRef {
            context: Some(context(&session)),
            operation_id: base.operation_id,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(evidence.records.len(), 1);
    assert_eq!(evidence.first_seq, 1);
    assert_eq!(
        FileDevice::effects(f.directory.path().join("device"))
            .unwrap()
            .len(),
        1
    );
    drop(client);
    f.finish().await;
}

#[tokio::test]
async fn valid_ca_without_registered_fingerprint_cannot_impersonate_platform() {
    let f = Fixture::start().await;
    let stranger = connect(&f.address, &f.ca, Some(f.stranger.clone()))
        .await
        .unwrap();
    let error = base::session_service_client::SessionServiceClient::new(stranger)
        .open(f.hello())
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::PermissionDenied);
    let without = connect(&f.address, &f.ca, None).await;
    if let Ok(channel) = without {
        assert!(
            base::session_service_client::SessionServiceClient::new(channel)
                .open(f.hello())
                .await
                .is_err()
        );
    }
    assert!(
        FileDevice::effects(f.directory.path().join("device"))
            .unwrap()
            .is_empty()
    );
    f.finish().await;
}

#[tokio::test]
async fn cell_manifest_and_session_are_mandatory_before_grant() {
    let f = Fixture::start().await;
    let mut hello = f.hello();
    hello.supported_versions[0].schema_hash = vec![0; 32];
    assert!(
        base::session_service_client::SessionServiceClient::new(f.channel.clone())
            .open(hello)
            .await
            .is_err()
    );
    let session = f.session().await;
    let error = base::host_service_client::HostServiceClient::new(f.channel.clone())
        .acquire_grant(base::GrantRequest {
            context: Some(context(&session)),
            resource_set: vec!["sim/controller".into()],
            fence: 1,
            owner_id: "platform".into(),
            requested_ttl_ms: 1000,
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::FailedPrecondition);
    let error = cell::cell_service_client::CellServiceClient::new(f.channel.clone())
        .open(cell::CellHello {
            base_manifest_hash: base_manifest_hash(),
            cell_manifest_hash: vec![0; 32],
            peer_id: "platform".into(),
            base_session_id: session.session_id,
            cell_definition_digest: binding().definition.sha256.as_bytes().to_vec(),
            shared_clock_id: "simulation/boottime".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::FailedPrecondition);
    f.finish().await;
}

#[tokio::test]
async fn changed_content_under_rpc_key_is_rejected_and_legacy_dispatch_cannot_bypass_cell() {
    let f = Fixture::start().await;
    let session = f.session().await;
    f.cell_open(&session).await;
    let (request, _) = prepare(&f, &session).await;
    let mut changed = request.clone();
    changed.permit.as_mut().unwrap().permit_id = id().to_string();
    let error = cell::cell_host_service_client::CellHostServiceClient::new(f.channel.clone())
        .prepare(changed)
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::AlreadyExists);
    let error = base::host_service_client::HostServiceClient::new(f.channel.clone())
        .prepare_operation(request.base_request.unwrap())
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::FailedPrecondition);
    assert!(
        FileDevice::effects(f.directory.path().join("device"))
            .unwrap()
            .is_empty()
    );
    f.finish().await;
}

#[tokio::test]
async fn concurrent_hello_retries_produce_one_usable_session() {
    let f = Fixture::start().await;
    let hello = f.hello();
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let channel = f.channel.clone();
        let hello = hello.clone();
        tasks.push(tokio::spawn(async move {
            base::session_service_client::SessionServiceClient::new(channel)
                .open(hello)
                .await
                .unwrap()
                .into_inner()
        }));
    }
    let mut sessions = Vec::new();
    for task in tasks {
        sessions.push(task.await.unwrap());
    }
    assert!(
        sessions
            .iter()
            .all(|s| s.session_id == sessions[0].session_id)
    );
    f.cell_open(&sessions[0]).await;
    prepare(&f, &sessions[0]).await;
    f.finish().await;
}
