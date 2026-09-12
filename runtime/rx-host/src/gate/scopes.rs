use super::*;

impl<N: NativeAdapter, C: Clock, H: BoundaryHook> Host<N, C, H> {
    pub fn fence(
        &self,
        caller: &Caller,
        request_id: Id,
        cell: &Name,
        epoch: Counter,
        scopes: BTreeMap<Name, Counter>,
        blocks: BTreeSet<Id>,
    ) -> Result<GateReceipt> {
        let mut core = self.lock()?;
        authorized(&core, caller)?;
        let binding = core.bindings.get(cell).ok_or(HostError::Forbidden)?.clone();
        validate_scopes(&binding, &scopes)?;
        let fingerprint = canonical::digest("RX-HOST-FENCE-v1", &(cell, epoch, &scopes, &blocks))
            .map_err(invalid)?;
        let boot = core.boot.clone();
        let (receipt, replayed) = core.store.transact(|tx| {
            let cache = key("fence-request", &request_id);
            if let Some(old) = tx.get(&cache)? {
                let (digest, receipt): (Digest, GateReceipt) =
                    decode(&old, "rx.host.gate-receipt.v1")?;
                if digest != fingerprint {
                    return Err(rx_ports::StoreError::KeyConflict);
                }
                return Ok((receipt, true));
            }
            let row = tx
                .get(&key("cell", cell))?
                .ok_or(rx_ports::StoreError::Invalid("unknown cell".into()))?;
            let mut state: CellState = decode(&row, "rx.host.cell.v1")?;
            if epoch < state.epoch
                || scopes
                    .iter()
                    .any(|(k, v)| state.scopes.get(k).is_some_and(|old| v < old))
            {
                return Err(rx_ports::StoreError::Invalid("stale cell fence".into()));
            }
            state.epoch = epoch;
            state.scopes.extend(scopes);
            state.blocked.extend(blocks);
            tx.put(
                &row.key,
                Some(row.revision),
                &doc("rx.host.cell.v1", &state)?,
            )?;
            let (sequence, journal) = next_sequence(tx)?;
            let receipt = GateReceipt {
                id: request_id,
                host_boot: boot,
                journal,
                sequence,
                cell: cell.clone(),
                state,
            };
            tx.put(
                &cache,
                None,
                &doc("rx.host.gate-receipt.v1", &(fingerprint, &receipt))?,
            )?;
            Ok((receipt, false))
        })?;
        if !replayed {
            core.armed.remove(cell);
        }
        Ok(receipt)
    }
    pub fn arm(
        &self,
        caller: &Caller,
        attempt: Id,
        cell: &Name,
        epoch: Counter,
        scopes: &BTreeMap<Name, Counter>,
        clear: &BTreeSet<Id>,
    ) -> Result<GateReceipt> {
        let mut core = self.lock()?;
        authorized(&core, caller)?;
        require_admission(&core)?;
        qualification::current_target(&mut core, cell)?;
        let fingerprint =
            canonical::digest("RX-HOST-ARM-v1", &(cell, epoch, scopes, clear)).map_err(invalid)?;
        let boot = core.boot.clone();
        let (receipt, replayed) = core.store.transact(|tx| {
            let cache = key("arm-request", &attempt);
            if let Some(old) = tx.get(&cache)? {
                let (digest, receipt): (Digest, GateReceipt) =
                    decode(&old, "rx.host.gate-receipt.v1")?;
                if digest != fingerprint {
                    return Err(rx_ports::StoreError::KeyConflict);
                }
                return Ok((receipt, true));
            }
            let row = tx
                .get(&key("cell", cell))?
                .ok_or(rx_ports::StoreError::Invalid("unknown cell".into()))?;
            let mut state: CellState = decode(&row, "rx.host.cell.v1")?;
            if state.epoch != epoch || &state.scopes != scopes || clear != &state.blocked {
                return Err(rx_ports::StoreError::Invalid("arm target mismatch".into()));
            }
            state.blocked.retain(|id| !clear.contains(id));
            tx.put(
                &row.key,
                Some(row.revision),
                &doc("rx.host.cell.v1", &state)?,
            )?;
            let (sequence, journal) = next_sequence(tx)?;
            let receipt = GateReceipt {
                id: attempt,
                host_boot: boot,
                journal,
                sequence,
                cell: cell.clone(),
                state,
            };
            tx.put(
                &cache,
                None,
                &doc("rx.host.gate-receipt.v1", &(fingerprint, &receipt))?,
            )?;
            Ok((receipt, false))
        })?;
        if !replayed && receipt.state.blocked.is_empty() {
            core.armed.insert(cell.clone());
        }
        Ok(receipt)
    }
}
