use crate::{journal::*, model::*, native::*};
use rx_domain::{canonical, types::*};
use rx_ports::Repository;
use rx_storage::SqliteRepository;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};

pub struct Host<N, C, H = NoHooks> {
    core: Mutex<Core<N>>,
    clock: C,
    hooks: H,
    protection: Arc<dyn LocalProtection>,
    accepting: Arc<std::sync::atomic::AtomicBool>,
}
struct Core<N> {
    store: SqliteRepository,
    native: N,
    boot: Id,
    bindings: BTreeMap<Name, Binding>,
    caller: Option<Caller>,
    armed: BTreeSet<Name>,
    accepting: Arc<std::sync::atomic::AtomicBool>,
}
impl<N: NativeAdapter, C: Clock> Host<N, C, NoHooks> {
    pub fn open(
        path: impl AsRef<Path>,
        native: N,
        clock: C,
        bindings: Vec<Binding>,
    ) -> Result<Self> {
        Self::with_hooks(path, native, clock, bindings, NoHooks)
    }
}

mod configuration;
mod dispatch;
mod grants;
mod publication;
mod qualification;
mod receipts;
mod scopes;
mod service_stop;
mod snapshot;
pub use service_stop::StopSnapshot;
mod validation;

use receipts::persist_capture;
use validation::*;

impl<N: NativeAdapter, C: Clock, H: BoundaryHook> Host<N, C, H> {
    pub fn with_hooks(
        path: impl AsRef<Path>,
        native: N,
        clock: C,
        bindings: Vec<Binding>,
        hooks: H,
    ) -> Result<Self> {
        rx_solution_catalog::builtin_catalog().map_err(|e| HostError::Invalid(e.to_string()))?;
        let mut store = SqliteRepository::open(path)?;
        if bindings.is_empty() {
            return Err(HostError::Invalid("no approved bindings".into()));
        }
        let host = &bindings[0].host;
        let platform = &bindings[0].platform;
        for binding in &bindings {
            if &binding.host != host
                || &binding.platform != platform
                || binding.environment != native.environment()
                || binding.scope_ids.is_empty()
                || binding.condition_ids.is_empty()
                || binding.allowed_intents.is_empty()
                || binding.qualification_revision.0 == 0
                || binding.purposes.is_empty()
            {
                return Err(HostError::Invalid("invalid binding".into()));
            }
            if binding.scope_ids.iter().collect::<BTreeSet<_>>().len() != binding.scope_ids.len()
                || binding.condition_ids.iter().collect::<BTreeSet<_>>().len()
                    != binding.condition_ids.len()
            {
                return Err(HostError::Invalid(
                    "duplicate binding scope/condition".into(),
                ));
            }
            for intent in &binding.allowed_intents {
                intent
                    .normalized()
                    .map_err(|e| HostError::Invalid(e.to_string()))?;
            }
        }
        let len = bindings.len();
        let bindings: BTreeMap<_, _> = bindings.into_iter().map(|b| (b.cell.clone(), b)).collect();
        if bindings.len() != len {
            return Err(HostError::Invalid("duplicate cell binding".into()));
        }
        store.transact(|tx| {
            if tx.get(&name("host/meta"))?.is_none() {
                tx.put(
                    &name("host/meta"),
                    None,
                    &doc(
                        "rx.host.meta.v1",
                        &HostMeta {
                            delivery_journal: id(),
                            evidence_journal: id(),
                            delivery_seq: Counter(0),
                        },
                    )?,
                )?;
            }
            for binding in bindings.values() {
                let k = key("cell", &binding.cell);
                if tx.get(&k)?.is_none() {
                    tx.put(
                        &k,
                        None,
                        &doc(
                            "rx.host.cell.v1",
                            &CellState {
                                epoch: Counter(1),
                                scopes: binding
                                    .scope_ids
                                    .iter()
                                    .cloned()
                                    .map(|n| (n, Counter(1)))
                                    .collect(),
                                blocked: BTreeSet::new(),
                            },
                        )?,
                    )?;
                }
            }
            Ok(())
        })?;
        let protection = native.protection();
        let accepting = Arc::new(std::sync::atomic::AtomicBool::new(true));
        Ok(Self {
            core: Mutex::new(Core {
                store,
                native,
                boot: id(),
                bindings,
                caller: None,
                armed: BTreeSet::new(),
                accepting: accepting.clone(),
            }),
            clock,
            hooks,
            protection,
            accepting,
        })
    }
    fn lock(&self) -> Result<MutexGuard<'_, Core<N>>> {
        self.core
            .lock()
            .map_err(|_| HostError::NativeUnknown("command gate faulted".into()))
    }
    pub fn boot_id(&self) -> Result<Id> {
        Ok(self.lock()?.boot.clone())
    }
    pub fn bindings(&self) -> Result<Vec<Binding>> {
        Ok(self.lock()?.bindings.values().cloned().collect())
    }
    pub fn current_time(&self) -> TimePoint {
        self.clock.now()
    }
    pub fn claim_rpc(
        &self,
        caller: &Caller,
        method: &Name,
        request_key: &Id,
        semantic: Digest,
        effect: Option<&Name>,
    ) -> Result<()> {
        let mut core = self.lock()?;
        authorized(&core, caller)?;
        core.store
            .transact(|tx| {
                let mut keys = vec![key("rpc-key", (&caller.peer, method, request_key))];
                if let Some(effect) = effect {
                    keys.push(key("rpc-effect", (&caller.peer, method, effect)));
                }
                for key in keys {
                    if let Some(old) = tx.get(&key)? {
                        let old: Digest = decode(&old, "rx.host.rpc-identity.v1")?;
                        if old != semantic {
                            return Err(rx_ports::StoreError::KeyConflict);
                        }
                    } else {
                        tx.put(&key, None, &doc("rx.host.rpc-identity.v1", &semantic)?)?;
                    }
                }
                Ok(())
            })
            .map_err(Into::into)
    }
    pub fn stored_grant(&self, caller: &Caller, grant: &Id) -> Result<StoredGrant> {
        let mut core = self.lock()?;
        authorized(&core, caller)?;
        core.store
            .transact(|tx| {
                let record = tx
                    .get(&key("grant", grant))?
                    .ok_or(rx_ports::StoreError::Invalid("unknown grant".into()))?;
                decode(&record, "rx.host.grant.v1")
            })
            .map_err(Into::into)
    }
    pub fn inspect_cell(&self, caller: &Caller, cell: &Name) -> Result<Inspection> {
        let mut core = self.lock()?;
        authorized(&core, caller)?;
        let definition = core
            .bindings
            .get(cell)
            .ok_or(HostError::Forbidden)?
            .definition
            .sha256;
        let host_boot = core.boot.clone();
        core.store
            .transact(|tx| {
                let row = tx
                    .get(&key("cell", cell))?
                    .ok_or(rx_ports::StoreError::Invalid("unknown cell".into()))?;
                let state = decode(&row, "rx.host.cell.v1")?;
                let mut pending_operations = Vec::new();
                let mut pending_permits = Vec::new();
                for row in tx.scan("delivery/")? {
                    let r: DeliveryRecord = decode(&row, "rx.host.delivery.v1")?;
                    if &r.cell == cell
                        && matches!(
                            r.state,
                            ReceiptState::Prepared
                                | ReceiptState::SendEntered
                                | ReceiptState::NativeAccepted
                        )
                    {
                        pending_operations.push(r.operation);
                        pending_permits.push(r.permit);
                    }
                }
                Ok(Inspection {
                    state,
                    host_boot,
                    definition,
                    pending_operations,
                    pending_permits,
                })
            })
            .map_err(Into::into)
    }
    pub fn journals(&self) -> Result<HostMeta> {
        self.lock()?
            .store
            .transact(|tx| {
                let r = tx
                    .get(&name("host/meta"))?
                    .ok_or(rx_ports::StoreError::Integrity("missing host meta".into()))?;
                decode(&r, "rx.host.meta.v1")
            })
            .map_err(Into::into)
    }
    pub fn protection(&self) -> Arc<dyn LocalProtection> {
        self.protection.clone()
    }
    /// Authentication establishes a caller, never a grant or Arm state.
    pub fn bind_platform(&self, caller: Caller) -> Result<()> {
        let mut core = self.lock()?;
        if !core.bindings.values().all(|b| b.platform == caller.peer) {
            return Err(HostError::Forbidden);
        }
        if core
            .caller
            .as_ref()
            .is_some_and(|old| old.session != caller.session)
        {
            core.armed.clear();
        }
        core.caller = Some(caller);
        Ok(())
    }
}

fn authorized<N>(core: &Core<N>, caller: &Caller) -> Result<()> {
    if !core
        .caller
        .as_ref()
        .is_some_and(|c| c.peer == caller.peer && c.session == caller.session)
    {
        return Err(HostError::Forbidden);
    }
    Ok(())
}

fn invalid(e: impl std::fmt::Display) -> HostError {
    HostError::Invalid(e.to_string())
}

fn before(now: &TimePoint, until: &TimePoint) -> bool {
    now.clock_id == until.clock_id && now.ticks_ns < until.ticks_ns
}

fn require_admission<N>(core: &Core<N>) -> Result<()> {
    if !core.accepting.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(HostError::Guard);
    }
    Ok(())
}
