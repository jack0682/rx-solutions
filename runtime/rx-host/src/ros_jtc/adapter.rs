use super::{
    journal,
    protocol::{Reply, State, Transport},
    *,
};
use crate::{native::*, *};
use rx_domain::{canonical, intent::Intent, types::*};
use serde::Deserialize;
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
pub trait Boundary: Send {
    fn after_entry(&self) {}
    fn after_send(&self) {}
    fn after_capture(&self) {}
}
pub struct NoBoundary;
impl Boundary for NoBoundary {}
struct Protection {
    latched: AtomicBool,
    external: Arc<dyn LocalProtection>,
}
impl LocalProtection for Protection {
    fn react(&self, cause: ProtectionIncident) {
        self.latched.store(true, Ordering::SeqCst);
        self.external.react(cause);
    }
}
struct Core<T> {
    store: rx_storage::SqliteRepository,
    transport: T,
    closed: bool,
}
pub struct Jtc<T: Transport, C: Clock, A: Authority, H: Boundary = NoBoundary> {
    core: Mutex<Core<T>>,
    profile: Profile,
    clock: C,
    authority: A,
    protection: Arc<Protection>,
    hooks: H,
    dispatch_context: Option<NativeDispatch>,
}
fn invalid(e: impl std::fmt::Display) -> HostError {
    HostError::NativeUnknown(e.to_string())
}
fn decode<T: serde::de::DeserializeOwned>(value: &serde_json::Value) -> Result<T> {
    canonical::decode_json(&canonical::bytes(value).map_err(invalid)?).map_err(invalid)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Observation {
    controllers: Vec<serde_json::Value>,
    selected_matches: bool,
    observed_start_ns: Counter,
    observed_end_ns: Counter,
    controller_generation_known: bool,
    physical_readiness_proven: bool,
    send_service_ready: bool,
    result_service_ready: bool,
    cancel_service_ready: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SendFact {
    state: String,
    goal_id: Id,
    accepted: bool,
    entered_at_ns: Counter,
    captured_at_ns: Counter,
    stamp_sec: i32,
    stamp_nanosec: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultFact {
    goal_id: Id,
    known_to_bridge: bool,
    ros_goal_status: u8,
    controller_error_code: i32,
    controller_error_string: String,
    controller_generation_known: bool,
    query_started_at_ns: Counter,
    captured_at_ns: Counter,
}
pub(super) fn validate_record(e: &Entry) -> Result<()> {
    if e.disputed && e.capture.is_some() {
        return Err(invalid("disputed record has a capture"));
    }
    if let Some(capture) = &e.capture {
        let (reply, time, schema, status) = if let Some(reply) = &e.result {
            if !matches!(reply.state, State::ResultCaptured) {
                return Err(invalid("result capture state"));
            }
            let fact: ResultFact = decode(&reply.value)?;
            if fact.goal_id != e.invocation
                || fact.query_started_at_ns > fact.captured_at_ns
                || fact.controller_generation_known
                || fact.controller_error_string.len() > 4096
                || !matches!(fact.ros_goal_status, 4..=6)
                || (fact.ros_goal_status == 4 && fact.controller_error_code != 0)
            {
                return Err(invalid("stored ROS result inconsistent"));
            }
            let schema = match fact.ros_goal_status {
                4 => "rx.ros-jtc.succeeded.v1",
                5 => "rx.ros-jtc.canceled.v1",
                _ => "rx.ros-jtc.aborted.v1",
            };
            (
                reply,
                fact.captured_at_ns,
                schema,
                i64::from(fact.controller_error_code),
            )
        } else {
            let reply = e
                .send
                .as_ref()
                .ok_or_else(|| invalid("rejection source absent"))?;
            let fact: SendFact = decode(&reply.value)?;
            if !matches!(reply.state, State::SendRecorded)
                || fact.accepted
                || fact.state != "REJECTED"
                || fact.goal_id != e.invocation
                || fact.entered_at_ns > fact.captured_at_ns
            {
                return Err(invalid("stored rejection inconsistent"));
            }
            (reply, fact.captured_at_ns, "rx.ros-jtc.goal-rejected.v1", 0)
        };
        if reply.schema.as_str() != "rx.ros-jtc-reply.v1"
            || reply.fault.is_some()
            || time > reply.ticks_ns
            || capture.captured_at.clock_id != reply.clock_id
            || capture.captured_at.ticks_ns != time
            || capture.status_schema.as_str() != schema
            || capture.status != status
            || capture.device_session != e.controller_session
            || capture.native_id.as_deref() != Some(e.invocation.as_str())
            || (capture.captured_at.clock_id == e.entered_at.clock_id
                && capture.captured_at.ticks_ns < e.entered_at.ticks_ns)
        {
            return Err(invalid("capture differs from stored native fact"));
        }
    }
    Ok(())
}
impl<T: Transport, C: Clock, A: Authority> Jtc<T, C, A> {
    pub fn initialize(directory: &Path, profile: &Profile) -> Result<Identity> {
        journal::initialize(directory, profile)
    }
    pub fn open(
        directory: &Path,
        identity: &Identity,
        profile: Profile,
        transport: T,
        clock: C,
        authority: A,
    ) -> Result<Self> {
        Self::with_boundary(
            directory, identity, profile, transport, clock, authority, NoBoundary,
        )
    }
}
impl<T: Transport, C: Clock, A: Authority, H: Boundary> Jtc<T, C, A, H> {
    pub fn with_boundary(
        directory: &Path,
        identity: &Identity,
        profile: Profile,
        transport: T,
        clock: C,
        authority: A,
        hooks: H,
    ) -> Result<Self> {
        let store = journal::open(directory, identity, &profile)?;
        let protection = Arc::new(Protection {
            latched: AtomicBool::new(false),
            external: authority.protection(),
        });
        Ok(Self {
            core: Mutex::new(Core {
                store,
                transport,
                closed: false,
            }),
            profile,
            clock,
            authority,
            protection,
            hooks,
            dispatch_context: None,
        })
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Core<T>>> {
        self.core.lock().map_err(|_| {
            self.protection.react(ProtectionIncident::StoreFault);
            HostError::Guard
        })
    }
    fn stored<R>(&self, value: Result<R>) -> Result<R> {
        value.inspect_err(|_| self.protection.react(ProtectionIncident::StoreFault))
    }
    fn snapshot(&self) -> Result<AuthoritySnapshot> {
        let s = self.authority.snapshot()?;
        s.validate(&self.profile, &self.clock)?;
        Ok(s)
    }
    fn ready(&self, s: &AuthoritySnapshot) -> bool {
        let now = self.clock.now();
        self.dispatch_context.as_ref().is_none_or(|c| {
            c.device_session == s.controller_session
                && c.expires_at.clock_id == now.clock_id
                && now.ticks_ns < c.expires_at.ticks_ns
        }) && s.exclusive_control
            && s.no_external_goals
            && s.control_available
            && self.profile.conditions.is_subset(&s.conditions)
    }
    fn until(&self) -> Result<TimePoint> {
        let now = self.clock.now();
        if !self.clock.healthy() {
            return Err(HostError::Guard);
        }
        let mut until = TimePoint {
            clock_id: now.clock_id.clone(),
            ticks_ns: Counter(
                now.ticks_ns
                    .0
                    .checked_add(900_000_000)
                    .ok_or(HostError::Guard)?,
            ),
        };
        if let Some(context) = &self.dispatch_context {
            if context.expires_at.clock_id != now.clock_id
                || context.expires_at.ticks_ns <= now.ticks_ns
            {
                return Err(HostError::Stale);
            }
            until.ticks_ns = until.ticks_ns.min(context.expires_at.ticks_ns);
        }
        Ok(until)
    }
    fn observe_bridge(&self, core: &mut Core<T>) -> Result<()> {
        let reply = core
            .transport
            .exchange("inspect", serde_json::json!({}), &self.until()?)
            .inspect_err(|_| self.protection.react(ProtectionIncident::OwnerLost))?;
        if !matches!(reply.state, State::Observed) {
            return Err(HostError::Guard);
        }
        let v: Observation = decode(&reply.value)?;
        let now = self.clock.now();
        if v.controllers.len() > 128
            || !v.selected_matches
            || !v.send_service_ready
            || !v.result_service_ready
            || !v.cancel_service_ready
            || v.controller_generation_known
            || v.physical_readiness_proven
            || reply.clock_id != now.clock_id
            || v.observed_end_ns > now.ticks_ns
            || v.observed_start_ns > v.observed_end_ns
            || now.ticks_ns.0 - v.observed_start_ns.0 > 100_000_000
        {
            return Err(HostError::Guard);
        }
        Ok(())
    }
    pub fn entry(&self, operation: &Id) -> Result<Option<Entry>> {
        self.stored(journal::read(&mut self.lock()?.store, operation))
    }
    fn capture(
        &self,
        e: &Entry,
        schema: &str,
        status: i64,
        time: Counter,
        reply: &Reply,
    ) -> Result<NativeCapture> {
        if time > reply.ticks_ns || reply.clock_id != self.clock.now().clock_id {
            return Err(HostError::Guard);
        }
        if reply.clock_id == e.entered_at.clock_id && time < e.entered_at.ticks_ns {
            return Err(invalid("capture predates native entry"));
        }
        Ok(NativeCapture {
            status_schema: crate::journal::name(schema),
            status,
            native_id: Some(e.invocation.to_string()),
            captured_at: TimePoint {
                clock_id: reply.clock_id.clone(),
                ticks_ns: time,
            },
            device_session: e.controller_session.clone(),
        })
    }
}
impl<T: Transport, C: Clock, A: Authority, H: Boundary> NativeAdapter for Jtc<T, C, A, H> {
    fn submit_with_context(
        &mut self,
        op: &Id,
        inv: &Id,
        intent: &Intent,
        context: &NativeDispatch,
    ) -> Result<NativeCapture> {
        self.dispatch_context = Some(context.clone());
        let result = self.submit(op, inv, intent);
        self.dispatch_context = None;
        result
    }
    fn environment(&self) -> Environment {
        self.profile.environment
    }
    fn protection(&self) -> Arc<dyn LocalProtection> {
        self.protection.clone()
    }
    fn guard(&self, intent: &Intent, now: &TimePoint) -> Result<Guard> {
        self.profile.trajectory(intent)?;
        if self.protection.latched.load(Ordering::SeqCst) {
            return Err(HostError::Guard);
        }
        let mut core = self.lock()?;
        if core.closed || self.stored(journal::pending(&mut core.store))?.is_some() {
            return Err(HostError::Busy);
        }
        let before = self.snapshot()?;
        if !self.ready(&before) {
            return Err(HostError::Guard);
        }
        self.observe_bridge(&mut core)?;
        let after = self.snapshot()?;
        if before.controller_session != after.controller_session
            || !self.ready(&after)
            || now.clock_id != after.observed_at.clock_id
            || self.protection.latched.load(Ordering::SeqCst)
        {
            return Err(HostError::Guard);
        }
        Ok(Guard {
            device_session: after.controller_session,
            valid_until: TimePoint {
                clock_id: after.observed_at.clock_id,
                ticks_ns: Counter(
                    after
                        .observed_at
                        .ticks_ns
                        .0
                        .checked_add(self.profile.authority_max_age_ms.0 * 1_000_000)
                        .ok_or(HostError::Guard)?,
                ),
            },
            satisfied: after.conditions,
        })
    }
    fn can_handover(&self, r: &[Name]) -> bool {
        self.handover_snapshot(r)
            .is_ok_and(|s| s.no_pending_commands && s.control_available && s.support_stable)
    }
    fn handover_snapshot(&self, r: &[Name]) -> Result<LocalHandover> {
        if !self.profile.resources_match(r) {
            return Err(HostError::Guard);
        }
        let mut core = self.lock()?;
        let pending = self.stored(journal::pending(&mut core.store))?.is_some();
        let s = self.snapshot()?;
        Ok(LocalHandover {
            device_session: s.controller_session,
            observed_at: s.observed_at,
            uncertainty_ns: s.uncertainty_ns,
            no_pending_commands: !pending && s.no_external_goals,
            control_available: s.exclusive_control && s.control_available,
            support_stable: s.support_stable,
        })
    }
    fn prepare_shutdown(&mut self, r: &[Name]) -> Result<()> {
        if !self.profile.resources_match(r) {
            return Err(HostError::Guard);
        }
        let mut core = self.lock()?;
        let s = self.snapshot()?;
        if self.stored(journal::pending(&mut core.store))?.is_some()
            || !s.no_external_goals
            || !s.support_stable
            || !s.client_drop_allowed
        {
            return Err(HostError::Busy);
        }
        self.protection.latched.store(true, Ordering::SeqCst);
        core.closed = core.transport.try_close()?;
        if core.closed {
            Ok(())
        } else {
            Err(HostError::Busy)
        }
    }
    fn shutdown_snapshot(&self, r: &[Name]) -> Result<NativeShutdown> {
        if !self.profile.resources_match(r) {
            return Err(HostError::Guard);
        }
        let mut core = self.lock()?;
        let pending = self.stored(journal::pending(&mut core.store))?.is_some();
        let s = self.snapshot()?;
        Ok(NativeShutdown {
            device_session: s.controller_session,
            observed_at: s.observed_at,
            uncertainty_ns: s.uncertainty_ns,
            resources: r.to_vec(),
            no_pending_commands: !pending && s.no_external_goals,
            safe_to_drop: core.closed
                && !pending
                && s.no_external_goals
                && s.support_stable
                && s.client_drop_allowed,
        })
    }
    fn submit(
        &mut self,
        operation: &Id,
        invocation: &Id,
        intent: &Intent,
    ) -> Result<NativeCapture> {
        let trajectory = self.profile.trajectory(intent)?;
        let digest = intent.digest().map_err(invalid)?;
        let mut core = self.lock()?;
        if let Some(e) = self.stored(journal::read(&mut core.store, operation))? {
            if e.invocation != *invocation || e.intent_digest != digest {
                return Err(HostError::Conflict);
            }
            return e
                .capture
                .ok_or_else(|| invalid("native entry already exists; never resend"));
        }
        if self.protection.latched.load(Ordering::SeqCst) || core.closed {
            return Err(HostError::Guard);
        }
        if self.stored(journal::pending(&mut core.store))?.is_some() {
            return Err(HostError::Busy);
        }
        let authority = self.snapshot()?;
        if !self.ready(&authority) {
            return Err(HostError::Guard);
        }
        self.observe_bridge(&mut core)?;
        let mut entry = Entry {
            operation: operation.clone(),
            invocation: invocation.clone(),
            intent: intent.clone(),
            intent_digest: digest,
            profile_digest: intent.profile_digest,
            controller_session: authority.controller_session,
            bridge_instance: core.transport.instance().clone(),
            entered_at: self.clock.now(),
            goal_digest: trajectory.reference.sha256,
            send: None,
            result: None,
            capture: None,
            issue: None,
            disputed: false,
        };
        self.stored(journal::enter(&mut core.store, &entry))?;
        self.hooks.after_entry();
        let current = self.snapshot()?;
        if current.controller_session != entry.controller_session
            || !self.ready(&current)
            || self.protection.latched.load(Ordering::SeqCst)
        {
            return Err(HostError::Guard);
        }
        let reply=core.transport.exchange("send",serde_json::json!({"operation":operation,"invocation":invocation,"goal":trajectory.goal}),&self.until()?);
        self.hooks.after_send();
        match reply {
            Ok(reply) => {
                let validation = (|| -> Result<()> {
                    if matches!(reply.state, State::SendRecorded) {
                        let fact: SendFact = decode(&reply.value)?;
                        let _ = fact.stamp_sec;
                        if fact.goal_id != *invocation
                            || fact.entered_at_ns > fact.captured_at_ns
                            || fact.stamp_nanosec >= 1_000_000_000
                            || fact.state
                                != if fact.accepted {
                                    "ACCEPTED"
                                } else {
                                    "REJECTED"
                                }
                        {
                            return Err(invalid("native send fact differs"));
                        }
                        if !fact.accepted {
                            entry.capture = Some(self.capture(
                                &entry,
                                "rx.ros-jtc.goal-rejected.v1",
                                0,
                                fact.captured_at_ns,
                                &reply,
                            )?);
                        }
                    } else {
                        entry.issue = Some(format!("native send {:?}", reply.state));
                        self.protection.react(ProtectionIncident::OwnerLost);
                    }
                    Ok(())
                })();
                if let Err(error) = validation {
                    entry.issue = Some(error.to_string());
                    entry.disputed = true;
                    self.protection.react(ProtectionIncident::OwnerLost);
                    entry.send = Some(reply);
                    self.stored(journal::update(&mut core.store, &entry))?;
                    return Err(error);
                }
                entry.send = Some(reply);
                self.stored(journal::update(&mut core.store, &entry))?;
            }
            Err(e) => {
                entry.issue = Some(e.to_string());
                self.protection.react(ProtectionIncident::OwnerLost);
                self.stored(journal::update(&mut core.store, &entry))?;
                return Err(e);
            }
        }
        if let Some(c) = entry.capture {
            self.hooks.after_capture();
            Ok(c)
        } else {
            Err(invalid(
                "ROS goal accepted or unresolved; completion requires lookup",
            ))
        }
    }
    fn lookup(&mut self, operation: &Id, invocation: &Id) -> Result<Option<NativeCapture>> {
        let mut core = self.lock()?;
        let Some(mut entry) = self.stored(journal::read(&mut core.store, operation))? else {
            return Ok(None);
        };
        if entry.invocation != *invocation {
            return Err(HostError::Conflict);
        }
        if entry.capture.is_some() {
            return Ok(entry.capture);
        }
        if entry.disputed {
            return Err(invalid(
                "stored native result dispute requires explicit recovery",
            ));
        }
        let before = self.snapshot()?;
        if before.controller_session != entry.controller_session {
            return Ok(None);
        }
        let reply = core
            .transport
            .exchange(
                "result",
                serde_json::json!({"invocation":invocation}),
                &self.until()?,
            )
            .inspect_err(|_| self.protection.react(ProtectionIncident::OwnerLost))?;
        if !matches!(reply.state, State::ResultCaptured) {
            return Ok(None);
        }
        let fact: ResultFact = decode(&reply.value)?;
        let _ = fact.known_to_bridge;
        if fact.goal_id != *invocation
            || fact.controller_generation_known
            || fact.controller_error_string.len() > 4096
            || fact.query_started_at_ns > fact.captured_at_ns
            || !matches!(fact.ros_goal_status, 4..=6)
        {
            return Err(invalid("native result shape differs"));
        }
        let after = self.snapshot()?;
        if after.controller_session != entry.controller_session {
            return Ok(None);
        }
        if fact.ros_goal_status == 4 && fact.controller_error_code != 0 {
            entry.result = Some(reply);
            entry.issue = Some("contradictory ROS success and controller error".into());
            entry.disputed = true;
            self.stored(journal::update(&mut core.store, &entry))?;
            self.protection.react(ProtectionIncident::OwnerLost);
            return Err(invalid("native result disputed"));
        }
        let schema = match fact.ros_goal_status {
            4 => "rx.ros-jtc.succeeded.v1",
            5 => "rx.ros-jtc.canceled.v1",
            _ => "rx.ros-jtc.aborted.v1",
        };
        let capture = self.capture(
            &entry,
            schema,
            i64::from(fact.controller_error_code),
            fact.captured_at_ns,
            &reply,
        )?;
        entry.result = Some(reply);
        entry.capture = Some(capture.clone());
        self.stored(journal::update(&mut core.store, &entry))?;
        self.hooks.after_capture();
        Ok(Some(capture))
    }
}
impl<T: Transport, C: Clock, A: Authority, H: Boundary> Drop for Jtc<T, C, A, H> {
    fn drop(&mut self) {
        if self.core.get_mut().map_or(true, |c| !c.closed) {
            self.protection.react(ProtectionIncident::OwnerLost);
        }
    }
}
