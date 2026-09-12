//! Authenticated discovery of already-started work; never admission or a chosen Run.
use super::*;
use rx_process_contract::assignment;

/// Only a verified P response can construct this read view. Every witness is retained,
/// including old sessions, expired pending attempts, and paused or recovery work.
pub struct ValidatedAssignment {
    raw: assignment::View,
    clock: Arc<dyn crate::clock::Clock>,
    deadline: Instant,
}

impl ValidatedAssignment {
    pub fn data(&self) -> &assignment::View {
        &self.raw
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
    /// Inspect the pinned cell without selecting a candidate or granting execution.
    /// A later Production.Inspect must establish the current Run authority.
    pub async fn assignment_view(&mut self) -> Result<ValidatedAssignment, Error> {
        let started = Instant::now();
        let sent = self.clock.now()?;
        if sent.clock_id != self.pin.clock_id {
            return Err(invalid("local clock changed"));
        }
        let reply = self
            .assignments
            .inspect(rx_protocol::assignment::InspectCell {
                context: Some(self.context()),
                cell_id: self.pin.cell.to_string(),
                binding_hash: assignment_hash(),
            })
            .await?
            .into_inner();
        self.accept_assignment(reply, sent, started)
    }

    fn accept_assignment(
        &mut self,
        reply: wire::ReadPayload,
        sent: TimePoint,
        started: Instant,
    ) -> Result<ValidatedAssignment, Error> {
        let (reference, bytes) =
            checked_payload(reply, self.max_payload.min(assignment::MAX_PAYLOAD))?;
        if reference.schema_id.as_str() != assignment::SCHEMA {
            return Err(invalid("assignment schema differs"));
        }
        let raw: assignment::View = decode(&bytes)?;
        assignment::validate(&raw).map_err(invalid)?;
        if raw.installation != self.pin.installation
            || raw.store_generation != self.pin.store_generation
            || raw.definition.sha256 != self.pin.definition
            || raw.cell != self.pin.cell
            || raw.executor != self.pin.principal
            || raw.caller_session.as_str() != self.session.session_id
            || raw.checked_at.clock_id != self.pin.clock_id
            || self
                .runtime_boot
                .as_ref()
                .is_some_and(|boot| boot != &raw.runtime_boot)
            || raw.sequence < self.last_sequence
            || raw.checked_at.ticks_ns < self.last_checked
        {
            return Err(invalid("foreign or regressed assignment view"));
        }
        let received = self.clock.now()?;
        if sent.clock_id != self.pin.clock_id
            || received.clock_id != self.pin.clock_id
            || received.ticks_ns < sent.ticks_ns
            || raw.checked_at.ticks_ns < sent.ticks_ns
            || raw.checked_at.ticks_ns > received.ticks_ns
        {
            return Err(invalid("assignment clock interval differs"));
        }
        // The contract has established a positive lifetime of at most 100ms.
        // Starting this deadline before sending also bounds a stalled same-host clock.
        let deadline = started
            .checked_add(Duration::from_nanos(
                raw.valid_until.ticks_ns.0 - raw.checked_at.ticks_ns.0,
            ))
            .ok_or_else(|| invalid("assignment deadline overflow"))?;
        if Instant::now() >= deadline || received.ticks_ns >= raw.valid_until.ticks_ns {
            return Err(Error::Expired);
        }
        self.runtime_boot = Some(raw.runtime_boot.clone());
        self.last_sequence = raw.sequence;
        self.last_checked = raw.checked_at.ticks_ns;
        Ok(ValidatedAssignment {
            raw,
            clock: self.clock.clone(),
            deadline,
        })
    }
}

fn assignment_hash() -> Vec<u8> {
    manifest(include_str!(
        "../../../../sdk/spec/assignment/v1/binding.json"
    ))
}

#[cfg(test)]
mod tests;
