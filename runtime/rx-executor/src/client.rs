mod assignment;
mod preparation;
mod production;
pub use assignment::ValidatedAssignment;
pub(crate) use preparation::Preparation;
pub(crate) use production::ProductionBasis;
pub use production::{CompletedVisit, ValidatedProduction};
use rx_domain::{canonical, types::*};
use rx_process_contract::{ResolvedProcess, execution::*, execution_validation, frontier};
use rx_protocol::{base, cell, executor as wire};
use serde::de::DeserializeOwned;
use sha2::{Digest as _, Sha256};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity as TlsIdentity};
const MAX_BYTES: usize = 1_000_000;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("transport: {0}")]
    Transport(#[from] tonic::transport::Error),
    #[error("RPC: {0}")]
    Rpc(#[from] tonic::Status),
    #[error("invalid P data: {0}")]
    Invalid(String),
    #[error("local request journal: {0}")]
    Store(#[from] rx_ports::StoreError),
    #[error("execution view expired before use")]
    Expired,
}
fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}
fn id(value: &str) -> Result<Id, Error> {
    Id::new(value).map_err(|e| invalid(e.to_string()))
}
fn manifest(text: &str) -> Vec<u8> {
    let value: serde_json::Value = serde_json::from_str(text).expect("bundled manifest");
    Sha256::digest(canonical::bytes(&value).expect("manifest canonical bytes")).to_vec()
}
fn base_hash() -> Vec<u8> {
    manifest(include_str!(
        "../../../sdk/spec/contracts/v1.0/protocol_manifest.json"
    ))
}
fn cell_hash() -> Vec<u8> {
    manifest(include_str!(
        "../../../sdk/spec/cell_operations/v1.0/protocol_manifest.json"
    ))
}
fn read_hash() -> Vec<u8> {
    manifest(include_str!("../../../sdk/spec/executor/v1/binding.json"))
}
pub struct TlsEndpoint {
    pub uri: String,
    pub server_name: String,
    pub ca_pem: Vec<u8>,
    pub certificate_pem: Vec<u8>,
    pub private_key_pem: Vec<u8>,
}
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerPin {
    pub principal: Name,
    pub peer_boot: Id,
    pub installation: Id,
    pub store_generation: Id,
    pub release: Digest,
    pub clock_id: String,
    pub cell: Name,
    pub definition: Digest,
}
pub struct Client {
    pin: PeerPin,
    clock: Arc<dyn crate::clock::Clock>,
    session: base::Session,
    read: wire::executor_read_service_client::ExecutorReadServiceClient<Channel>,
    workflow: base::workflow_service_client::WorkflowServiceClient<Channel>,
    plans: rx_protocol::executor_plan::executor_plan_service_client::ExecutorPlanServiceClient<
        Channel,
    >,
    cells: cell::cell_service_client::CellServiceClient<Channel>,
    operations: base::operation_service_client::OperationServiceClient<Channel>,
    production:
        rx_protocol::production::production_service_client::ProductionServiceClient<Channel>,
    assignments: rx_protocol::assignment::executor_assignment_service_client::ExecutorAssignmentServiceClient<Channel>,
    runtime_boot: Option<Id>,
    last_sequence: Counter,
    last_checked: Counter,
    process: Option<Arc<ResolvedProcess>>,
    max_payload: usize,
}
/// Only the authenticated client can construct this. It remains a read view, never a permit.
pub struct ValidatedSnapshot {
    pub(crate) raw: ExecutionSnapshot,
    pub(crate) process: Arc<ResolvedProcess>,
    pub(crate) frontier: frontier::Frontier,
    pub(crate) deadline: Instant,
    pub(crate) clock: Arc<dyn crate::clock::Clock>,
}
impl ValidatedSnapshot {
    pub fn data(&self) -> &ExecutionSnapshot {
        &self.raw
    }
    pub fn process(&self) -> &ResolvedProcess {
        &self.process
    }
    pub fn frontier(&self) -> &frontier::Frontier {
        &self.frontier
    }
    pub fn expires_at(&self) -> Instant {
        self.deadline
    }
    pub fn is_current(&self) -> bool {
        Instant::now() < self.deadline
            && self.clock.now().is_ok_and(|now| {
                now.clock_id == self.raw.checked_at.clock_id
                    && now.ticks_ns >= self.raw.checked_at.ticks_ns
                    && now.ticks_ns < self.raw.valid_until.ticks_ns
            })
    }
}
impl Client {
    pub async fn connect(
        endpoint: TlsEndpoint,
        pin: PeerPin,
        clock: Arc<dyn crate::clock::Clock>,
    ) -> Result<Self, Error> {
        if clock.now()?.clock_id != pin.clock_id {
            return Err(invalid("local/shared clock identity mismatch"));
        }
        if !endpoint.uri.starts_with("https://") || pin.clock_id.is_empty() {
            return Err(invalid("TLS endpoint/shared clock required"));
        }
        let channel = Channel::from_shared(endpoint.uri)
            .map_err(|e| invalid(e.to_string()))?
            .tls_config(
                ClientTlsConfig::new()
                    .domain_name(endpoint.server_name)
                    .ca_certificate(Certificate::from_pem(endpoint.ca_pem))
                    .identity(TlsIdentity::from_pem(
                        endpoint.certificate_pem,
                        endpoint.private_key_pem,
                    )),
            )?
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(2))
            .connect()
            .await?;
        let session = base::session_service_client::SessionServiceClient::new(channel.clone())
            .open(base::PeerHello {
                peer_id: pin.principal.to_string(),
                role: base::Role::Executor as i32,
                boot_id: pin.peer_boot.to_string(),
                installation_id: pin.installation.to_string(),
                store_generation: pin.store_generation.to_string(),
                supported_versions: vec![base::Version {
                    major: 1,
                    minor: 0,
                    schema_hash: base_hash(),
                }],
                release_digest: pin.release.as_bytes().to_vec(),
                journal_id: None,
                last_seq: None,
                shared_clock_id: pin.clock_id.clone(),
            })
            .await?
            .into_inner();
        id(&session.session_id)?;
        if session.peer_id != pin.principal.as_str()
            || session.boot_id != pin.peer_boot.as_str()
            || !session
                .selected_version
                .as_ref()
                .is_some_and(|v| v.major == 1 && v.minor == 0 && v.schema_hash == base_hash())
            || session
                .required_features
                .iter()
                .any(|v| !matches!(v.as_str(), "rx.cell.v1" | "strict-wire-v1"))
        {
            return Err(invalid("base session/feature mismatch"));
        }
        let limit = session
            .limits
            .as_ref()
            .ok_or_else(|| invalid("negotiated limits missing"))?
            .max_message_bytes as usize;
        if !(4096..=1_048_576).contains(&limit) {
            return Err(invalid("unsupported negotiated message limit"));
        }
        let mut cells = cell::cell_service_client::CellServiceClient::new(channel.clone())
            .max_decoding_message_size(limit);
        let negotiated = cells
            .open(cell::CellHello {
                base_session_id: session.session_id.clone(),
                peer_id: pin.principal.to_string(),
                base_manifest_hash: base_hash(),
                cell_manifest_hash: cell_hash(),
                cell_definition_digest: pin.definition.as_bytes().to_vec(),
                shared_clock_id: pin.clock_id.clone(),
            })
            .await?
            .into_inner();
        id(&negotiated.session_id)?;
        if negotiated.base_session_id != session.session_id
            || negotiated.peer_id != pin.principal.as_str()
            || negotiated.manifest_hash != cell_hash()
        {
            return Err(invalid("cell negotiation mismatch"));
        }
        Ok(Self {
            pin,
            clock,
            session,
            read: wire::executor_read_service_client::ExecutorReadServiceClient::new(
                channel.clone(),
            )
            .max_decoding_message_size(limit),
            operations: base::operation_service_client::OperationServiceClient::new(
                channel.clone(),
            )
            .max_decoding_message_size(limit),
            plans: rx_protocol::executor_plan::executor_plan_service_client::ExecutorPlanServiceClient::new(channel.clone()).max_decoding_message_size(limit),
            production: rx_protocol::production::production_service_client::ProductionServiceClient::new(channel.clone()).max_decoding_message_size(limit),
            assignments: rx_protocol::assignment::executor_assignment_service_client::ExecutorAssignmentServiceClient::new(channel.clone()).max_decoding_message_size(limit),
            workflow: base::workflow_service_client::WorkflowServiceClient::new(channel)
                .max_decoding_message_size(limit),
            cells,
            runtime_boot: None,
            last_sequence: Counter(0),
            last_checked: Counter(0),
            process: None,
            max_payload: MAX_BYTES.min(limit - 1024),
        })
    }
    pub fn peer_pin(&self) -> &PeerPin {
        &self.pin
    }
    pub fn session_id(&self) -> &str {
        &self.session.session_id
    }
    fn context(&self) -> base::CallContext {
        base::CallContext {
            session_id: self.session.session_id.clone(),
            call_id: uuid::Uuid::new_v4().to_string(),
            request_key: None,
            expected_revision: None,
        }
    }
    async fn artifact(&mut self, run: &Id, reference: &ArtifactRef) -> Result<Vec<u8>, Error> {
        let reply = self
            .read
            .get_artifact(wire::ArtifactRequest {
                context: Some(self.context()),
                run_id: run.to_string(),
                binding_hash: read_hash(),
                reference: Some(base::ArtifactRef {
                    sha256: reference.sha256.as_bytes().to_vec(),
                    schema_id: reference.schema_id.to_string(),
                    size_bytes: reference.size_bytes.0,
                }),
            })
            .await?
            .into_inner();
        let (actual, bytes) = checked_payload(reply, self.max_payload)?;
        if actual != *reference {
            return Err(invalid("artifact reference changed"));
        }
        Ok(bytes)
    }
    pub async fn inspect_run(&mut self, run: &Id) -> Result<crate::journal::RunResponse, Error> {
        let value = self
            .workflow
            .get_run(base::RunRequest {
                context: Some(self.context()),
                run_id: Some(run.to_string()),
                recipe_digest: vec![],
                site_config_digest: vec![],
            })
            .await?
            .into_inner();
        rx_protocol::json::to_value(&value)?;
        let result = run_response(value)?;
        if result.run != *run {
            return Err(invalid("run read identity differs"));
        }
        let state: ExecutorState = decode(&self.artifact(run, &result.checkpoint.payload).await?)?;
        if state.schema.as_str() != CHECKPOINT_SCHEMA
            || state.run.id != *run
            || state.run.cell != self.pin.cell
            || state.run.recipe_digest != result.recipe
            || state.revision != result.revision
            || state.run.state != result.state
            || state.run.executor_session != result.executor_session
            || canonical::bytes(&state.activations).map_err(|e| invalid(e.to_string()))?
                != canonical::bytes(&result.checkpoint.activations)
                    .map_err(|e| invalid(e.to_string()))?
        {
            return Err(invalid("run control artifact/scope differs"));
        }
        Ok(result)
    }
    pub(crate) async fn pause_lifecycle(
        &mut self,
        key: &Id,
        run: &Id,
        revision: Counter,
    ) -> Result<crate::journal::RunResponse, Error> {
        let mut context = self.context();
        context.request_key = Some(key.to_string());
        context.expected_revision = Some(revision.0);
        let value = self
            .workflow
            .pause_run(base::RunRequest {
                context: Some(context),
                run_id: Some(run.to_string()),
                recipe_digest: vec![],
                site_config_digest: vec![],
            })
            .await?
            .into_inner();
        rx_protocol::json::to_value(&value)?;
        let result = run_response(value)?;
        if result.run != *run
            || result.revision < revision
            || !crate::lifecycle::restricted(result.state)
        {
            return Err(invalid("pause did not restrict requested run"));
        }
        Ok(result)
    }
    /// Recover data only. Historical EXECUTING is never used as current authority.
    pub async fn restore(&mut self, run: &Id) -> Result<ExecutorState, Error> {
        let view = self
            .workflow
            .get_run(base::RunRequest {
                context: Some(self.context()),
                run_id: Some(run.to_string()),
                recipe_digest: vec![],
                site_config_digest: vec![],
            })
            .await?
            .into_inner();
        rx_protocol::json::to_value(&view)?;
        let checkpoint = view
            .checkpoint
            .ok_or_else(|| invalid("checkpoint missing"))?;
        let reference = reference(
            checkpoint
                .payload
                .ok_or_else(|| invalid("checkpoint artifact missing"))?,
        )?;
        if reference.schema_id.as_str() != CHECKPOINT_SCHEMA
            || checkpoint.executor_schema != CHECKPOINT_SCHEMA
        {
            return Err(invalid("checkpoint schema mismatch"));
        }
        let bytes = self.artifact(run, &reference).await?;
        let state: ExecutorState = decode(&bytes)?;
        let state_code = match state.run.state {
            RunState::Prepared => base::RunState::Prepared,
            RunState::Executing => base::RunState::Executing,
            RunState::Paused => base::RunState::Paused,
            RunState::RecoveryRequired => base::RunState::RecoveryRequired,
            RunState::Completed => base::RunState::Completed,
            RunState::Abandoned => base::RunState::Abandoned,
        };
        if view.revision == 0 || view.state != state_code as i32 {
            return Err(invalid("run state/revision mismatch"));
        }
        if state.schema.as_str() != CHECKPOINT_SCHEMA
            || state.run.id != *run
            || state.run.cell != self.pin.cell
            || state.revision.0 != view.revision
            || checkpoint.revision != view.revision
            || checkpoint.run_id != run.as_str()
            || view.recipe_digest != state.run.recipe_digest.as_bytes()
            || state.run.executor_session.as_ref().map(ToString::to_string)
                != view.executor_session_id
        {
            return Err(invalid("checkpoint/run identity mismatch"));
        }
        if state.activations.len() != checkpoint.activations.len() {
            return Err(invalid("checkpoint activation coverage mismatch"));
        }
        for (a, b) in state.activations.iter().zip(&checkpoint.activations) {
            if a.id.as_str() != b.activation_id
                || a.node.as_str() != b.node_id
                || a.visit.0 != b.visit
                || a.slots.len() != b.slots.len()
            {
                return Err(invalid("checkpoint activation differs"));
            }
            for (left, right) in a.slots.iter().zip(&b.slots) {
                if left.slot.as_str() != right.slot
                    || left.operation.as_str() != right.operation_id
                    || right.intent_digest != left.intent_digest.as_bytes()
                {
                    return Err(invalid("checkpoint slot differs"));
                }
            }
        }
        Ok(state)
    }
    pub async fn snapshot(&mut self, run: &Id, visit: Counter) -> Result<ValidatedSnapshot, Error> {
        let started = Instant::now();
        let sent_at = self.clock.now()?;
        if sent_at.clock_id != self.pin.clock_id {
            return Err(invalid("local clock changed"));
        }
        let reply = self
            .read
            .get_snapshot(wire::SnapshotRequest {
                context: Some(self.context()),
                run_id: run.to_string(),
                visit: visit.0,
                binding_hash: read_hash(),
            })
            .await?
            .into_inner();
        let (reference, bytes) = checked_payload(reply, self.max_payload)?;
        if reference.schema_id.as_str() != SNAPSHOT_SCHEMA {
            return Err(invalid("snapshot schema mismatch"));
        }
        let raw: ExecutionSnapshot = decode(&bytes)?;
        if raw.installation != self.pin.installation
            || raw.store_generation != self.pin.store_generation
            || raw.caller_session.as_str() != self.session.session_id
            || raw.definition.sha256 != self.pin.definition
            || raw.run.run.cell != self.pin.cell
            || raw.run.run.id != *run
            || raw.visit != visit
            || raw.checked_at.clock_id != self.pin.clock_id
            || self
                .runtime_boot
                .as_ref()
                .is_some_and(|b| b != &raw.runtime_boot)
            || raw.sequence < self.last_sequence
            || raw.checked_at.ticks_ns < self.last_checked
        {
            return Err(invalid("foreign or regressed snapshot"));
        }
        let process = if let Some(process) = &self.process
            && frontier::resolved_digest(process).map_err(invalid)? == raw.resolved.sha256
        {
            process.clone()
        } else {
            let bytes = self.artifact(run, &raw.resolved).await?;
            let process: ResolvedProcess = decode(&bytes)?;
            Arc::new(process)
        };
        let frontier = execution_validation::validate(&raw, &process).map_err(invalid)?;
        let received = self.clock.now()?;
        if received.clock_id != self.pin.clock_id
            || received.ticks_ns < sent_at.ticks_ns
            || raw.checked_at.ticks_ns < sent_at.ticks_ns
            || raw.checked_at.ticks_ns > received.ticks_ns
        {
            return Err(invalid("P read time is outside same-host request interval"));
        }
        let duration = raw
            .valid_until
            .ticks_ns
            .0
            .checked_sub(raw.checked_at.ticks_ns.0)
            .ok_or_else(|| invalid("negative view lifetime"))?;
        let deadline = started
            .checked_add(Duration::from_nanos(duration))
            .ok_or_else(|| invalid("view deadline overflow"))?;
        self.process = Some(process.clone());
        if Instant::now() >= deadline || received.ticks_ns >= raw.valid_until.ticks_ns {
            return Err(Error::Expired);
        }
        self.runtime_boot = Some(raw.runtime_boot.clone());
        self.last_sequence = raw.sequence;
        self.last_checked = raw.checked_at.ticks_ns;
        Ok(ValidatedSnapshot {
            raw,
            process,
            frontier,
            deadline,
            clock: self.clock.clone(),
        })
    }
}
fn reference(value: base::ArtifactRef) -> Result<ArtifactRef, Error> {
    Ok(ArtifactRef {
        sha256: Digest::from_bytes(
            value
                .sha256
                .try_into()
                .map_err(|_| invalid("digest length"))?,
        ),
        schema_id: Name::new(value.schema_id).map_err(|e| invalid(e.to_string()))?,
        size_bytes: Counter(value.size_bytes),
    })
}
fn checked_payload(value: wire::ReadPayload, max: usize) -> Result<(ArtifactRef, Vec<u8>), Error> {
    let reference = reference(
        value
            .reference
            .ok_or_else(|| invalid("payload reference missing"))?,
    )?;
    if value.payload.len() > max
        || value.payload.len() as u64 != reference.size_bytes.0
        || Digest::from_bytes(Sha256::digest(&value.payload).into()) != reference.sha256
    {
        return Err(invalid("payload size/hash mismatch"));
    }
    Ok((reference, value.payload))
}
fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Error> {
    canonical::decode_json(bytes).map_err(|e| invalid(e.to_string()))
}

impl Client {
    /// Crate-private: the worker must durably enter the request before calling this method.
    pub(crate) async fn emit(
        &mut self,
        key: &Id,
        body: &crate::journal::Body,
    ) -> Result<crate::journal::Response, Error> {
        use crate::journal::{Admission, Body, Response};
        let context = |revision: Option<Counter>| base::CallContext {
            session_id: self.session.session_id.clone(),
            call_id: uuid::Uuid::new_v4().to_string(),
            request_key: Some(key.to_string()),
            expected_revision: revision.map(|r| r.0),
        };
        match body {
            Body::BeginPart {
                cell: cell_id,
                run,
                mandate,
                expected_budget,
                expected_cell,
                ..
            } => {
                let value = self
                    .cells
                    .begin_part_attempt(cell::BeginPartAttemptRequest {
                        call: Some(cell::CellCall {
                            context: Some(context(None)),
                            cell_id: cell_id.to_string(),
                            expected_cell_revision: Some(expected_cell.0),
                        }),
                        run_id: run.to_string(),
                        mandate_id: mandate.to_string(),
                        expected_budget_revision: expected_budget.0,
                    })
                    .await?
                    .into_inner();
                Ok(Response::Part(production::part_response(value)?))
            }
            Body::CompletePart { command, .. } => {
                let value = self
                    .production
                    .complete_part(rx_protocol::production::CompletePart {
                        context: Some(context(None)),
                        cell_id: command.cell.to_string(),
                        run_id: command.run.to_string(),
                        part_attempt_id: command.part.to_string(),
                        expected_run_revision: command.expected_run.0,
                        expected_part_revision: command.expected_part.0,
                        binding_hash: production::production_hash(),
                    })
                    .await?
                    .into_inner();
                Ok(Response::Part(production::part_response(value)?))
            }
            Body::CommitCheckpoint { command, .. } => {
                let response = self
                    .workflow
                    .commit_checkpoint(base::ChangeCheckpoint {
                        context: Some(context(None)),
                        run_id: command.run.to_string(),
                        expected_revision: command.expected_revision.0,
                        new_checkpoint: Some(checkpoint_wire(&command.new_checkpoint)),
                    })
                    .await?
                    .into_inner();
                rx_protocol::json::to_value(&response)?;
                Ok(Response::Run(Box::new(run_response(response)?)))
            }
            Body::ReconcileOperation {
                operation,
                intent_digest,
                ..
            } => {
                let value = self
                    .operations
                    .reconcile(base::OperationRef {
                        context: Some(context(None)),
                        operation_id: operation.to_string(),
                    })
                    .await?
                    .into_inner();
                rx_protocol::json::to_value(&value)?;
                if value.operation_id != operation.as_str()
                    || value.intent_digest != intent_digest.as_bytes()
                    || value.revision == 0
                {
                    return Err(invalid("reconciliation operation differs"));
                }
                Ok(Response::ReconciliationAccepted {
                    operation: operation.clone(),
                    intent_digest: *intent_digest,
                })
            }
            Body::PauseRun {
                run, expected_run, ..
            } => {
                let value = self
                    .workflow
                    .pause_run(base::RunRequest {
                        context: Some(context(Some(*expected_run))),
                        run_id: Some(run.to_string()),
                        recipe_digest: vec![],
                        site_config_digest: vec![],
                    })
                    .await?
                    .into_inner();
                rx_protocol::json::to_value(&value)?;
                Ok(Response::Run(Box::new(run_response(value)?)))
            }
            Body::ResolveActivation {
                run,
                node,
                visit,
                expected_run,
            } => {
                let value = self
                    .workflow
                    .resolve_activation(base::ResolveActivation {
                        context: Some(context(Some(*expected_run))),
                        run_id: run.to_string(),
                        node_id: node.to_string(),
                        visit: visit.0,
                    })
                    .await?
                    .into_inner();
                rx_protocol::json::to_value(&value)?;
                if value.node_id != node.as_str() || value.visit != visit.0 {
                    return Err(invalid("activation reply differs"));
                }
                let slots = value
                    .slots
                    .into_iter()
                    .map(|slot| {
                        Ok(SlotSnapshot {
                            slot: Name::new(slot.slot).map_err(|e| invalid(e.to_string()))?,
                            operation: id(&slot.operation_id)?,
                            intent_digest: Digest::from_bytes(
                                slot.intent_digest
                                    .try_into()
                                    .map_err(|_| invalid("slot digest length"))?,
                            ),
                        })
                    })
                    .collect::<Result<Vec<_>, Error>>()?;
                let names = slots
                    .iter()
                    .map(|s| &s.slot)
                    .collect::<std::collections::BTreeSet<_>>();
                if names.len() != slots.len() {
                    return Err(invalid("duplicate activation slots"));
                }
                Ok(Response::Activation(ActivationSnapshot {
                    id: id(&value.activation_id)?,
                    node: node.clone(),
                    visit: *visit,
                    slots,
                }))
            }
            Body::SubmitOperation {
                cell,
                run,
                activation,
                part,
                slot,
                intent,
                mandate,
                expected_cell,
                expected_run,
            } => {
                let context = context(None);
                let intent_wire = rx_protocol::json::from_slice(
                    &canonical::bytes(intent.as_ref()).map_err(|e| invalid(e.to_string()))?,
                )?;
                let value = self
                    .cells
                    .submit_operation(cell::SubmitOperationRequest {
                        call: Some(cell::CellCall {
                            context: Some(context.clone()),
                            cell_id: cell.to_string(),
                            expected_cell_revision: Some(expected_cell.0),
                        }),
                        request: Some(base::SubmitOperation {
                            context: Some(context),
                            intent: Some(intent_wire),
                            run_id: Some(run.to_string()),
                            activation_id: Some(activation.to_string()),
                            slot: Some(slot.to_string()),
                        }),
                        parent: Some(cell::PermitParent {
                            value: Some(cell::permit_parent::Value::MandateId(mandate.to_string())),
                        }),
                        part_attempt_id: part.as_ref().map(ToString::to_string),
                        expected_run_revision: Some(expected_run.0),
                        expected_case_revision: None,
                    })
                    .await?
                    .into_inner();
                rx_protocol::json::to_value(&value)?;
                let digest = Digest::from_bytes(
                    value
                        .intent_digest
                        .try_into()
                        .map_err(|_| invalid("receipt digest length"))?,
                );
                if digest != intent.digest().map_err(|e| invalid(e.to_string()))?
                    || value.stage != base::ReceiptStage::Admitted as i32
                    || value.operation_revision != Some(1)
                    || value.journal_seq == 0
                    || value.invocation_id.is_some()
                    || value.host_state.is_some()
                    || value.cancel_id.is_some()
                {
                    return Err(invalid("not the immutable P admission receipt"));
                }
                Ok(Response::Admission(Admission {
                    operation: id(&value.operation_id)?,
                    intent_digest: digest,
                    revision: Counter(1),
                    journal: id(&value.journal_id)?,
                    sequence: Counter(value.journal_seq),
                }))
            }
        }
    }
}

fn checkpoint_wire(value: &CheckpointView) -> base::Checkpoint {
    base::Checkpoint {
        run_id: value.run.to_string(),
        revision: value.revision.0,
        executor_schema: value.executor_schema.to_string(),
        payload: Some(base::ArtifactRef {
            sha256: value.payload.sha256.as_bytes().to_vec(),
            schema_id: value.payload.schema_id.to_string(),
            size_bytes: value.payload.size_bytes.0,
        }),
        activations: value
            .activations
            .iter()
            .map(|a| base::ActivationView {
                activation_id: a.id.to_string(),
                node_id: a.node.to_string(),
                visit: a.visit.0,
                slots: a
                    .slots
                    .iter()
                    .map(|s| base::SlotBinding {
                        slot: s.slot.to_string(),
                        operation_id: s.operation.to_string(),
                        intent_digest: s.intent_digest.as_bytes().to_vec(),
                    })
                    .collect(),
            })
            .collect(),
    }
}
fn checkpoint_response(value: base::Checkpoint) -> Result<CheckpointView, Error> {
    if value.revision == 0 || value.executor_schema != CHECKPOINT_SCHEMA {
        return Err(invalid("checkpoint revision/schema differs"));
    }
    let payload = reference(
        value
            .payload
            .ok_or_else(|| invalid("checkpoint artifact missing"))?,
    )?;
    if payload.schema_id.as_str() != CHECKPOINT_SCHEMA {
        return Err(invalid("checkpoint payload schema differs"));
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut nodes = std::collections::BTreeSet::new();
    let activations = value
        .activations
        .into_iter()
        .map(|a| {
            if a.visit == 0
                || !ids.insert(a.activation_id.clone())
                || !nodes.insert((a.node_id.clone(), a.visit))
            {
                return Err(invalid("checkpoint activation coverage"));
            }
            let mut slots = std::collections::BTreeSet::new();
            Ok(ActivationSnapshot {
                id: id(&a.activation_id)?,
                node: Name::new(a.node_id).map_err(|e| invalid(e.to_string()))?,
                visit: Counter(a.visit),
                slots: a
                    .slots
                    .into_iter()
                    .map(|s| {
                        if !slots.insert(s.slot.clone()) {
                            return Err(invalid("duplicate checkpoint slot"));
                        }
                        Ok(SlotSnapshot {
                            slot: Name::new(s.slot).map_err(|e| invalid(e.to_string()))?,
                            operation: id(&s.operation_id)?,
                            intent_digest: Digest::from_bytes(
                                s.intent_digest
                                    .try_into()
                                    .map_err(|_| invalid("slot digest"))?,
                            ),
                        })
                    })
                    .collect::<Result<_, Error>>()?,
            })
        })
        .collect::<Result<_, Error>>()?;
    Ok(CheckpointView {
        run: id(&value.run_id)?,
        revision: Counter(value.revision),
        executor_schema: Name::new(CHECKPOINT_SCHEMA).map_err(|e| invalid(e.to_string()))?,
        payload,
        activations,
    })
}
fn run_response(value: base::RunView) -> Result<crate::journal::RunResponse, Error> {
    let state = match base::RunState::try_from(value.state).map_err(|_| invalid("run state"))? {
        base::RunState::Prepared => RunState::Prepared,
        base::RunState::Executing => RunState::Executing,
        base::RunState::Paused => RunState::Paused,
        base::RunState::RecoveryRequired => RunState::RecoveryRequired,
        base::RunState::Completed => RunState::Completed,
        base::RunState::Abandoned => RunState::Abandoned,
        _ => return Err(invalid("unspecified run state")),
    };
    let run = id(&value.run_id)?;
    let revision = Counter(value.revision);
    let checkpoint = checkpoint_response(
        value
            .checkpoint
            .ok_or_else(|| invalid("checkpoint missing"))?,
    )?;
    if checkpoint.run != run || checkpoint.revision != revision {
        return Err(invalid("run/checkpoint identity differs"));
    }
    Ok(crate::journal::RunResponse {
        run,
        revision,
        state,
        recipe: Digest::from_bytes(
            value
                .recipe_digest
                .try_into()
                .map_err(|_| invalid("recipe digest"))?,
        ),
        executor_session: value.executor_session_id.as_deref().map(id).transpose()?,
        checkpoint,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn artifact_hash_does_not_replace_size_schema_or_typed_payload_validation() {
        let bytes = b"{}".to_vec();
        let good = wire::ReadPayload {
            reference: Some(base::ArtifactRef {
                sha256: Sha256::digest(&bytes).to_vec(),
                schema_id: SNAPSHOT_SCHEMA.into(),
                size_bytes: 2,
            }),
            payload: bytes,
        };
        assert!(checked_payload(good.clone(), 100).is_ok());
        assert!(decode::<ExecutionSnapshot>(&good.payload).is_err());
        let mut wrong = good.clone();
        wrong.reference.as_mut().unwrap().size_bytes = 3;
        assert!(checked_payload(wrong, 100).is_err());
        let mut wrong = good.clone();
        wrong.payload.push(b' ');
        assert!(checked_payload(wrong, 100).is_err());
        assert!(checked_payload(good, 1).is_err());
    }
}
