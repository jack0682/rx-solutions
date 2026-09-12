//! Durable per-node request handling. P remains the only operation/authority writer.
mod checkpoint;
mod lifecycle;
mod production;
use crate::{
    Client, Error, ValidatedSnapshot,
    frame::{Request, RequestKind},
    journal::*,
};
pub use lifecycle::StopController;
pub use production::Coordination;
use rx_domain::{intent::Kind, types::*};
use rx_ports::Repository;
use rx_process_contract::{
    CompiledBody,
    execution::{ActivationSnapshot, Purpose},
};
#[derive(Clone, Debug)]
pub enum Outcome {
    CheckpointObserved {
        node: Name,
        action: rx_process_contract::checkpoint_change::CheckpointAction,
    },
    CheckpointCommitted {
        node: Name,
        action: rx_process_contract::checkpoint_change::CheckpointAction,
    },
    WaitingCondition,
    ObservedOperation(Id),
    Admitted(Id),
    RefreshRequired,
    WaitingForAuthority,
    ContextChanged,
    Attention(Name),
    Unsupported(RequestKind),
    ReconciliationPending(Id),
    ResourcesReleased(Id),
    Paused {
        run: Id,
        revision: Counter,
    },
}
pub struct Worker<R> {
    client: Client,
    journal: Journal<R>,
    #[cfg(feature = "test-harness")]
    test_hook: Option<TestHook>,
}
#[cfg(feature = "test-harness")]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TestPoint {
    Entered,
    RemoteReply,
}
#[cfg(feature = "test-harness")]
pub type TestHook = std::sync::Arc<dyn Fn(TestPoint, &Entry) + Send + Sync>;
impl<R: Repository> Worker<R> {
    pub fn new(client: Client, journal: Journal<R>) -> Result<Self, Error> {
        let pin = client.peer_pin();
        let scope = journal.scope();
        if pin.installation != scope.installation
            || pin.store_generation != scope.store_generation
            || pin.principal != scope.principal
            || pin.release != scope.release
            || pin.cell != scope.cell
            || pin.definition != scope.definition
        {
            return Err(Error::Invalid("client and journal scopes differ".into()));
        }
        Ok(Self {
            client,
            journal,
            #[cfg(feature = "test-harness")]
            test_hook: None,
        })
    }
    #[cfg(feature = "test-harness")]
    pub fn with_test_hook(mut self, hook: TestHook) -> Self {
        self.test_hook = Some(hook);
        self
    }
    pub fn journal(&mut self) -> &mut Journal<R> {
        &mut self.journal
    }
    /// Return the same authenticated Client and durable journal without reopening either.
    /// The caller must separately check the service's planner cleanup and P authority.
    pub fn into_parts(self) -> (Client, Journal<R>) {
        (self.client, self.journal)
    }
    pub async fn snapshot(&mut self, visit: Counter) -> Result<ValidatedSnapshot, Error> {
        let snapshot = self
            .client
            .snapshot(&self.journal.scope().run, visit)
            .await?;
        if snapshot.data().resolved.sha256 != self.journal.scope().resolved_digest {
            return Err(Error::Invalid("journal recipe changed".into()));
        }
        Ok(snapshot)
    }
    /// Query-based recovery records P identities but does not claim an RPC reply or native success.
    pub fn recover(&mut self, snapshot: &ValidatedSnapshot) -> Result<(), Error> {
        self.check_snapshot_scope(snapshot)?;
        for activation in snapshot
            .data()
            .run
            .checkpoint
            .activations
            .iter()
            .filter(|a| a.visit == snapshot.data().visit)
        {
            self.journal.observe(Observation {
                logical: Logical {
                    visit: activation.visit,
                    node: activation.node.clone(),
                    stage: Stage::ResolveActivation,
                    control: None,
                },
                target: ObservedTarget::Activation {
                    value: activation.clone(),
                },
                basis: basis(snapshot),
            })?;
            if let Some(progress) = snapshot.data().progress.operations.get(&activation.node) {
                self.journal.observe(Observation {
                    logical: Logical {
                        visit: activation.visit,
                        node: activation.node.clone(),
                        stage: Stage::SubmitOperation,
                        control: None,
                    },
                    target: ObservedTarget::Operation {
                        activation: activation.id.clone(),
                        operation: progress.operation.id().clone(),
                        intent_digest: progress.intent_digest,
                    },
                    basis: basis(snapshot),
                })?;
                if progress.operation.disposition() == rx_domain::operation::Disposition::Released {
                    self.journal.observe(Observation {
                        logical: Logical {
                            visit: activation.visit,
                            node: activation.node.clone(),
                            stage: Stage::ReconcileOperation,
                            control: None,
                        },
                        target: ObservedTarget::Released {
                            operation: progress.operation.id().clone(),
                            intent_digest: progress.intent_digest,
                        },
                        basis: basis(snapshot),
                    })?;
                }
            }
        }
        self.recover_checkpoints(snapshot)?;
        Ok(())
    }
    pub async fn handle(&mut self, request: Request) -> Result<Outcome, Error> {
        let mut snapshot = self.snapshot(request.identity.visit).await?;
        if request.kind == RequestKind::PauseExecutor {
            if request.node != snapshot.process().root.id
                || request.identity.run != snapshot.data().run.run.id
                || request.identity.resolved_digest != snapshot.data().resolved.sha256
                || !request.argument.is_empty()
                || request.timeout_ns.0 != 0
            {
                return Err(Error::Invalid("invalid pause request scope".into()));
            }
            return self.pause(request, &snapshot).await;
        }
        if request.identity != snapshot.context_identity() {
            return Ok(Outcome::ContextChanged);
        }
        let nodes = rx_process_contract::validation::nodes(snapshot.process());
        let node = nodes
            .iter()
            .find(|n| n.id == request.node)
            .ok_or_else(|| Error::Invalid("request node is not in resolved process".into()))?;
        let body = node.body.clone();
        validate_request(&request, &body, &snapshot)?;
        if matches!(
            request.kind,
            RequestKind::ResolveBranch | RequestKind::BeginWait
        ) {
            return self.checkpoint(request, &snapshot).await;
        }
        if request.kind == RequestKind::RequestHandover {
            return self.handover(request, &snapshot).await;
        }
        if request.kind != RequestKind::SubmitOperation {
            return Ok(Outcome::Unsupported(request.kind));
        }
        self.recover(&snapshot)?;
        if let Some(progress) = snapshot.data().progress.operations.get(&request.node) {
            return Ok(Outcome::ObservedOperation(progress.operation.id().clone()));
        }
        if !snapshot.is_current() {
            return Ok(Outcome::RefreshRequired);
        }
        if !snapshot.data().request_admission_allowed {
            return Ok(Outcome::WaitingForAuthority);
        }
        if !snapshot.frontier().operations.contains(&request.node) {
            return Ok(Outcome::RefreshRequired);
        }
        let CompiledBody::Operation { binding } = body else {
            return Err(Error::Invalid("operation node required".into()));
        };
        let intent = snapshot.process().bindings[&binding]
            .intent
            .clone()
            .normalized()
            .map_err(|e| Error::Invalid(e.to_string()))?;
        if intent.kind == Kind::ControlSession {
            return Ok(Outcome::Attention(name(
                "continuous-control-not-implemented",
            )));
        }
        let activation = if let Some(value) = find_activation(&snapshot, &request.node) {
            value.clone()
        } else {
            let logical = Logical {
                visit: request.identity.visit,
                node: request.node.clone(),
                stage: Stage::ResolveActivation,
                control: None,
            };
            let body = Body::ResolveActivation {
                run: request.identity.run.clone(),
                node: request.node.clone(),
                visit: request.identity.visit,
                expected_run: snapshot.data().run.revision,
            };
            match self.execute(logical, body, &snapshot).await? {
                Execution::Reply(Response::Activation(value)) => value,
                Execution::Reply(_) => {
                    return Err(Error::Invalid("wrong activation response".into()));
                }
                Execution::Deferred(value) => return Ok(value),
            }
        };
        // T3 changes the run revision. Obtain a new live cut before preparing T1.
        snapshot = self.snapshot(request.identity.visit).await?;
        if snapshot.context_identity() != request.identity {
            return Ok(Outcome::ContextChanged);
        }
        self.recover(&snapshot)?;
        if let Some(progress) = snapshot.data().progress.operations.get(&request.node) {
            return Ok(Outcome::ObservedOperation(progress.operation.id().clone()));
        }
        if !snapshot.is_current() {
            return Ok(Outcome::RefreshRequired);
        }
        if !snapshot.data().request_admission_allowed {
            return Ok(Outcome::WaitingForAuthority);
        }
        if !snapshot.frontier().operations.contains(&request.node) {
            return Ok(Outcome::RefreshRequired);
        }
        let current = find_activation(&snapshot, &request.node)
            .ok_or_else(|| Error::Invalid("P lost the resolved activation".into()))?;
        if current.id != activation.id {
            return Err(Error::Invalid("activation identity changed".into()));
        }
        let run = &snapshot.data().run.run;
        let part = match run.purpose {
            Some(Purpose::Production) => Some(
                run.part_ids
                    .get((request.identity.visit.0 - 1) as usize)
                    .ok_or_else(|| Error::Invalid("part missing".into()))?
                    .clone(),
            ),
            Some(Purpose::Setup) => None,
            _ => return Err(Error::Invalid("run purpose missing".into())),
        };
        let body = Body::SubmitOperation {
            cell: run.cell.clone(),
            run: run.id.clone(),
            activation: activation.id,
            part,
            slot: name("main"),
            intent: Box::new(intent),
            mandate: run
                .mandate
                .clone()
                .ok_or_else(|| Error::Invalid("mandate missing".into()))?,
            expected_cell: snapshot.data().cell_revision,
            expected_run: snapshot.data().run.revision,
        };
        let logical = Logical {
            visit: request.identity.visit,
            node: request.node,
            stage: Stage::SubmitOperation,
            control: None,
        };
        match self.execute(logical, body, &snapshot).await? {
            Execution::Reply(Response::Admission(value)) => Ok(Outcome::Admitted(value.operation)),
            Execution::Reply(_) => Err(Error::Invalid("wrong admission response".into())),
            Execution::Deferred(value) => Ok(value),
        }
    }
    async fn handover(
        &mut self,
        request: Request,
        snapshot: &ValidatedSnapshot,
    ) -> Result<Outcome, Error> {
        use rx_domain::operation::{Disposition, Integrity, Outcome as WorkOutcome};
        let Some(progress) = snapshot.data().progress.operations.get(&request.node) else {
            return Ok(Outcome::RefreshRequired);
        };
        let operation = progress.operation.id().clone();
        let logical = Logical {
            visit: request.identity.visit,
            node: request.node,
            stage: Stage::ReconcileOperation,
            control: None,
        };
        if progress.operation.disposition() == Disposition::Released {
            self.journal.observe(Observation {
                logical,
                target: ObservedTarget::Released {
                    operation: operation.clone(),
                    intent_digest: progress.intent_digest,
                },
                basis: basis(snapshot),
            })?;
            return Ok(Outcome::ResourcesReleased(operation));
        }
        if progress.operation.integrity() != Integrity::Valid
            || progress.operation.outcome() != WorkOutcome::Succeeded
        {
            return Ok(Outcome::Attention(name(
                "handover-needs-result-reconciliation",
            )));
        }
        let body = Body::ReconcileOperation {
            run: request.identity.run,
            operation: operation.clone(),
            intent_digest: progress.intent_digest,
        };
        if let Some(entry) = self.journal.get(&logical)? {
            if entry.context != snapshot.context_identity() {
                return Ok(Outcome::ContextChanged);
            }
            if entry.body.semantic_digest()? != body.semantic_digest()? {
                return Err(Error::Invalid("reconciliation mapping changed".into()));
            }
            if let Resolution::Reply { response } = &entry.resolution
                && matches!(response.as_ref(), Response::ReconciliationAccepted { .. })
            {
                return Ok(Outcome::ReconciliationPending(operation));
            }
        }
        match self.execute(logical, body, snapshot).await? {
            Execution::Reply(Response::ReconciliationAccepted { .. }) => {
                Ok(Outcome::ReconciliationPending(operation))
            }
            Execution::Reply(_) => Err(Error::Invalid("query response differs".into())),
            Execution::Deferred(value) => Ok(value),
        }
    }
    async fn pause(
        &mut self,
        request: Request,
        snapshot: &ValidatedSnapshot,
    ) -> Result<Outcome, Error> {
        use rx_process_contract::execution::RunState;
        let logical = Logical {
            visit: request.identity.visit,
            node: request.node.clone(),
            stage: Stage::PauseRun,
            control: Some(ControlIdentity {
                session: request.identity.executor_session.clone(),
                epoch: request.identity.epoch,
            }),
        };
        let run = &snapshot.data().run;
        if !matches!(run.run.state, RunState::Executing | RunState::Prepared) {
            self.journal.observe(Observation {
                logical,
                target: ObservedTarget::Run {
                    run: run.run.id.clone(),
                    revision: run.revision,
                    state: run.run.state,
                },
                basis: basis(snapshot),
            })?;
            return Ok(Outcome::Paused {
                run: run.run.id.clone(),
                revision: run.revision,
            });
        }
        if request.identity != snapshot.context_identity() {
            return Ok(Outcome::ContextChanged);
        }
        let body = Body::PauseRun {
            run: run.run.id.clone(),
            root: request.node,
            expected_run: run.revision,
        };
        match self.execute(logical, body, snapshot).await? {
            Execution::Reply(Response::Run(reply)) => Ok(Outcome::Paused {
                run: reply.run,
                revision: reply.revision,
            }),
            Execution::Reply(_) => Err(Error::Invalid("wrong pause response".into())),
            Execution::Deferred(value) => Ok(value),
        }
    }
    async fn execute<B: RequestBasis>(
        &mut self,
        logical: Logical,
        body: Body,
        snapshot: &B,
    ) -> Result<Execution, Error> {
        let context = snapshot.context_identity();
        let entry = if let Some(old) = self.journal.get(&logical)? {
            if old.context != context {
                return Ok(Execution::Deferred(Outcome::ContextChanged));
            }
            match old.resolution {
                Resolution::Reply { .. } => {
                    return Err(Error::Invalid(
                        "P snapshot lost an acknowledged action; reconciliation required".into(),
                    ));
                }
                Resolution::Attention { code } => {
                    return Ok(Execution::Deferred(Outcome::Attention(code)));
                }
                Resolution::Pending => {
                    if old.body.semantic_digest()? != body.semantic_digest()? {
                        return Err(Error::Invalid("pending action meaning changed".into()));
                    }
                    old
                }
                Resolution::RevisionRejected | Resolution::CheckpointRejected { .. } => self
                    .journal
                    .prepare(logical, context, body, snapshot.journal_basis())?,
            }
        } else {
            self.journal
                .prepare(logical, context, body, snapshot.journal_basis())?
        };
        if matches!(
            entry.resolution,
            Resolution::RevisionRejected | Resolution::CheckpointRejected { .. }
        ) {
            return Ok(Execution::Deferred(Outcome::RefreshRequired));
        }
        if !snapshot.is_current() {
            return Ok(Execution::Deferred(Outcome::RefreshRequired));
        }
        let entered = self.journal.enter(&entry.key)?;
        #[cfg(feature = "test-harness")]
        if let Some(hook) = &self.test_hook {
            hook(TestPoint::Entered, &entered);
        }
        // Losing the process between this durable boundary and the RPC leaves EmitEntered.
        // Such a request can only reuse its original key/body, never be silently replaced.
        if !snapshot.is_current() {
            return Ok(Execution::Deferred(Outcome::RefreshRequired));
        }
        match self.client.emit(&entered.key, &entered.body).await {
            Ok(response) => {
                #[cfg(feature = "test-harness")]
                if let Some(hook) = &self.test_hook {
                    hook(TestPoint::RemoteReply, &entered);
                }
                self.journal.reply(&entered.key, response.clone())?;
                Ok(Execution::Reply(response))
            }
            Err(Error::Rpc(status))
                if entered.logical.stage.is_checkpoint()
                    && rx_protocol::checkpoint_rejection::confirmed(&status).is_some() =>
            {
                let reason = match rx_protocol::checkpoint_rejection::confirmed(&status)
                    .expect("validated rejection")
                {
                    rx_protocol::checkpoint_rejection::Rejection::Revision => {
                        CheckpointRejection::Revision
                    }
                    rx_protocol::checkpoint_rejection::Rejection::Expired => {
                        CheckpointRejection::Expired
                    }
                };
                self.journal.checkpoint_rejected(&entered.key, reason)?;
                Ok(Execution::Deferred(Outcome::RefreshRequired))
            }
            Err(Error::Rpc(status))
                if status.code() == tonic::Code::Aborted
                    && entered.logical.stage != Stage::ReconcileOperation
                    && !entered.logical.stage.is_checkpoint() =>
            {
                self.journal.revision_rejected(&entered.key)?;
                Ok(Execution::Deferred(Outcome::RefreshRequired))
            }
            Err(Error::Rpc(status))
                if matches!(
                    status.code(),
                    tonic::Code::Unavailable
                        | tonic::Code::DeadlineExceeded
                        | tonic::Code::Cancelled
                        | tonic::Code::Internal
                        | tonic::Code::Unknown
                ) =>
            {
                Err(Error::Rpc(status))
            }
            Err(error) => {
                let code = name("request-needs-reconciliation");
                self.journal.attention(&entered.key, code)?;
                Err(error)
            }
        }
    }
    fn check_snapshot_scope(&self, snapshot: &ValidatedSnapshot) -> Result<(), Error> {
        let scope = self.journal.scope();
        let data = snapshot.data();
        if data.installation != scope.installation
            || data.store_generation != scope.store_generation
            || data.run.run.id != scope.run
            || data.resolved.sha256 != scope.resolved_digest
            || data.run.run.cell != scope.cell
            || data.definition.sha256 != scope.definition
        {
            return Err(Error::Invalid(
                "recovery snapshot differs from journal scope".into(),
            ));
        }
        Ok(())
    }
}
enum Execution {
    Reply(Response),
    Deferred(Outcome),
}
fn name(value: &str) -> Name {
    Name::new(value).expect("internal name")
}
fn basis(snapshot: &ValidatedSnapshot) -> Basis {
    let v = snapshot.data();
    Basis {
        runtime_boot: v.runtime_boot.clone(),
        sequence: v.sequence,
        run_revision: v.run.revision,
        cell_revision: v.cell_revision,
        checked_at: v.checked_at.clone(),
    }
}
fn find_activation<'a>(
    snapshot: &'a ValidatedSnapshot,
    node: &Name,
) -> Option<&'a ActivationSnapshot> {
    snapshot
        .data()
        .run
        .checkpoint
        .activations
        .iter()
        .find(|a| a.node == *node && a.visit == snapshot.data().visit)
}
fn validate_request(
    request: &Request,
    body: &CompiledBody,
    snapshot: &ValidatedSnapshot,
) -> Result<(), Error> {
    let (argument, timeout) = match (request.kind, body) {
        (
            RequestKind::SubmitOperation | RequestKind::RequestHandover,
            CompiledBody::Operation { binding },
        ) => (binding.to_string(), 0),
        (RequestKind::ResolveBranch, CompiledBody::Branch { condition, .. }) => {
            (condition.to_string(), 0)
        }
        (
            RequestKind::BeginWait,
            CompiledBody::Wait {
                condition,
                timeout_ns,
            },
        ) => (condition.to_string(), timeout_ns.0),
        (RequestKind::RequestIntervention, CompiledBody::Intervention { procedure }) => {
            (procedure.sha256.to_string(), 0)
        }
        (RequestKind::PauseExecutor, _) if request.node == snapshot.process().root.id => {
            (String::new(), 0)
        }
        _ => {
            return Err(Error::Invalid(
                "BT request kind differs from resolved node".into(),
            ));
        }
    };
    if request.argument != argument || request.timeout_ns.0 != timeout {
        return Err(Error::Invalid(
            "BT request argument differs from resolved process".into(),
        ));
    }
    Ok(())
}

pub(crate) trait RequestBasis {
    fn context_identity(&self) -> crate::frame::Identity;
    fn journal_basis(&self) -> Basis;
    fn is_current(&self) -> bool;
}
impl RequestBasis for ValidatedSnapshot {
    fn context_identity(&self) -> crate::frame::Identity {
        ValidatedSnapshot::context_identity(self)
    }
    fn journal_basis(&self) -> Basis {
        basis(self)
    }
    fn is_current(&self) -> bool {
        ValidatedSnapshot::is_current(self)
    }
}
