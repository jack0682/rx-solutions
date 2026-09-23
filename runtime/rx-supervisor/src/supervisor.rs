use crate::{Error, Result, model::*, process::Backend};
use rx_domain::{canonical, types::*};
use rx_ports::{Document, Repository, StoreError};
use rx_solution_catalog::DeviceCatalog;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
fn name(s: &str) -> Name {
    Name::new(s).expect("internal name")
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).expect("generated UUID")
}
const KEY: &str = "supervisor/state";
const SCHEMA: &str = "rx.supervisor-state.v1";
fn document(state: &State) -> rx_ports::Result<Document> {
    Ok(Document {
        schema: name(SCHEMA),
        value: serde_json::to_value(state).map_err(|e| StoreError::Invalid(e.to_string()))?,
    })
}
fn decode(record: &rx_ports::Record) -> rx_ports::Result<State> {
    if record.document.schema != name(SCHEMA) {
        return Err(StoreError::Integrity("supervisor schema differs".into()));
    }
    let state: State = canonical::decode_json(
        &canonical::bytes(&record.document.value)
            .map_err(|e| StoreError::Integrity(e.to_string()))?,
    )
    .map_err(|e| StoreError::Integrity(e.to_string()))?;
    for record in state.records.values() {
        if let Some(exit) = &record.guarded_exit {
            if record.phase != Phase::Exited
                || record.instance.is_none()
                || record.pid.is_none_or(|pid| pid == 0)
            {
                return Err(StoreError::Integrity(
                    "guarded exit without owned process identity".into(),
                ));
            }
            if let GuardedExit::Confirmed {
                sequence,
                observed_at,
                ..
            } = exit
                && (record.exit_code != Some(0)
                    || sequence.0 == 0
                    || observed_at.clock_id.is_empty())
            {
                return Err(StoreError::Integrity(
                    "guarded exit proof shape differs".into(),
                ));
            }
        }
    }
    Ok(state)
}
pub struct Supervisor<R, B, A> {
    repository: R,
    backend: B,
    authority: A,
    plan: Plan,
    programs: BTreeMap<Name, Program>,
    started: BTreeMap<Id, Instant>,
    stopping: BTreeMap<Id, Instant>,
    retry_after: BTreeMap<Name, Instant>,
    stop_latched: bool,
    observed_guarded_exits: BTreeMap<Id, GuardedExit>,
    execution_admission: BTreeMap<Name, (crate::execution::Request, crate::execution::Status)>,
}
impl<R: Repository, B: Backend, A: LifecycleAuthority> Supervisor<R, B, A> {
    pub fn open(
        mut repository: R,
        backend: B,
        authority: A,
        plan: Plan,
        programs: BTreeMap<Name, Program>,
        support: &DeviceCatalog,
    ) -> Result<Self> {
        let digest = plan.validate(&programs, support)?;
        let existing = repository.snapshot()?.1;
        if !existing.iter().any(|r| r.key == name(KEY))
            && (!existing.is_empty() || repository.journal_head()?.0 != 0)
        {
            return Err(Error::Invalid(
                "supervisor requires a dedicated empty store".into(),
            ));
        }

        repository.transact(|tx| {
            if let Some(old) = tx.get(&name(KEY))? {
                let mut state = decode(&old)?;
                if state.plan != plan.id || state.plan_digest != digest {
                    return Err(StoreError::KeyConflict);
                }
                for record in state.records.values_mut() {
                    if matches!(
                        record.phase,
                        Phase::SpawnEntered
                            | Phase::Starting
                            | Phase::ProcessReady
                            | Phase::Unready
                            | Phase::StopRequested
                    ) {
                        record.phase = Phase::Unknown;
                        record.error =
                            Some("supervisor ownership lost; no PID adoption or replay".into());
                    }
                }
                tx.put(&name(KEY), Some(old.revision), &document(&state)?)?;
            } else {
                if tx.control_head()?.0 != 0 {
                    return Err(StoreError::Invalid(
                        "supervisor requires a dedicated empty store".into(),
                    ));
                }
                let records = plan
                    .processes
                    .iter()
                    .map(|p| {
                        (
                            p.id.clone(),
                            Record {
                                id: p.id.clone(),
                                instance: None,
                                phase: Phase::Pending,
                                attempts: Counter(0),
                                pid: None,
                                exit_code: None,
                                error: None,
                                guarded_exit: None,
                            },
                        )
                    })
                    .collect();
                tx.put(
                    &name(KEY),
                    None,
                    &document(&State {
                        schema: name(SCHEMA),
                        plan: plan.id.clone(),
                        plan_digest: digest,
                        stop_requested: false,
                        records,
                    })?,
                )?;
            }
            Ok(())
        })?;
        Ok(Self {
            repository,
            backend,
            authority,
            plan,
            programs,
            started: BTreeMap::new(),
            stopping: BTreeMap::new(),
            retry_after: BTreeMap::new(),
            stop_latched: false,
            observed_guarded_exits: BTreeMap::new(),
            execution_admission: BTreeMap::new(),
        })
    }
    pub fn state(&mut self) -> Result<State> {
        Ok(self.repository.transact(|tx| {
            decode(
                &tx.get(&name(KEY))?
                    .ok_or_else(|| StoreError::Integrity("supervisor state missing".into()))?,
            )
        })?)
    }
    /// Read the current owner's requirement observations without admission,
    /// spawning, readiness probes or lifecycle-authority evaluation.
    pub fn execution_admission(&mut self) -> Result<BTreeMap<Name, crate::execution::Status>> {
        let state = self.state()?;
        Ok(self.execution_status(&state))
    }
    fn change(&mut self, change: impl FnOnce(&mut State)) -> Result<()> {
        self.repository.transact(|tx| {
            let old = tx
                .get(&name(KEY))?
                .ok_or_else(|| StoreError::Integrity("supervisor state missing".into()))?;
            let mut state = decode(&old)?;
            change(&mut state);
            tx.put(&name(KEY), Some(old.revision), &document(&state)?)?;
            tx.append(&id(), &document(&state)?)?;
            Ok(())
        })?;
        Ok(())
    }
    pub fn request_stop(&mut self) -> Result<()> {
        self.stop_latched = true;
        self.change(|s| s.stop_requested = true)
    }
    /// Explicit non-actuating reactivation only, after a durably completed shutdown.
    pub fn rearm_software(&mut self) -> Result<()> {
        let state = self.state()?;
        if !state.stop_requested
            || state
                .records
                .values()
                .any(|r| !matches!(r.phase, Phase::Exited | Phase::Skipped | Phase::StartFailed))
            || self
                .plan
                .processes
                .iter()
                .any(|p| self.programs[&p.program].effect != Effect::NonActuating)
        {
            return Err(Error::Reconciliation(
                "software reactivation prerequisites not satisfied".into(),
            ));
        }
        for record in state.records.values() {
            if let Some(instance) = &record.instance
                && self.backend.owns(instance)
            {
                self.backend.forget_exited(instance)?;
            }
        }
        self.started.clear();
        self.stopping.clear();
        self.change(|s| {
            s.stop_requested = false;
            for r in s.records.values_mut() {
                r.phase = Phase::Pending;
                r.instance = None;
                r.attempts = Counter(0);
                r.pid = None;
                r.exit_code = None;
                r.error = None;
            }
        })?;
        self.stop_latched = false;
        self.retry_after.clear();
        self.execution_admission.clear();
        Ok(())
    }
    fn execution_status(&self, state: &State) -> BTreeMap<Name, crate::execution::Status> {
        use crate::execution::{Application, Status};
        self.plan.processes.iter().map(|process| {
            let program = &self.programs[&process.program];
            let requested = program.execution_requirements.clone();
            let current_request = state.records[&process.id].instance.as_ref().zip(requested.as_ref())
                .map(|(instance, requirements)| crate::execution::Request::new(program.id.clone(),
                    process.id.clone(), instance.clone(), state.plan_digest, requirements.clone()));
            let value = self.execution_admission.get(&process.id)
                .filter(|(request,_)| current_request.as_ref()==Some(request))
                .map(|(_,status)| status.clone()).unwrap_or_else(|| {
                let application = if requested.is_none() {
                    Application::LegacyNotDeclared
                } else if state.records[&process.id].attempts.0 > 0
                    || state.records[&process.id].phase == Phase::Unknown {
                    Application::Unconfirmed { reason: "no current-owner application receipt; no resource recovery is inferred".into() }
                } else { Application::NotApplied };
                Status { requested, application, not_applied_reasons: vec![] }
            });
            (process.id.clone(), value)
        }).collect()
    }

    fn spawn_required(
        &mut self,
        process: &Process,
        launch: &Launch,
        request: &crate::execution::Request,
    ) -> std::result::Result<u32, crate::process::SpawnFailure> {
        use crate::{
            execution::{Application, Decision, Status, Unmet},
            process::SpawnFailure,
        };
        let authority = &self.authority;
        let plan = &self.plan;
        // An unknown declaration can never be promoted to an empty requirement by a backend.
        let unknown = request.unknown();
        let decision = if unknown.is_empty() {
            self.backend
                .spawn_with_requirements(launch, request, &mut || {
                    authority.may_start(plan, process, launch)
                })
        } else {
            Ok(Decision::Rejected { unmet: unknown })
        };
        let mut status = Status {
            requested: Some(request.requirements().clone()),
            application: Application::NotApplied,
            not_applied_reasons: vec![],
        };
        let outcome = match decision {
            Ok(Decision::Admitted { pid, receipt }) if pid != 0 && receipt.matches(request) => {
                status.application = Application::ReportedAtStart { receipt };
                Ok(pid)
            }
            Ok(Decision::Admitted { .. }) => Err(SpawnFailure::Uncertain(Error::Reconciliation(
                "execution admission receipt does not match the complete current launch".into(),
            ))),
            Ok(Decision::Rejected { unmet }) => {
                if unmet.is_empty()
                    || unmet.len() > request.requirements().0.len()
                    || unmet
                        .iter()
                        .map(|r| &r.requirement)
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        != unmet.len()
                    || unmet.iter().any(|r| {
                        request
                            .requirements()
                            .0
                            .get(&r.requirement)
                            .is_none_or(|r| matches!(r, crate::execution::Requirement::NotRequired))
                            || r.reason.trim().is_empty()
                            || r.reason.len() > 1024
                    })
                {
                    Err(SpawnFailure::Uncertain(Error::Reconciliation(
                        "backend rejection has no valid named requirement reason".into(),
                    )))
                } else {
                    let reason = unmet
                        .iter()
                        .map(|r| format!("{}: {}", r.requirement, r.reason))
                        .collect::<Vec<_>>()
                        .join("; ");
                    status.not_applied_reasons = unmet;
                    Err(SpawnFailure::NotStarted(Error::Invalid(format!(
                        "execution admission rejected: {reason}"
                    ))))
                }
            }
            Err(error) => Err(error),
        };
        match &outcome {
            Err(SpawnFailure::Uncertain(error)) => {
                status.application = Application::Unconfirmed {
                    reason: error.to_string(),
                }
            }
            Err(SpawnFailure::NotStarted(error)) if status.not_applied_reasons.is_empty() => {
                status.not_applied_reasons = request.reject(&error.to_string());
                if status.not_applied_reasons.is_empty() {
                    status.not_applied_reasons.push(Unmet {
                        requirement: name("execution/start"),
                        reason: error.to_string(),
                    });
                }
            }
            _ => {}
        }
        self.execution_admission
            .insert(process.id.clone(), (request.clone(), status));
        outcome
    }
    pub fn tick(&mut self) -> Result<Status> {
        let mut blocked = vec![];
        for process in self.plan.processes.clone() {
            let state = self.state()?;
            let record = state.records[&process.id].clone();
            let program = self.programs[&process.program].clone();
            if record.phase == Phase::Exited
                && let Some(instance) = &record.instance
            {
                if self.backend.owns(instance) {
                    self.backend.forget_exited(instance)?;
                }
                self.started.remove(instance);
                self.stopping.remove(instance);
                self.observed_guarded_exits.remove(instance);
            }

            if record.phase == Phase::Unknown {
                blocked.push(format!("{}: process ownership unknown", process.id));
                continue;
            }
            if let Some(instance) = &record.instance
                && matches!(
                    record.phase,
                    Phase::Starting | Phase::ProcessReady | Phase::Unready | Phase::StopRequested
                )
            {
                if !self.backend.owns(instance) {
                    self.change(|s| {
                        let r = s.records.get_mut(&process.id).unwrap();
                        r.phase = Phase::Unknown;
                        r.error = Some("live process handle unavailable".into());
                    })?;
                    continue;
                }
                if let Some(code) = self.backend.exited(instance)? {
                    let guarded = program.effect == Effect::ProtocolGuardedService;
                    let exit = if let Some(observed) = self.observed_guarded_exits.get(instance) {
                        Some(observed.clone())
                    } else if guarded {
                        let launch = process.launch(&program, instance.clone())?;
                        let observed = match self.backend.guarded_status(&launch) {
                            Ok(Some(observation)) if code == Some(0) => match observation.state {
                                GuardedState::Stopped {
                                    reconciliation_required,
                                } => GuardedExit::Confirmed {
                                    reconciliation_required,
                                    sequence: observation.sequence,
                                    observed_at: observation.observed_at,
                                    payload_digest: observation.payload_digest,
                                },
                                _ => GuardedExit::Unconfirmed {
                                    reason: "exited without terminal guarded status".into(),
                                },
                            },
                            Ok(_) => GuardedExit::Unconfirmed {
                                reason: "zero exit and current guarded stop report required".into(),
                            },
                            Err(error) => GuardedExit::Unconfirmed {
                                reason: error.to_string(),
                            },
                        };
                        // This exact report was read after observing this owned child's exit.
                        // Keep its original source time across a local commit failure; never refresh it.
                        self.observed_guarded_exits
                            .insert(instance.clone(), observed.clone());
                        Some(observed)
                    } else {
                        None
                    };
                    let unexpected_guarded_exit = guarded && !state.stop_requested;
                    if unexpected_guarded_exit {
                        // Restrict the remaining RX daemons; never reactivate a failed owner.
                        self.stop_latched = true;
                    }
                    self.change(|s| {
                        let r = s.records.get_mut(&process.id).unwrap();
                        r.phase = Phase::Exited;
                        r.exit_code = code;
                        r.guarded_exit = exit;
                        if !s.stop_requested {
                            r.error = Some("unexpected service exit".into());
                        }
                        if unexpected_guarded_exit {
                            s.stop_requested = true;
                        }
                    })?;
                    self.backend.forget_exited(instance)?;
                    self.started.remove(instance);
                    self.stopping.remove(instance);
                    self.observed_guarded_exits.remove(instance);
                    self.retry_after.insert(
                        process.id.clone(),
                        Instant::now() + Duration::from_millis(process.restart_backoff_ms.0),
                    );
                    continue;
                }
            }
            if matches!(record.phase, Phase::Starting | Phase::ProcessReady)
                && process
                    .depends_on
                    .iter()
                    .any(|d| state.records[d].phase != Phase::ProcessReady)
            {
                blocked.push(format!(
                    "{}: dependency lost; readiness is not control authority",
                    process.id
                ));
            }
            if state.stop_requested || self.stop_latched {
                if matches!(record.phase, Phase::Pending | Phase::Prepared) {
                    self.change(|s| {
                        s.records.get_mut(&process.id).unwrap().phase = Phase::Skipped
                    })?;
                    continue;
                }
                if matches!(
                    record.phase,
                    Phase::Exited | Phase::StartFailed | Phase::Skipped
                ) {
                    continue;
                }
                let Some(instance) = record.instance.clone() else {
                    blocked.push(format!("{}: no process identity", process.id));
                    continue;
                };
                let launch = process.launch(&program, instance.clone())?;
                if self.plan.processes.iter().any(|p| {
                    p.depends_on.contains(&process.id)
                        && !matches!(
                            state.records[&p.id].phase,
                            Phase::Exited
                                | Phase::StartFailed
                                | Phase::Skipped
                                | Phase::Pending
                                | Phase::Prepared
                        )
                }) {
                    blocked.push(format!("{}: dependent process still present", process.id));
                    continue;
                }
                if !self.authority.may_stop(&self.plan, &process, &launch) {
                    blocked.push(format!("{}: release/stop authority required", process.id));
                    continue;
                }
                if record.phase != Phase::StopRequested {
                    self.change(|s| {
                        s.records.get_mut(&process.id).unwrap().phase = Phase::StopRequested
                    })?;
                    if !self.authority.may_stop(&self.plan, &process, &launch) {
                        blocked.push(format!("{}: stop authority changed", process.id));
                        continue;
                    }
                    self.backend.terminate(&instance, false)?;
                    self.stopping.insert(instance.clone(), Instant::now());
                } else if !self.stopping.contains_key(&instance) {
                    self.backend.terminate(&instance, false)?;
                    self.stopping.insert(instance.clone(), Instant::now());
                } else if self.stopping.get(&instance).is_some_and(|t| {
                    t.elapsed() >= Duration::from_millis(process.shutdown_timeout_ms.0)
                }) {
                    if program.effect == Effect::NonActuating {
                        self.backend.terminate(&instance, true)?;
                    } else {
                        blocked.push(format!(
                            "{}: control process is retained; forced termination not allowed",
                            process.id
                        ));
                    }
                }
                continue;
            }
            match record.phase {
                Phase::Pending | Phase::Prepared | Phase::Exited | Phase::StartFailed => {
                    if record.attempts.0 > 0
                        && (program.effect != Effect::NonActuating
                            || record.attempts.0 > process.restart_limit.0)
                    {
                        blocked.push(format!(
                            "{}: restart budget exhausted or explicit control restart required",
                            process.id
                        ));
                        continue;
                    }
                    if self
                        .retry_after
                        .get(&process.id)
                        .is_some_and(|t| Instant::now() < *t)
                        || process
                            .depends_on
                            .iter()
                            .any(|d| state.records[d].phase != Phase::ProcessReady)
                    {
                        continue;
                    }
                    let instance = if record.phase == Phase::Prepared {
                        record
                            .instance
                            .clone()
                            .ok_or_else(|| Error::Invalid("prepared identity missing".into()))?
                    } else {
                        id()
                    };
                    let launch = process.launch(&program, instance.clone())?;
                    if !self.authority.may_start(&self.plan, &process, &launch) {
                        blocked.push(format!(
                            "{}: platform startup authority required",
                            process.id
                        ));
                        continue;
                    }
                    if record.phase != Phase::Prepared {
                        self.change(|s| {
                            let r = s.records.get_mut(&process.id).unwrap();
                            r.phase = Phase::Prepared;
                            r.instance = Some(instance.clone());
                            r.pid = None;
                            r.exit_code = None;
                            r.error = None;
                            r.guarded_exit = None;
                        })?;
                    }
                    self.change(|s| {
                        let r = s.records.get_mut(&process.id).unwrap();
                        r.phase = Phase::SpawnEntered;
                        r.attempts = Counter(r.attempts.0 + 1);
                    })?;
                    if !self.authority.may_start(&self.plan, &process, &launch) {
                        self.change(|s| {
                            let r = s.records.get_mut(&process.id).unwrap();
                            r.phase = Phase::StartFailed;
                            r.error = Some("startup authority changed before OS invocation".into());
                        })?;
                        continue;
                    }
                    let spawned = if let Some(requirements) = &program.execution_requirements {
                        let request = crate::execution::Request::new(
                            program.id.clone(),
                            process.id.clone(),
                            instance.clone(),
                            state.plan_digest,
                            requirements.clone(),
                        );
                        self.spawn_required(&process, &launch, &request)
                    } else {
                        // Legacy undeclared programs retain the original execution path.
                        let authority = &self.authority;
                        let plan = &self.plan;
                        self.backend.spawn(&launch, &mut || {
                            authority.may_start(plan, &process, &launch)
                        })
                    };
                    match spawned {
                        Ok(pid) => {
                            self.started.insert(instance, Instant::now());
                            self.change(|s| {
                                let r = s.records.get_mut(&process.id).unwrap();
                                r.pid = Some(pid);
                                r.phase = Phase::Starting;
                            })?;
                        }
                        Err(failure) => {
                            let (phase, error) = match failure {
                                crate::process::SpawnFailure::NotStarted(error) => {
                                    (Phase::StartFailed, error)
                                }
                                crate::process::SpawnFailure::Uncertain(error) => {
                                    (Phase::Unknown, error)
                                }
                            };
                            if program.execution_requirements.is_some() {
                                blocked.push(format!("{}: {error}", process.id));
                            }
                            self.change(|s| {
                                let r = s.records.get_mut(&process.id).unwrap();
                                r.phase = phase;
                                r.error = Some(error.to_string());
                            })?;
                            self.retry_after.insert(
                                process.id.clone(),
                                Instant::now()
                                    + Duration::from_millis(process.restart_backoff_ms.0),
                            );
                        }
                    }
                }
                Phase::SpawnEntered => {
                    let owned = record
                        .instance
                        .as_ref()
                        .filter(|id| self.started.contains_key(*id))
                        .and_then(|id| self.backend.pid(id));
                    self.change(|s| {
                        let r = s.records.get_mut(&process.id).unwrap();
                        if let Some(pid) = owned {
                            r.phase = Phase::Starting;
                            r.pid = Some(pid);
                        } else {
                            r.phase = Phase::Unknown;
                            r.error = Some("spawn boundary is unconfirmed; retry forbidden".into());
                        }
                    })?;
                }
                Phase::Starting => {
                    let instance = record.instance.clone().unwrap();
                    let launch = process.launch(&program, instance.clone())?;
                    let ready = if program.effect == Effect::ProtocolGuardedService {
                        match self.backend.guarded_status(&launch) {
                            Ok(value) => value.is_some_and(|o| o.state == GuardedState::Ready),
                            Err(error) => {
                                blocked.push(format!("{}: {error}", process.id));
                                false
                            }
                        }
                    } else {
                        self.backend.ready(&launch)?
                    };
                    if ready {
                        self.change(|s| {
                            s.records.get_mut(&process.id).unwrap().phase = Phase::ProcessReady
                        })?;
                    } else if self.started.get(&instance).is_some_and(|t| {
                        t.elapsed() >= Duration::from_millis(process.startup_timeout_ms.0)
                    }) {
                        self.change(|s| {
                            let r = s.records.get_mut(&process.id).unwrap();
                            r.phase = Phase::Unready;
                            r.error = Some(
                                "startup probe deadline exceeded; process may still be alive"
                                    .into(),
                            );
                        })?;
                    }
                }
                Phase::Unready => blocked.push(format!(
                    "{}: startup not confirmed; process retained",
                    process.id
                )),
                Phase::ProcessReady => {
                    if program.effect == Effect::ProtocolGuardedService {
                        let launch = process.launch(&program, record.instance.clone().unwrap())?;
                        let observed = self.backend.guarded_status(&launch);
                        if !matches!(observed, Ok(Some(ref o)) if o.state == GuardedState::Ready) {
                            self.stop_latched = true;
                            self.change(|s| {
                                s.stop_requested = true;
                                s.records.get_mut(&process.id).unwrap().error = Some(
                                    "protocol readiness lost; cooperative stop required".into(),
                                );
                            })?;
                            blocked.push(format!("{}: protocol readiness lost", process.id));
                        }
                    }
                }
                Phase::Skipped | Phase::StopRequested | Phase::Unknown => {}
            }
        }
        let state = self.state()?;
        let all_exited = state.stop_requested
            && state
                .records
                .values()
                .all(|r| matches!(r.phase, Phase::Exited | Phase::Skipped | Phase::StartFailed));
        let guarded_shutdown_confirmed = all_exited
            && self.plan.processes.iter().all(|p| {
                self.programs[&p.program].effect != Effect::ProtocolGuardedService
                    || matches!(
                        state.records[&p.id].phase,
                        Phase::Skipped | Phase::StartFailed
                    )
                    || matches!(
                        state.records[&p.id].guarded_exit,
                        Some(GuardedExit::Confirmed { .. })
                    )
            });
        let reconciliation_required = state.records.values().any(|r| {
            matches!(
                r.guarded_exit,
                Some(GuardedExit::Unconfirmed { .. })
                    | Some(GuardedExit::Confirmed {
                        reconciliation_required: true,
                        ..
                    })
            )
        });
        Ok(Status {
            schema: "rx.supervisor-status.v1",
            execution_admission: self.execution_status(&state),
            state,
            blocked,
            all_exited,
            control_prepared: false,
            physical_shutdown_assessed: false,
            guarded_shutdown_confirmed,
            reconciliation_required,
        })
    }
    pub fn into_parts(self) -> (R, B, A) {
        (self.repository, self.backend, self.authority)
    }
}
