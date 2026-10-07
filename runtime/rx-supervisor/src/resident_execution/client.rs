use super::*;
use crate::registration::Registry;
use rx_protocol::resident_execution as wire;
use rx_storage::SqliteRepository;
use serde::{Serialize, de::DeserializeOwned};
use sha2::Digest as _;
use std::{collections::BTreeSet, time::Duration};
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};
const MAX: usize = 262_144;
fn binding() -> Vec<u8> {
    sha2::Sha256::digest(include_bytes!(
        "../../../../sdk/spec/resident-execution/v1/binding.json"
    ))
    .to_vec()
}
fn decode<T: DeserializeOwned + Serialize>(p: wire::Payload, schema: &str) -> Result<T> {
    if p.schema != schema
        || p.data.len() > MAX
        || p.sha256 != sha2::Sha256::digest(&p.data).as_slice()
    {
        return Err(invalid("resident execution response schema/size/digest"));
    }
    let value = canonical::decode_json(&p.data).map_err(invalid)?;
    if canonical::bytes(&value).map_err(invalid)? != p.data {
        return Err(invalid("canonical execution response required"));
    }
    Ok(value)
}
pub struct Client {
    peer: data::Peer,
    clock: Arc<dyn Clock>,
    transport: wire::resident_execution_service_client::ResidentExecutionServiceClient<Channel>,
    minted: BTreeSet<Id>,
}
impl Client {
    pub async fn connect(
        c: Connection,
        clock: Arc<dyn Clock>,
        registry: &Registry<SqliteRepository>,
    ) -> Result<Self> {
        let now = clock.now()?;
        if !c.endpoint.starts_with("https://")
            || c.server_name.is_empty()
            || c.shared_clock_id != now.clock_id
        {
            return Err(invalid(
                "execution requires TLS and the same actual shared boot clock",
            ));
        }
        let endpoint = Endpoint::from_shared(c.endpoint)?;
        if endpoint
            .uri()
            .authority()
            .is_none_or(|a| a.as_str().contains('@'))
            || endpoint.uri().path() != "/"
            || endpoint.uri().query().is_some()
        {
            return Err(invalid("exact execution TLS origin required"));
        }
        let channel = endpoint
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(2))
            .tls_config(
                ClientTlsConfig::new()
                    .domain_name(c.server_name)
                    .ca_certificate(Certificate::from_pem(c.ca_pem))
                    .identity(Identity::from_pem(c.certificate_pem, c.private_key_pem)),
            )?
            .connect()
            .await?;
        let mut transport =
            wire::resident_execution_service_client::ResidentExecutionServiceClient::new(channel)
                .max_decoding_message_size(1_048_576)
                .max_encoding_message_size(1_048_576);
        let boot = Id::new(uuid::Uuid::new_v4().to_string()).map_err(invalid)?;
        let location = registry.platform_binding()?;
        let peer: data::Peer = decode(
            transport
                .open(wire::OpenSupervisor {
                    peer_id: c.principal.to_string(),
                    peer_boot: boot.to_string(),
                    installation_id: c.installation.to_string(),
                    store_generation: c.store_generation.to_string(),
                    shared_clock_id: now.clock_id,
                    release_digest: c.release_digest.as_bytes().to_vec(),
                    binding_hash: binding(),
                    registry_binding: location.as_bytes().to_vec(),
                })
                .await?
                .into_inner(),
            "rx.resident-execution-peer.v1",
        )?;
        if peer.principal != c.principal
            || peer.peer_boot != boot
            || peer.installation != c.installation
            || peer.store_generation != c.store_generation
            || peer.registry != location
        {
            return Err(invalid("Supervisor peer context differs"));
        }
        Ok(Self {
            peer,
            clock,
            transport,
            minted: BTreeSet::new(),
        })
    }
    pub fn peer(&self) -> &data::Peer {
        &self.peer
    }
    /// Fetch the original P assignment and inspect actual local source records without execution.
    pub async fn investigate(
        &mut self,
        id: &Id,
        catalog: Catalog,
        registry: &mut Registry<SqliteRepository>,
    ) -> Result<SourceInspection> {
        let view = self.inspect(id).await?;
        recovery::inspect(
            view.assignment,
            &self.peer,
            catalog,
            registry,
            self.clock.as_ref(),
        )
    }
    pub async fn inspect(&mut self, id: &Id) -> Result<data::View> {
        let view: data::View = decode(
            self.transport
                .inspect(wire::InspectAssignment {
                    session_id: self.peer.id.to_string(),
                    assignment_id: id.to_string(),
                    binding_hash: binding(),
                })
                .await?
                .into_inner(),
            "rx.resident-execution-view.v1",
        )?;
        if view.assignment.intent.id != *id
            || view.assignment.intent.supervisor != self.peer.principal
        {
            return Err(invalid("assignment identity differs"));
        }
        Ok(view)
    }
    fn mutation(&self, key: &Id, value: &impl Serialize) -> Result<wire::Mutation> {
        let payload = canonical::bytes(value).map_err(invalid)?;
        if payload.len() > MAX {
            return Err(invalid("execution request too large"));
        }
        Ok(wire::Mutation {
            session_id: self.peer.id.to_string(),
            request_key: key.to_string(),
            payload_sha256: sha2::Sha256::digest(&payload).to_vec(),
            payload,
            binding_hash: binding(),
        })
    }
    pub async fn prepare(
        &mut self,
        id: &Id,
        catalog: Catalog,
        registry: &mut Registry<SqliteRepository>,
    ) -> Result<Prepared> {
        let view = self.inspect(id).await?;
        if !matches!(
            view.assignment.phase,
            data::Phase::Proposed | data::Phase::Prepared | data::Phase::Granted
        ) || view.assignment.stop_requested
        {
            return Err(invalid("assignment is not available for preparation"));
        }
        let prepared = Prepared::build(view.assignment.intent, &self.peer, catalog, registry)?;
        let receipt: data::ContentReceipt = if let Some(old) = view.assignment.content {
            old
        } else {
            let request = self.mutation(id, &prepared.offer)?;
            decode(
                self.transport.prepare(request).await?.into_inner(),
                "rx.resident-execution-content.v1",
            )?
        };
        if receipt.preparation != prepared.offer
            || receipt.basis != data::ContentBasis::EnrolledSupervisorVerifiedRelease
        {
            return Err(invalid(
                "preparation receipt differs; do not rebind another incarnation",
            ));
        }
        Ok(prepared)
    }
    /// At most one live capability is returned per assignment/client incarnation.
    pub async fn take_grant(&mut self, prepared: &Prepared) -> Result<LiveGrant> {
        if self.minted.contains(&prepared.intent.id) {
            return Err(invalid("live grant already returned; replay forbidden"));
        }
        let view = self.inspect(&prepared.intent.id).await?;
        if view.assignment.phase != data::Phase::Granted
            || view.assignment.stop_requested
            || !view.peer_current
            || view.assignment.intent != prepared.intent
            || view
                .assignment
                .content
                .as_ref()
                .is_none_or(|c| c.preparation != prepared.offer)
        {
            return Err(invalid(
                "assignment has no current owner-approved start grant",
            ));
        }
        let grant = view
            .assignment
            .grant
            .ok_or_else(|| invalid("start grant absent"))?;
        if grant.peer != self.peer
            || grant.assignment != prepared.intent.id
            || grant.intent_digest != prepared.intent.digest().map_err(invalid)?
            || grant.preparation_digest != prepared.offer.digest().map_err(invalid)?
        {
            return Err(invalid("start grant context differs"));
        }
        let live = LiveGrant::new(grant, prepared, self.clock.clone())?;
        self.minted.insert(prepared.intent.id.clone());
        Ok(live)
    }
    pub async fn observe(
        &mut self,
        key: &Id,
        observation: &data::Observation,
    ) -> Result<data::ObservationReceipt> {
        let request = self.mutation(key, observation)?;
        let receipt: data::ObservationReceipt = decode(
            self.transport.observe(request).await?.into_inner(),
            "rx.resident-execution-observation.v1",
        )?;
        if receipt.observer != self.peer || receipt.observation != *observation {
            return Err(invalid("execution observation receipt differs"));
        }
        Ok(receipt)
    }
}
