use super::*;

pub(super) fn validate_scopes(binding: &Binding, scopes: &BTreeMap<Name, Counter>) -> Result<()> {
    if scopes.keys().collect::<BTreeSet<_>>() != binding.scope_ids.iter().collect()
        || scopes.values().any(|e| e.0 == 0)
    {
        return Err(HostError::Stale);
    }
    Ok(())
}

pub(super) fn validate_identity<N>(core: &mut Core<N>, r: &Request) -> Result<()> {
    let qualified = qualification::current_target(core, &r.permit.cell)?;
    let b = core
        .bindings
        .get(&r.permit.cell)
        .ok_or(HostError::Forbidden)?;
    if !b.purposes.contains(&r.permit.purpose)
        || r.intent.digest().map_err(invalid)? != r.digest
        || r.permit.operation != r.operation
        || r.permit.digest != r.digest
        || r.permit.grant != r.grant
        || r.permit.envelope != b.envelope.sha256
        || r.permit.qualification
            != qualified
                .as_ref()
                .map_or(&b.qualification, |q| &q.qualification)
                .clone()
        || r.permit.qualification_revision
            != qualified
                .as_ref()
                .map_or(b.qualification_revision, |q| q.qualification_revision)
        || qualified.as_ref().is_some_and(|q| {
            !q.allowed_intents.contains(&r.digest)
                || !q.purposes.iter().any(|p| {
                    p.as_str()
                        == match r.permit.purpose {
                            Purpose::Production => "PRODUCTION",
                            Purpose::Setup => "SETUP",
                            Purpose::Recovery => "RECOVERY",
                        }
                })
        })
        || !b
            .allowed_intents
            .iter()
            .any(|i| i.digest().ok() == Some(r.digest))
    {
        return Err(HostError::Conflict);
    }
    match (&r.permit.purpose, &r.permit.parent) {
        (Purpose::Production | Purpose::Setup, PermitParent::Mandate(_)) => {}
        (Purpose::Recovery, PermitParent::Recovery { visit, .. }) if visit.0 > 0 => {}
        _ => return Err(HostError::Conflict),
    }
    validate_scopes(b, &r.permit.scopes)
}

pub(super) fn same_operation(record: &DeliveryRecord, r: &Request) -> Result<()> {
    if record.digest != r.digest
        || record.cell != r.permit.cell
        || (record.permit == r.permit.id
            && record.permit_digest
                != canonical::digest("RX-HOST-PERMIT-v1", &r.permit).map_err(invalid)?)
    {
        return Err(HostError::Conflict);
    }
    Ok(())
}

pub(super) fn validate_current<N: NativeAdapter>(
    core: &mut Core<N>,
    caller: &Caller,
    r: &Request,
    now: &TimePoint,
) -> Result<Guard> {
    require_admission(core)?;
    if r.permit.host_boot != core.boot
        || !before(now, &r.permit.expires_at)
        || !core.armed.contains(&r.permit.cell)
    {
        return Err(HostError::Stale);
    }
    qualification::current_target(core, &r.permit.cell)?;
    let (cell, grant) = core.store.transact(|tx| {
        if tx.get(&key("retired-permit", &r.permit.id))?.is_some() {
            return Err(rx_ports::StoreError::Invalid("retired permit".into()));
        }
        let cell = tx
            .get(&key("cell", &r.permit.cell))?
            .ok_or(rx_ports::StoreError::Invalid("unknown cell".into()))?;
        let grant = tx
            .get(&key("grant", &r.grant))?
            .ok_or(rx_ports::StoreError::Invalid("unknown grant".into()))?;
        Ok((
            decode::<CellState>(&cell, "rx.host.cell.v1")?,
            decode::<StoredGrant>(&grant, "rx.host.grant.v1")?,
        ))
    })?;
    if cell.epoch != r.permit.epoch
        || cell.scopes != r.permit.scopes
        || !cell.blocked.is_empty()
        || grant.host_boot != core.boot
        || grant.owner != caller.peer
        || grant.session != caller.session
        || !before(now, &grant.expires_at)
        || r.permit.expires_at.clock_id != grant.expires_at.clock_id
        || r.permit.expires_at.ticks_ns > grant.expires_at.ticks_ns
        || r.intent
            .resource_set
            .iter()
            .any(|r| !grant.resources.contains(r))
    {
        return Err(HostError::Stale);
    }
    core.store.transact(|tx| {
        for resource in &r.intent.resource_set {
            let row = tx
                .get(&key("resource", resource))?
                .ok_or(rx_ports::StoreError::Invalid("unknown resource".into()))?;
            let f: ResourceFence = decode(&row, "rx.host.resource.v1")?;
            if grant.fence != f.maximum {
                return Err(rx_ports::StoreError::Invalid("stale resource fence".into()));
            }
        }
        Ok(())
    })?;
    let binding = &core.bindings[&r.permit.cell];
    let required: BTreeSet<_> = binding.condition_ids.iter().cloned().collect();
    if r.permit.conditions != required {
        return Err(HostError::Guard);
    }
    let guard = core.native.guard(&r.intent, now)?;
    if qualification::device_session(core, &r.permit.cell)?
        .is_some_and(|session| session != guard.device_session)
    {
        return Err(HostError::Guard);
    }
    if !required.is_subset(&guard.satisfied) || !before(now, &guard.valid_until) {
        return Err(HostError::Guard);
    }
    Ok(guard)
}
