use super::*;
use rx_process_contract::production;
pub struct ValidatedProduction {
    raw: production::View,
    clock: Arc<dyn crate::clock::Clock>,
    deadline: Instant,
}
/// Only a verified P read can create proof for retiring this exact planner visit.
pub struct CompletedVisit {
    pub(crate) identity: crate::frame::Identity,
    pub(crate) part: Id,
}
impl ValidatedProduction {
    pub fn data(&self) -> &production::View {
        &self.raw
    }
    pub fn is_current(&self) -> bool {
        Instant::now() < self.deadline
            && self.clock.now().is_ok_and(|t| {
                t.clock_id == self.raw.checked_at.clock_id
                    && t.ticks_ns >= self.raw.checked_at.ticks_ns
                    && t.ticks_ns < self.raw.valid_until.ticks_ns
            })
    }
    pub fn identity(&self, visit: Counter) -> crate::frame::Identity {
        crate::frame::Identity {
            run: self.raw.run.run.id.clone(),
            executor_session: self.raw.caller_session.clone(),
            resolved_digest: self.raw.resolved.sha256,
            visit,
            epoch: self.raw.cell_epoch,
        }
    }
    pub fn completed_visit(&self, visit: Counter) -> Option<CompletedVisit> {
        if !self.is_current() {
            return None;
        }
        self.raw
            .parts
            .iter()
            .find(|p| p.ordinal == visit && p.disposition == PartDisposition::ConfirmedCompleted)
            .map(|p| CompletedVisit {
                identity: self.identity(visit),
                part: p.id.clone(),
            })
    }
}
pub(crate) struct ProductionBasis<'a> {
    pub view: &'a ValidatedProduction,
    pub visit: Counter,
}
impl crate::worker::RequestBasis for ProductionBasis<'_> {
    fn context_identity(&self) -> crate::frame::Identity {
        self.view.identity(self.visit)
    }
    fn journal_basis(&self) -> crate::journal::Basis {
        let v = &self.view.raw;
        crate::journal::Basis {
            runtime_boot: v.runtime_boot.clone(),
            sequence: v.sequence,
            run_revision: v.run.revision,
            cell_revision: v.cell_revision,
            checked_at: v.checked_at.clone(),
        }
    }
    fn is_current(&self) -> bool {
        self.view.is_current()
    }
}
impl Client {
    pub async fn production_view(&mut self, run: &Id) -> Result<ValidatedProduction, Error> {
        let started = Instant::now();
        let sent = self.clock.now()?;
        let reply = self
            .production
            .inspect(rx_protocol::production::InspectRun {
                context: Some(self.context()),
                run_id: run.to_string(),
                binding_hash: production_hash(),
            })
            .await?
            .into_inner();
        let (reference, bytes) = checked_payload(reply, self.max_payload)?;
        if reference.schema_id.as_str() != production::SCHEMA {
            return Err(invalid("production schema differs"));
        }
        let raw: production::View = decode(&bytes)?;
        production::validate(&raw).map_err(invalid)?;
        if raw.installation != self.pin.installation
            || raw.store_generation != self.pin.store_generation
            || raw.definition.sha256 != self.pin.definition
            || raw.run.run.cell != self.pin.cell
            || raw.run.run.id != *run
            || raw.caller_session.as_str() != self.session.session_id
            || raw.checked_at.clock_id != self.pin.clock_id
            || self
                .runtime_boot
                .as_ref()
                .is_some_and(|b| b != &raw.runtime_boot)
            || raw.sequence < self.last_sequence
            || raw.checked_at.ticks_ns < self.last_checked
        {
            return Err(invalid("foreign or regressed production view"));
        }
        let state: ExecutorState = decode(&self.artifact(run, &raw.run.checkpoint.payload).await?)?;
        if state.revision != raw.run.revision
            || state.schema.as_str() != CHECKPOINT_SCHEMA
            || canonical::bytes(&state.run).map_err(|e| invalid(e.to_string()))?
                != canonical::bytes(&raw.run.run).map_err(|e| invalid(e.to_string()))?
            || canonical::bytes(&state.activations).map_err(|e| invalid(e.to_string()))?
                != canonical::bytes(&raw.run.checkpoint.activations)
                    .map_err(|e| invalid(e.to_string()))?
        {
            return Err(invalid("production checkpoint cut differs"));
        }
        let now = self.clock.now()?;
        if sent.clock_id != self.pin.clock_id
            || now.clock_id != self.pin.clock_id
            || raw.checked_at.ticks_ns < sent.ticks_ns
            || raw.checked_at.ticks_ns > now.ticks_ns
        {
            return Err(invalid("production clock interval differs"));
        }
        let deadline =
            started + Duration::from_nanos(raw.valid_until.ticks_ns.0 - raw.checked_at.ticks_ns.0);
        if Instant::now() >= deadline || now.ticks_ns >= raw.valid_until.ticks_ns {
            return Err(Error::Expired);
        }
        self.runtime_boot = Some(raw.runtime_boot.clone());
        self.last_sequence = raw.sequence;
        self.last_checked = raw.checked_at.ticks_ns;
        Ok(ValidatedProduction {
            raw,
            clock: self.clock.clone(),
            deadline,
        })
    }
}
pub(super) fn production_hash() -> Vec<u8> {
    manifest(include_str!(
        "../../../../sdk/spec/production/v1/binding.json"
    ))
}
pub(super) fn part_response(value: cell::PartAttempt) -> Result<production::Part, Error> {
    rx_protocol::json::to_value(&value)?;
    if value.material_id.is_some() || value.ordinal == 0 || value.revision == 0 {
        return Err(invalid("unsupported material identity or part position"));
    }
    let disposition = match cell::PartDisposition::try_from(value.disposition)
        .map_err(|_| invalid("part disposition"))?
    {
        cell::PartDisposition::InProgress => PartDisposition::InProgress,
        cell::PartDisposition::ConfirmedCompleted => PartDisposition::ConfirmedCompleted,
        cell::PartDisposition::Rejected => PartDisposition::Rejected,
        cell::PartDisposition::Unresolved => PartDisposition::Unresolved,
        cell::PartDisposition::NotProcessed => PartDisposition::NotProcessed,
        _ => return Err(invalid("part disposition unspecified")),
    };
    Ok(production::Part {
        id: id(&value.part_attempt_id)?,
        run: id(&value.run_id)?,
        ordinal: Counter(value.ordinal),
        revision: Counter(value.revision),
        disposition,
    })
}

#[cfg(test)]
impl ValidatedProduction {
    /// Unit-test construction only: no RPC/authority claim. Used to exercise the real local
    /// coordinator with deterministic clock advancement during an observation transaction.
    pub(crate) fn with_test_client(
        raw: production::View,
        clock: Arc<dyn crate::clock::Clock>,
        principal: Name,
        release: Digest,
    ) -> (Client, Self) {
        production::validate(&raw).expect("valid test production view");
        let channel = Channel::from_static("http://127.0.0.1:1").connect_lazy();
        let client = Client {
            pin: PeerPin {
                principal,
                peer_boot: Id::new(uuid::Uuid::new_v4().to_string()).expect("UUID"),
                installation: raw.installation.clone(),
                store_generation: raw.store_generation.clone(),
                release,
                clock_id: raw.checked_at.clock_id.clone(),
                cell: raw.run.run.cell.clone(),
                definition: raw.definition.sha256,
            },
            clock: clock.clone(),
            session: base::Session { session_id: raw.caller_session.to_string(), ..Default::default() },
            read: wire::executor_read_service_client::ExecutorReadServiceClient::new(channel.clone()),
            workflow: base::workflow_service_client::WorkflowServiceClient::new(channel.clone()),
            plans: rx_protocol::executor_plan::executor_plan_service_client::ExecutorPlanServiceClient::new(channel.clone()),
            cells: cell::cell_service_client::CellServiceClient::new(channel.clone()),
            operations: base::operation_service_client::OperationServiceClient::new(channel.clone()),
            production: rx_protocol::production::production_service_client::ProductionServiceClient::new(channel.clone()),
            assignments: rx_protocol::assignment::executor_assignment_service_client::ExecutorAssignmentServiceClient::new(channel),
            runtime_boot: Some(raw.runtime_boot.clone()),
            last_sequence: raw.sequence,
            last_checked: raw.checked_at.ticks_ns,
            process: None,
            max_payload: MAX_BYTES,
        };
        // Keep the production read's actual bounded lifetime; the test does not extend it.
        let deadline = Instant::now()
            + Duration::from_nanos(raw.valid_until.ticks_ns.0 - raw.checked_at.ticks_ns.0);
        (
            client,
            Self {
                raw,
                clock,
                deadline,
            },
        )
    }
}
