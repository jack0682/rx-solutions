//! mTLS adapter for the frozen Host contract. It never serves platform business mutations.
mod cell_host;
mod configuration;
mod host_read;
mod host_service;
mod mapping;
mod qualification;
mod session;
use crate::{Host, model::*, native::NativeAdapter};
pub(crate) use mapping::evidence_view;
use mapping::*;
use rx_domain::{
    canonical,
    types::{Digest, Id, Name},
};
use rx_protocol::{base, cell};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};
use tonic::{
    Request as RpcRequest, Response, Status,
    transport::{Certificate, Identity as TlsIdentity, Server, ServerTlsConfig},
};

#[derive(Clone)]
pub struct Configuration {
    pub installation: Id,
    pub release_digest: Digest,
    pub allowed_certificates: BTreeMap<Digest, Name>,
}
pub struct TlsMaterial {
    pub server_certificate_pem: Vec<u8>,
    pub server_key_pem: Vec<u8>,
    pub client_ca_pem: Vec<u8>,
}
#[derive(Clone)]
struct SessionState {
    fingerprint: Digest,
    hello_digest: Digest,
    session: base::Session,
    cells: BTreeMap<Name, cell::CellSession>,
}
struct Shared<N, C> {
    #[cfg(feature = "test-harness")]
    lose_configuration_reply: std::sync::atomic::AtomicBool,
    #[cfg(feature = "test-harness")]
    lose_qualification_reply: std::sync::atomic::AtomicBool,
    host: Arc<Host<N, C>>,
    configuration: Configuration,
    bindings: Vec<Binding>,
    sessions: Mutex<BTreeMap<Id, SessionState>>,
    capacity: Arc<tokio::sync::Semaphore>,
    handshake: Arc<tokio::sync::Mutex<()>>,
}
pub struct RpcHost<N, C> {
    shared: Arc<Shared<N, C>>,
}
impl<N, C> Clone for RpcHost<N, C> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
        }
    }
}
impl<N: NativeAdapter + 'static, C: Clock + 'static> RpcHost<N, C> {
    pub fn new(host: Arc<Host<N, C>>, configuration: Configuration) -> crate::Result<Self> {
        let bindings = host.bindings()?;
        if configuration.allowed_certificates.is_empty() {
            return Err(HostError::Forbidden);
        }
        Ok(Self {
            shared: Arc::new(Shared {
                #[cfg(feature = "test-harness")]
                lose_configuration_reply: std::sync::atomic::AtomicBool::new(false),
                #[cfg(feature = "test-harness")]
                lose_qualification_reply: std::sync::atomic::AtomicBool::new(false),
                host,
                configuration,
                bindings,
                sessions: Mutex::new(BTreeMap::new()),
                capacity: Arc::new(tokio::sync::Semaphore::new(32)),
                handshake: Arc::new(tokio::sync::Mutex::new(())),
            }),
        })
    }
    #[cfg(feature = "test-harness")]
    pub fn lose_next_configuration_reply(&self) {
        self.shared
            .lose_configuration_reply
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
    #[cfg(feature = "test-harness")]
    pub fn lose_next_qualification_reply(&self) {
        self.shared
            .lose_qualification_reply
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
    pub async fn serve(
        self,
        listener: tokio::net::TcpListener,
        tls: TlsMaterial,
        shutdown: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> std::result::Result<(), tonic::transport::Error> {
        Server::builder()
            .tls_config(
                ServerTlsConfig::new()
                    .identity(TlsIdentity::from_pem(
                        tls.server_certificate_pem,
                        tls.server_key_pem,
                    ))
                    .client_ca_root(Certificate::from_pem(tls.client_ca_pem))
                    .client_auth_optional(false),
            )?
            .add_service(
                base::session_service_server::SessionServiceServer::new(self.clone())
                    .max_decoding_message_size(1_048_576)
                    .max_encoding_message_size(1_048_576),
            )
            .add_service(
                cell::cell_service_server::CellServiceServer::new(self.clone())
                    .max_decoding_message_size(1_048_576)
                    .max_encoding_message_size(1_048_576),
            )
            .add_service(
                base::host_service_server::HostServiceServer::new(self.clone())
                    .max_decoding_message_size(1_048_576)
                    .max_encoding_message_size(1_048_576),
            )
            .add_service(
                rx_protocol::host_read::host_read_service_server::HostReadServiceServer::new(
                    self.clone(),
                )
                .max_decoding_message_size(1_048_576)
                .max_encoding_message_size(1_048_576),
            )
            .add_service(rx_protocol::host_qualification::host_qualification_service_server::HostQualificationServiceServer::new(self.clone()).max_decoding_message_size(1_048_576).max_encoding_message_size(1_048_576))
            .add_service(
                rx_protocol::host_configuration::host_configuration_service_server::HostConfigurationServiceServer::new(self.clone()).max_decoding_message_size(1_048_576).max_encoding_message_size(1_048_576),
            )
            .add_service(
                cell::cell_host_service_server::CellHostServiceServer::new(self)
                    .max_decoding_message_size(1_048_576)
                    .max_encoding_message_size(1_048_576),
            )
            .serve_with_incoming_shutdown(
                tokio_stream::wrappers::TcpListenerStream::new(listener),
                shutdown,
            )
            .await
    }
    fn certificate<T>(
        &self,
        request: &RpcRequest<T>,
    ) -> std::result::Result<(Digest, Name), Status> {
        use sha2::Digest as _;
        use tonic::transport::server::{TcpConnectInfo, TlsConnectInfo};
        let connection = request
            .extensions()
            .get::<TlsConnectInfo<TcpConnectInfo>>()
            .ok_or_else(|| Status::unauthenticated("mTLS required"))?;
        let certificates = connection
            .peer_certs()
            .ok_or_else(|| Status::unauthenticated("client certificate required"))?;
        let leaf = certificates
            .first()
            .ok_or_else(|| Status::unauthenticated("client certificate missing"))?;
        let fingerprint = Digest::from_bytes(sha2::Sha256::digest(leaf.as_ref()).into());
        let peer = self
            .shared
            .configuration
            .allowed_certificates
            .get(&fingerprint)
            .cloned()
            .ok_or_else(|| Status::permission_denied("certificate is not registered"))?;
        Ok((fingerprint, peer))
    }
    fn caller<T>(
        &self,
        request: &RpcRequest<T>,
        context: Option<&base::CallContext>,
        cell: Option<&str>,
        mutation: bool,
    ) -> std::result::Result<Caller, Status> {
        let (fingerprint, peer) = self.certificate(request)?;
        let context = context.ok_or_else(|| Status::invalid_argument("CallContext required"))?;
        parse_id(&context.call_id)?;
        let session = parse_id(&context.session_id)?;
        if mutation {
            parse_id(
                context
                    .request_key
                    .as_deref()
                    .ok_or_else(|| Status::invalid_argument("request key required"))?,
            )?;
        }
        let sessions = self
            .shared
            .sessions
            .lock()
            .map_err(|_| Status::unavailable("session registry unavailable"))?;
        let record = sessions
            .get(&session)
            .ok_or_else(|| Status::unauthenticated("unknown session"))?;
        if record.fingerprint != fingerprint || record.session.peer_id != peer.as_str() {
            return Err(Status::unauthenticated("session/certificate mismatch"));
        }
        if let Some(cell) = cell
            && !record.cells.contains_key(&parse_name(cell)?)
        {
            return Err(Status::failed_precondition(
                "UPGRADE_REQUIRED: cell contract not negotiated",
            ));
        }
        Ok(Caller { peer, session })
    }
    fn cell_caller<T>(
        &self,
        request: &RpcRequest<T>,
        call: Option<&cell::CellCall>,
        mutation: bool,
    ) -> std::result::Result<Caller, Status> {
        let call = call.ok_or_else(|| Status::invalid_argument("CellCall required"))?;
        if call
            .context
            .as_ref()
            .is_some_and(|c| c.expected_revision.is_some())
        {
            return Err(Status::invalid_argument(
                "base revision must be absent in CellCall",
            ));
        }
        self.caller(
            request,
            call.context.as_ref(),
            Some(&call.cell_id),
            mutation,
        )
    }
    async fn run<T: Send + 'static>(
        &self,
        operation: impl FnOnce(Arc<Host<N, C>>) -> std::result::Result<T, Status> + Send + 'static,
    ) -> std::result::Result<T, Status> {
        let permit = self
            .shared
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("Host request capacity reached"))?;
        let host = self.shared.host.clone();
        tokio::task::spawn_blocking(move || {
            let _capacity = permit;
            operation(host)
        })
        .await
        .map_err(|_| Status::unavailable("Host worker failed; inspect the original operation"))?
    }
}
pub fn base_manifest_hash() -> Vec<u8> {
    manifest_hash(include_str!(
        "../../../../sdk/spec/contracts/v1.0/protocol_manifest.json"
    ))
}
pub fn cell_manifest_hash() -> Vec<u8> {
    manifest_hash(include_str!(
        "../../../../sdk/spec/cell_operations/v1.0/protocol_manifest.json"
    ))
}
fn manifest_hash(text: &str) -> Vec<u8> {
    use sha2::Digest as _;
    let value: serde_json::Value = serde_json::from_str(text).expect("embedded manifest");
    sha2::Sha256::digest(canonical::bytes(&value).expect("manifest canonicalization")).to_vec()
}
fn wrong_owner() -> Status {
    Status::failed_precondition("This business method belongs to the platform, not a Device Host")
}

pub fn validate_tls(tls: &TlsMaterial) -> std::result::Result<(), tonic::transport::Error> {
    Server::builder()
        .tls_config(
            ServerTlsConfig::new()
                .identity(TlsIdentity::from_pem(
                    &tls.server_certificate_pem,
                    &tls.server_key_pem,
                ))
                .client_ca_root(Certificate::from_pem(&tls.client_ca_pem))
                .client_auth_optional(false),
        )
        .map(|_| ())
}
