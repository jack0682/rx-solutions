//! Generic external process provider on the existing Host gate and evidence lifecycle.
mod owned;
pub mod profile;
pub mod wire;
use crate::{native::*, *};
use owned::{Call, Finished, Protection};
use profile::{Profile, Program, invalid};
use rx_domain::{canonical, host_snapshot::SourceObservation, intent::Intent, types::*};
use rx_process_contract::execution_v2::host_inputs::{BoundInput, NativeEntry};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::Duration,
};

struct Pending {
    request: wire::Request,
    receiver: Option<mpsc::Receiver<Finished>>,
    result: Option<Finished>,
    recovered: Option<NativeCapture>,
}
pub struct External<C: Clock> {
    profile: Profile,
    program: Program,
    root: PathBuf,
    session: Id,
    clock: C,
    pending: BTreeMap<Id, Pending>,
    cold_uncertain: bool,
    protection: Arc<Protection>,
    _owner: fs::File,
}
pub fn initialize(root: &Path) -> Result<Id> {
    fs::create_dir(root).map_err(invalid)?;
    let session = crate::journal::id();
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join("device-session"))
        .map_err(invalid)?;
    file.write_all(session.as_str().as_bytes())
        .map_err(invalid)?;
    file.sync_all().map_err(invalid)?;
    fs::File::open(root)
        .and_then(|f| f.sync_all())
        .map_err(invalid)?;
    Ok(session)
}
impl<C: Clock> External<C> {
    pub fn device_session(&self) -> &Id {
        &self.session
    }
    pub fn open(profile: Profile, program: Program, root: PathBuf, clock: C) -> Result<Self> {
        profile.validate(&program)?;
        program.validate(true)?;
        let owner = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("owner.lock"))
            .map_err(invalid)?;
        owner.try_lock().map_err(|_| HostError::Busy)?;
        let session = Id::new(fs::read_to_string(root.join("device-session")).map_err(invalid)?)
            .map_err(invalid)?;
        // Reopening native facts does not restore a previous Host's live process custody.
        let cold_uncertain = fs::read_dir(&root)
            .map_err(invalid)?
            .filter_map(|e| e.ok())
            .any(|e| e.path().is_dir());
        Ok(Self {
            profile,
            program,
            root,
            session,
            clock,
            pending: BTreeMap::new(),
            cold_uncertain,
            protection: Arc::new(Protection::default()),
            _owner: owner,
        })
    }
    fn request(&self) -> Result<wire::Request> {
        Ok(wire::Request::new(
            self.profile.digest()?,
            self.session.clone(),
            self.clock.now(),
        ))
    }
    fn passive(&self, mode: &str, request: &wire::Request) -> Result<serde_json::Value> {
        let bytes = canonical::bytes(request).map_err(invalid)?;
        let finished = Call::start(
            &self.program,
            &self.root,
            mode,
            &bytes,
            Duration::from_millis(500),
            &self.protection,
        )?
        .finish();
        if !finished.custody_clear {
            return Err(HostError::NativeUnknown(
                "external passive process custody unresolved".into(),
            ));
        }
        finished.value.map_err(HostError::NativeUnknown)
    }
    fn snapshot(&self, request: &wire::Request, raw: serde_json::Value) -> Result<wire::Snapshot> {
        let snapshot: wire::Snapshot = serde_json::from_value(raw).map_err(invalid)?;
        let now = self.clock.now();
        if snapshot.schema.as_str() != "rx.external-native-snapshot.v1"
            || snapshot.challenge != request.challenge
            || snapshot.profile_digest != request.profile_digest
            || snapshot.device_session != self.session
            || snapshot.observed_at.clock_id != now.clock_id
            || snapshot.observed_at.ticks_ns > now.ticks_ns
            || now.ticks_ns.0 - snapshot.observed_at.ticks_ns.0 > 500_000_000
            || snapshot.uncertainty_ns.0 > 500_000_000
            || snapshot.samples.keys().collect::<BTreeSet<_>>()
                != request.sources.iter().collect::<BTreeSet<_>>()
        {
            return Err(HostError::Guard);
        }
        for (source, sample) in &snapshot.samples {
            let declaration = self
                .profile
                .observations
                .get(source)
                .ok_or(HostError::Guard)?;
            if !declaration.value_type.matches(&sample.value)
                || !sample.quality_good
                || !sample.origin_age_bounded
                || sample.acquired_at.clock_id != now.clock_id
                || sample.acquired_at.ticks_ns > now.ticks_ns
                || sample.uncertainty_ns > declaration.maximum_uncertainty_ns
                || now.ticks_ns.0 - sample.acquired_at.ticks_ns.0 > declaration.maximum_age_ns.0
            {
                return Err(HostError::Guard);
            }
        }
        Ok(snapshot)
    }
    fn current(&self, sources: Vec<Name>) -> Result<wire::Snapshot> {
        if sources.len() > 64
            || sources
                .iter()
                .any(|s| !self.profile.observations.contains_key(s))
        {
            return Err(HostError::Guard);
        }
        let mut request = self.request()?;
        request.sources = sources;
        self.snapshot(&request, self.passive("observe", &request)?)
    }
    fn capture(
        &self,
        request: &wire::Request,
        value: serde_json::Value,
    ) -> Result<Option<NativeCapture>> {
        let response: wire::Completion = serde_json::from_value(value).map_err(invalid)?;
        let dispatch = request.dispatch.as_ref().ok_or(HostError::Guard)?;
        if response.schema.as_str() != "rx.external-native-completion.v1"
            || response.challenge != request.challenge
            || response.dispatch_digest
                != rx_package::content_digest(&canonical::bytes(dispatch).map_err(invalid)?)
            || response.operation != dispatch.operation
            || response.invocation != dispatch.invocation
            || response.intent_digest != dispatch.intent.digest().map_err(invalid)?
            || response.profile_digest != request.profile_digest
            || response.device_session != self.session
        {
            return Err(HostError::Conflict);
        }
        let current = self.snapshot(
            request,
            serde_json::to_value(response.current).map_err(invalid)?,
        )?;
        if !current.no_pending_commands || !current.control_available || !current.support_stable {
            return Err(HostError::Guard);
        }
        if let Some(capture) = &response.capture {
            let now = self.clock.now();
            if capture.device_session != self.session
                || capture.native_id.as_deref() != Some(dispatch.invocation.as_str())
                || capture.captured_at.clock_id != now.clock_id
                || capture.captured_at.ticks_ns < dispatch.admitted_at.ticks_ns
                || capture.captured_at.ticks_ns > now.ticks_ns
            {
                return Err(HostError::Conflict);
            }
        }
        Ok(response.capture)
    }
}
impl<C: Clock> NativeAdapter for External<C> {
    fn environment(&self) -> Environment {
        Environment::Simulation
    }
    fn protection(&self) -> Arc<dyn LocalProtection> {
        self.protection.clone()
    }
    fn guard(&self, _: &Intent, _: &TimePoint) -> Result<Guard> {
        Err(HostError::Guard)
    }
    fn guard_with_input(
        &self,
        intent: &Intent,
        now: &TimePoint,
        input: Option<&BoundInput>,
    ) -> Result<Guard> {
        self.profile.input(intent, input.ok_or(HostError::Guard)?)?;
        if self.cold_uncertain || !self.pending.is_empty() {
            return Err(HostError::Guard);
        }
        let sources = self
            .profile
            .conditions
            .values()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let current = self.current(sources)?;
        if !current.no_pending_commands || !current.control_available {
            return Err(HostError::Guard);
        }
        let satisfied = self
            .profile
            .conditions
            .iter()
            .filter_map(|(condition, source)| {
                matches!(
                    current.samples.get(source).map(|s| &s.value),
                    Some(TypedValue::Boolean(true))
                )
                .then_some(condition.clone())
            })
            .collect();
        Ok(Guard {
            device_session: self.session.clone(),
            valid_until: TimePoint {
                clock_id: now.clock_id.clone(),
                ticks_ns: Counter(now.ticks_ns.0.saturating_add(500_000_000)),
            },
            satisfied,
        })
    }
    fn observe_sources(&self, _: &Name, sources: &[Name]) -> Result<Vec<SourceObservation>> {
        let current = self.current(sources.to_vec())?;
        Ok(current
            .samples
            .into_iter()
            .map(|(source, sample)| {
                let declaration = &self.profile.observations[&source];
                SourceObservation {
                    source,
                    generation: self.session.clone(),
                    schema: declaration.schema.clone(),
                    unit: declaration.unit.clone(),
                    value: sample.value,
                    acquired_at: sample.acquired_at,
                    uncertainty_ns: sample.uncertainty_ns,
                    quality_good: sample.quality_good,
                    origin_age_bounded: sample.origin_age_bounded,
                    disputed: false,
                    evidence_id: crate::journal::id(),
                }
            })
            .collect())
    }
    fn submit(&mut self, _: &Id, _: &Id, _: &Intent) -> Result<NativeCapture> {
        Err(HostError::Guard)
    }
    fn begin_with_context(
        &mut self,
        op: &Id,
        inv: &Id,
        intent: &Intent,
        context: &NativeDispatch,
    ) -> Result<NativeSubmission> {
        let input = context.execution.as_ref().ok_or(HostError::Guard)?;
        self.guard_with_input(intent, &self.clock.now(), Some(input))?;
        if input.binding.operation != *op || context.device_session != self.session {
            return Err(HostError::Conflict);
        }
        let mut request = self.request()?;
        request.dispatch = Some(wire::Dispatch {
            operation: op.clone(),
            invocation: inv.clone(),
            intent: intent.clone(),
            input: input.clone(),
            device_session: self.session.clone(),
            admitted_at: self.clock.now(),
            expires_at: context.expires_at.clone(),
        });
        let bytes = canonical::bytes(&request).map_err(invalid)?;
        self.pending.insert(
            op.clone(),
            Pending {
                request: request.clone(),
                receiver: None,
                result: None,
                recovered: None,
            },
        );
        let mut call = Call::start(
            &self.program,
            &self.root,
            "execute",
            &bytes,
            Duration::from_millis(intent.execution_timeout_ms.0),
            &self.protection,
        )?;
        let now = self.clock.now();
        if now.clock_id != context.expires_at.clock_id
            || now.ticks_ns >= context.expires_at.ticks_ns
        {
            return Err(HostError::Stale);
        }
        let entry = call.entry(Duration::from_nanos(
            context.expires_at.ticks_ns.0 - now.ticks_ns.0,
        ))?;
        let typed: wire::Entry = serde_json::from_value(entry.clone()).map_err(invalid)?;
        if typed.schema.as_str() != "rx.external-native-entry.v1"
            || typed.challenge != request.challenge
            || typed.request_sha256 != rx_package::content_digest(&bytes)
            || typed.operation != *op
            || typed.invocation != *inv
            || typed.intent_digest != intent.digest().map_err(invalid)?
            || typed.profile_digest != request.profile_digest
            || typed.device_session != self.session
        {
            return Err(HostError::Conflict);
        }
        let payload = canonical::bytes(&entry).map_err(invalid)?;
        let proof = NativeEntry {
            operation: op.clone(),
            invocation: inv.clone(),
            intent_digest: typed.intent_digest,
            device_session: self.session.clone(),
            profile_digest: request.profile_digest,
            evidence: ArtifactRef {
                schema_id: typed.schema,
                sha256: rx_package::content_digest(&payload),
                size_bytes: Counter(payload.len() as u64),
            },
            payload,
        };
        proof.validate().map_err(invalid)?;
        let confirmed = self.clock.now();
        if confirmed.clock_id != context.expires_at.clock_id
            || confirmed.ticks_ns >= context.expires_at.ticks_ns
        {
            return Err(HostError::Stale);
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("rx-external-completion".into())
            .spawn(move || {
                let _ = sender.send(call.finish());
            })
            .map_err(invalid)?;
        self.pending.insert(
            op.clone(),
            Pending {
                request,
                receiver: Some(receiver),
                result: None,
                recovered: None,
            },
        );
        Ok(NativeSubmission::Entered(proof))
    }
    fn completed(&mut self) -> Result<Vec<NativeCompletion>> {
        let mut ready = vec![];
        for (op, pending) in &mut self.pending {
            if pending.result.is_none()
                && let Some(receiver) = &pending.receiver
            {
                match receiver.try_recv() {
                    Ok(result) => pending.result = Some(result),
                    Err(mpsc::TryRecvError::Empty) => (),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        pending.result = Some(Finished {
                            value: Err("external waiter custody lost".into()),
                            custody_clear: false,
                        })
                    }
                }
            }
            if let Some(capture) = &pending.recovered {
                ready.push((
                    op.clone(),
                    pending.request.clone(),
                    Some(capture.clone()),
                    None,
                ));
            } else if let Some(Finished {
                value: Ok(value),
                custody_clear: true,
            }) = &pending.result
            {
                ready.push((
                    op.clone(),
                    pending.request.clone(),
                    None,
                    Some(value.clone()),
                ));
            }
        }
        let mut results = vec![];
        for (operation, request, recovered, value) in ready {
            let capture = if let Some(c) = recovered {
                Some(c)
            } else {
                match self.capture(&request, value.ok_or(HostError::Guard)?) {
                    Ok(capture) => capture,
                    Err(error) => {
                        if let Some(pending) = self.pending.get_mut(&operation) {
                            pending.result = Some(Finished {
                                value: Err(error.to_string()),
                                custody_clear: true,
                            });
                        }
                        continue;
                    }
                }
            };
            if let Some(capture) = capture {
                results.push(NativeCompletion {
                    operation,
                    invocation: request.dispatch.ok_or(HostError::Guard)?.invocation,
                    capture,
                });
            }
        }
        Ok(results)
    }
    fn acknowledge_completion(&mut self, op: &Id) {
        self.pending.remove(op);
    }
    fn lookup(&mut self, _: &Id, _: &Id) -> Result<Option<NativeCapture>> {
        Err(HostError::Guard)
    }
    fn lookup_with_input(
        &mut self,
        op: &Id,
        inv: &Id,
        intent: &Intent,
        input: Option<&BoundInput>,
    ) -> Result<Option<NativeCapture>> {
        let input = input.ok_or(HostError::Guard)?;
        self.profile.input(intent, input)?;
        if self.cold_uncertain {
            return Err(HostError::Guard);
        }
        let pending = self.pending.get(op).ok_or(HostError::Guard)?;
        let dispatch = pending.request.dispatch.clone().ok_or(HostError::Guard)?;
        if dispatch.invocation != *inv
            || dispatch.intent.digest().map_err(invalid)? != intent.digest().map_err(invalid)?
            || canonical::bytes(&dispatch.input).map_err(invalid)?
                != canonical::bytes(input).map_err(invalid)?
        {
            return Err(HostError::Conflict);
        }
        // A still-owned finite call remains pending; a lost/unknown owner cannot be reconstructed.
        if !pending.result.as_ref().is_some_and(|r| r.custody_clear) {
            return Ok(None);
        }
        let mut request = self.request()?;
        request.dispatch = Some(dispatch);
        let capture = self.capture(&request, self.passive("lookup", &request)?)?;
        if let Some(capture) = &capture {
            self.pending.get_mut(op).ok_or(HostError::Guard)?.recovered = Some(capture.clone());
        }
        // The existing Host writer commits this fact; completed/ack clears native uncertainty afterward.
        Ok(capture)
    }
    fn can_handover(&self, _: &[Name]) -> bool {
        !self.cold_uncertain && self.pending.is_empty()
    }
    fn handover_snapshot(&self, resources: &[Name]) -> Result<LocalHandover> {
        if !self.can_handover(resources) {
            return Err(HostError::Guard);
        }
        let current = self.current(vec![])?;
        Ok(LocalHandover {
            device_session: self.session.clone(),
            observed_at: current.observed_at,
            uncertainty_ns: current.uncertainty_ns,
            no_pending_commands: current.no_pending_commands,
            control_available: current.control_available,
            support_stable: current.support_stable,
        })
    }
    fn prepare_shutdown(&mut self, resources: &[Name]) -> Result<()> {
        if !self.can_handover(resources) {
            return Err(HostError::Guard);
        }
        Ok(())
    }
    fn shutdown_snapshot(&self, resources: &[Name]) -> Result<NativeShutdown> {
        if !self.can_handover(resources) {
            return Err(HostError::Guard);
        }
        let current = self.current(vec![])?;
        Ok(NativeShutdown {
            device_session: self.session.clone(),
            observed_at: current.observed_at,
            uncertainty_ns: current.uncertainty_ns,
            resources: resources.to_vec(),
            no_pending_commands: current.no_pending_commands,
            safe_to_drop: current.safe_to_drop
                && current.no_pending_commands
                && current.control_available
                && current.support_stable,
        })
    }
}
