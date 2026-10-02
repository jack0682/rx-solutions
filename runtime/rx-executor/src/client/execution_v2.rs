//! Explicit v2 peer reads. No method in this module sends a v1 execution fallback.
use super::*;
use rx_process_contract::execution_v2::{executor as data, snapshot as snap};
use rx_protocol::execution_v2 as wire2;
pub struct ValidatedExecution {
    raw: snap::Snapshot,
    process: ResolvedProcess,
    frontier: frontier::Frontier,
    deadline: Instant,
    clock: Arc<dyn crate::clock::Clock>,
}
impl ValidatedExecution {
    pub fn data(&self) -> &snap::Snapshot {
        &self.raw
    }
    pub fn process(&self) -> &ResolvedProcess {
        &self.process
    }
    pub fn frontier(&self) -> &frontier::Frontier {
        &self.frontier
    }
    pub fn is_current(&self) -> bool {
        let c = &self.raw.context;
        Instant::now() < self.deadline
            && self.clock.now().is_ok_and(|n| {
                n.clock_id == c.checked_at.clock_id
                    && n.ticks_ns >= c.checked_at.ticks_ns
                    && n.ticks_ns < c.valid_until.ticks_ns
            })
    }
}
fn payload<T: DeserializeOwned>(
    v: wire2::ExecutionPayload,
    schema: &str,
    max: usize,
) -> Result<T, Error> {
    let (r, bytes) = checked_payload(
        wire::ReadPayload {
            reference: v.reference,
            payload: v.payload,
        },
        max,
    )?;
    if r.schema_id.as_str() != schema {
        return Err(invalid("execution v2 payload schema differs"));
    }
    decode(&bytes)
}
impl Client {
    pub async fn negotiate_execution(&mut self) -> Result<(), Error> {
        if self.execution_session.is_some() {
            return Ok(());
        }
        let value = self
            .execution
            .negotiate(wire2::NegotiateExecution {
                context: Some(self.context()),
                cell: self.pin.cell.to_string(),
                binding_hash: data::binding_hash().as_bytes().to_vec(),
            })
            .await?
            .into_inner();
        let declared: data::Session = payload(value, data::SESSION_SCHEMA, self.max_payload)?;
        declared.validate().map_err(invalid)?;
        if declared.installation != self.pin.installation
            || declared.store_generation != self.pin.store_generation
            || declared.principal != self.pin.principal
            || declared.session.as_str() != self.session.session_id
            || declared.cell != self.pin.cell
            || declared.definition != self.pin.definition
            || self
                .runtime_boot
                .as_ref()
                .is_some_and(|b| b != &declared.runtime_boot)
        {
            return Err(invalid("foreign execution v2 declaration"));
        }
        self.runtime_boot = Some(declared.runtime_boot.clone());
        self.execution_session = Some(declared);
        Ok(())
    }
    /// The caller pins expected_plan to its durable Run journal/assignment before reading.
    pub async fn execution_snapshot(
        &mut self,
        run: &Id,
        visit: Counter,
        expected_plan: Digest,
    ) -> Result<ValidatedExecution, Error> {
        let declared = self
            .execution_session
            .as_ref()
            .ok_or_else(|| invalid("explicit v2 negotiation required"))?;
        let boot = declared.runtime_boot.clone();
        let started = Instant::now();
        let sent = self.clock.now()?;
        let reply = self
            .execution
            .get_snapshot(wire2::ReadExecutionSnapshot {
                context: Some(self.context()),
                run_id: run.to_string(),
                visit: visit.0,
                binding_hash: data::binding_hash().as_bytes().to_vec(),
            })
            .await?
            .into_inner();
        let raw: snap::Snapshot = payload(reply, snap::SNAPSHOT_SCHEMA, self.max_payload)?;
        let frontier = raw.validate().map_err(invalid)?;
        let c = &raw.context;
        if c.installation != self.pin.installation
            || c.store_generation != self.pin.store_generation
            || c.caller_session.as_str() != self.session.session_id
            || c.definition.sha256 != self.pin.definition
            || c.run.run.cell != self.pin.cell
            || c.run.run.id != *run
            || c.visit != visit
            || c.resolved.sha256 != expected_plan
            || c.runtime_boot != boot
            || self.runtime_boot.as_ref() != Some(&c.runtime_boot)
            || c.sequence < self.last_sequence
            || c.checked_at.ticks_ns < self.last_checked
        {
            return Err(invalid("foreign or regressed execution v2 snapshot"));
        }
        let received = self.clock.now()?;
        if sent.clock_id != self.pin.clock_id
            || received.clock_id != self.pin.clock_id
            || c.checked_at.clock_id != self.pin.clock_id
            || received.ticks_ns < sent.ticks_ns
            || c.checked_at.ticks_ns < sent.ticks_ns
            || c.checked_at.ticks_ns > received.ticks_ns
        {
            return Err(invalid("execution v2 shared-clock interval differs"));
        }
        let deadline = started
            .checked_add(Duration::from_nanos(
                c.valid_until.ticks_ns.0 - c.checked_at.ticks_ns.0,
            ))
            .ok_or_else(|| invalid("execution deadline overflow"))?;
        if Instant::now() >= deadline || received.ticks_ns >= c.valid_until.ticks_ns {
            return Err(Error::Expired);
        }
        let process = raw.plan.instantiate(&raw.part).map_err(invalid)?;
        self.last_sequence = c.sequence;
        self.last_checked = c.checked_at.ticks_ns;
        Ok(ValidatedExecution {
            raw,
            process,
            frontier,
            deadline,
            clock: self.clock.clone(),
        })
    }
}

impl Client {
    fn execution_call(&self, key: &Id, revision: Counter) -> Result<cell::CellCall, Error> {
        if self.execution_session.is_none() {
            return Err(invalid("explicit v2 declaration required"));
        }
        let mut context = self.context();
        context.request_key = Some(key.to_string());
        Ok(cell::CellCall {
            context: Some(context),
            cell_id: self.pin.cell.to_string(),
            expected_cell_revision: Some(revision.0),
        })
    }
    pub(super) async fn emit_execution_part(
        &mut self,
        key: &Id,
        b: &crate::journal::execution_v2::Begin,
    ) -> Result<crate::journal::Response, Error> {
        let reply = self
            .execution
            .begin_part(wire2::BeginExecutionPart {
                call: Some(self.execution_call(key, b.expected_cell)?),
                run_id: b.run.to_string(),
                mandate_id: b.mandate.to_string(),
                expected_budget: b.expected_budget.0,
                binding_hash: data::binding_hash().as_bytes().to_vec(),
            })
            .await?
            .into_inner();
        let value: data::Part = payload(reply, data::PART_SCHEMA, self.max_payload)?;
        value.validate().map_err(invalid)?;
        Ok(crate::journal::Response::ExecutionPart(Box::new(value)))
    }
    pub(super) async fn emit_execution_node(
        &mut self,
        key: &Id,
        b: &crate::journal::execution_v2::Submit,
    ) -> Result<crate::journal::Response, Error> {
        let reply = self
            .execution
            .submit_node(wire2::SubmitExecutionNode {
                call: Some(self.execution_call(key, b.expected_cell)?),
                run_id: b.part.run.to_string(),
                part_id: b.part.part.to_string(),
                node: b.node.to_string(),
                mandate_id: b.mandate.to_string(),
                expected_run: b.expected_run.0,
                binding_hash: data::binding_hash().as_bytes().to_vec(),
            })
            .await?
            .into_inner();
        let value: data::Admission = payload(reply, data::ADMISSION_SCHEMA, self.max_payload)?;
        value.validate().map_err(invalid)?;
        Ok(crate::journal::Response::ExecutionAdmission(Box::new(
            value,
        )))
    }
}

#[cfg(test)]
#[path = "execution_v2_tests.rs"]
mod tests;
