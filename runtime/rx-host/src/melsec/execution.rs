use super::*;
use crate::journal::name;
use rx_domain::host_snapshot::SourceObservation;
use std::collections::BTreeSet;

impl<C: Clock, H: NativeBoundary> NativeAdapter for Melsec<C, H> {
    fn submit_with_context(
        &mut self,
        op: &Id,
        inv: &Id,
        intent: &Intent,
        context: &NativeDispatch,
    ) -> Result<NativeCapture> {
        self.dispatch_context = Some(context.clone());
        let result = self.submit(op, inv, intent);
        self.dispatch_context = None;
        result
    }
    fn environment(&self) -> Environment {
        self.profile.environment()
    }
    fn protection(&self) -> Arc<dyn LocalProtection> {
        self.protection.clone()
    }
    fn observe_sources(&self, cell: &Name, sources: &[Name]) -> Result<Vec<SourceObservation>> {
        if cell != &self.profile.cell
            || sources.len() > 16
            || sources.iter().collect::<BTreeSet<_>>().len() != sources.len()
            || sources
                .iter()
                .any(|s| !self.profile.sources.contains_key(s))
        {
            return Err(HostError::Guard);
        }
        let mut core = self.lock()?;
        let snapshot = self.snapshot(&mut core)?;
        Ok(sources
            .iter()
            .map(|source| SourceObservation {
                source: source.clone(),
                generation: self.session.clone(),
                schema: name("boolean/v1"),
                unit: name("unitless"),
                value: TypedValue::Boolean(snapshot.bit(self.profile.sources[source])),
                acquired_at: snapshot.acquired_at.clone(),
                uncertainty_ns: snapshot.uncertainty_ns,
                quality_good: true,
                origin_age_bounded: true,
                disputed: false,
                evidence_id: crate::journal::id(),
            })
            .collect())
    }
    fn guard(&self, intent: &Intent, now: &TimePoint) -> Result<Guard> {
        self.profile.mapping(intent)?;
        if self.protection.is_latched() {
            return Err(HostError::Guard);
        }
        let mut core = self.lock()?;
        if self
            .check_store(journal::pending(&mut core.store))?
            .is_some()
        {
            return Err(HostError::Busy);
        }
        let snapshot = self.snapshot(&mut core)?;
        if !self.ready(&snapshot)
            || self.protection.is_latched()
            || now.clock_id != snapshot.acquired_at.clock_id
            || now.ticks_ns > snapshot.acquired_at.ticks_ns
        {
            return Err(HostError::Guard);
        }
        Ok(Guard {
            device_session: self.session.clone(),
            valid_until: TimePoint {
                clock_id: snapshot.acquired_at.clock_id,
                ticks_ns: Counter(
                    snapshot
                        .acquired_at
                        .ticks_ns
                        .0
                        .checked_add(u64::from(self.profile.status.guard_validity_ms) * 1_000_000)
                        .ok_or(HostError::Guard)?,
                ),
            },
            satisfied: self.profile.conditions.keys().cloned().collect(),
        })
    }
    fn can_handover(&self, resources: &[Name]) -> bool {
        self.handover_snapshot(resources)
            .is_ok_and(|s| s.no_pending_commands && s.control_available && s.support_stable)
    }
    fn handover_snapshot(&self, resources: &[Name]) -> Result<LocalHandover> {
        if !self.profile.resources_match(resources) {
            return Err(HostError::Guard);
        }
        let mut core = self.lock()?;
        let pending = self
            .check_store(journal::pending(&mut core.store))?
            .is_some();
        let snapshot = self.snapshot(&mut core)?;
        Ok(LocalHandover {
            device_session: self.session.clone(),
            observed_at: snapshot.acquired_at.clone(),
            uncertainty_ns: snapshot.uncertainty_ns,
            no_pending_commands: !pending && snapshot.bit(self.profile.status.queue_empty_bit),
            control_available: snapshot.bit(self.profile.status.control_bit),
            support_stable: snapshot.bit(self.profile.status.support_bit),
        })
    }
    fn shutdown_snapshot(&self, resources: &[Name]) -> Result<NativeShutdown> {
        if !self.profile.resources_match(resources) {
            return Err(HostError::Guard);
        }
        let mut core = self.lock()?;
        let pending = self
            .check_store(journal::pending(&mut core.store))?
            .is_some();
        let snapshot = self.snapshot(&mut core)?;
        let no_pending = !pending && snapshot.bit(self.profile.status.queue_empty_bit);
        Ok(NativeShutdown {
            device_session: self.session.clone(),
            observed_at: snapshot.acquired_at.clone(),
            uncertainty_ns: snapshot.uncertainty_ns,
            resources: resources.to_vec(),
            no_pending_commands: no_pending,
            safe_to_drop: no_pending
                && snapshot.bit(self.profile.status.support_bit)
                && snapshot.bit(self.profile.status.drop_allowed_bit),
        })
    }
    fn submit(
        &mut self,
        operation: &Id,
        invocation: &Id,
        intent: &Intent,
    ) -> Result<NativeCapture> {
        let (mapping, target) = self.profile.mapping(intent)?;
        let digest = intent
            .digest()
            .map_err(|e| HostError::Invalid(e.to_string()))?;
        let mut core = self.lock()?;
        if let Some(old) = self.check_store(journal::read(&mut core.store, operation))? {
            if old.invocation != *invocation || old.intent_digest != digest {
                return Err(HostError::Conflict);
            }
            return old.capture.ok_or_else(|| {
                HostError::NativeUnknown(
                    "original native entry retained; submit never retries".into(),
                )
            });
        }
        if self.protection.is_latched() {
            return Err(HostError::Guard);
        }
        if self
            .check_store(journal::pending(&mut core.store))?
            .is_some()
        {
            return Err(HostError::Busy);
        }
        let snapshot = self.snapshot(&mut core)?;
        if !self.ready(&snapshot) || self.protection.is_latched() {
            return Err(HostError::Guard);
        }
        let satisfied = snapshot.bit(mapping.completion_bit) == target;
        let request_frame = self
            .profile
            .transport
            .write_m_frame(mapping.command_m, target)
            .map_err(|e| HostError::Invalid(e.to_string()))?;
        let mut entry = Entry {
            operation: operation.clone(),
            invocation: invocation.clone(),
            intent: intent.clone(),
            intent_digest: digest,
            profile_digest: intent.profile_digest,
            device_session: self.session.clone(),
            plc_epoch: snapshot.plc_epoch,
            command_m: mapping.command_m,
            target,
            request_sha256: rx_package::content_digest(&request_frame),
            request_frame,
            snapshot: snapshot.clone(),
            entered_at: self.clock.now(),
            capture: satisfied.then(|| self.capture(&snapshot)),
            completion_snapshot: satisfied.then(|| snapshot.clone()),
            acknowledgement: None,
            write_error: None,
        };
        self.check_store(journal::enter(&mut core.store, &entry))?;
        self.hooks.after_entry_commit();
        if let Some(capture) = entry.capture {
            self.hooks.after_capture_commit();
            return Ok(capture);
        }
        // Recheck independent latch after the durable boundary; never run a compensating write.
        if self.protection.is_latched() {
            return Err(HostError::Guard);
        }
        let final_snapshot = self.snapshot(&mut core)?;
        if !self.ready(&final_snapshot)
            || self.protection.is_latched()
            || final_snapshot.plc_epoch != entry.plc_epoch
        {
            return Err(HostError::Guard);
        }
        match core
            .client
            .as_mut()
            .ok_or(HostError::Guard)?
            .write_m(mapping.command_m, target)
        {
            Ok(_) => {
                self.hooks.after_write_before_ack_commit();
                entry.acknowledgement = Some(self.clock.now());
                self.check_store(journal::update(&mut core.store, &entry))?;
            }
            Err(error) => {
                entry.write_error = Some(error.to_string());
                self.protection.react(ProtectionIncident::OwnerLost);
                self.check_store(journal::update(&mut core.store, &entry))?;
                return Err(HostError::NativeUnknown(
                    "PLC write result unknown; original entry retained".into(),
                ));
            }
        }
        Err(HostError::NativeUnknown(
            "PLC acknowledged memory write; completion requires read-only reconciliation".into(),
        ))
    }
    fn lookup(&mut self, operation: &Id, invocation: &Id) -> Result<Option<NativeCapture>> {
        let mut core = self.lock()?;
        let Some(mut entry) = self.check_store(journal::read(&mut core.store, operation))? else {
            return Ok(None);
        };
        if entry.invocation != *invocation {
            return Err(HostError::Conflict);
        }
        if entry.capture.is_some() {
            return Ok(entry.capture);
        }
        // Do not invent continuity after a process restart, reconnect, or uncertain native entry.
        if entry.device_session != self.session || entry.acknowledgement.is_none() {
            return Ok(None);
        }
        let snapshot = self.snapshot(&mut core)?;
        if snapshot.plc_epoch != entry.plc_epoch
            || snapshot.sequence <= entry.snapshot.sequence
            || !snapshot.bit(self.profile.status.queue_empty_bit)
        {
            return Ok(None);
        }
        let (mapping, target) = self.profile.mapping(&entry.intent)?;
        if snapshot.bit(mapping.completion_bit) != target {
            return Ok(None);
        }
        let capture = self.capture(&snapshot);
        entry.capture = Some(capture.clone());
        entry.completion_snapshot = Some(snapshot);
        self.check_store(journal::update(&mut core.store, &entry))?;
        self.hooks.after_capture_commit();
        Ok(Some(capture))
    }
}
