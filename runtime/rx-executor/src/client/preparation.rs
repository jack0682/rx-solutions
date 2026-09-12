use super::*;
use rx_process_contract::{CompiledBody, checkpoint_change::*};
use rx_protocol::executor_plan as plan;
pub(crate) enum Preparation {
    Ready(Box<PreparedAction>),
    Waiting,
    Changed,
}
pub(crate) struct PreparedAction {
    body: crate::journal::Body,
    deadline: Instant,
    valid_until: TimePoint,
    prepared_at: TimePoint,
    clock: Arc<dyn crate::clock::Clock>,
}
impl PreparedAction {
    pub(crate) fn into_body(self) -> Result<crate::journal::Body, Error> {
        let now = self.clock.now()?;
        if Instant::now() >= self.deadline
            || now.clock_id != self.valid_until.clock_id
            || now.ticks_ns < self.prepared_at.ticks_ns
            || now.ticks_ns >= self.valid_until.ticks_ns
        {
            return Err(Error::Expired);
        }
        Ok(self.body)
    }
}
impl Client {
    pub(crate) async fn prepare_checkpoint(
        &mut self,
        target: CheckpointTarget,
        snapshot: &ValidatedSnapshot,
    ) -> Result<Preparation, Error> {
        let started = Instant::now();
        let context = self.context();
        let response = self
            .plans
            .prepare_checkpoint(plan::PrepareCheckpoint {
                context: Some(context),
                run_id: target.run.to_string(),
                node_id: target.node.to_string(),
                visit: target.visit.0,
                action: match target.action {
                    CheckpointAction::ChooseBranch => plan::CheckpointAction::ChooseBranch,
                    CheckpointAction::StartWait => plan::CheckpointAction::StartWait,
                    CheckpointAction::CheckWait => plan::CheckpointAction::CheckWait,
                } as i32,
                binding_hash: manifest(include_str!(
                    "../../../../sdk/spec/executor-plan/v1/binding.json"
                )),
            })
            .await?
            .into_inner();
        rx_protocol::json::to_value(&response)?;
        if response.expected_revision == 0 {
            return Err(invalid("preparation revision missing"));
        }
        let state = plan::PreparationState::try_from(response.state)
            .map_err(|_| invalid("preparation state"))?;
        if matches!(
            state,
            plan::PreparationState::Waiting | plan::PreparationState::AlreadyApplied
        ) {
            if response.checkpoint.is_some()
                || response.prepared_at.is_some()
                || response.valid_until.is_some()
            {
                return Err(invalid("non-ready preparation carries candidate"));
            }
            return Ok(
                if state == plan::PreparationState::Waiting
                    && response.expected_revision == snapshot.raw.run.revision.0
                {
                    Preparation::Waiting
                } else {
                    Preparation::Changed
                },
            );
        }
        if state != plan::PreparationState::Ready {
            return Err(invalid("preparation state unspecified"));
        }
        if response.expected_revision != snapshot.raw.run.revision.0 {
            return Ok(Preparation::Changed);
        }
        let time = |value: Option<base::TimePoint>| {
            value
                .map(|v| TimePoint {
                    clock_id: v.clock_id,
                    ticks_ns: Counter(v.ticks_ns),
                })
                .ok_or_else(|| invalid("preparation clock missing"))
        };
        let proposal = PreparedCheckpoint {
            expected_revision: Counter(response.expected_revision),
            checkpoint: checkpoint_response(
                response
                    .checkpoint
                    .ok_or_else(|| invalid("candidate missing"))?,
            )?,
            prepared_at: time(response.prepared_at)?,
            valid_until: time(response.valid_until)?,
        };
        let now = self.clock.now()?;
        if proposal.prepared_at.clock_id != self.pin.clock_id
            || proposal.valid_until.clock_id != self.pin.clock_id
            || proposal
                .valid_until
                .ticks_ns
                .0
                .checked_sub(proposal.prepared_at.ticks_ns.0)
                .is_none_or(|n| n == 0 || n > 100_000_000)
        {
            return Err(invalid("preparation time interval"));
        }
        if now.clock_id != self.pin.clock_id
            || now.ticks_ns < proposal.prepared_at.ticks_ns
            || now.ticks_ns >= proposal.valid_until.ticks_ns
        {
            return Err(Error::Expired);
        }
        let current: ExecutorState = decode(
            &self
                .artifact(&target.run, &snapshot.raw.run.checkpoint.payload)
                .await?,
        )?;
        let candidate: ExecutorState = decode(
            &self
                .artifact(&target.run, &proposal.checkpoint.payload)
                .await?,
        )?;
        let decision = validate_successor(snapshot, &current, &proposal, &candidate, &target)?;
        Ok(Preparation::Ready(Box::new(PreparedAction {
            body: crate::journal::Body::CommitCheckpoint {
                target: target.clone(),
                command: Box::new(CommitCheckpoint {
                    run: target.run,
                    expected_revision: proposal.expected_revision,
                    new_checkpoint: proposal.checkpoint,
                }),
                decision,
            },
            deadline: started + Duration::from_millis(100),
            valid_until: proposal.valid_until,
            prepared_at: proposal.prepared_at,
            clock: self.clock.clone(),
        })))
    }
}
fn same<T: serde::Serialize>(a: &T, b: &T) -> Result<bool, Error> {
    Ok(canonical::bytes(a).map_err(|e| invalid(e.to_string()))?
        == canonical::bytes(b).map_err(|e| invalid(e.to_string()))?)
}
fn require(ok: bool, why: &str) -> Result<(), Error> {
    if ok { Ok(()) } else { Err(invalid(why)) }
}
fn valid_evidence(ids: &[Id]) -> bool {
    !ids.is_empty() && ids.iter().collect::<std::collections::BTreeSet<_>>().len() == ids.len()
}

fn validate_successor(
    snapshot: &ValidatedSnapshot,
    current: &ExecutorState,
    proposal: &PreparedCheckpoint,
    candidate: &ExecutorState,
    target: &CheckpointTarget,
) -> Result<CheckpointDecision, Error> {
    require(
        target.run == snapshot.raw.run.run.id && target.visit == snapshot.raw.visit,
        "proposal scope differs",
    )?;
    require(
        current.schema.as_str() == CHECKPOINT_SCHEMA
            && current.revision == snapshot.raw.run.revision
            && same(&current.run, &snapshot.raw.run.run)?
            && same(
                &current.activations,
                &snapshot.raw.run.checkpoint.activations,
            )?,
        "base artifact differs from snapshot",
    )?;
    require(
        proposal.checkpoint.run == target.run
            && proposal.expected_revision == current.revision
            && proposal.checkpoint.revision.0
                == current
                    .revision
                    .0
                    .checked_add(1)
                    .ok_or_else(|| invalid("revision overflow"))?,
        "candidate revision/run differs",
    )?;
    let mut cp = current
        .process_checkpoints
        .iter()
        .find(|c| c.visit == target.visit)
        .cloned()
        .unwrap_or_else(|| ProcessCheckpoint {
            run: target.run.clone(),
            visit: target.visit,
            revision: Counter(1),
            branches: Default::default(),
            waits: Default::default(),
            wait_windows: Default::default(),
            decision_times: Default::default(),
        });
    require(
        same(&cp, &snapshot.raw.process_checkpoint)?,
        "base process checkpoint differs",
    )?;
    let proposed_cp = candidate
        .process_checkpoints
        .iter()
        .find(|c| c.visit == target.visit)
        .ok_or_else(|| invalid("candidate visit missing"))?;
    let decision = CheckpointDecision::at(proposed_cp, target)
        .ok_or_else(|| invalid("candidate transition missing"))?;
    let node = rx_process_contract::validation::nodes(&snapshot.process)
        .into_iter()
        .find(|n| n.id == target.node)
        .ok_or_else(|| invalid("candidate node missing"))?;
    let mut decision_id = None;
    match (&decision, &node.body) {
        (CheckpointDecision::Branch(choice), CompiledBody::Branch { .. }) => {
            require(
                snapshot.frontier.decisions.contains(&target.node)
                    && !cp.branches.contains_key(&target.node)
                    && valid_evidence(&choice.evidence_ids),
                "invalid branch proposal",
            )?;
            cp.branches.insert(target.node.clone(), choice.clone());
            decision_id = Some(choice.decision.clone());
        }
        (CheckpointDecision::WaitWindow(window), CompiledBody::Wait { timeout_ns, .. }) => {
            require(
                snapshot.frontier.waits.contains(&target.node)
                    && !cp.wait_windows.contains_key(&target.node)
                    && window.started_at == proposal.prepared_at
                    && window.expires_at.clock_id == window.started_at.clock_id
                    && window.started_at.ticks_ns.0.checked_add(timeout_ns.0)
                        == Some(window.expires_at.ticks_ns.0),
                "invalid wait window",
            )?;
            require(
                !current
                    .process_checkpoints
                    .iter()
                    .any(|c| c.wait_windows.values().any(|w| w.id == window.id)),
                "reused window ID",
            )?;
            cp.wait_windows.insert(target.node.clone(), window.clone());
        }
        (CheckpointDecision::WaitResult(result), CompiledBody::Wait { .. }) => {
            require(
                snapshot.frontier.waits.contains(&target.node)
                    && !cp.waits.contains_key(&target.node),
                "invalid wait result",
            )?;
            let window = cp
                .wait_windows
                .get(&target.node)
                .ok_or_else(|| invalid("wait result without window"))?;
            require(
                window.expires_at.clock_id == proposal.prepared_at.clock_id,
                "wait clock differs",
            )?;
            decision_id = Some(match result {
                frontier::WaitProgress::Satisfied {
                    decision,
                    evidence_ids,
                } => {
                    require(
                        proposal.prepared_at.ticks_ns < window.expires_at.ticks_ns
                            && valid_evidence(evidence_ids),
                        "invalid wait satisfaction",
                    )?;
                    decision.clone()
                }
                frontier::WaitProgress::TimedOut { decision } => {
                    require(
                        proposal.prepared_at.ticks_ns >= window.expires_at.ticks_ns,
                        "early timeout proposal",
                    )?;
                    decision.clone()
                }
            });
            cp.waits.insert(target.node.clone(), result.clone());
        }
        _ => return Err(invalid("candidate node/transition kind differs")),
    }
    if let Some(id) = decision_id {
        require(
            !current
                .process_checkpoints
                .iter()
                .any(|c| c.decision_times.contains_key(&id)),
            "reused decision ID",
        )?;
        cp.decision_times.insert(id, proposal.prepared_at.clone());
    }
    cp.revision = cp
        .revision
        .increment()
        .map_err(|e| invalid(e.to_string()))?;
    let mut expected = current.clone();
    expected.revision = proposal.checkpoint.revision;
    expected
        .process_checkpoints
        .retain(|c| c.visit != target.visit);
    expected.process_checkpoints.push(cp);
    expected.process_checkpoints.sort_by_key(|c| c.visit);
    require(
        same(&expected, candidate)?
            && same(&proposal.checkpoint.activations, &candidate.activations)?,
        "candidate changed unrelated state or child mapping",
    )?;
    Ok(decision)
}
