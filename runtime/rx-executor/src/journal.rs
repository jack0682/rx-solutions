//! S-owned request journal. A saved request is never a P admission or native result.
mod lifecycle;
pub mod recovery;
use crate::frame::Identity;
use rx_domain::{canonical, intent::Intent, types::*};
use rx_ports::*;
use rx_process_contract::checkpoint_change::{
    CheckpointAction, CheckpointDecision, CheckpointTarget, CommitCheckpoint,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
const HEADER: &str = "rx.executor-journal.v1";
const ENTRY: &str = "rx.executor-request.v1";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub installation: Id,
    pub store_generation: Id,
    pub principal: Name,
    pub release: Digest,
    pub cell: Name,
    pub definition: Digest,
    pub run: Id,
    pub resolved_digest: Digest,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Stage {
    ResolveActivation,
    SubmitOperation,
    PauseRun,
    ReconcileOperation,
    CheckpointBranch,
    CheckpointStartWait,
    CheckpointCheckWait,
    BeginPart,
    CompletePart,
}
impl Stage {
    pub fn checkpoint(action: CheckpointAction) -> Self {
        match action {
            CheckpointAction::ChooseBranch => Self::CheckpointBranch,
            CheckpointAction::StartWait => Self::CheckpointStartWait,
            CheckpointAction::CheckWait => Self::CheckpointCheckWait,
        }
    }
    pub fn is_checkpoint(self) -> bool {
        matches!(
            self,
            Self::CheckpointBranch | Self::CheckpointStartWait | Self::CheckpointCheckWait
        )
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CheckpointRejection {
    Revision,
    Expired,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Logical {
    pub visit: Counter,
    pub node: Name,
    pub stage: Stage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<ControlIdentity>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlIdentity {
    pub session: Id,
    pub epoch: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "method",
    rename_all = "SCREAMING_SNAKE_CASE",
    deny_unknown_fields
)]
pub enum Body {
    BeginPart {
        cell: Name,
        run: Id,
        ordinal: Counter,
        mandate: Id,
        expected_budget: Counter,
        expected_cell: Counter,
    },
    CompletePart {
        ordinal: Counter,
        command: Box<rx_process_contract::production::CompletePart>,
    },
    CommitCheckpoint {
        target: CheckpointTarget,
        command: Box<CommitCheckpoint>,
        decision: CheckpointDecision,
    },
    ReconcileOperation {
        run: Id,
        operation: Id,
        intent_digest: Digest,
    },
    PauseRun {
        run: Id,
        root: Name,
        expected_run: Counter,
    },
    ResolveActivation {
        run: Id,
        node: Name,
        visit: Counter,
        expected_run: Counter,
    },
    SubmitOperation {
        cell: Name,
        run: Id,
        activation: Id,
        part: Option<Id>,
        slot: Name,
        intent: Box<Intent>,
        mandate: Id,
        expected_cell: Counter,
        expected_run: Counter,
    },
}
impl Body {
    pub fn stage(&self) -> Stage {
        match self {
            Self::BeginPart { .. } => Stage::BeginPart,
            Self::CompletePart { .. } => Stage::CompletePart,
            Self::CommitCheckpoint { target, .. } => Stage::checkpoint(target.action),
            Self::ReconcileOperation { .. } => Stage::ReconcileOperation,
            Self::PauseRun { .. } => Stage::PauseRun,
            Self::ResolveActivation { .. } => Stage::ResolveActivation,
            Self::SubmitOperation { .. } => Stage::SubmitOperation,
        }
    }
    fn validate(&self, scope: &Scope, logical: &Logical) -> Result<()> {
        if logical.stage != self.stage()
            || logical.visit.0 == 0
            || (logical.stage != Stage::PauseRun && logical.control.is_some())
        {
            return invalid("logical action differs");
        }
        match self {
            Self::BeginPart {
                cell,
                run,
                ordinal,
                expected_budget,
                expected_cell,
                ..
            } => {
                if cell != &scope.cell
                    || run != &scope.run
                    || ordinal != &logical.visit
                    || logical.node.as_str() != "production/part"
                    || expected_budget.0 == 0
                    || expected_cell.0 == 0
                {
                    return invalid("begin part scope differs");
                }
            }
            Self::CompletePart { ordinal, command } => {
                if command.cell != scope.cell
                    || command.run != scope.run
                    || ordinal != &logical.visit
                    || logical.node.as_str() != "production/part"
                    || command.expected_run.0 == 0
                    || command.expected_part.0 == 0
                {
                    return invalid("complete part scope differs");
                }
            }
            Self::CommitCheckpoint {
                target,
                command,
                decision,
            } => {
                if target.run != scope.run
                    || target.node != logical.node
                    || target.visit != logical.visit
                    || target.action != decision.action()
                    || command.run != scope.run
                    || command.expected_revision.0 == 0
                    || command.new_checkpoint.run != scope.run
                    || command.expected_revision.0.checked_add(1)
                        != Some(command.new_checkpoint.revision.0)
                    || command.new_checkpoint.executor_schema.as_str()
                        != rx_process_contract::execution::CHECKPOINT_SCHEMA
                    || command.new_checkpoint.payload.schema_id.as_str()
                        != rx_process_contract::execution::CHECKPOINT_SCHEMA
                {
                    return invalid("checkpoint body/scope differs");
                }
            }
            Self::ReconcileOperation { run, .. } => {
                if run != &scope.run {
                    return invalid("query run differs");
                }
            }
            Self::PauseRun {
                run,
                root,
                expected_run,
            } => {
                if run != &scope.run
                    || root != &logical.node
                    || expected_run.0 == 0
                    || logical.control.as_ref().is_none_or(|c| c.epoch.0 == 0)
                {
                    return invalid("pause scope/body mismatch");
                }
            }
            Self::ResolveActivation {
                run,
                node,
                visit,
                expected_run,
            } => {
                if run != &scope.run
                    || node != &logical.node
                    || visit != &logical.visit
                    || expected_run.0 == 0
                {
                    return invalid("activation body differs");
                }
            }
            Self::SubmitOperation {
                cell,
                run,
                intent,
                expected_cell,
                expected_run,
                ..
            } => {
                if cell != &scope.cell
                    || run != &scope.run
                    || expected_cell.0 == 0
                    || expected_run.0 == 0
                {
                    return invalid("submission body differs");
                }
                let normalized = intent
                    .clone()
                    .normalized()
                    .map_err(|e| StoreError::Invalid(e.to_string()))?;
                if bytes(intent.as_ref())? != bytes(&normalized)? {
                    return invalid("intent must be normalized before journaling");
                }
            }
        }
        Ok(())
    }
    pub(crate) fn semantic_digest(&self) -> Result<Digest> {
        match self {
            Self::BeginPart {
                cell,
                run,
                ordinal,
                mandate,
                ..
            } => digest("RX-E-BEGIN-PART-v1", &(cell, run, ordinal, mandate)),
            Self::CompletePart { ordinal, command } => digest(
                "RX-E-COMPLETE-PART-v1",
                &(&command.cell, &command.run, &command.part, ordinal),
            ),
            Self::CommitCheckpoint { target, .. } => digest("RX-E-CHECKPOINT-SEMANTIC-v1", target),
            Self::ReconcileOperation {
                run,
                operation,
                intent_digest,
            } => digest(
                "RX-E-RECONCILE-SEMANTIC-v1",
                &(run, operation, intent_digest),
            ),
            Self::PauseRun { run, root, .. } => digest("RX-E-PAUSE-SEMANTIC-v1", &(run, root)),
            Self::ResolveActivation {
                run, node, visit, ..
            } => digest("RX-E-RESOLVE-SEMANTIC-v1", &(run, node, visit)),
            Self::SubmitOperation {
                cell,
                run,
                activation,
                part,
                slot,
                intent,
                mandate,
                ..
            } => digest(
                "RX-E-SUBMIT-SEMANTIC-v1",
                &(cell, run, activation, part, slot, intent, mandate),
            ),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SendState {
    Prepared,
    EmitEntered,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Admission {
    pub operation: Id,
    pub intent_digest: Digest,
    pub revision: Counter,
    pub journal: Id,
    pub sequence: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "SCREAMING_SNAKE_CASE",
    deny_unknown_fields
)]
pub enum Response {
    Part(rx_process_contract::production::Part),
    ReconciliationAccepted {
        operation: Id,
        intent_digest: Digest,
    },
    Run(Box<RunResponse>),
    Activation(rx_process_contract::execution::ActivationSnapshot),
    Admission(Admission),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunResponse {
    pub run: Id,
    pub revision: Counter,
    pub recipe: Digest,
    pub state: rx_process_contract::execution::RunState,
    pub executor_session: Option<Id>,
    pub checkpoint: rx_process_contract::execution::CheckpointView,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "SCREAMING_SNAKE_CASE",
    deny_unknown_fields
)]
pub enum Resolution {
    Pending,
    Reply { response: Box<Response> },
    RevisionRejected,
    CheckpointRejected { reason: CheckpointRejection },
    Attention { code: Name },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Basis {
    pub runtime_boot: Id,
    pub sequence: Counter,
    pub run_revision: Counter,
    pub cell_revision: Counter,
    pub checked_at: TimePoint,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub key: Id,
    pub logical: Logical,
    pub generation: Counter,
    pub context: Identity,
    pub body: Body,
    pub body_digest: Digest,
    pub basis: Basis,
    pub send: SendState,
    pub resolution: Resolution,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ObservedTarget {
    Part {
        value: rx_process_contract::production::Part,
    },
    Checkpoint {
        decision: CheckpointDecision,
    },
    Released {
        operation: Id,
        intent_digest: Digest,
    },
    Run {
        run: Id,
        revision: Counter,
        state: rx_process_contract::execution::RunState,
    },
    Activation {
        value: rx_process_contract::execution::ActivationSnapshot,
    },
    Operation {
        activation: Id,
        operation: Id,
        intent_digest: Digest,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub logical: Logical,
    pub target: ObservedTarget,
    pub basis: Basis,
}
pub struct Journal<R> {
    repository: R,
    scope: Scope,
}
impl<R: Repository> Journal<R> {
    pub fn open(mut repository: R, scope: Scope) -> Result<Self> {
        repository.transact(|tx| {
            let key = name("executor/header");
            if let Some(row) = tx.get(&key)? {
                let stored: Scope = decode(&row, HEADER)?;
                if stored != scope {
                    return invalid("journal belongs to another installation/store/run/release");
                }
            } else {
                tx.put(&key, None, &doc(HEADER, &scope)?)?;
            }
            Ok(())
        })?;
        Ok(Self { repository, scope })
    }
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    #[cfg(feature = "test-harness")]
    pub fn test_attempts(&mut self) -> Result<Vec<Entry>> {
        self.repository.transact(|tx| {
            tx.scan("executor-attempt/")?
                .into_iter()
                .map(|row| {
                    let entry: Entry = decode(&row, ENTRY)?;
                    validate_entry(&self.scope, &entry)?;
                    Ok(entry)
                })
                .collect()
        })
    }
    pub fn into_repository(self) -> R {
        self.repository
    }
    pub fn get(&mut self, logical: &Logical) -> Result<Option<Entry>> {
        self.repository
            .transact(|tx| current(tx, logical, &self.scope))
    }
    pub fn attempt(&mut self, request: &Id) -> Result<Option<Entry>> {
        self.repository.transact(|tx| {
            let Some(row) = tx.get(&key("executor-attempt", request)?)? else {
                return Ok(None);
            };
            let entry: Entry = decode(&row, ENTRY)?;
            validate_entry(&self.scope, &entry)?;
            if entry.key != *request {
                return Err(StoreError::Integrity("request key differs".into()));
            }
            Ok(Some(entry))
        })
    }
    pub fn prepare(
        &mut self,
        logical: Logical,
        context: Identity,
        body: Body,
        basis: Basis,
    ) -> Result<Entry> {
        body.validate(&self.scope, &logical)?;
        if logical
            .control
            .as_ref()
            .is_some_and(|c| c.session != context.executor_session || c.epoch != context.epoch)
        {
            return invalid("control context mismatch");
        }
        if context.run != self.scope.run
            || context.resolved_digest != self.scope.resolved_digest
            || context.visit != logical.visit
            || context.epoch.0 == 0
        {
            return invalid("request context differs from journal");
        }
        if basis.sequence.0 == 0 || basis.run_revision.0 == 0 || basis.cell_revision.0 == 0 {
            return invalid("request basis is incomplete");
        }
        let body_digest = digest("RX-E-REQUEST-BODY-v1", &body)?;
        self.repository.transact(|tx| {
            let previous = current(tx, &logical, &self.scope)?;
            let generation = if let Some(old) = &previous {
                if old.context == context && old.body_digest == body_digest {
                    return Ok(old.clone());
                }
                if !matches!(
                    old.resolution,
                    Resolution::RevisionRejected | Resolution::CheckpointRejected { .. }
                ) || old.context != context
                    || old.body.semantic_digest()? != body.semantic_digest()?
                {
                    return Err(StoreError::KeyConflict);
                }
                old.generation
                    .increment()
                    .map_err(|e| StoreError::Invalid(e.to_string()))?
            } else {
                Counter(1)
            };
            let entry = Entry {
                key: id(),
                logical: logical.clone(),
                generation,
                context,
                body,
                body_digest,
                basis,
                send: SendState::Prepared,
                resolution: Resolution::Pending,
            };
            tx.put(
                &key("executor-attempt", &entry.key)?,
                None,
                &doc(ENTRY, &entry)?,
            )?;
            let latest = key("executor-current", &logical)?;
            let prior = tx.get(&latest)?;
            tx.put(
                &latest,
                prior.map(|p| p.revision),
                &doc("rx.executor-current.v1", &entry.key)?,
            )?;
            event(tx, "rx.executor-request-prepared.v1", &entry)?;
            Ok(entry)
        })
    }
    /// Durable before any network call. Repeating this never manufactures a new request key.
    pub fn enter(&mut self, request: &Id) -> Result<Entry> {
        self.change(
            request,
            |entry| {
                if !matches!(entry.resolution, Resolution::Pending) {
                    return invalid("resolved/blocked request cannot emit");
                }
                entry.send = SendState::EmitEntered;
                Ok(())
            },
            "rx.executor-request-entered.v1",
        )
    }
    pub fn reply(&mut self, request: &Id, response: Response) -> Result<Entry> {
        self.change(
            request,
            |entry| {
                validate_response(&entry.body, &response)?;
                if entry.send != SendState::EmitEntered {
                    return invalid("RPC reply without entered request");
                }
                if let Resolution::Reply { response: old } = &entry.resolution {
                    if bytes(old.as_ref())? == bytes(&response)? {
                        return Ok(());
                    }
                    return Err(StoreError::Integrity("immutable RPC reply changed".into()));
                }
                if !matches!(entry.resolution, Resolution::Pending) {
                    return invalid("request already resolved");
                }
                entry.resolution = Resolution::Reply {
                    response: Box::new(response),
                };
                Ok(())
            },
            "rx.executor-request-replied.v1",
        )
    }
    pub fn revision_rejected(&mut self, request: &Id) -> Result<Entry> {
        self.change(
            request,
            |entry| {
                if entry.send != SendState::EmitEntered
                    || !matches!(entry.resolution, Resolution::Pending)
                    || entry.logical.stage == Stage::ReconcileOperation
                    || entry.logical.stage.is_checkpoint()
                {
                    return invalid("revision rejection without pending RPC");
                }
                entry.resolution = Resolution::RevisionRejected;
                Ok(())
            },
            "rx.executor-request-revision-rejected.v1",
        )
    }
    pub fn checkpoint_rejected(
        &mut self,
        request: &Id,
        reason: CheckpointRejection,
    ) -> Result<Entry> {
        self.change(
            request,
            |entry| {
                if entry.send != SendState::EmitEntered
                    || !matches!(entry.resolution, Resolution::Pending)
                    || !entry.logical.stage.is_checkpoint()
                {
                    return invalid("checkpoint rejection without entered checkpoint request");
                }
                entry.resolution = Resolution::CheckpointRejected { reason };
                Ok(())
            },
            "rx.executor-checkpoint-rejected.v1",
        )
    }
    pub fn attention(&mut self, request: &Id, code: Name) -> Result<Entry> {
        self.change(
            request,
            |entry| {
                if !matches!(entry.resolution, Resolution::Pending) {
                    return invalid("request already resolved");
                }
                entry.resolution = Resolution::Attention { code };
                Ok(())
            },
            "rx.executor-request-attention.v1",
        )
    }
    /// P projection evidence is separate from an RPC acknowledgment or native outcome.
    pub fn observation(&mut self, logical: &Logical) -> Result<Option<Observation>> {
        self.repository.transact(|tx| {
            tx.get(&key("executor-observed", logical)?)?
                .map(|row| decode(&row, "rx.executor-observation.v1"))
                .transpose()
        })
    }
    pub fn observe(&mut self, observation: Observation) -> Result<()> {
        self.repository.transact(|tx| {
            if observation.basis.sequence.0 == 0
                || observation.basis.run_revision.0 == 0
                || observation.basis.cell_revision.0 == 0
                || observation.logical.visit.0 == 0
            {
                return invalid("incomplete observation basis");
            }
            match (&observation.logical.stage, &observation.target) {
                (Stage::BeginPart | Stage::CompletePart,ObservedTarget::Part {value}) if value.run==self.scope.run && value.ordinal==observation.logical.visit && value.revision.0>0 && (observation.logical.stage!=Stage::CompletePart||value.disposition==rx_process_contract::execution::PartDisposition::ConfirmedCompleted)=>{},
                (stage, ObservedTarget::Checkpoint { decision })
                    if *stage == Stage::checkpoint(decision.action()) => {}
                (Stage::ReconcileOperation, ObservedTarget::Released { .. }) => {}
                (
                    Stage::PauseRun,
                    ObservedTarget::Run {
                        run,
                        revision,
                        state,
                    },
                ) if run == &self.scope.run
                    && revision.0 > 0
                    && !matches!(
                        state,
                        rx_process_contract::execution::RunState::Executing
                            | rx_process_contract::execution::RunState::Prepared
                    ) => {}
                (Stage::ResolveActivation, ObservedTarget::Activation { value })
                    if value.node == observation.logical.node
                        && value.visit == observation.logical.visit => {}
                (Stage::SubmitOperation, ObservedTarget::Operation { .. }) => {}
                _ => return invalid("observed target differs from logical action"),
            }
            if let Some(entry) = current(tx, &observation.logical, &self.scope)? {
                match (&entry.body, &observation.target) {
                    (Body::BeginPart {..},ObservedTarget::Part {value})=>{
                        if let Resolution::Reply {response}=&entry.resolution && let Response::Part(old)=response.as_ref() && old.id!=value.id{return Err(StoreError::Integrity("acknowledged part identity changed".into()));}
                    }
                    (Body::CompletePart {command,..},ObservedTarget::Part {value}) if command.part==value.id=>{},
                    (
                        Body::CommitCheckpoint {
                            decision: expected, ..
                        },
                        ObservedTarget::Checkpoint { decision: actual },
                    ) => {
                        if matches!(entry.resolution, Resolution::Reply { .. })
                            && bytes(expected)? != bytes(actual)?
                        {
                            return Err(StoreError::Integrity(
                                "acknowledged checkpoint decision changed".into(),
                            ));
                        }
                    }
                    (
                        Body::ReconcileOperation {
                            operation: a,
                            intent_digest: x,
                            ..
                        },
                        ObservedTarget::Released {
                            operation: b,
                            intent_digest: y,
                        },
                    ) if a == b && x == y => {}
                    (Body::PauseRun { run, .. }, ObservedTarget::Run { run: actual, .. })
                        if run == actual => {}
                    (Body::ResolveActivation { .. }, ObservedTarget::Activation { value }) => {
                        if let Resolution::Reply { response } = &entry.resolution
                            && let Response::Activation(prior) = response.as_ref()
                            && prior.id != value.id
                        {
                            return Err(StoreError::Integrity(
                                "observed activation identity changed".into(),
                            ));
                        }
                    }
                    (
                        Body::SubmitOperation {
                            activation, intent, ..
                        },
                        ObservedTarget::Operation {
                            activation: actual,
                            operation,
                            intent_digest,
                        },
                    ) => {
                        if activation != actual
                            || intent
                                .digest()
                                .map_err(|e| StoreError::Invalid(e.to_string()))?
                                != *intent_digest
                        {
                            return Err(StoreError::Integrity(
                                "observed operation binding changed".into(),
                            ));
                        }
                        if let Resolution::Reply { response } = &entry.resolution
                            && let Response::Admission(prior) = response.as_ref()
                            && &prior.operation != operation
                        {
                            return Err(StoreError::Integrity(
                                "observed operation identity changed".into(),
                            ));
                        }
                    }
                    _ => return invalid("observation/request kind mismatch"),
                }
            }

            let k = key("executor-observed", &observation.logical)?;
            let old = tx.get(&k)?;
            if let Some(row) = &old {
                let prior: Observation = decode(row, "rx.executor-observation.v1")?;
                let same_identity = match (&prior.target, &observation.target) {
                    (ObservedTarget::Part {value:a},ObservedTarget::Part {value:b})=>a.id==b.id&&a.run==b.run&&a.ordinal==b.ordinal&&(b.revision>a.revision||bytes(a)?==bytes(b)?),
                    (
                        ObservedTarget::Checkpoint { decision: a },
                        ObservedTarget::Checkpoint { decision: b },
                    ) => bytes(a)? == bytes(b)?,
                    (
                        ObservedTarget::Released {
                            operation: a,
                            intent_digest: x,
                        },
                        ObservedTarget::Released {
                            operation: b,
                            intent_digest: y,
                        },
                    ) => a == b && x == y,
                    (
                        ObservedTarget::Run {
                            run: a,
                            revision: x,
                            ..
                        },
                        ObservedTarget::Run {
                            run: b,
                            revision: y,
                            ..
                        },
                    ) => a == b && y >= x,
                    (
                        ObservedTarget::Activation { value: a },
                        ObservedTarget::Activation { value: b },
                    ) => a.id == b.id,
                    (
                        ObservedTarget::Operation {
                            activation: a,
                            operation: x,
                            intent_digest: i,
                        },
                        ObservedTarget::Operation {
                            activation: b,
                            operation: y,
                            intent_digest: j,
                        },
                    ) => a == b && x == y && i == j,
                    _ => false,
                };
                if !same_identity {
                    return Err(StoreError::Integrity(
                        "observed logical identity changed".into(),
                    ));
                }
                if observation.basis.sequence < prior.basis.sequence {
                    return invalid("observation source regressed");
                }
                if bytes(&prior.target)? == bytes(&observation.target)?
                    && prior.basis.runtime_boot == observation.basis.runtime_boot
                {
                    return Ok(());
                }
            }
            tx.put(
                &k,
                old.map(|v| v.revision),
                &doc("rx.executor-observation.v1", &observation)?,
            )?;
            event(tx, "rx.executor-observed.v1", &observation)
        })
    }
    fn change(
        &mut self,
        request: &Id,
        update: impl FnOnce(&mut Entry) -> Result<()>,
        event_schema: &str,
    ) -> Result<Entry> {
        self.repository.transact(|tx| {
            let k = key("executor-attempt", request)?;
            let row = tx
                .get(&k)?
                .ok_or(StoreError::Invalid("request not found".into()))?;
            let mut entry: Entry = decode(&row, ENTRY)?;
            if entry.key != *request {
                return Err(StoreError::Integrity("request key/record mismatch".into()));
            }
            validate_entry(&self.scope, &entry)?;
            let before = bytes(&entry)?;
            update(&mut entry)?;
            validate_entry(&self.scope, &entry)?;
            if let Resolution::Reply { response } = &entry.resolution
                && let Response::Part(reply) = response.as_ref()
                && let Some(row) = tx.get(&key("executor-observed", &entry.logical)?)? {
                let observation:Observation=decode(&row,"rx.executor-observation.v1")?;
                if !matches!(observation.target,ObservedTarget::Part { value } if value.id==reply.id && value.run==reply.run && value.ordinal==reply.ordinal) {
                    return Err(StoreError::Integrity("late part reply contradicts P identity".into()));
                }
            }
            if matches!(entry.resolution, Resolution::Reply { .. })
                && let Body::CommitCheckpoint { decision, .. } = &entry.body
                && let Some(row) = tx.get(&key("executor-observed", &entry.logical)?)? {
                let observed: Observation = decode(&row, "rx.executor-observation.v1")?;
                if !matches!(observed.target, ObservedTarget::Checkpoint { decision: actual } if bytes(&actual)? == bytes(decision)?) {
                    return Err(StoreError::Integrity("late checkpoint reply contradicts observed P decision".into()));
                }
            }
            if bytes(&entry)? != before {
                tx.put(&k, Some(row.revision), &doc(ENTRY, &entry)?)?;
                event(tx, event_schema, &entry)?;
            }
            Ok(entry)
        })
    }
}
fn validate_entry(scope: &Scope, entry: &Entry) -> Result<()> {
    entry.body.validate(scope, &entry.logical)?;
    if entry.logical.control.as_ref().is_some_and(|v| {
        v.session != entry.context.executor_session || v.epoch != entry.context.epoch
    }) {
        return Err(StoreError::Integrity(
            "stored control context mismatch".into(),
        ));
    }
    if let Resolution::Reply { response } = &entry.resolution
        && let Response::Run(value) = response.as_ref()
        && value.recipe != scope.resolved_digest
    {
        return Err(StoreError::Integrity("run response recipe differs".into()));
    }
    if entry.context.run != scope.run
        || entry.context.resolved_digest != scope.resolved_digest
        || entry.context.visit != entry.logical.visit
        || entry.context.epoch.0 == 0
    {
        return Err(StoreError::Integrity("stored context differs".into()));
    }
    if matches!(
        entry.resolution,
        Resolution::Reply { .. }
            | Resolution::RevisionRejected
            | Resolution::CheckpointRejected { .. }
    ) && entry.send != SendState::EmitEntered
    {
        return Err(StoreError::Integrity(
            "RPC resolution without entered stage".into(),
        ));
    }
    if matches!(entry.resolution, Resolution::CheckpointRejected { .. })
        && !entry.logical.stage.is_checkpoint()
    {
        return invalid("checkpoint rejection on another method");
    }
    if let Resolution::Reply { response } = &entry.resolution
        && let Response::Run(run) = response.as_ref()
        && entry.logical.stage.is_checkpoint()
        && run.executor_session.as_ref() != Some(&entry.context.executor_session)
    {
        return invalid("checkpoint response executor differs");
    }
    if entry.generation.0 == 0
        || entry.logical.visit.0 == 0
        || entry.body_digest != digest("RX-E-REQUEST-BODY-v1", &entry.body)?
    {
        return Err(StoreError::Integrity(
            "request body identity differs".into(),
        ));
    }
    if let Resolution::Reply { response } = &entry.resolution {
        validate_response(&entry.body, response)?;
    }
    Ok(())
}
fn validate_response(body: &Body, response: &Response) -> Result<()> {
    match (body, response) {
        (Body::BeginPart { run, ordinal, .. }, Response::Part(value))
            if *run == value.run
                && *ordinal == value.ordinal
                && value.revision == Counter(1)
                && value.disposition
                    == rx_process_contract::execution::PartDisposition::InProgress =>
        {
            Ok(())
        }
        (Body::CompletePart { ordinal, command }, Response::Part(value))
            if command.run == value.run
                && command.part == value.id
                && *ordinal == value.ordinal
                && value.revision.0 > 0
                && value.disposition
                    == rx_process_contract::execution::PartDisposition::ConfirmedCompleted =>
        {
            Ok(())
        }
        (Body::CommitCheckpoint { command, .. }, Response::Run(value))
            if value.run == command.run
                && value.revision == command.new_checkpoint.revision
                && value.state == rx_process_contract::execution::RunState::Executing
                && bytes(&value.checkpoint)? == bytes(&command.new_checkpoint)? =>
        {
            Ok(())
        }
        (
            Body::ReconcileOperation {
                operation: a,
                intent_digest: x,
                ..
            },
            Response::ReconciliationAccepted {
                operation: b,
                intent_digest: y,
            },
        ) if a == b && x == y => Ok(()),
        (Body::PauseRun { run, .. }, Response::Run(value))
            if run == &value.run
                && value.revision.0 > 0
                && value.checkpoint.run == value.run
                && value.checkpoint.revision == value.revision
                && !matches!(
                    value.state,
                    rx_process_contract::execution::RunState::Executing
                        | rx_process_contract::execution::RunState::Prepared
                ) =>
        {
            Ok(())
        }
        (Body::ResolveActivation { node, visit, .. }, Response::Activation(value))
            if &value.node == node && &value.visit == visit =>
        {
            Ok(())
        }
        (Body::SubmitOperation { intent, .. }, Response::Admission(value))
            if value.intent_digest
                == intent
                    .digest()
                    .map_err(|e| StoreError::Invalid(e.to_string()))?
                && value.revision == Counter(1)
                && value.sequence.0 > 0 =>
        {
            Ok(())
        }
        _ => Err(StoreError::Integrity(
            "RPC response does not match request".into(),
        )),
    }
}
fn current(tx: &mut dyn Transaction, logical: &Logical, scope: &Scope) -> Result<Option<Entry>> {
    let Some(row) = tx.get(&key("executor-current", logical)?)? else {
        return Ok(None);
    };
    let request: Id = decode(&row, "rx.executor-current.v1")?;
    let row = tx
        .get(&key("executor-attempt", &request)?)?
        .ok_or(StoreError::Integrity("current request missing".into()))?;
    let entry: Entry = decode(&row, ENTRY)?;
    validate_entry(scope, &entry)?;
    if entry.logical != *logical || entry.key != request {
        return Err(StoreError::Integrity(
            "current request identity differs".into(),
        ));
    }
    Ok(Some(entry))
}
fn invalid<T>(why: &str) -> Result<T> {
    Err(StoreError::Invalid(why.into()))
}
fn name(s: &str) -> Name {
    Name::new(s).expect("internal schema/key")
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).expect("UUID")
}
fn bytes<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    canonical::bytes(value).map_err(|e| StoreError::Invalid(e.to_string()))
}
fn digest<T: Serialize>(domain: &str, value: &T) -> Result<Digest> {
    canonical::digest(domain, value).map_err(|e| StoreError::Invalid(e.to_string()))
}
fn key<T: Serialize>(prefix: &str, value: &T) -> Result<Name> {
    Ok(name(&format!(
        "{prefix}/{}",
        digest("RX-E-JOURNAL-KEY-v1", value)?
    )))
}
fn doc<T: Serialize>(schema: &str, value: &T) -> Result<Document> {
    Ok(Document {
        schema: name(schema),
        value: serde_json::to_value(value).map_err(|e| StoreError::Invalid(e.to_string()))?,
    })
}
fn decode<T: DeserializeOwned>(row: &Record, schema: &str) -> Result<T> {
    if row.document.schema.as_str() != schema {
        return Err(StoreError::Integrity("journal schema differs".into()));
    }
    canonical::decode_json(&bytes(&row.document.value)?)
        .map_err(|e| StoreError::Integrity(e.to_string()))
}
fn event<T: Serialize>(tx: &mut dyn Transaction, schema: &str, value: &T) -> Result<()> {
    tx.append(&id(), &doc(schema, value)?)?;
    Ok(())
}
