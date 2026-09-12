use super::*;
use rx_domain::host_configuration as config;
const CONTEXT: &str = "rx.host.applied-process-context.v1";
const RECEIPT: &str = "rx.host-process-configuration-receipt.v1";
pub(super) fn binding_digest(bindings: &BTreeMap<Name, Binding>) -> Result<Digest> {
    let mut values = bindings.values().cloned().collect::<Vec<_>>();
    for b in &mut values {
        for i in &mut b.allowed_intents {
            *i = i.normalized().map_err(invalid)?;
        }
        b.allowed_intents
            .sort_by_cached_key(|i| i.digest().expect("normalized intent"));
        b.scope_ids.sort();
        b.condition_ids.sort();
    }
    canonical::digest("RX-HOST-CONFIGURATION-BINDINGS-v1", &values).map_err(invalid)
}
pub(super) fn observation<N>(
    core: &mut Core<N>,
    receipt: Option<config::Receipt>,
) -> Result<config::Observation> {
    let binding = binding_digest(&core.bindings)?;
    let boot = core.boot.clone();
    let bindings = core.bindings.values().cloned().collect::<Vec<_>>();
    let snapshot = core.store.transact(|tx| {
        let meta: HostMeta = decode(
            &tx.get(&name("host/meta"))?
                .ok_or(rx_ports::StoreError::Integrity("host meta missing".into()))?,
            "rx.host.meta.v1",
        )?;
        let mut cells = Vec::new();
        for b in &bindings {
            let state: CellState = decode(
                &tx.get(&key("cell", &b.cell))?
                    .ok_or(rx_ports::StoreError::Integrity("cell missing".into()))?,
                "rx.host.cell.v1",
            )?;
            let applied = tx
                .get(&key("process-context", &b.cell))?
                .map(|r| decode::<config::AppliedContext>(&r, CONTEXT))
                .transpose()?;
            cells.push(config::CellObservation {
                cell: b.cell.clone(),
                definition: b.definition.sha256,
                envelope: b.envelope.sha256,
                environment: name(match b.environment {
                    Environment::Simulation => "SIMULATION",
                    Environment::Physical => "PHYSICAL",
                }),
                epoch: state.epoch,
                scopes: state.scopes,
                blocked: state.blocked.into_iter().collect(),
                applied,
            });
        }
        Ok(config::Snapshot {
            schema: name("rx.host-process-configuration-snapshot.v1"),
            host: bindings[0].host.clone(),
            host_boot: boot,
            delivery_journal: meta.delivery_journal,
            binding_digest: binding,
            cells,
        })
    })?;
    let matches = receipt.as_ref().is_some_and(|r| {
        r.status == config::Status::AppliedUnqualified
            && r.host_boot == snapshot.host_boot
            && r.journal == snapshot.delivery_journal
            && r.request.binding_digest == snapshot.binding_digest
            && r.request.cells.iter().all(|target| {
                snapshot.cells.iter().any(|c| {
                    c.cell == target.cell
                        && c.epoch == target.epoch
                        && c.scopes == target.scopes
                        && c.applied.as_ref().is_some_and(|a| {
                            a.configuration == target.after_configuration
                                && a.binding_digest == binding
                                && a.request == r.request.id
                                && a.receipt_sequence == r.sequence
                                && a.change == r.request.change
                        })
                })
            })
    });
    let value = config::Observation {
        schema: name("rx.host-process-configuration-observation.v1"),
        snapshot,
        receipt,
        context_matches_current_host: matches,
        activation_authorized: false,
    };
    value.validate().map_err(invalid)?;
    Ok(value)
}
impl<N: NativeAdapter, C: Clock, H: BoundaryHook> Host<N, C, H> {
    pub fn inspect_process_configuration(&self, caller: &Caller) -> Result<config::Observation> {
        let mut core = self.lock()?;
        authorized(&core, caller)?;
        observation(&mut core, None)
    }
    pub fn lookup_process_configuration(
        &self,
        caller: &Caller,
        request: &Id,
    ) -> Result<config::Observation> {
        let mut core = self.lock()?;
        authorized(&core, caller)?;
        let receipt = core.store.transact(|tx| {
            tx.get(&key("configuration-request", (&caller.peer, request)))?
                .map(|r| decode::<config::Receipt>(&r, RECEIPT))
                .transpose()
        })?;
        if let Some(r) = &receipt {
            r.validate().map_err(invalid)?;
        }
        observation(&mut core, receipt)
    }
    pub fn accept_process_configuration(
        &self,
        caller: &Caller,
        request: config::Request,
    ) -> Result<config::Observation> {
        request.validate().map_err(invalid)?;
        let digest = request.digest().map_err(invalid)?;
        let mut core = self.lock()?;
        authorized(&core, caller)?;
        require_admission(&core)?;
        let cache = key("configuration-request", (&caller.peer, &request.id));
        if let Some(row) = core.store.transact(|tx| tx.get(&cache))? {
            let receipt: config::Receipt = decode(&row, RECEIPT)?;
            receipt.validate().map_err(invalid)?;
            if receipt.request_digest != digest {
                return Err(HostError::Conflict);
            }
            return observation(&mut core, Some(receipt));
        }
        let actual_binding = binding_digest(&core.bindings)?;
        let boot = core.boot.clone();
        if request.host != core.bindings.values().next().expect("binding").host
            || request.expected_host_boot != boot
            || request.binding_digest != actual_binding
            || request
                .cells
                .iter()
                .map(|c| &c.cell)
                .collect::<BTreeSet<_>>()
                != core.bindings.keys().collect()
        {
            return Err(HostError::Stale);
        }
        let bindings = core.bindings.clone();
        let mut reason = None;
        for target in &request.cells {
            let b = &bindings[&target.cell];
            let allowed = b
                .allowed_intents
                .iter()
                .map(|i| i.digest().map_err(invalid))
                .collect::<Result<BTreeSet<_>>>()?;
            if target.definition != b.definition.sha256
                || target.envelope != b.envelope.sha256
                || target.environment.as_str()
                    != match b.environment {
                        Environment::Simulation => "SIMULATION",
                        Environment::Physical => "PHYSICAL",
                    }
                || !target.required_intents.iter().all(|i| allowed.contains(i))
                || !target
                    .required_conditions
                    .iter()
                    .all(|id| b.condition_ids.contains(id))
            {
                reason = Some(name("UNSUPPORTED_BINDING_CHANGE"));
            }
            validate_scopes(b, &target.scopes)?;
            if core.armed.contains(&target.cell) {
                return Err(HostError::Guard);
            }
        }
        let resources = bindings
            .values()
            .flat_map(|b| {
                b.allowed_intents
                    .iter()
                    .flat_map(|i| i.resource_set.iter().cloned())
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let pending = core.store.transact(|tx| {
            let meta: HostMeta = decode(
                &tx.get(&name("host/meta"))?
                    .ok_or(rx_ports::StoreError::Integrity("meta missing".into()))?,
                "rx.host.meta.v1",
            )?;
            if meta.delivery_journal != request.expected_delivery_journal {
                return Err(rx_ports::StoreError::Invalid(
                    "configuration journal differs".into(),
                ));
            }
            let slot = key(
                "configuration-slot",
                (&caller.peer, &request.change, request.preparation),
            );
            if tx.get(&slot)?.is_some() {
                return Err(rx_ports::StoreError::KeyConflict);
            }
            for target in &request.cells {
                let state: CellState = decode(
                    &tx.get(&key("cell", &target.cell))?
                        .ok_or(rx_ports::StoreError::Invalid("unknown cell".into()))?,
                    "rx.host.cell.v1",
                )?;
                if state.epoch != target.epoch
                    || state.scopes != target.scopes
                    || state.blocked.is_empty()
                {
                    return Err(rx_ports::StoreError::Invalid(
                        "configuration requires a current blocked fence".into(),
                    ));
                }
                let (_, fence): (Digest, GateReceipt) = decode(
                    &tx.get(&key("fence-request", &target.fence_request))?
                        .ok_or(rx_ports::StoreError::Invalid(
                            "fence receipt missing".into(),
                        ))?,
                    "rx.host.gate-receipt.v1",
                )?;
                if fence.host_boot != boot
                    || fence.journal != meta.delivery_journal
                    || fence.cell != target.cell
                    || fence.state.epoch != target.epoch
                    || fence.state.scopes != target.scopes
                    || fence.state.blocked != state.blocked
                {
                    return Err(rx_ports::StoreError::Invalid(
                        "configuration fence differs".into(),
                    ));
                }
                let current = tx
                    .get(&key("process-context", &target.cell))?
                    .map(|r| decode::<config::AppliedContext>(&r, CONTEXT))
                    .transpose()?;
                if current.as_ref().map(|c| c.configuration) != target.expected_context {
                    return Err(rx_ports::StoreError::Invalid(
                        "expected Host process context differs".into(),
                    ));
                }
            }
            let mut pending = false;
            for row in tx.scan("delivery/")? {
                let r: DeliveryRecord = decode(&row, "rx.host.delivery.v1")?;
                if matches!(
                    r.state,
                    ReceiptState::Prepared
                        | ReceiptState::SendEntered
                        | ReceiptState::NativeAccepted
                ) {
                    pending = true;
                }
            }
            Ok(pending)
        })?;
        if pending {
            reason = Some(name("HOST_WORK_UNRESOLVED"));
        }
        let mut quiescence = None;
        if reason.is_none() {
            match core.native.handover_snapshot(&resources) {
                Ok(q) => {
                    let now = self.clock.now();
                    if !q.no_pending_commands
                        || !q.control_available
                        || !q.support_stable
                        || now
                            .age_ns(&q.observed_at)
                            .is_none_or(|age| age.saturating_add(q.uncertainty_ns.0) > 100_000_000)
                    {
                        reason = Some(name("QUIESCENCE_NOT_CONFIRMED"));
                    } else {
                        quiescence = Some(config::Quiescence {
                            device_session: q.device_session,
                            observed_at: q.observed_at,
                            uncertainty_ns: q.uncertainty_ns,
                            resources,
                        });
                    }
                }
                Err(_) => reason = Some(name("QUIESCENCE_UNAVAILABLE")),
            }
        }
        let now = self.clock.now();
        if quiescence.as_ref().is_some_and(|q| {
            now.age_ns(&q.observed_at)
                .is_none_or(|age| age.saturating_add(q.uncertainty_ns.0) > 100_000_000)
        }) {
            reason = Some(name("QUIESCENCE_EXPIRED"));
        }
        let receipt = core.store.transact(|tx| {
            let (sequence, journal) = next_sequence(tx)?;
            let status = if reason.is_none() {
                config::Status::AppliedUnqualified
            } else {
                config::Status::NotApplied
            };
            let mut changed = false;
            if status == config::Status::AppliedUnqualified {
                for target in &request.cells {
                    let k = key("process-context", &target.cell);
                    let old = tx.get(&k)?;
                    let previous = old
                        .as_ref()
                        .map(|r| decode::<config::AppliedContext>(r, CONTEXT))
                        .transpose()?;
                    changed |= previous
                        .as_ref()
                        .is_none_or(|p| p.configuration != target.after_configuration);
                    let context = config::AppliedContext {
                        cell: target.cell.clone(),
                        configuration: target.after_configuration,
                        change: request.change.clone(),
                        request: request.id.clone(),
                        receipt_sequence: sequence,
                        binding_digest: actual_binding,
                    };
                    tx.put(&k, old.map(|r| r.revision), &doc(CONTEXT, &context)?)?;
                }
            }
            let effect = if status == config::Status::NotApplied {
                config::Effect::None
            } else if changed {
                config::Effect::Installed
            } else {
                config::Effect::AlreadyPresent
            };
            let receipt = config::Receipt {
                schema: name(RECEIPT),
                request: request.clone(),
                request_digest: digest,
                host_boot: boot,
                journal,
                sequence,
                status,
                effect,
                reason,
                quiescence,
                recorded_at: now,
            };
            receipt.validate().map_err(rx_ports::StoreError::Invalid)?;
            tx.put(&cache, None, &doc(RECEIPT, &receipt)?)?;
            tx.put(
                &key(
                    "configuration-slot",
                    (&caller.peer, &request.change, request.preparation),
                ),
                None,
                &doc("rx.host.configuration-slot.v1", &request.id)?,
            )?;
            tx.put(
                &name(format!("configuration-history/{:020}", sequence.0)),
                None,
                &doc(RECEIPT, &receipt)?,
            )?;
            self.hooks
                .before_configuration_commit()
                .map_err(rx_ports::StoreError::Unavailable)?;
            Ok(receipt)
        })?;
        if receipt.status == config::Status::AppliedUnqualified {
            for target in &receipt.request.cells {
                core.armed.remove(&target.cell);
            }
        }
        self.hooks.after_configuration_commit();
        observation(&mut core, Some(receipt))
    }
}
