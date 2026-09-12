//! Native predicate adapter. Product startup resolves a signed release-owned device reference.
pub mod authoring;
mod execution;
mod journal;
mod profile;
use crate::{model::*, native::*};
pub use journal::{Entry, Identity};
pub use profile::{PredicateMapping, Profile, StatusImage};
use rx_domain::{intent::Intent, types::*};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
};

/// This hook is supplied by compiled tests, never from configuration or wire input.
pub trait NativeBoundary: Send {
    fn after_entry_commit(&self) {}
    fn after_write_before_ack_commit(&self) {}
    fn after_capture_commit(&self) {}
}
pub struct NoBoundary;
impl NativeBoundary for NoBoundary {}
pub struct Protection {
    latched: AtomicBool,
}
impl LocalProtection for Protection {
    fn react(&self, _: ProtectionIncident) {
        self.latched.store(true, Ordering::SeqCst);
    }
}
impl Protection {
    pub fn is_latched(&self) -> bool {
        self.latched.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub plc_epoch: u64,
    pub sequence: u64,
    pub flags: u16,
    pub acquired_at: TimePoint,
    pub uncertainty_ns: Counter,
}
impl Snapshot {
    fn bit(&self, bit: u8) -> bool {
        self.flags & (1_u16 << bit) != 0
    }
}
struct Core {
    store: rx_storage::SqliteRepository,
    client: Option<rx_melsec_mc::Client>,
    epoch: Option<u64>,
    sequence: Option<u64>,
    last_flags: Option<u16>,
    sequence_seen: Option<TimePoint>,
    witnessed_publication: bool,
}
pub struct Melsec<C: Clock, H: NativeBoundary = NoBoundary> {
    profile: Profile,
    session: Id,
    core: Mutex<Core>,
    clock: C,
    protection: Arc<Protection>,
    hooks: H,
    dispatch_context: Option<NativeDispatch>,
}
impl<C: Clock> Melsec<C> {
    /// Creates only a new local journal; never connects to PLC.
    pub fn initialize(directory: &Path, profile: &Profile) -> Result<Identity> {
        journal::initialize(directory, profile)
    }
    /// Existing journal only. PLC connection is opened on the first read, not here.
    pub fn open(directory: &Path, identity: &Identity, profile: Profile, clock: C) -> Result<Self> {
        Self::with_boundary(directory, identity, profile, clock, NoBoundary)
    }
}
impl<C: Clock, H: NativeBoundary> Melsec<C, H> {
    pub fn with_boundary(
        directory: &Path,
        identity: &Identity,
        profile: Profile,
        clock: C,
        hooks: H,
    ) -> Result<Self> {
        let store = journal::open(directory, identity, &profile)?;
        Ok(Self {
            profile,
            session: crate::journal::id(),
            core: Mutex::new(Core {
                store,
                client: None,
                epoch: None,
                sequence: None,
                last_flags: None,
                sequence_seen: None,
                witnessed_publication: false,
            }),
            clock,
            protection: Arc::new(Protection {
                latched: AtomicBool::new(false),
            }),
            hooks,
            dispatch_context: None,
        })
    }
    fn lock(&self) -> Result<MutexGuard<'_, Core>> {
        self.core.lock().map_err(|_| {
            self.protection.react(ProtectionIncident::StoreFault);
            HostError::Guard
        })
    }
    fn fault(&self, error: impl std::fmt::Display) -> HostError {
        self.protection.react(ProtectionIncident::OwnerLost);
        HostError::NativeUnknown(error.to_string())
    }
    fn check_store<T>(&self, result: Result<T>) -> Result<T> {
        result.inspect_err(|_| self.protection.react(ProtectionIncident::StoreFault))
    }
    pub fn entry(&self, operation: &Id) -> Result<Option<Entry>> {
        self.check_store(journal::read(&mut self.lock()?.store, operation))
    }
    fn snapshot(&self, core: &mut Core) -> Result<Snapshot> {
        if !self.clock.healthy() {
            return Err(self.fault("native clock unavailable"));
        }
        let begin = self.clock.now();
        if core.client.is_none() {
            if self.protection.is_latched() {
                return Err(HostError::Guard);
            }
            core.client = Some(
                rx_melsec_mc::Client::connect(self.profile.transport.clone())
                    .map_err(|e| self.fault(e))?,
            );
        }
        let words = core
            .client
            .as_mut()
            .ok_or(HostError::Guard)?
            .read_d(self.profile.status.first_d, 9)
            .map_err(|e| self.fault(e))?;
        let end = self.clock.now();
        if !self.clock.healthy()
            || begin.clock_id != end.clock_id
            || end.ticks_ns < begin.ticks_ns
            || end.ticks_ns.0 - begin.ticks_ns.0
                > u64::from(self.profile.status.read_budget_ms) * 1_000_000
        {
            return Err(self.fault("native snapshot read budget/clock changed"));
        }
        let u64_words = |slice: &[u16]| {
            slice
                .iter()
                .enumerate()
                .fold(0_u64, |v, (i, w)| v | (u64::from(*w) << (i * 16)))
        };
        let epoch = u64_words(&words[..4]);
        let sequence = u64_words(&words[4..8]);
        let flags = words[8];
        if epoch == 0
            || sequence == 0
            || core.epoch.is_some_and(|v| v != epoch)
            || core.sequence.is_some_and(|v| v > sequence)
            || (core.sequence == Some(sequence) && core.last_flags.is_some_and(|f| f != flags))
        {
            return Err(self.fault("PLC generation/sequence/atomic publication changed"));
        }
        if core.sequence != Some(sequence) {
            core.witnessed_publication |= core.sequence.is_some();
            core.sequence_seen = Some(begin.clone());
        }
        let seen = core.sequence_seen.as_ref().ok_or(HostError::Guard)?;
        if seen.clock_id != end.clock_id
            || end.ticks_ns < seen.ticks_ns
            || end.ticks_ns.0 - seen.ticks_ns.0
                > u64::from(self.profile.status.source_max_age_ms) * 1_000_000
        {
            return Err(self.fault("PLC publication stopped advancing"));
        }
        core.epoch = Some(epoch);
        core.sequence = Some(sequence);
        core.last_flags = Some(flags);
        let snapshot = Snapshot {
            plc_epoch: epoch,
            sequence,
            flags,
            acquired_at: begin,
            uncertainty_ns: Counter(
                end.ticks_ns.0 - seen.ticks_ns.0
                    + u64::from(self.profile.status.source_max_age_ms) * 1_000_000,
            ),
        };
        if !core.witnessed_publication || !snapshot.bit(self.profile.status.valid_bit) {
            return Err(HostError::Guard);
        }
        Ok(snapshot)
    }
    fn capture(&self, snapshot: &Snapshot) -> NativeCapture {
        NativeCapture {
            status_schema: crate::journal::name("rx.melsec.predicate-satisfied.v1"),
            status: 0,
            native_id: Some(format!("{}:{}", snapshot.plc_epoch, snapshot.sequence)),
            captured_at: snapshot.acquired_at.clone(),
            device_session: self.session.clone(),
        }
    }
    fn ready(&self, snapshot: &Snapshot) -> bool {
        let now = self.clock.now();
        self.clock.healthy()
            && self.dispatch_context.as_ref().is_none_or(|c| {
                c.device_session == self.session
                    && c.expires_at.clock_id == now.clock_id
                    && now.ticks_ns < c.expires_at.ticks_ns
            })
            && now.clock_id == snapshot.acquired_at.clock_id
            && now.ticks_ns >= snapshot.acquired_at.ticks_ns
            && now.ticks_ns.0 - snapshot.acquired_at.ticks_ns.0
                < u64::from(self.profile.status.guard_validity_ms) * 1_000_000
            && snapshot.bit(self.profile.status.ready_bit)
            && snapshot.bit(self.profile.status.queue_empty_bit)
            && snapshot.bit(self.profile.status.control_bit)
            && self
                .profile
                .conditions
                .values()
                .all(|bit| snapshot.bit(*bit))
    }
}
