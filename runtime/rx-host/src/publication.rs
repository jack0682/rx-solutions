//! Ordered Host evidence publication with durable per-destination acknowledgment.
//! This module contains no native submission, grant, permit, cancellation or retry of production.
use crate::{
    Host,
    model::{Clock, EvidenceRecord, HostError},
    native::NativeAdapter,
    rpc::{base_manifest_hash, cell_manifest_hash, evidence_view},
};
use prost::Message;
use rx_domain::types::*;
use rx_protocol::{base, cell};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};
use tonic::{
    Code, Status,
    transport::{Certificate, Channel, ClientTlsConfig, Identity},
};
pub const CONTROL_VIEW: &str = "site-cell-control-v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Destination {
    pub platform: Name,
    pub installation: Id,
    pub store_generation: Id,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PublicationCursor {
    pub journal: Id,
    pub through: Counter,
    pub platform_sequence: Counter,
}
pub struct PublicationChunk {
    pub host: Name,
    pub boot: Id,
    pub journal: Id,
    pub first: Counter,
    pub tail: Counter,
    pub records: Vec<EvidenceRecord>,
    pub cursor: PublicationCursor,
}
pub struct PublicationAck {
    pub installation: Id,
    pub store_generation: Id,
    pub view_id: String,
    pub journal: Id,
    pub through: Counter,
    pub platform_sequence: Counter,
}
pub struct Endpoint {
    pub uri: String,
    pub server_name: String,
    pub server_ca_pem: Vec<u8>,
    pub client_certificate_pem: Vec<u8>,
    pub client_key_pem: Vec<u8>,
}
pub struct Configuration {
    pub endpoint: Endpoint,
    pub destination: Destination,
    pub release_digest: Digest,
}

#[derive(Clone, Debug)]
pub enum StatusView {
    Connecting,
    Idle(PublicationCursor),
    Published(PublicationCursor),
    Retrying(Code),
    Blocked(Code),
    Stopped,
}
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("publisher local journal failed: {0}")]
    Local(#[from] HostError),
    #[error("publisher transport: {0}")]
    Rpc(#[from] Status),
    #[error("publisher TLS connection failed: {0}")]
    Connect(#[from] tonic::transport::Error),
    #[error("publisher worker failed")]
    Worker,
}
struct Connection {
    channel: Channel,
    session: base::Session,
    probe: bool,
    max_batch: usize,
    max_bytes: usize,
}
pub struct Publisher<N, C> {
    host: Arc<Host<N, C>>,
    configuration: Configuration,
    connection: Option<Connection>,
    status: tokio::sync::watch::Sender<StatusView>,
}
impl<N: NativeAdapter + 'static, C: Clock + 'static> Publisher<N, C> {
    pub fn new(host: Arc<Host<N, C>>, configuration: Configuration) -> Self {
        let (status, _) = tokio::sync::watch::channel(StatusView::Connecting);
        Self {
            host,
            configuration,
            connection: None,
            status,
        }
    }
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<StatusView> {
        self.status.subscribe()
    }
    async fn chunk(&self) -> Result<PublicationChunk, Error> {
        let host = self.host.clone();
        let destination = self.configuration.destination.clone();
        tokio::task::spawn_blocking(move || host.publication_chunk(&destination))
            .await
            .map_err(|_| Error::Worker)?
            .map_err(Error::Local)
    }
    async fn connect(&mut self, chunk: &PublicationChunk) -> Result<(), Error> {
        self.status.send_replace(StatusView::Connecting);
        let endpoint = &self.configuration.endpoint;
        let channel = Channel::from_shared(endpoint.uri.clone())
            .map_err(|_| Status::invalid_argument("publisher endpoint URI"))?
            .tls_config(
                ClientTlsConfig::new()
                    .domain_name(&endpoint.server_name)
                    .ca_certificate(Certificate::from_pem(endpoint.server_ca_pem.clone()))
                    .identity(Identity::from_pem(
                        endpoint.client_certificate_pem.clone(),
                        endpoint.client_key_pem.clone(),
                    )),
            )?
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(3))
            .connect()
            .await?;
        let session = base::session_service_client::SessionServiceClient::new(channel.clone())
            .open(base::PeerHello {
                peer_id: chunk.host.to_string(),
                role: base::Role::Host as i32,
                boot_id: chunk.boot.to_string(),
                installation_id: self.configuration.destination.installation.to_string(),
                store_generation: self.configuration.destination.store_generation.to_string(),
                supported_versions: vec![base::Version {
                    major: 1,
                    minor: 0,
                    schema_hash: base_manifest_hash(),
                }],
                release_digest: self.configuration.release_digest.as_bytes().to_vec(),
                journal_id: Some(chunk.journal.to_string()),
                last_seq: Some(chunk.tail.0),
                shared_clock_id: self.host.current_time().clock_id,
            })
            .await?
            .into_inner();
        if session.peer_id != chunk.host.as_str()
            || session.boot_id != chunk.boot.as_str()
            || session.selected_version.as_ref().is_none_or(|v| {
                v.major != 1 || v.minor != 0 || v.schema_hash != base_manifest_hash()
            })
            || session
                .required_features
                .iter()
                .any(|f| !["rx.cell.v1", "strict-wire-v1"].contains(&f.as_str()))
        {
            return Err(
                Status::failed_precondition("producer session negotiation mismatch").into(),
            );
        }
        parse_id(&session.session_id)?;
        let limits = session
            .limits
            .as_ref()
            .ok_or_else(|| Status::failed_precondition("peer limits missing"))?;
        if limits.max_batch_records == 0
            || limits.max_message_bytes < 1024
            || limits.max_inflight == 0
        {
            return Err(Status::failed_precondition("invalid peer limits").into());
        }
        let max_batch = (limits.max_batch_records as usize).min(128);
        let max_bytes = (limits.max_message_bytes as usize).min(1_048_576);
        let host = self.host.clone();
        let bindings = tokio::task::spawn_blocking(move || host.bindings())
            .await
            .map_err(|_| Error::Worker)??;
        for binding in bindings {
            let negotiated = cell::cell_service_client::CellServiceClient::new(channel.clone())
                .open(cell::CellHello {
                    base_manifest_hash: base_manifest_hash(),
                    cell_manifest_hash: cell_manifest_hash(),
                    peer_id: chunk.host.to_string(),
                    base_session_id: session.session_id.clone(),
                    cell_definition_digest: binding.definition.sha256.as_bytes().to_vec(),
                    shared_clock_id: self.host.current_time().clock_id,
                })
                .await?
                .into_inner();
            if negotiated.base_session_id != session.session_id
                || negotiated.peer_id != chunk.host.as_str()
                || negotiated.manifest_hash != cell_manifest_hash()
            {
                return Err(
                    Status::failed_precondition("cell evidence negotiation mismatch").into(),
                );
            }
            parse_id(&negotiated.session_id)?;
        }
        self.connection = Some(Connection {
            channel,
            session,
            probe: true,
            max_batch,
            max_bytes,
        });
        Ok(())
    }
    /// One bounded publication transaction. Reconnection probes the accepted prefix before sending data.
    pub async fn tick(&mut self) -> Result<PublicationCursor, Error> {
        let chunk = self.chunk().await?;
        if self.connection.is_none() {
            self.connect(&chunk).await?;
        }
        let connection = self.connection.as_mut().expect("connected publisher");
        if !connection.probe && chunk.records.is_empty() {
            self.status
                .send_replace(StatusView::Idle(chunk.cursor.clone()));
            return Ok(chunk.cursor);
        }
        let records = if connection.probe {
            vec![]
        } else {
            chunk
                .records
                .into_iter()
                .take(connection.max_batch)
                .map(evidence_view)
                .collect()
        };
        let mut batch = base::EvidenceBatch {
            context: Some(base::CallContext {
                session_id: connection.session.session_id.clone(),
                call_id: crate::journal::id().to_string(),
                request_key: None,
                expected_revision: None,
            }),
            producer_journal_id: chunk.journal.to_string(),
            first_seq: chunk.first.0,
            records,
        };
        while batch.encoded_len() > connection.max_bytes && !batch.records.is_empty() {
            batch.records.pop();
        }
        if batch.encoded_len() > connection.max_bytes
            || (!connection.probe && batch.records.is_empty())
        {
            return Err(Error::Local(HostError::Invalid(
                "one evidence record exceeds negotiated transport size".into(),
            )));
        }
        let minimum_ack = chunk.first.0 - 1 + batch.records.len() as u64;
        let response =
            base::evidence_service_client::EvidenceServiceClient::new(connection.channel.clone())
                .publish(batch)
                .await;
        let ack = match response {
            Ok(response) => response.into_inner(),
            Err(error) => {
                if error.code() == Code::Unauthenticated {
                    self.connection = None;
                }
                return Err(error.into());
            }
        };
        if ack.through_seq < minimum_ack {
            return Err(Status::data_loss("partial batch acknowledgment").into());
        }
        let platform = ack
            .platform_cursor
            .ok_or_else(|| Status::data_loss("durable ack cursor missing"))?;
        let ack = PublicationAck {
            installation: parse_id(&platform.installation_id)?,
            store_generation: parse_id(&platform.store_generation)?,
            view_id: platform.view_id,
            journal: parse_id(&ack.producer_journal_id)?,
            through: Counter(ack.through_seq),
            platform_sequence: Counter(platform.seq),
        };
        let host = self.host.clone();
        let destination = self.configuration.destination.clone();
        let cursor =
            tokio::task::spawn_blocking(move || host.acknowledge_publication(&destination, ack))
                .await
                .map_err(|_| Error::Worker)??;
        self.connection.as_mut().expect("connected publisher").probe = false;
        self.status
            .send_replace(StatusView::Published(cursor.clone()));
        Ok(cursor)
    }
    pub async fn run(mut self, mut stop: tokio::sync::watch::Receiver<bool>) -> Result<(), Error> {
        let mut retry = Duration::from_millis(100);
        loop {
            if *stop.borrow() {
                break;
            }
            let result = tokio::select! {result=self.tick()=>result,changed=stop.changed()=>{if changed.is_err() || *stop.borrow() {break;} continue;}};
            let delay = match result {
                Ok(_) => {
                    retry = Duration::from_millis(100);
                    Duration::from_millis(100)
                }
                Err(error) => {
                    let code = match &error {
                        Error::Rpc(status) => status.code(),
                        Error::Connect(_) => Code::Unavailable,
                        _ => Code::DataLoss,
                    };
                    if !matches!(
                        code,
                        Code::Unavailable
                            | Code::DeadlineExceeded
                            | Code::ResourceExhausted
                            | Code::Cancelled
                            | Code::Unauthenticated
                    ) {
                        self.status.send_replace(StatusView::Blocked(code));
                        return Err(error);
                    }
                    self.status.send_replace(StatusView::Retrying(code));
                    let wait = retry;
                    retry = (retry * 2).min(Duration::from_secs(5));
                    wait
                }
            };
            tokio::select! {_=tokio::time::sleep(delay)=>{},changed=stop.changed()=>{if changed.is_err() {break;}}}
        }
        self.status.send_replace(StatusView::Stopped);
        Ok(())
    }
}
fn parse_id(value: &str) -> Result<Id, Status> {
    Id::new(value).map_err(|_| Status::data_loss("invalid acknowledgment identity"))
}
