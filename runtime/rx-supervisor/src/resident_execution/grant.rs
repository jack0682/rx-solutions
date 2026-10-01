use super::*;
use crate::model::{Effect, Launch, LifecycleAuthority, Plan, Process};
use rx_domain::component::Binding;
use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Clone, Copy, PartialEq, Eq)]
enum Use {
    Ready,
    Entered,
    Spent,
}
/// This capability cannot be cloned, serialized or reconstructed from stored grant JSON.
pub struct LiveGrant {
    pub(crate) lease: Arc<Lease>,
}
pub(crate) struct Lease {
    pub(crate) owner: Id,
    pub(crate) grant: data::Grant,
    pub(crate) intent: data::Intent,
    pub(crate) plan_digest: Digest,
    clock: Arc<dyn Clock>,
    launches: BTreeMap<Name, Digest>,
    uses: Mutex<BTreeMap<Id, Use>>,
    blocked: AtomicBool,
}
pub struct Authority(pub(crate) Arc<Lease>);
impl LiveGrant {
    pub(super) fn new(
        grant: data::Grant,
        prepared: &Prepared,
        clock: Arc<dyn Clock>,
    ) -> Result<Self> {
        let now = clock.now()?;
        if now.clock_id != grant.issued_at.clock_id
            || now.clock_id != grant.valid_until.clock_id
            || now.ticks_ns < grant.issued_at.ticks_ns
            || now.ticks_ns >= grant.valid_until.ticks_ns
            || grant
                .valid_until
                .ticks_ns
                .0
                .saturating_sub(grant.issued_at.ticks_ns.0)
                > 30_000_000_000
        {
            return Err(invalid("start grant clock/window is invalid or expired"));
        }
        let mut launches = BTreeMap::new();
        for process in &prepared.plan.processes {
            let program = &prepared.catalog.programs[&process.program];
            if program.effect == Effect::RequiresPlatformAuthority
                || program.execution_requirements.is_none()
            {
                return Err(invalid("start effect/requirements are unsupported"));
            }
            let node = &prepared.intent.nodes[&process.id];
            let launch = process.launch(program, node.instance.clone())?;
            launches.insert(
                process.id.clone(),
                canonical::digest("RX-RESIDENT-AUTHORIZED-LAUNCH-v1", &launch).map_err(invalid)?,
            );
        }
        let uses = prepared
            .intent
            .nodes
            .values()
            .map(|n| (n.instance.clone(), Use::Ready))
            .collect();
        Ok(Self {
            lease: Arc::new(Lease {
                owner: Id::new(uuid::Uuid::new_v4().to_string()).map_err(invalid)?,
                grant,
                intent: prepared.intent.clone(),
                plan_digest: prepared.offer.plan_digest,
                clock,
                launches,
                uses: Mutex::new(uses),
                blocked: AtomicBool::new(false),
            }),
        })
    }
    pub fn view(&self) -> &data::Grant {
        &self.lease.grant
    }
}
impl Lease {
    pub(crate) fn block(&self) {
        self.blocked.store(true, Ordering::SeqCst);
    }
    pub(crate) fn start_window(&self) -> bool {
        !self.blocked.load(Ordering::SeqCst)
            && self.clock.now().is_ok_and(|now| {
                now.clock_id == self.grant.valid_until.clock_id
                    && now.ticks_ns >= self.grant.issued_at.ticks_ns
                    && now.ticks_ns < self.grant.valid_until.ticks_ns
            })
    }
    pub(crate) fn binding(&self, selection: &Name) -> Result<Binding> {
        let n = self
            .intent
            .nodes
            .get(selection)
            .ok_or_else(|| invalid("unassigned selection"))?;
        Ok(Binding {
            registration: n.registration.id.clone(),
            registration_revision: n.revision,
            catalog: n.registration.declaration.catalog.clone(),
            run: self.intent.run.clone(),
            selection: selection.clone(),
            instance: n.instance.clone(),
        })
    }
    pub(crate) fn matches(&self, binding: &Binding) -> bool {
        self.binding(&binding.selection)
            .is_ok_and(|b| b == *binding)
    }
    pub(crate) fn enter(&self, binding: &Binding) -> Result<()> {
        if !self.start_window() || !self.matches(binding) {
            return Err(invalid("current start grant does not cover this binding"));
        }
        let mut uses = self.uses.lock().map_err(|_| invalid("start grant lock"))?;
        let state = uses
            .get_mut(&binding.instance)
            .ok_or_else(|| invalid("start instance absent"))?;
        if *state != Use::Ready {
            return Err(invalid("start grant instance already consumed"));
        }
        *state = Use::Entered;
        Ok(())
    }
    pub(crate) fn finish(&self, instance: &Id) {
        if let Ok(mut uses) = self.uses.lock()
            && let Some(v) = uses.get_mut(instance)
        {
            *v = Use::Spent;
        }
    }
    pub(crate) fn check_entered(&self, binding: &Binding) -> bool {
        self.start_window()
            && self.matches(binding)
            && self
                .uses
                .lock()
                .is_ok_and(|v| v.get(&binding.instance) == Some(&Use::Entered))
    }
    fn launch_matches(&self, plan: &Plan, process: &Process, launch: &Launch) -> bool {
        plan.id == self.intent.run
            && process.id == launch.selection
            && self.intent.nodes.get(&process.id).is_some_and(|n| {
                n.instance == launch.instance
                    && n.registration.declaration.catalog.program == process.program
            })
            && canonical::digest("RX-RESIDENT-AUTHORIZED-LAUNCH-v1", launch)
                .ok()
                .as_ref()
                == self.launches.get(&process.id)
    }
}
impl LifecycleAuthority for Authority {
    fn may_start(&self, plan: &Plan, process: &Process, launch: &Launch) -> bool {
        self.0.start_window()
            && self.0.launch_matches(plan, process, launch)
            && self
                .0
                .uses
                .lock()
                .is_ok_and(|v| v.get(&launch.instance).is_some_and(|s| *s != Use::Spent))
    }
    fn may_stop(&self, plan: &Plan, process: &Process, launch: &Launch) -> bool {
        self.0.launch_matches(plan, process, launch)
            && (launch.effect == Effect::NonActuating
                || (launch.effect == Effect::ProtocolGuardedService
                    && matches!(launch.ready, crate::model::ReadyProbe::GuardedStatus(_))))
    }
}
