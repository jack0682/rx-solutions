use super::*;
use rx_domain::host_snapshot::HostSnapshot;
impl<N: NativeAdapter, C: Clock, H: BoundaryHook> Host<N, C, H> {
    pub fn bootstrap_snapshot(
        &self,
        caller: &Caller,
        cell: &Name,
        sources: &[Name],
    ) -> Result<HostSnapshot> {
        let mut core = self.lock()?;
        authorized(&core, caller)?;
        if sources.len() > 128 || sources.iter().collect::<BTreeSet<_>>().len() != sources.len() {
            return Err(HostError::Invalid("source request".into()));
        }
        let binding = core
            .bindings
            .get(cell)
            .cloned()
            .ok_or(HostError::Forbidden)?;
        let (sources_available, observations) = match core.native.observe_sources(cell, sources) {
            Ok(v) => (true, v),
            Err(HostError::Guard) => (false, vec![]),
            Err(e) => return Err(e),
        };
        if sources_available
            && observations
                .iter()
                .map(|o| &o.source)
                .collect::<BTreeSet<_>>()
                != sources.iter().collect()
        {
            return Err(HostError::Invalid("native source reply differs".into()));
        }
        let captured_at = self.clock.now();
        let boot = core.boot.clone();
        let resources: BTreeSet<_> = binding
            .allowed_intents
            .iter()
            .flat_map(|i| i.resource_set.iter().cloned())
            .collect();
        let snapshot = core.store.transact(|tx| {
            let row = tx
                .get(&name("host/meta"))?
                .ok_or(rx_ports::StoreError::Integrity("Host meta missing".into()))?;
            let meta: HostMeta = decode(&row, "rx.host.meta.v1")?;
            let row = tx
                .get(&key("cell", cell))?
                .ok_or(rx_ports::StoreError::Invalid("cell missing".into()))?;
            let state: CellState = decode(&row, "rx.host.cell.v1")?;
            let mut pending_operations = vec![];
            let mut pending_permits = vec![];
            for row in tx.scan("delivery/")? {
                let d: DeliveryRecord = decode(&row, "rx.host.delivery.v1")?;
                if d.cell == *cell
                    && matches!(
                        d.state,
                        ReceiptState::Prepared
                            | ReceiptState::SendEntered
                            | ReceiptState::NativeAccepted
                    )
                {
                    pending_operations.push(d.operation);
                    pending_permits.push(d.permit);
                }
            }
            let mut resource_fences = BTreeMap::new();
            for r in resources {
                let value = if let Some(row) = tx.get(&key("resource", &r))? {
                    decode::<ResourceFence>(&row, "rx.host.resource.v1")?.maximum
                } else {
                    Counter(0)
                };
                resource_fences.insert(r, value);
            }
            Ok(HostSnapshot {
                schema: name("rx.host-snapshot.v1"),
                host: binding.host,
                host_boot: boot,
                delivery_journal: meta.delivery_journal,
                evidence_journal: meta.evidence_journal,
                cell: cell.clone(),
                definition: binding.definition.sha256,
                envelope: binding.envelope.sha256,
                environment: name(match binding.environment {
                    Environment::Simulation => "SIMULATION",
                    Environment::Physical => "PHYSICAL",
                }),
                epoch: state.epoch,
                scopes: state.scopes,
                resource_fences,
                captured_at,
                sources_available,
                observations,
                block_ids: state.blocked.into_iter().collect(),
                pending_operations,
                pending_permits,
            })
        })?;
        snapshot.validate().map_err(invalid)?;
        Ok(snapshot)
    }
}
