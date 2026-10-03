//! V2 requests use the same journal/emit machinery; P's existing frontier owns order.
use super::*;
use crate::client::{ValidatedProduction, execution_v2::ValidatedExecution};
use rx_process_contract::production;
impl RequestBasis for ValidatedExecution {
    fn context_identity(&self) -> crate::frame::Identity {
        let c = &self.data().context;
        crate::frame::Identity {
            run: c.run.run.id.clone(),
            executor_session: c.caller_session.clone(),
            resolved_digest: c.resolved.sha256,
            visit: c.visit,
            epoch: c.cell_epoch,
        }
    }
    fn journal_basis(&self) -> Basis {
        let c = &self.data().context;
        Basis {
            runtime_boot: c.runtime_boot.clone(),
            sequence: c.sequence,
            run_revision: c.run.revision,
            cell_revision: c.cell_revision,
            checked_at: c.checked_at.clone(),
        }
    }
    fn is_current(&self) -> bool {
        self.is_current()
    }
}
impl<R: Repository> Worker<R> {
    pub(crate) fn select_execution_v2(&mut self) {
        self.execution_v2 = true;
    }
    pub async fn negotiate_execution(&mut self) -> Result<(), Error> {
        self.select_execution_v2();
        self.client.negotiate_execution().await
    }
    pub async fn execution_snapshot(
        &mut self,
        visit: Counter,
    ) -> Result<ValidatedExecution, Error> {
        let scope = self.journal.scope();
        self.client
            .execution_snapshot(&scope.run, visit, scope.resolved_digest)
            .await
    }
    pub(super) async fn complete_execution_part(
        &mut self,
        view: &ValidatedProduction,
        part: &production::Part,
    ) -> Result<Coordination, Error> {
        let snapshot = self.execution_snapshot(part.ordinal).await?;
        if snapshot.context_identity() != view.identity(part.ordinal) {
            return Ok(Coordination::Deferred(Outcome::ContextChanged));
        }
        if snapshot.frontier().state != rx_process_contract::frontier::State::Completed {
            return Ok(Coordination::Visit(part.ordinal));
        }
        let c = &snapshot.data().context;
        let body = Body::CompletePart {
            ordinal: part.ordinal,
            command: Box::new(production::CompletePart {
                cell: c.run.run.cell.clone(),
                run: part.run.clone(),
                part: part.id.clone(),
                expected_run: c.run.revision,
                expected_part: part.revision,
            }),
        };
        let logical = Logical {
            visit: part.ordinal,
            node: name("production/part"),
            stage: Stage::CompletePart,
            control: None,
        };
        match self.execute(logical, body, &snapshot).await? {
            Execution::Reply(Response::Part(_)) => Ok(Coordination::Waiting),
            Execution::Deferred(v) => Ok(Coordination::Deferred(v)),
            _ => Err(Error::Invalid("v2 completion reply differs".into())),
        }
    }
    pub async fn handle_execution_snapshot(
        &mut self,
        snapshot: &ValidatedExecution,
    ) -> Result<Outcome, Error> {
        let data = snapshot.data();
        let c = &data.context;
        let scope = self.journal.scope();
        if c.installation != scope.installation
            || c.store_generation != scope.store_generation
            || c.run.run.cell != scope.cell
            || c.run.run.id != scope.run
            || c.definition.sha256 != scope.definition
            || c.resolved.sha256 != scope.resolved_digest
        {
            return Err(Error::Invalid("v2 snapshot/journal scope differs".into()));
        }
        for activation in c
            .run
            .checkpoint
            .activations
            .iter()
            .filter(|a| a.visit == c.visit)
        {
            if let Some(p) = c.progress.operations.get(&activation.node) {
                self.journal.observe(Observation {
                    logical: Logical {
                        visit: c.visit,
                        node: activation.node.clone(),
                        stage: Stage::SubmitOperation,
                        control: None,
                    },
                    target: ObservedTarget::Operation {
                        activation: activation.id.clone(),
                        operation: p.operation.id().clone(),
                        intent_digest: p.intent_digest,
                    },
                    basis: snapshot.journal_basis(),
                })?;
            }
        }
        // Reconciliation reads preserve original identity even while new admission is disabled.
        if let Some(node) = snapshot.frontier().handovers.first() {
            let p = c
                .progress
                .operations
                .get(node)
                .ok_or_else(|| Error::Invalid("v2 handover operation missing".into()))?;
            let logical = Logical {
                visit: c.visit,
                node: node.clone(),
                stage: Stage::ReconcileOperation,
                control: None,
            };
            if let Some(entry) = self.journal.get(&logical)?
                && matches!(entry.resolution, Resolution::Reply { .. })
            {
                return Ok(Outcome::ReconciliationPending(p.operation.id().clone()));
            }
            let body = Body::ReconcileOperation {
                run: c.run.run.id.clone(),
                operation: p.operation.id().clone(),
                intent_digest: p.intent_digest,
            };
            return match self.execute(logical, body, snapshot).await? {
                Execution::Reply(Response::ReconciliationAccepted { operation, .. }) => {
                    Ok(Outcome::ReconciliationPending(operation))
                }
                Execution::Deferred(v) => Ok(v),
                _ => Err(Error::Invalid("v2 reconciliation reply differs".into())),
            };
        }
        if !snapshot.is_current() {
            return Ok(Outcome::RefreshRequired);
        }
        if !c.request_admission_allowed {
            return Ok(Outcome::WaitingForAuthority);
        }
        let Some(node) = snapshot.frontier().operations.first() else {
            return Ok(snapshot
                .frontier()
                .pending_operations
                .first()
                .map(|operation| Outcome::ObservedOperation(operation.clone()))
                .unwrap_or(Outcome::WaitingForAuthority));
        };
        if snapshot.frontier().operations.len() != 1 {
            return Err(Error::Invalid(
                "v2 finite sequence has competing frontier".into(),
            ));
        }
        let compiled = rx_process_contract::validation::nodes(snapshot.process())
            .into_iter()
            .find(|n| &n.id == node)
            .ok_or_else(|| Error::Invalid("v2 node absent".into()))?;
        let CompiledBody::Operation { binding } = &compiled.body else {
            return Err(Error::Invalid("v2 operation required".into()));
        };
        let action = &snapshot.process().bindings[binding];
        let body = Body::SubmitExecutionNode(Box::new(crate::journal::execution_v2::Submit {
            cell: c.run.run.cell.clone(),
            node: node.clone(),
            mandate: c
                .run
                .run
                .mandate
                .clone()
                .ok_or_else(|| Error::Invalid("v2 mandate missing".into()))?,
            expected_cell: c.cell_revision,
            expected_run: c.run.revision,
            part: data.part.clone(),
            workflow_node: data.plan.binding.nodes[node].clone(),
            host: action.host.clone(),
            intent_digest: action
                .intent
                .digest()
                .map_err(|e| Error::Invalid(e.to_string()))?,
            epoch: c.cell_epoch,
        }));
        let logical = Logical {
            visit: c.visit,
            node: node.clone(),
            stage: Stage::SubmitOperation,
            control: None,
        };
        match self.execute(logical, body, snapshot).await? {
            Execution::Reply(Response::ExecutionAdmission(v)) => {
                Ok(Outcome::Admitted(v.binding.operation.clone()))
            }
            Execution::Deferred(v) => Ok(v),
            _ => Err(Error::Invalid("v2 admission reply differs".into())),
        }
    }
}
