use super::*;

impl<N: NativeAdapter, C: Clock, H: BoundaryHook> Host<N, C, H> {
    pub fn prepare(&self, caller: &Caller, request: Request) -> Result<DeliveryRecord> {
        let mut core = self.lock()?;
        authorized(&core, caller)?;
        require_admission(&core)?;
        validate_identity(&mut core, &request)?;
        if let Some(row) = core
            .store
            .transact(|tx| tx.get(&key("void", &request.operation)))?
        {
            let v: VoidRecord = decode(&row, "rx.host.void.v1")?;
            if v.digest != request.digest {
                return Err(HostError::Conflict);
            }
            return Err(HostError::Voided);
        }
        if let Some(row) = core
            .store
            .transact(|tx| tx.get(&key("delivery", &request.operation)))?
        {
            let record: DeliveryRecord = decode(&row, "rx.host.delivery.v1")?;
            same_operation(&record, &request)?;
            if record.state != ReceiptState::Prepared || record.permit == request.permit.id {
                return Ok(record);
            }
            let now = self.clock.now();
            let guard = validate_current(&mut core, caller, &request, &now)?;
            let mut rebound = record;
            let retired = rebound.permit.clone();
            rebound.permit = request.permit.id.clone();
            rebound.permit_digest =
                canonical::digest("RX-HOST-PERMIT-v1", &request.permit).map_err(invalid)?;
            rebound.prepared_boot = core.boot.clone();
            rebound.device_session = guard.device_session;
            let duration = request
                .intent
                .prepare_validity_ms
                .0
                .checked_mul(1_000_000)
                .ok_or_else(|| HostError::Invalid("prepare duration".into()))?;
            rebound.prepare_until = TimePoint {
                clock_id: now.clock_id,
                ticks_ns: Counter(
                    now.ticks_ns
                        .0
                        .checked_add(duration)
                        .ok_or_else(|| HostError::Invalid("prepare expiry".into()))?,
                ),
            };
            core.store.transact(|tx| {
                tx.put(
                    &key("retired-permit", &retired),
                    None,
                    &doc("rx.host.retired-permit.v1", &retired)?,
                )?;
                record_delivery(tx, &mut rebound, Some(row.revision))
            })?;
            return Ok(rebound);
        }
        let now = self.clock.now();
        let guard = validate_current(&mut core, caller, &request, &now)?;
        let duration = request
            .intent
            .prepare_validity_ms
            .0
            .checked_mul(1_000_000)
            .ok_or_else(|| HostError::Invalid("prepare duration overflow".into()))?;
        let mut record = DeliveryRecord {
            operation: request.operation.clone(),
            digest: request.digest,
            invocation: Some(id()),
            state: ReceiptState::Prepared,
            journal_seq: Counter(0),
            prepared_boot: core.boot.clone(),
            prepare_until: TimePoint {
                clock_id: now.clock_id.clone(),
                ticks_ns: Counter(
                    now.ticks_ns
                        .0
                        .checked_add(duration)
                        .ok_or_else(|| HostError::Invalid("prepare expiry overflow".into()))?,
                ),
            },
            cell: request.permit.cell.clone(),
            intent: request.intent,
            permit: request.permit.id.clone(),
            device_session: guard.device_session,
            evidence_ids: vec![],
            permit_digest: canonical::digest("RX-HOST-PERMIT-v1", &request.permit)
                .map_err(invalid)?,
        };
        core.store
            .transact(|tx| record_delivery(tx, &mut record, None))?;
        Ok(record)
    }
    pub fn authorize(
        &self,
        caller: &Caller,
        request: Request,
        invocation: &Id,
    ) -> Result<DeliveryRecord> {
        let mut core = self.lock()?;
        authorized(&core, caller)?;
        require_admission(&core)?;
        validate_identity(&mut core, &request)?;
        let row = core
            .store
            .transact(|tx| tx.get(&key("delivery", &request.operation)))?
            .ok_or(HostError::NotFound)?;
        let mut record: DeliveryRecord = decode(&row, "rx.host.delivery.v1")?;
        same_operation(&record, &request)?;
        if record.invocation.as_ref() != Some(invocation) || record.permit != request.permit.id {
            return Err(HostError::Conflict);
        }
        if record.state != ReceiptState::Prepared {
            return Ok(record);
        }
        let now = self.clock.now();
        if record.prepared_boot != core.boot || !before(&now, &record.prepare_until) {
            return Err(HostError::Stale);
        }
        let guard = validate_current(&mut core, caller, &request, &now)?;
        if guard.device_session != record.device_session {
            return Err(HostError::Stale);
        }
        record.state = ReceiptState::SendEntered;
        if let Err(error) = core.store.transact(|tx| {
            let used = key("consumed-permit", &request.permit.id);
            if tx.get(&used)?.is_some() {
                return Err(rx_ports::StoreError::Invalid(
                    "permit already consumed".into(),
                ));
            }
            tx.put(
                &used,
                None,
                &doc(
                    "rx.host.permit-consumed.v1",
                    &(request.permit.id.clone(), request.operation.clone()),
                )?,
            )?;
            record_delivery(tx, &mut record, Some(row.revision))
        }) {
            self.protection.react(ProtectionIncident::StoreFault);
            return Err(error.into());
        }
        self.hooks.after_send_commit();
        // Still under the same gate: no handover/fence can interleave with submission.
        let guard = validate_current(&mut core, caller, &request, &self.clock.now())?;
        if guard.device_session != record.device_session {
            return Err(HostError::Stale);
        }
        let context = NativeDispatch {
            device_session: guard.device_session.clone(),
            expires_at: if guard.valid_until.ticks_ns < request.permit.expires_at.ticks_ns {
                guard.valid_until
            } else {
                request.permit.expires_at.clone()
            },
        };
        let capture = core.native.submit_with_context(
            &record.operation,
            invocation,
            &record.intent,
            &context,
        )?;
        self.hooks.after_native_entry();
        if let Err(error) = persist_capture(&mut core, &mut record, capture) {
            if matches!(error, HostError::Store(_)) {
                self.protection.react(ProtectionIncident::StoreFault);
            }
            return Err(error);
        }
        Ok(record)
    }
}
