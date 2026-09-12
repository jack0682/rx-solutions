use super::*;
use serde::Serialize;
#[derive(Clone, Debug, Serialize)]
pub struct StopSnapshot {
    pub host_boot: Id,
    pub admission_open: bool,
    pub pending_operations: Vec<Id>,
    pub retained_evidence: Counter,
    pub native: Option<NativeShutdown>,
    pub safe_to_drop: bool,
    pub physical_shutdown_assessed: bool,
}
impl<N: NativeAdapter, C: Clock, H: BoundaryHook> Host<N, C, H> {
    /// Immediate software admission latch. No database lock and no implicit native stop/torque change.
    pub fn service_owner_lost(&self) {
        self.request_service_stop();
        self.protection.react(ProtectionIncident::OwnerLost);
    }
    pub fn request_service_stop(&self) {
        self.accepting
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }
    pub fn service_stop_snapshot(&self) -> Result<StopSnapshot> {
        let mut core = self.lock()?;
        if core.accepting.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(HostError::Guard);
        }
        core.armed.clear();
        let resources = core
            .bindings
            .values()
            .flat_map(|b| {
                b.allowed_intents
                    .iter()
                    .flat_map(|i| i.resource_set.iter().cloned())
            })
            .collect::<BTreeSet<_>>();
        let pending = core.store.transact(|tx| {
            let mut p = Vec::new();
            for row in tx.scan("delivery/")? {
                let r: DeliveryRecord = decode(&row, "rx.host.delivery.v1")?;
                if matches!(
                    r.state,
                    ReceiptState::Prepared
                        | ReceiptState::SendEntered
                        | ReceiptState::NativeAccepted
                ) {
                    p.push(r.operation);
                }
            }
            Ok(p)
        })?;
        let preparing = core
            .native
            .prepare_shutdown(&resources.iter().cloned().collect::<Vec<_>>());
        let native = if preparing.is_ok() {
            core.native
                .shutdown_snapshot(&resources.iter().cloned().collect::<Vec<_>>())
                .ok()
        } else {
            None
        };
        let now = self.clock.now();
        let safe = self.clock.healthy()
            && native.as_ref().is_some_and(|n| {
                n.safe_to_drop
                    && n.no_pending_commands
                    && n.resources.iter().collect::<BTreeSet<_>>() == resources.iter().collect()
                    && now
                        .age_ns(&n.observed_at)
                        .is_some_and(|age| age.saturating_add(n.uncertainty_ns.0) <= 100_000_000)
            });
        Ok(StopSnapshot {
            host_boot: core.boot.clone(),
            admission_open: false,
            pending_operations: pending,
            retained_evidence: core.store.journal_head()?,
            native,
            safe_to_drop: safe,
            physical_shutdown_assessed: safe && core.native.environment() == Environment::Physical,
        })
    }
}
