use super::*;
use rx_domain::{host_configuration as config, host_qualification as data};
const ACTIVE: &str = "rx.host.accepted-qualification.v1";
const RECEIPT: &str = "rx.host-qualification-receipt.v1";
#[derive(serde::Serialize, serde::Deserialize)]
struct Accepted {
    request: Id,
    sequence: Counter,
    host_boot: Id,
    journal: Id,
    binding_digest: Digest,
    device_session: Id,
    target: data::CellTarget,
}
fn project(a: Accepted) -> data::AcceptedCell {
    let t = a.target;
    data::AcceptedCell {
        cell: t.cell,
        request: a.request,
        qualification: t.qualification,
        qualification_revision: t.qualification_revision,
        acceptance_sequence: a.sequence,
        host_boot: a.host_boot,
        epoch: t.epoch,
        scopes: t.scopes,
        configuration: t.configuration,
        context_request: t.context_request,
        context_sequence: t.context_sequence,
        binding_digest: a.binding_digest,
    }
}
fn observation<N>(core: &mut Core<N>, receipt: Option<data::Receipt>) -> Result<data::Observation> {
    let snapshot = configuration::observation(core, None)?.snapshot;
    let accepted = core.store.transact(|tx| {
        let mut values = Vec::new();
        for c in &snapshot.cells {
            if let Some(row) = tx.get(&key("accepted-qualification", &c.cell))? {
                let a: Accepted = decode(&row, ACTIVE)?;
                if a.target.cell != c.cell {
                    return Err(rx_ports::StoreError::Integrity(
                        "qualification cell differs".into(),
                    ));
                }
                values.push(project(a));
            }
        }
        Ok(values)
    })?;
    let mut v = data::Observation {
        schema: name("rx.host-qualification-observation.v1"),
        snapshot,
        receipt,
        accepted,
        receipt_matches_current_host: false,
        activation_authorized: false,
    };
    v.receipt_matches_current_host = v.current();
    v.validate().map_err(invalid)?;
    Ok(v)
}
/// A process context never falls back to the startup qualification. No Arm is restored here.
pub(super) fn current_target<N>(
    core: &mut Core<N>,
    cell: &Name,
) -> Result<Option<data::CellTarget>> {
    let binding = configuration::binding_digest(&core.bindings)?;
    let physical = core
        .bindings
        .get(cell)
        .ok_or(HostError::Forbidden)?
        .environment
        == Environment::Physical;
    let boot = core.boot.clone();
    let platform = core
        .bindings
        .values()
        .next()
        .expect("binding")
        .platform
        .clone();
    core.store
        .transact(|tx| {
            let context = tx.get(&key("process-context", cell))?;
            let accepted = tx.get(&key("accepted-qualification", cell))?;
            let Some(context) = context else {
                if accepted.is_some() {
                    return Err(rx_ports::StoreError::Integrity(
                        "qualification without process context".into(),
                    ));
                }
                if physical {
                    return Err(rx_ports::StoreError::Invalid(
                        "physical binding requires current process qualification acceptance".into(),
                    ));
                }
                return Ok(None);
            };
            let context: config::AppliedContext =
                decode(&context, "rx.host.applied-process-context.v1")?;
            let a: Accepted = decode(
                &accepted.ok_or(rx_ports::StoreError::Invalid(
                    "process context is unqualified".into(),
                ))?,
                ACTIVE,
            )?;
            let state: CellState = decode(
                &tx.get(&key("cell", cell))?
                    .ok_or(rx_ports::StoreError::Invalid("cell missing".into()))?,
                "rx.host.cell.v1",
            )?;
            let meta: HostMeta = decode(
                &tx.get(&name("host/meta"))?
                    .ok_or(rx_ports::StoreError::Integrity("meta missing".into()))?,
                "rx.host.meta.v1",
            )?;
            let t = &a.target;
            if t.cell != *cell
                || a.host_boot != boot
                || a.journal != meta.delivery_journal
                || a.binding_digest != binding
                || context.binding_digest != binding
                || context.cell != *cell
                || context.configuration != t.configuration
                || context.request != t.context_request
                || context.receipt_sequence != t.context_sequence
                || state.epoch != t.epoch
                || state.scopes != t.scopes
            {
                return Err(rx_ports::StoreError::Invalid(
                    "qualification context changed".into(),
                ));
            }
            let receipt: data::Receipt = decode(
                &tx.get(&key("qualification-request", (&platform, &a.request)))?
                    .ok_or(rx_ports::StoreError::Integrity(
                        "qualification receipt missing".into(),
                    ))?,
                RECEIPT,
            )?;
            receipt
                .validate()
                .map_err(rx_ports::StoreError::Integrity)?;
            if receipt.sequence != a.sequence
                || receipt.host_boot != boot
                || receipt.journal != meta.delivery_journal
            {
                return Err(rx_ports::StoreError::Integrity(
                    "qualification receipt identity differs".into(),
                ));
            }
            for target in &receipt.request.cells {
                let state: CellState = decode(
                    &tx.get(&key("cell", &target.cell))?
                        .ok_or(rx_ports::StoreError::Invalid(
                            "qualification cohort missing".into(),
                        ))?,
                    "rx.host.cell.v1",
                )?;
                let accepted: Accepted = decode(
                    &tx.get(&key("accepted-qualification", &target.cell))?
                        .ok_or(rx_ports::StoreError::Invalid(
                            "qualification cohort acceptance missing".into(),
                        ))?,
                    ACTIVE,
                )?;
                let context: config::AppliedContext = decode(
                    &tx.get(&key("process-context", &target.cell))?.ok_or(
                        rx_ports::StoreError::Invalid(
                            "qualification cohort context missing".into(),
                        ),
                    )?,
                    "rx.host.applied-process-context.v1",
                )?;
                if state.epoch != target.epoch
                    || state.scopes != target.scopes
                    || accepted.request != a.request
                    || accepted.sequence != a.sequence
                    || accepted.host_boot != boot
                    || canonical::bytes(&accepted.target)
                        .map_err(|e| rx_ports::StoreError::Integrity(e.to_string()))?
                        != canonical::bytes(target)
                            .map_err(|e| rx_ports::StoreError::Integrity(e.to_string()))?
                    || context.configuration != target.configuration
                    || context.request != target.context_request
                    || context.receipt_sequence != target.context_sequence
                {
                    return Err(rx_ports::StoreError::Invalid(
                        "qualification cohort changed".into(),
                    ));
                }
            }
            Ok(Some(a.target))
        })
        .map_err(|e| match e {
            rx_ports::StoreError::Invalid(_) => HostError::Guard,
            other => other.into(),
        })
}
pub(super) fn device_session<N>(core: &mut Core<N>, cell: &Name) -> Result<Option<Id>> {
    core.store
        .transact(|tx| {
            tx.get(&key("accepted-qualification", cell))?
                .map(|r| decode::<Accepted>(&r, ACTIVE).map(|a| a.device_session))
                .transpose()
        })
        .map_err(Into::into)
}
impl<N: NativeAdapter, C: Clock, H: BoundaryHook> Host<N, C, H> {
    pub fn inspect_qualification(&self, caller: &Caller) -> Result<data::Observation> {
        let mut core = self.lock()?;
        authorized(&core, caller)?;
        observation(&mut core, None)
    }
    pub fn lookup_qualification(&self, caller: &Caller, id: &Id) -> Result<data::Observation> {
        let mut core = self.lock()?;
        authorized(&core, caller)?;
        let receipt = core.store.transact(|tx| {
            tx.get(&key("qualification-request", (&caller.peer, id)))?
                .map(|r| decode::<data::Receipt>(&r, RECEIPT))
                .transpose()
        })?;
        if let Some(r) = &receipt {
            r.validate().map_err(invalid)?;
        }
        observation(&mut core, receipt)
    }
    pub fn accept_qualification(
        &self,
        caller: &Caller,
        request: data::Request,
    ) -> Result<data::Observation> {
        request.validate().map_err(invalid)?;
        let digest = request.digest().map_err(invalid)?;
        let mut core = self.lock()?;
        authorized(&core, caller)?;
        require_admission(&core)?;
        let cache = key("qualification-request", (&caller.peer, &request.id));
        if let Some(row) = core.store.transact(|tx| tx.get(&cache))? {
            let receipt: data::Receipt = decode(&row, RECEIPT)?;
            receipt.validate().map_err(invalid)?;
            if receipt.request_digest != digest {
                return Err(HostError::Conflict);
            }
            return observation(&mut core, Some(receipt));
        }
        let binding = configuration::binding_digest(&core.bindings)?;
        let boot = core.boot.clone();
        if request.host != core.bindings.values().next().unwrap().host
            || request.expected_host_boot != boot
            || request.binding_digest != binding
            || request
                .cells
                .iter()
                .map(|c| &c.cell)
                .collect::<BTreeSet<_>>()
                != core.bindings.keys().collect()
        {
            return Err(HostError::Stale);
        }
        let slot = key(
            "qualification-slot",
            (&caller.peer, &request.review, request.review_revision),
        );
        let mut reason = None;
        for t in &request.cells {
            let b = &core.bindings[&t.cell];
            validate_scopes(b, &t.scopes)?;
            if core.armed.contains(&t.cell) {
                return Err(HostError::Guard);
            }
            let allowed = b
                .allowed_intents
                .iter()
                .map(|i| i.digest().map_err(invalid))
                .collect::<Result<BTreeSet<_>>>()?;
            let purposes = b
                .purposes
                .iter()
                .map(|p| match p {
                    Purpose::Production => "PRODUCTION",
                    Purpose::Setup => "SETUP",
                    Purpose::Recovery => "RECOVERY",
                })
                .collect::<BTreeSet<_>>();
            if t.definition != b.definition.sha256
                || t.envelope != b.envelope.sha256
                || t.environment.as_str()
                    != match b.environment {
                        Environment::Simulation => "SIMULATION",
                        Environment::Physical => "PHYSICAL",
                    }
                || !t.allowed_intents.iter().all(|i| allowed.contains(i))
                || !t.purposes.iter().all(|p| purposes.contains(p.as_str()))
            {
                reason = Some(name("UNSUPPORTED_QUALIFICATION_SCOPE"));
            }
        }
        let pending = core.store.transact(|tx| {
            if tx.get(&slot)?.is_some() {
                return Err(rx_ports::StoreError::KeyConflict);
            }
            let meta: HostMeta = decode(
                &tx.get(&name("host/meta"))?
                    .ok_or(rx_ports::StoreError::Integrity("meta missing".into()))?,
                "rx.host.meta.v1",
            )?;
            if meta.delivery_journal != request.delivery_journal {
                return Err(rx_ports::StoreError::Invalid(
                    "qualification journal differs".into(),
                ));
            }
            for t in &request.cells {
                let state: CellState = decode(
                    &tx.get(&key("cell", &t.cell))?
                        .ok_or(rx_ports::StoreError::Invalid("cell missing".into()))?,
                    "rx.host.cell.v1",
                )?;
                if state.epoch != t.epoch
                    || state.scopes != t.scopes
                    || state.blocked.is_empty()
                    || !t.required_blocks.iter().all(|b| state.blocked.contains(b))
                {
                    return Err(rx_ports::StoreError::Invalid(
                        "qualification blocked epoch differs".into(),
                    ));
                }
                let (_, f): (Digest, GateReceipt) = decode(
                    &tx.get(&key("fence-request", &t.fence_request))?.ok_or(
                        rx_ports::StoreError::Invalid("qualification fence missing".into()),
                    )?,
                    "rx.host.gate-receipt.v1",
                )?;
                if f.id != t.fence_request
                    || f.cell != t.cell
                    || f.host_boot != boot
                    || f.journal != request.delivery_journal
                    || f.state.epoch != t.epoch
                    || f.state.scopes != t.scopes
                    || !t
                        .required_blocks
                        .iter()
                        .all(|b| f.state.blocked.contains(b))
                {
                    return Err(rx_ports::StoreError::Invalid(
                        "qualification fence differs".into(),
                    ));
                }
                let ctx: config::AppliedContext = decode(
                    &tx.get(&key("process-context", &t.cell))?
                        .ok_or(rx_ports::StoreError::Invalid("no applied context".into()))?,
                    "rx.host.applied-process-context.v1",
                )?;
                if ctx.change != request.change
                    || ctx.configuration != t.configuration
                    || ctx.request != t.context_request
                    || ctx.receipt_sequence != t.context_sequence
                    || ctx.binding_digest != binding
                {
                    return Err(rx_ports::StoreError::Invalid(
                        "qualification process context differs".into(),
                    ));
                }
            }
            let mut pending = false;
            for row in tx.scan("delivery/")? {
                let d: DeliveryRecord = decode(&row, "rx.host.delivery.v1")?;
                pending |= matches!(
                    d.state,
                    ReceiptState::Prepared
                        | ReceiptState::SendEntered
                        | ReceiptState::NativeAccepted
                );
            }
            Ok(pending)
        })?;
        if pending {
            reason = Some(name("HOST_WORK_UNRESOLVED"));
        }
        let resources = core
            .bindings
            .values()
            .flat_map(|b| {
                b.allowed_intents
                    .iter()
                    .flat_map(|i| i.resource_set.iter().cloned())
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let mut quiescence = None;
        if reason.is_none() {
            match core.native.handover_snapshot(&resources) {
                Ok(q) => {
                    if !q.no_pending_commands
                        || !q.control_available
                        || !q.support_stable
                        || self
                            .clock
                            .now()
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
                data::Status::Accepted
            } else {
                data::Status::NotAccepted
            };
            if status == data::Status::Accepted {
                for target in &request.cells {
                    let mut semantic_target = target.clone();
                    semantic_target.dependencies.sort();
                    semantic_target.allowed_intents.sort();
                    semantic_target.purposes.sort();
                    let identity = canonical::digest(
                        "RX-HOST-QUALIFICATION-IDENTITY-v1",
                        &(
                            &target.cell,
                            &target.qualification,
                            target.qualification_revision,
                            target.configuration,
                            target.definition,
                            target.envelope,
                            &target.environment,
                            &target.limitations,
                            &semantic_target.dependencies,
                            &semantic_target.allowed_intents,
                            &semantic_target.purposes,
                            (
                                &request.review,
                                request.review_revision,
                                request.review_digest,
                                request.decision_revision,
                                request.policy_digest,
                                request.application_digest,
                            ),
                        ),
                    )
                    .map_err(|e| rx_ports::StoreError::Invalid(e.to_string()))?;
                    let identity_key = key(
                        "qualification-identity",
                        (
                            &caller.peer,
                            &target.cell,
                            &target.qualification,
                            target.qualification_revision,
                        ),
                    );
                    if let Some(old) = tx.get(&identity_key)? {
                        let old: Digest = decode(&old, "rx.host.qualification-identity.v1")?;
                        if old != identity {
                            return Err(rx_ports::StoreError::KeyConflict);
                        }
                    } else {
                        tx.put(
                            &identity_key,
                            None,
                            &doc("rx.host.qualification-identity.v1", &identity)?,
                        )?;
                    }
                    let maximum_key = key(
                        "qualification-maximum",
                        (&caller.peer, &target.cell, &target.qualification),
                    );
                    let previous = tx.get(&maximum_key)?;
                    if let Some(old) = &previous {
                        let maximum: Counter = decode(old, "rx.host.qualification-maximum.v1")?;
                        if target.qualification_revision < maximum {
                            return Err(rx_ports::StoreError::Invalid(
                                "qualification revision regressed".into(),
                            ));
                        }
                    }
                    tx.put(
                        &maximum_key,
                        previous.map(|r| r.revision),
                        &doc(
                            "rx.host.qualification-maximum.v1",
                            &target.qualification_revision,
                        )?,
                    )?;
                    let k = key("accepted-qualification", &target.cell);
                    let old = tx.get(&k)?;
                    if let Some(previous) = &old {
                        let previous: Accepted = decode(previous, ACTIVE)?;
                        if target.epoch <= previous.target.epoch {
                            return Err(rx_ports::StoreError::Invalid(
                                "replacement qualification requires a newer cell epoch".into(),
                            ));
                        }
                    }
                    tx.put(
                        &k,
                        old.map(|r| r.revision),
                        &doc(
                            ACTIVE,
                            &Accepted {
                                request: request.id.clone(),
                                sequence,
                                host_boot: boot.clone(),
                                journal: journal.clone(),
                                binding_digest: binding,
                                device_session: quiescence
                                    .as_ref()
                                    .expect("accepted quiescence")
                                    .device_session
                                    .clone(),
                                target: target.clone(),
                            },
                        )?,
                    )?;
                }
            }
            let receipt = data::Receipt {
                schema: name(RECEIPT),
                request: request.clone(),
                request_digest: digest,
                host_boot: boot,
                journal,
                sequence,
                status,
                reason,
                quiescence,
                recorded_at: now,
            };
            receipt.validate().map_err(rx_ports::StoreError::Invalid)?;
            tx.put(&cache, None, &doc(RECEIPT, &receipt)?)?;
            tx.put(
                &slot,
                None,
                &doc("rx.host.qualification-slot.v1", &request.id)?,
            )?;
            tx.put(
                &name(format!("qualification-history/{:020}", sequence.0)),
                None,
                &doc(RECEIPT, &receipt)?,
            )?;
            self.hooks
                .before_qualification_commit()
                .map_err(rx_ports::StoreError::Unavailable)?;
            Ok(receipt)
        })?;
        for c in &request.cells {
            core.armed.remove(&c.cell);
        }
        self.hooks.after_qualification_commit();
        observation(&mut core, Some(receipt))
    }
}
