use super::*;
use rx_process_contract::checkpoint_change::{
    CheckpointAction, CheckpointDecision, CheckpointTarget,
};

impl<R: Repository> Worker<R> {
    pub(super) fn recover_checkpoints(
        &mut self,
        snapshot: &ValidatedSnapshot,
    ) -> Result<(), Error> {
        let cp = &snapshot.data().process_checkpoint;
        for (action, nodes) in [
            (
                CheckpointAction::ChooseBranch,
                cp.branches.keys().collect::<Vec<_>>(),
            ),
            (
                CheckpointAction::StartWait,
                cp.wait_windows.keys().collect(),
            ),
            (CheckpointAction::CheckWait, cp.waits.keys().collect()),
        ] {
            for node in nodes {
                let target = CheckpointTarget {
                    run: cp.run.clone(),
                    node: node.clone(),
                    visit: cp.visit,
                    action,
                };
                self.journal.observe(Observation {
                    logical: Logical {
                        visit: cp.visit,
                        node: node.clone(),
                        stage: Stage::checkpoint(action),
                        control: None,
                    },
                    target: ObservedTarget::Checkpoint {
                        decision: CheckpointDecision::at(cp, &target)
                            .ok_or_else(|| Error::Invalid("checkpoint decision missing".into()))?,
                    },
                    basis: basis(snapshot),
                })?;
            }
        }
        Ok(())
    }
    pub(super) async fn checkpoint(
        &mut self,
        request: Request,
        snapshot: &ValidatedSnapshot,
    ) -> Result<Outcome, Error> {
        self.recover(snapshot)?;
        let action = match request.kind {
            RequestKind::ResolveBranch => CheckpointAction::ChooseBranch,
            RequestKind::BeginWait
                if snapshot
                    .data()
                    .process_checkpoint
                    .wait_windows
                    .contains_key(&request.node) =>
            {
                CheckpointAction::CheckWait
            }
            RequestKind::BeginWait => CheckpointAction::StartWait,
            _ => return Err(Error::Invalid("checkpoint request kind".into())),
        };
        let target = CheckpointTarget {
            run: request.identity.run,
            visit: request.identity.visit,
            node: request.node.clone(),
            action,
        };
        let logical = Logical {
            node: request.node.clone(),
            visit: target.visit,
            stage: Stage::checkpoint(action),
            control: None,
        };
        if CheckpointDecision::at(&snapshot.data().process_checkpoint, &target).is_some() {
            return Ok(Outcome::CheckpointObserved {
                node: request.node,
                action,
            });
        }
        if !snapshot.is_current() {
            return Ok(Outcome::RefreshRequired);
        }
        if !snapshot.data().request_admission_allowed {
            return Ok(Outcome::WaitingForAuthority);
        }
        let body = if let Some(entry) = self.journal.get(&logical)? {
            if entry.context != snapshot.context_identity() {
                return Ok(Outcome::ContextChanged);
            }
            match entry.resolution {
                Resolution::Pending => Some(entry.body),
                Resolution::RevisionRejected | Resolution::CheckpointRejected { .. } => None,
                Resolution::Attention { code } => return Ok(Outcome::Attention(code)),
                Resolution::Reply { .. } => {
                    return Err(Error::Invalid(
                        "P lost an acknowledged process decision".into(),
                    ));
                }
            }
        } else {
            None
        };
        let body = if let Some(body) = body {
            body
        } else {
            match self.client.prepare_checkpoint(target, snapshot).await {
                Ok(crate::client::Preparation::Ready(value)) => match value.into_body() {
                    Ok(body) => body,
                    Err(Error::Expired) => return Ok(Outcome::RefreshRequired),
                    Err(error) => return Err(error),
                },
                Ok(crate::client::Preparation::Waiting) => return Ok(Outcome::WaitingCondition),
                Ok(crate::client::Preparation::Changed) | Err(Error::Expired) => {
                    return Ok(Outcome::RefreshRequired);
                }
                Err(error) => return Err(error),
            }
        };
        match self.execute(logical, body, snapshot).await? {
            Execution::Reply(Response::Run(_)) => Ok(Outcome::CheckpointCommitted {
                node: request.node,
                action,
            }),
            Execution::Reply(_) => Err(Error::Invalid("checkpoint response kind differs".into())),
            Execution::Deferred(outcome) => Ok(outcome),
        }
    }
}
