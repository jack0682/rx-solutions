use super::*;

impl<N: NativeAdapter, C: Clock, H: BoundaryHook> Host<N, C, H> {
    pub fn handover_observations(
        &self,
        caller: &Caller,
        operation: &Id,
    ) -> Result<Vec<rx_protocol::base::Observation>> {
        use rx_protocol::base;
        let mut core = self.lock()?;
        authorized(&core, caller)?;
        let row = core
            .store
            .transact(|tx| tx.get(&key("delivery", operation)))?
            .ok_or(HostError::NotFound)?;
        let record: DeliveryRecord = decode(&row, "rx.host.delivery.v1")?;
        if record.state != ReceiptState::ResultCaptured {
            return Err(HostError::Busy);
        }
        let overlap = core
            .store
            .transact(|tx| tx.scan("delivery/"))?
            .iter()
            .map(|row| decode::<DeliveryRecord>(row, "rx.host.delivery.v1"))
            .collect::<rx_ports::Result<Vec<_>>>()?
            .iter()
            .any(|r| {
                matches!(
                    r.state,
                    ReceiptState::SendEntered | ReceiptState::NativeAccepted
                ) && r
                    .intent
                    .resource_set
                    .iter()
                    .any(|resource| record.intent.resource_set.contains(resource))
            });
        let observation = core.native.handover_snapshot(&record.intent.resource_set)?;
        if observation.device_session != record.device_session {
            return Err(HostError::Stale);
        }
        let uncertainty_ms = observation.uncertainty_ns.0.div_ceil(1_000_000);
        let mut output = Vec::new();
        for (predicate, value) in [
            ("no-pending", observation.no_pending_commands && !overlap),
            ("control", observation.control_available),
            ("support", observation.support_stable),
        ] {
            output.push(base::Observation {
                observation_id: id().to_string(),
                source_id: format!("handover/{operation}/{predicate}"),
                source_boot_evidence: Some(observation.device_session.to_string()),
                receive_boot_id: core.boot.to_string(),
                receive_time: Some(base::TimePoint {
                    clock_id: observation.observed_at.clock_id.clone(),
                    ticks_ns: observation.observed_at.ticks_ns.0,
                }),
                sample_seq: None,
                source_time: None,
                quality: base::Quality::Good as i32,
                value_schema: "rx.handover.v1".into(),
                value: Some(base::TypedValue {
                    value: Some(base::typed_value::Value::Boolean(value)),
                }),
                correlation: Some(base::Correlation {
                    operation_id: Some(operation.to_string()),
                    invocation_id: record.invocation.as_ref().map(ToString::to_string),
                    native_id: None,
                    device_session_id: record.device_session.to_string(),
                    profile_digest: record.intent.profile_digest.as_bytes().to_vec(),
                    cancel_id: None,
                }),
                freshness_basis: base::FreshnessBasis::ReadTransaction as i32,
                uncertainty_ms: Some(uncertainty_ms),
            });
        }
        Ok(output)
    }
    pub fn receipt(&self, caller: &Caller, operation: &Id) -> Result<DeliveryRecord> {
        let mut core = self.lock()?;
        authorized(&core, caller)?;
        let row = core
            .store
            .transact(|tx| tx.get(&key("delivery", operation)))?
            .ok_or(HostError::NotFound)?;
        decode(&row, "rx.host.delivery.v1").map_err(Into::into)
    }
    /// Restrictive operation used by cancellation coordination. Never invokes native cancel.
    pub fn void_before_send(
        &self,
        caller: &Caller,
        operation: Id,
        digest: Digest,
    ) -> Result<VoidRecord> {
        let mut core = self.lock()?;
        authorized(&core, caller)?;
        let result = core.store.transact(|tx| {
            let void_key = key("void", &operation);
            if let Some(old) = tx.get(&void_key)? {
                let v: VoidRecord = decode(&old, "rx.host.void.v1")?;
                if v.digest != digest {
                    return Err(rx_ports::StoreError::KeyConflict);
                }
                return Ok(Some(v));
            }
            let row = tx.get(&key("delivery", &operation))?;
            if let Some(row) = row {
                let mut r: DeliveryRecord = decode(&row, "rx.host.delivery.v1")?;
                if r.digest != digest {
                    return Err(rx_ports::StoreError::KeyConflict);
                }
                if r.state != ReceiptState::Prepared {
                    return Ok(None);
                }
                r.state = ReceiptState::VoidedBeforeSend;
                record_delivery(tx, &mut r, Some(row.revision))?;
            }
            let (sequence, _) = next_sequence(tx)?;
            let v = VoidRecord {
                operation,
                digest,
                sequence,
            };
            tx.put(&void_key, None, &doc("rx.host.void.v1", &v)?)?;
            Ok(Some(v))
        })?;
        result.ok_or(HostError::Busy)
    }
    pub fn receipt_view(
        &self,
        caller: &Caller,
        operation: &Id,
    ) -> Result<rx_protocol::base::Receipt> {
        let mut core = self.lock()?;
        authorized(&core, caller)?;
        use rx_protocol::base;
        core.store
            .transact(|tx| {
                let meta = tx
                    .get(&name("host/meta"))?
                    .ok_or(rx_ports::StoreError::Integrity("Host meta missing".into()))?;
                let meta: HostMeta = decode(&meta, "rx.host.meta.v1")?;
                if let Some(v) = tx.get(&key("void", operation))? {
                    let v: VoidRecord = decode(&v, "rx.host.void.v1")?;
                    return Ok(base::Receipt {
                        operation_id: operation.to_string(),
                        intent_digest: v.digest.as_bytes().to_vec(),
                        operation_revision: None,
                        stage: base::ReceiptStage::NotDispatched as i32,
                        invocation_id: None,
                        journal_id: meta.delivery_journal.to_string(),
                        journal_seq: v.sequence.0,
                        host_state: Some(base::HostReceiptState::VoidedBeforeSend as i32),
                        cancel_id: None,
                    });
                }
                let row = tx
                    .get(&key("delivery", operation))?
                    .ok_or(rx_ports::StoreError::Invalid("receipt absent".into()))?;
                let r: DeliveryRecord = decode(&row, "rx.host.delivery.v1")?;
                let (stage, state) = match r.state {
                    ReceiptState::Prepared => (
                        base::ReceiptStage::HostPrepared,
                        base::HostReceiptState::Prepared,
                    ),
                    ReceiptState::SendEntered => (
                        base::ReceiptStage::SendEntered,
                        base::HostReceiptState::SendEntered,
                    ),
                    ReceiptState::NativeRejected => (
                        base::ReceiptStage::SendEntered,
                        base::HostReceiptState::NativeRejected,
                    ),
                    ReceiptState::NativeAccepted => (
                        base::ReceiptStage::NativeAccepted,
                        base::HostReceiptState::NativeAccepted,
                    ),
                    // RESULT_RECORDED belongs to platform T2, not this Host capture.
                    ReceiptState::ResultCaptured => (
                        base::ReceiptStage::SendEntered,
                        base::HostReceiptState::ResultCaptured,
                    ),
                    ReceiptState::VoidedBeforeSend => (
                        base::ReceiptStage::NotDispatched,
                        base::HostReceiptState::VoidedBeforeSend,
                    ),
                };
                Ok(base::Receipt {
                    operation_id: operation.to_string(),
                    intent_digest: r.digest.as_bytes().to_vec(),
                    operation_revision: None,
                    stage: stage as i32,
                    invocation_id: r.invocation.map(|i| i.to_string()),
                    journal_id: meta.delivery_journal.to_string(),
                    journal_seq: r.journal_seq.0,
                    host_state: Some(state as i32),
                    cancel_id: None,
                })
            })
            .map_err(Into::into)
    }
    pub fn reconcile(&self, caller: &Caller, operation: &Id) -> Result<DeliveryRecord> {
        let mut core = self.lock()?;
        authorized(&core, caller)?;
        let row = core
            .store
            .transact(|tx| tx.get(&key("delivery", operation)))?
            .ok_or(HostError::NotFound)?;
        let mut record: DeliveryRecord = decode(&row, "rx.host.delivery.v1")?;
        if matches!(
            record.state,
            ReceiptState::SendEntered | ReceiptState::NativeAccepted
        ) && let Some(invocation) = &record.invocation
            && let Some(capture) = core.native.lookup(&record.operation, invocation)?
        {
            persist_capture(&mut core, &mut record, capture)?;
        }
        Ok(record)
    }
    pub fn evidence_after(
        &self,
        caller: &Caller,
        after: Counter,
        limit: usize,
    ) -> Result<Vec<(Counter, EvidenceRecord)>> {
        let mut core = self.lock()?;
        authorized(&core, caller)?;
        core.store
            .events_after(after, limit)?
            .into_iter()
            .map(|event| {
                let record = rx_ports::Record {
                    key: name("evidence"),
                    revision: Counter(1),
                    document: event.document,
                };
                Ok((event.seq, decode(&record, "rx.host.evidence.v1")?))
            })
            .collect()
    }
}

pub(super) fn persist_capture<N>(
    core: &mut Core<N>,
    record: &mut DeliveryRecord,
    capture: NativeCapture,
) -> Result<()> {
    if capture.device_session != record.device_session {
        return Err(HostError::NativeUnknown("device generation changed".into()));
    }
    let evidence = EvidenceRecord {
        evidence_id: id(),
        operation: record.operation.clone(),
        invocation: record.invocation.clone().ok_or(HostError::Conflict)?,
        profile_digest: record.intent.profile_digest,
        capture,
    };
    core.store.transact(|tx| {
        let row = tx
            .get(&key("delivery", &record.operation))?
            .ok_or(rx_ports::StoreError::Invalid("receipt absent".into()))?;
        tx.append(
            &evidence.evidence_id,
            &doc("rx.host.evidence.v1", &evidence)?,
        )?;
        record.state = ReceiptState::ResultCaptured;
        record.evidence_ids.push(evidence.evidence_id.clone());
        record_delivery(tx, record, Some(row.revision))
    })?;
    Ok(())
}
