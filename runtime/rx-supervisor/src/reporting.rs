//! Optional mTLS publication of attributed registry snapshots. No process or work authority.
use rx_domain::{canonical, resident_reporting::*, types::*};
use rx_protocol::resident_reporting as wire;
use serde::{Serialize, de::DeserializeOwned};
use sha2::Digest as _;
use std::time::Duration;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid reporting context: {0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Transport(#[from] tonic::transport::Error),
    #[error(transparent)]
    Rpc(#[from] tonic::Status),
}
pub type Result<T> = std::result::Result<T, Error>;

/// Installation-selected TLS material. Credentials are never serialized by this client.
pub struct Connection {
    pub endpoint: String,
    pub server_name: String,
    pub ca_pem: Vec<u8>,
    pub certificate_pem: Vec<u8>,
    pub private_key_pem: Vec<u8>,
    pub principal: Name,
    pub installation: Id,
    pub store_generation: Id,
    pub shared_clock_id: String,
    pub release_digest: Digest,
}

pub struct VerifiedScope(Scope);
impl VerifiedScope {
    pub fn view(&self) -> &Scope {
        &self.0
    }
}

pub struct Client {
    peer: Peer,
    transport: wire::resident_reporting_service_client::ResidentReportingServiceClient<Channel>,
}

fn binding_hash() -> Vec<u8> {
    sha2::Sha256::digest(include_bytes!(
        "../../../sdk/spec/resident-reporting/v1/binding.json"
    ))
    .to_vec()
}

fn decode<T: DeserializeOwned + Serialize>(payload: wire::Payload, schema: &str) -> Result<T> {
    if payload.schema != schema
        || payload.data.len() > 65536
        || payload.sha256 != sha2::Sha256::digest(&payload.data).as_slice()
    {
        return Err(Error::Invalid("response schema, size or digest differs"));
    }
    let value =
        canonical::decode_json(&payload.data).map_err(|_| Error::Invalid("response value"))?;
    if canonical::bytes(&value).map_err(|_| Error::Invalid("response value"))? != payload.data {
        return Err(Error::Invalid("response is not canonical"));
    }
    Ok(value)
}

impl Client {
    pub async fn connect(connection: Connection) -> Result<Self> {
        if !connection.endpoint.starts_with("https://")
            || connection.server_name.is_empty()
            || connection.shared_clock_id.is_empty()
        {
            return Err(Error::Invalid(
                "TLS endpoint, server name and clock required",
            ));
        }
        let endpoint = Endpoint::from_shared(connection.endpoint)?;
        if endpoint
            .uri()
            .authority()
            .is_none_or(|a| a.as_str().contains('@'))
            || endpoint.uri().path() != "/"
            || endpoint.uri().query().is_some()
        {
            return Err(Error::Invalid("exact TLS service origin required"));
        }
        let channel = endpoint
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(2))
            .tls_config(
                ClientTlsConfig::new()
                    .domain_name(connection.server_name)
                    .ca_certificate(Certificate::from_pem(connection.ca_pem))
                    .identity(Identity::from_pem(
                        connection.certificate_pem,
                        connection.private_key_pem,
                    )),
            )?
            .connect()
            .await?;
        let mut transport =
            wire::resident_reporting_service_client::ResidentReportingServiceClient::new(channel)
                .max_decoding_message_size(1_048_576)
                .max_encoding_message_size(1_048_576);
        let boot =
            Id::new(uuid::Uuid::new_v4().to_string()).map_err(|_| Error::Invalid("peer boot"))?;
        let payload = transport
            .open(wire::OpenReporter {
                peer_id: connection.principal.to_string(),
                peer_boot: boot.to_string(),
                installation_id: connection.installation.to_string(),
                store_generation: connection.store_generation.to_string(),
                shared_clock_id: connection.shared_clock_id,
                release_digest: connection.release_digest.as_bytes().to_vec(),
                binding_hash: binding_hash(),
            })
            .await?
            .into_inner();
        let peer: Peer = decode(payload, "rx.resident-reporter.v1")?;
        if peer.principal != connection.principal
            || peer.peer_boot != boot
            || peer.installation != connection.installation
            || peer.store_generation != connection.store_generation
        {
            return Err(Error::Invalid("reporter identity differs"));
        }
        Ok(Self { peer, transport })
    }

    pub fn peer(&self) -> &Peer {
        &self.peer
    }

    pub async fn inspect_scope(
        &mut self,
        id: &Id,
        component: &Id,
        source_registration: &Id,
    ) -> Result<VerifiedScope> {
        let payload = self
            .transport
            .inspect(wire::InspectScope {
                session_id: self.peer.id.to_string(),
                scope_id: id.to_string(),
                binding_hash: binding_hash(),
            })
            .await?
            .into_inner();
        let scope: Scope = decode(payload, "rx.resident-reporting-scope.v1")?;
        if scope.id != *id
            || scope.component != *component
            || scope.source_registration != *source_registration
            || scope.reporter_session != self.peer.id
            || !scope.active
        {
            return Err(Error::Invalid("reporting scope differs"));
        }
        Ok(VerifiedScope(scope))
    }

    /// One transmission. On response loss retain this exact request key/report and retry explicitly.
    pub async fn publish(
        &mut self,
        scope: &VerifiedScope,
        key: &Id,
        report: Report,
    ) -> Result<Receipt> {
        let allowed = &scope.0;
        if allowed.reporter_session != self.peer.id
            || report.scope != allowed.id
            || report.source.registration != allowed.source_registration
            || report.source.registration_revision != allowed.source_revision
            || report.source.catalog != allowed.catalog
            || report.sequence.0 == 0
            || report.pid == Some(0)
            || report.detail.len() > 2048
        {
            return Err(Error::Invalid("report does not match accepted scope"));
        }
        let data = canonical::bytes(&report).map_err(|_| Error::Invalid("report"))?;
        if data.len() > 65536 {
            return Err(Error::Invalid("report exceeds size limit"));
        }
        let payload = self
            .transport
            .publish(wire::PublishReport {
                session_id: self.peer.id.to_string(),
                request_key: key.to_string(),
                payload_sha256: sha2::Sha256::digest(&data).to_vec(),
                payload: data,
                binding_hash: binding_hash(),
            })
            .await?
            .into_inner();
        let receipt: Receipt = decode(payload, "rx.resident-report-receipt.v1")?;
        if receipt.report != report
            || receipt.reporter != self.peer
            || receipt.component != allowed.component
            || receipt.component_revision != allowed.component_revision
        {
            return Err(Error::Invalid("report receipt differs"));
        }
        Ok(receipt)
    }
}

/// Copy a recorded observation without relabelling it as fresh OS evidence or ownership.
pub fn from_execution(
    scope: &VerifiedScope,
    execution: &crate::registration::Execution,
    sequence: Counter,
) -> Result<Report> {
    let allowed = &scope.0;
    if execution.binding.registration != allowed.source_registration
        || execution.binding.registration_revision != allowed.source_revision
        || execution.binding.catalog != allowed.catalog
    {
        return Err(Error::Invalid(
            "registry execution differs from report scope",
        ));
    }
    let observed = &execution.last_observed;
    if sequence.0 == 0 || observed.pid == Some(0) || observed.detail.len() > 2048 {
        return Err(Error::Invalid("registry report bounds"));
    }
    Ok(Report {
        scope: allowed.id.clone(),
        source: execution.binding.clone(),
        sequence,
        state: observed.state,
        pid: observed.pid,
        exit_code: observed.exit_code,
        detail: observed.detail.clone(),
    })
}
