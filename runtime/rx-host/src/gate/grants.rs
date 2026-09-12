use super::*;

impl<N: NativeAdapter, C: Clock, H: BoundaryHook> Host<N, C, H> {
    pub fn acquire_grant(
        &self,
        caller: &Caller,
        request: Id,
        resources: Vec<Name>,
        fence: Counter,
        ttl_ns: Counter,
    ) -> Result<StoredGrant> {
        let mut core = self.lock()?;
        authorized(&core, caller)?;
        require_admission(&core)?;
        let mut resources = resources;
        resources.sort();
        if resources.is_empty()
            || ttl_ns.0 == 0
            || fence.0 == 0
            || resources.windows(2).any(|w| w[0] == w[1])
        {
            return Err(HostError::Invalid("grant fields".into()));
        }
        let declared: BTreeSet<_> = core
            .bindings
            .values()
            .flat_map(|b| {
                b.allowed_intents
                    .iter()
                    .flat_map(|i| i.resource_set.iter().cloned())
            })
            .collect();
        if resources.iter().any(|r| !declared.contains(r)) {
            return Err(HostError::Invalid("undeclared resource".into()));
        }
        let fingerprint = canonical::digest(
            "RX-HOST-GRANT-v1",
            &(&caller.peer, &resources, fence, ttl_ns),
        )
        .map_err(invalid)?;
        let cached = core
            .store
            .transact(|tx| tx.get(&key("grant-request", &request)))?;
        if let Some(record) = cached {
            let (digest, grant): (Digest, StoredGrant) =
                decode(&record, "rx.host.grant-request.v1")?;
            if digest != fingerprint {
                return Err(HostError::Conflict);
            }
            return Ok(grant);
        }
        if !core.native.can_handover(&resources) {
            return Err(HostError::Busy);
        }
        let pending = core.store.transact(|tx| tx.scan("delivery/"))?;
        for row in &pending {
            let record: DeliveryRecord = decode(row, "rx.host.delivery.v1")?;
            if matches!(
                record.state,
                ReceiptState::SendEntered | ReceiptState::NativeAccepted
            ) && record
                .intent
                .resource_set
                .iter()
                .any(|r| resources.contains(r))
            {
                return Err(HostError::Busy);
            }
        }
        let now = self.clock.now();
        let expires = now
            .ticks_ns
            .0
            .checked_add(ttl_ns.0)
            .ok_or_else(|| HostError::Invalid("grant expiry overflow".into()))?;
        let grant = StoredGrant {
            id: id(),
            host_boot: core.boot.clone(),
            owner: caller.peer.clone(),
            session: caller.session.clone(),
            resources: resources.clone(),
            fence,
            expires_at: TimePoint {
                clock_id: now.clock_id,
                ticks_ns: Counter(expires),
            },
            renew_seq: Counter(0),
            ttl_ns,
        };
        core.store.transact(|tx| {
            for resource in &resources {
                if let Some(old) = tx.get(&key("resource", resource))? {
                    let r: ResourceFence = decode(&old, "rx.host.resource.v1")?;
                    if fence <= r.maximum {
                        return Err(rx_ports::StoreError::Invalid("stale resource fence".into()));
                    }
                }
            }
            for resource in &resources {
                let k = key("resource", resource);
                let old = tx.get(&k)?;
                tx.put(
                    &k,
                    old.map(|r| r.revision),
                    &doc(
                        "rx.host.resource.v1",
                        &ResourceFence {
                            resource: resource.clone(),
                            maximum: fence,
                        },
                    )?,
                )?;
            }
            tx.put(
                &key("grant", &grant.id),
                None,
                &doc("rx.host.grant.v1", &grant)?,
            )?;
            tx.put(
                &key("grant-request", &request),
                None,
                &doc("rx.host.grant-request.v1", &(fingerprint, &grant))?,
            )?;
            Ok(())
        })?;
        Ok(grant)
    }
    pub fn renew_grant(
        &self,
        caller: &Caller,
        grant_id: &Id,
        sequence: Counter,
    ) -> Result<StoredGrant> {
        let mut core = self.lock()?;
        authorized(&core, caller)?;
        require_admission(&core)?;
        let now = self.clock.now();
        let boot = core.boot.clone();
        core.store
            .transact(|tx| {
                let row = tx
                    .get(&key("grant", grant_id))?
                    .ok_or(rx_ports::StoreError::Invalid("unknown grant".into()))?;
                let mut grant: StoredGrant = decode(&row, "rx.host.grant.v1")?;
                if grant.host_boot != boot
                    || grant.session != caller.session
                    || grant.owner != caller.peer
                    || grant.expires_at.clock_id != now.clock_id
                    || grant.expires_at.ticks_ns <= now.ticks_ns
                    || sequence.0 == 0
                {
                    return Err(rx_ports::StoreError::Invalid("expired/stale grant".into()));
                }
                if sequence < grant.renew_seq {
                    return Err(rx_ports::StoreError::Invalid("old renewal".into()));
                }
                if sequence == grant.renew_seq {
                    return Ok(grant);
                }
                for resource in &grant.resources {
                    let r = tx
                        .get(&key("resource", resource))?
                        .ok_or(rx_ports::StoreError::Invalid("missing fence".into()))?;
                    if decode::<ResourceFence>(&r, "rx.host.resource.v1")?.maximum != grant.fence {
                        return Err(rx_ports::StoreError::Invalid("revoked grant".into()));
                    }
                }
                grant.renew_seq = sequence;
                grant.expires_at.ticks_ns = Counter(
                    now.ticks_ns
                        .0
                        .checked_add(grant.ttl_ns.0)
                        .ok_or(rx_ports::StoreError::Invalid("expiry overflow".into()))?,
                );
                tx.put(
                    &row.key,
                    Some(row.revision),
                    &doc("rx.host.grant.v1", &grant)?,
                )?;
                Ok(grant)
            })
            .map_err(Into::into)
    }
}
