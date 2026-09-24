//! Registration connected to one execution owner for the entire process plan.
//! Two independent stores: durable assignment precedes OS effects; a crash between
//! stores leaves an unresolved assignment, never permission to replay it.
use crate::{
    Error, Result, Supervisor, execution,
    model::*,
    process::{Backend, SpawnFailure},
    registration::*,
};
use rx_domain::{canonical, types::*};
use rx_ports::Repository;
use rx_solution_catalog::DeviceCatalog;
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

pub fn catalog_reference(program: &Program) -> Result<CatalogReference> {
    if let Some(policy) = &program.decision_policy {
        policy.fingerprint().map_err(Error::Invalid)?;
    }
    if let Some(requirements) = &program.execution_requirements {
        requirements.validate()?;
    }
    if let Some(contract) = &program.functional_readiness {
        contract.validate().map_err(Error::Invalid)?;
    }
    Ok(CatalogReference {
        program: program.id.clone(),
        digest: canonical::digest("RX-COMPONENT-PROGRAM-v1", program)
            .map_err(|e| Error::Invalid(e.to_string()))?,
    })
}

struct RegisteredBackend<B, R> {
    backend: B,
    registry: Rc<RefCell<Registry<R>>>,
    bindings: BTreeMap<Name, Binding>,
    legacy: bool,
    resume: Option<ResumePermit>,
}
impl<B: Backend, R: Repository> RegisteredBackend<B, R> {
    fn begin(&mut self, launch: &Launch) -> std::result::Result<Binding, SpawnFailure> {
        let mut binding = self
            .bindings
            .get(&launch.selection)
            .cloned()
            .ok_or_else(|| {
                SpawnFailure::NotStarted(Error::Reconciliation(
                    "launch selection has no registration binding".into(),
                ))
            })?;
        binding.instance = launch.instance.clone();
        if let Some(permit) = &self.resume {
            self.registry
                .borrow_mut()
                .assign_with_resume(&binding, Some(permit))
                .map_err(|e| SpawnFailure::NotStarted(e.into()))?;
            self.resume = None;
        } else {
            self.registry
                .borrow_mut()
                .assign(&binding)
                .map_err(|e| SpawnFailure::NotStarted(e.into()))?;
        }
        Ok(binding)
    }
}
impl<B: Backend, R: Repository> Backend for RegisteredBackend<B, R> {
    fn observe_status(
        &mut self,
        launch: &Launch,
        request: &crate::use_assessment::StatusObservationRequest,
    ) -> Result<crate::use_assessment::StatusObservationResult> {
        self.backend.observe_status(launch, request)
    }
    fn spawn(
        &mut self,
        launch: &Launch,
        authorize: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<u32, SpawnFailure> {
        if !self.legacy {
            return Err(SpawnFailure::NotStarted(Error::Invalid(
                "registered execution requires explicit catalog execution requirements".into(),
            )));
        }
        let binding = self.begin(launch)?;
        let registry = &self.registry;
        let mut denial = None;
        let outcome = self.backend.spawn(launch, &mut || {
            if !authorize() {
                return false;
            }
            match registry.borrow_mut().check_start(&binding) {
                Ok(()) => true,
                Err(error) => {
                    denial = Some(error);
                    false
                }
            }
        });
        match (outcome, denial) {
            (Err(SpawnFailure::NotStarted(_)), Some(error)) => {
                Err(SpawnFailure::NotStarted(error.into()))
            }
            (outcome, _) => outcome,
        }
    }
    fn spawn_with_requirements(
        &mut self,
        launch: &Launch,
        request: &execution::Request,
        authorize: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<execution::Decision, SpawnFailure> {
        if request.instance() != &launch.instance
            || request.process() != &launch.selection
            || self
                .bindings
                .get(&launch.selection)
                .is_none_or(|b| &b.catalog.program != request.program())
        {
            return Err(SpawnFailure::NotStarted(Error::Reconciliation(
                "execution request differs from registered launch selection/program".into(),
            )));
        }
        let binding = self.begin(launch)?;
        let registry = &self.registry;
        let mut denial = None;
        let outcome = self
            .backend
            .spawn_with_requirements(launch, request, &mut || {
                if !authorize() {
                    return false;
                }
                match registry.borrow_mut().check_start(&binding) {
                    Ok(()) => true,
                    Err(error) => {
                        denial = Some(error);
                        false
                    }
                }
            });
        match (outcome, denial) {
            (Err(SpawnFailure::NotStarted(_)), Some(error)) => {
                Err(SpawnFailure::NotStarted(error.into()))
            }
            (outcome, _) => outcome,
        }
    }
    fn pid(&self, id: &Id) -> Option<u32> {
        self.backend.pid(id)
    }
    fn owns(&self, id: &Id) -> bool {
        self.backend.owns(id)
    }
    fn forget_exited(&mut self, id: &Id) -> Result<()> {
        self.backend.forget_exited(id)
    }
    fn exited(&mut self, id: &Id) -> Result<Option<Option<i32>>> {
        self.backend.exited(id)
    }
    fn ready(&mut self, launch: &Launch) -> Result<bool> {
        self.backend.ready(launch)
    }
    fn guarded_status(&mut self, launch: &Launch) -> Result<Option<GuardedObservation>> {
        self.backend.guarded_status(launch)
    }
    fn terminate(&mut self, id: &Id, force: bool) -> Result<()> {
        self.backend.terminate(id, force)
    }
}

pub struct RegisteredSupervisor<S, B, A, R> {
    supervisor: Supervisor<S, RegisteredBackend<B, R>, A>,
    registry: Rc<RefCell<Registry<R>>>,
    run: Id,
    bindings: BTreeMap<Name, Binding>,
    single: Option<SingleComponent>,
    uses: BTreeMap<Name, AuthoredUse>,
}
struct SingleComponent {
    component: Id,
    selection: Name,
    catalog: CatalogReference,
}
struct AuthoredUse {
    effect: Effect,
    readiness: Option<crate::use_assessment::ReadinessContract>,
    decision_policy: Option<crate::decision::Policy>,
    decision_gate: crate::decision::Gate,
}
struct UseSnapshot {
    view: View,
    state: State,
    subject: crate::use_assessment::UseSubject,
    readiness: crate::use_assessment::ReadinessAssessment,
}
impl<S: Repository, B: Backend, A: LifecycleAuthority, R: Repository>
    RegisteredSupervisor<S, B, A, R>
{
    fn authored_uses(
        plan: &Plan,
        programs: &BTreeMap<Name, Program>,
    ) -> BTreeMap<Name, AuthoredUse> {
        plan.processes
            .iter()
            .map(|p| {
                let program = &programs[&p.program];
                (
                    p.id.clone(),
                    AuthoredUse {
                        effect: program.effect,
                        readiness: program.functional_readiness.clone(),
                        decision_policy: program.decision_policy.clone(),
                        decision_gate: crate::decision::Gate::new(),
                    },
                )
            })
            .collect()
    }
    /// This first connection supports one non-actuating component. It does not
    /// change the existing unregistered supervisor or enable physical authority.
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        store: S,
        backend: B,
        authority: A,
        plan: Plan,
        programs: BTreeMap<Name, Program>,
        support: &DeviceCatalog,
        registry: Registry<R>,
        component: Id,
    ) -> Result<Self> {
        Self::open_inner(
            store, backend, authority, plan, programs, support, registry, component, None,
        )
    }
    /// An explicit one-shot recovery request. Uses a fresh supervisor store/run,
    /// never resets the old UNKNOWN state or restores a previous F1 receipt.
    #[allow(clippy::too_many_arguments)]
    pub fn open_with_resume(
        mut store: S,
        backend: B,
        authority: A,
        plan: Plan,
        programs: BTreeMap<Name, Program>,
        support: &DeviceCatalog,
        registry: Registry<R>,
        component: Id,
        permit: ResumePermit,
    ) -> Result<Self> {
        if !store.snapshot()?.1.is_empty()
            || store.journal_head()?.0 != 0
            || store.control_snapshot()?.0.0 != 0
        {
            return Err(Error::Invalid("recovery requires a fresh execution store; never overwrite or reuse prior instance state".into()));
        }
        if plan.processes.iter().any(|p| p.restart_limit.0 != 0) {
            return Err(Error::Invalid(
                "recovery starts are explicit; automatic restart budget must be zero".into(),
            ));
        }
        Self::open_inner(
            store,
            backend,
            authority,
            plan,
            programs,
            support,
            registry,
            component,
            Some(permit),
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn open_inner(
        store: S,
        backend: B,
        authority: A,
        plan: Plan,
        programs: BTreeMap<Name, Program>,
        support: &DeviceCatalog,
        mut registry: Registry<R>,
        component: Id,
        resume: Option<ResumePermit>,
    ) -> Result<Self> {
        if plan.processes.len() != 1 {
            return Err(Error::Invalid(
                "registered execution supports one component per supervisor store".into(),
            ));
        }
        let process = &plan.processes[0];
        let program = programs
            .get(&process.program)
            .ok_or_else(|| Error::Invalid("registered program missing from catalog".into()))?;
        if program.effect != Effect::NonActuating || program.execution_requirements.is_none() {
            return Err(Error::Invalid("registered execution requires non-actuating effect and explicit catalog requirements".into()));
        }
        let catalog = catalog_reference(program)?;
        let uses = Self::authored_uses(&plan, &programs);
        let accepted = registry.query(&component)?.registration;
        if accepted.registration.declaration.catalog != catalog {
            return Err(Error::Invalid("program/catalog declaration differs from accepted registration; review an explicit revision, preserve prior records".into()));
        }
        let run = plan.id.clone();
        let selection = process.id.clone();
        let registry = Rc::new(RefCell::new(registry));
        let bindings: BTreeMap<Name, Binding> = [(
            selection.clone(),
            Binding {
                registration: component.clone(),
                registration_revision: accepted.revision,
                catalog: catalog.clone(),
                run: run.clone(),
                selection: selection.clone(),
                // Replaced by the supervisor-generated execution identity before assignment.
                instance: component.clone(),
            },
        )]
        .into();
        let backend = RegisteredBackend {
            backend,
            registry: registry.clone(),
            resume,
            bindings: bindings.clone(),
            legacy: false,
        };
        let supervisor = Supervisor::open(store, backend, authority, plan, programs, support)?;
        let mut this = Self {
            supervisor,
            registry,
            run,
            bindings,
            single: Some(SingleComponent {
                component,
                selection,
                catalog,
            }),
            uses,
        };
        this.synchronize()?;
        Ok(this)
    }
    pub fn query(&self) -> Result<View> {
        Ok(self
            .registry
            .borrow_mut()
            .query(&self.single()?.component)?)
    }
    fn single(&self) -> Result<&SingleComponent> {
        self.single.as_ref().ok_or_else(|| {
            Error::Invalid(
                "single-component assessment API is unavailable on the resident composition".into(),
            )
        })
    }

    /// Resident lifecycle registration only. This does not enable functional
    /// readiness, work decisions, or dependency consumption.
    /// Existing single-component constructors keep their narrower guarantees.
    #[allow(clippy::too_many_arguments)]
    pub fn open_resident(
        mut store: S,
        backend: B,
        authority: A,
        plan: Plan,
        programs: BTreeMap<Name, Program>,
        support: &DeviceCatalog,
        mut registry: Registry<R>,
    ) -> Result<Self> {
        plan.validate(&programs, support)?;
        let uses = Self::authored_uses(&plan, &programs);
        let declarations = plan
            .processes
            .iter()
            .map(|process| {
                Ok((
                    process.id.clone(),
                    Declaration {
                        label: process.id.clone(),
                        catalog: catalog_reference(&programs[&process.program])?,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        let fresh = store.snapshot()?.1.is_empty()
            && store.journal_head()?.0 == 0
            && store.control_snapshot()?.0.0 == 0;
        let accepted = registry.resident_registrations(&declarations, fresh)?;
        let run = plan.id.clone();
        let bindings: BTreeMap<_, _> = accepted
            .into_iter()
            .map(|(selection, accepted)| {
                (
                    selection.clone(),
                    Binding {
                        registration: accepted.registration.id.clone(),
                        registration_revision: accepted.revision,
                        catalog: accepted.registration.declaration.catalog,
                        run: run.clone(),
                        selection,
                        instance: accepted.registration.id,
                    },
                )
            })
            .collect();
        let registry = Rc::new(RefCell::new(registry));
        let backend = RegisteredBackend {
            backend,
            registry: registry.clone(),
            bindings: bindings.clone(),
            legacy: true,
            resume: None,
        };
        let supervisor = Supervisor::open(store, backend, authority, plan, programs, support)?;
        let mut this = Self {
            supervisor,
            registry,
            run,
            bindings,
            single: None,
            uses,
        };
        this.synchronize()?;
        Ok(this)
    }

    /// Explicit cold recovery for a single non-actuating resident selection.
    /// The resident catalog index is checked before the existing F3 constructor.
    #[allow(clippy::too_many_arguments)]
    pub fn open_resident_with_resume(
        store: S,
        backend: B,
        authority: A,
        plan: Plan,
        programs: BTreeMap<Name, Program>,
        support: &DeviceCatalog,
        mut registry: Registry<R>,
        permit: ResumePermit,
    ) -> Result<Self> {
        if plan.processes.len() != 1 {
            return Err(Error::Invalid(
                "resident resume supports one non-actuating selection".into(),
            ));
        }
        let process = &plan.processes[0];
        let program = programs
            .get(&process.program)
            .ok_or_else(|| Error::Invalid("resume program absent".into()))?;
        let declarations = [(
            process.id.clone(),
            Declaration {
                label: process.id.clone(),
                catalog: catalog_reference(program)?,
            },
        )]
        .into();
        let accepted = registry.resident_registrations(&declarations, false)?;
        let component = accepted[&process.id].registration.id.clone();
        let mut this = Self::open_with_resume(
            store, backend, authority, plan, programs, support, registry, component, permit,
        )?;
        this.single = None;
        Ok(this)
    }
    pub fn investigate_current(&mut self, selection: &Name) -> Result<ProcessInvestigation> {
        let binding = self
            .bindings
            .get(selection)
            .ok_or_else(|| Error::Invalid("investigation selection absent".into()))?;
        let state = self.supervisor.state()?;
        let record = &state.records[selection];
        if record.phase != Phase::Unknown {
            return Err(Error::Reconciliation(
                "cold investigation requires an UNKNOWN original record".into(),
            ));
        }
        let instance = record
            .instance
            .as_ref()
            .ok_or_else(|| Error::Reconciliation("original instance missing".into()))?;
        let mut registry = self.registry.borrow_mut();
        let target = registry.recovery_target(&binding.registration, instance)?;
        Ok(registry.investigate(&target)?)
    }
    /// Explicit caller action. Alive/unverifiable findings never automatically
    /// occupy the immutable disposition key, so later evidence remains usable.
    pub fn prepare_investigated_resume(
        &mut self,
        selection: &Name,
        next_run: Id,
        actor: Name,
        authority: &impl RecoveryAuthority,
    ) -> Result<ResumePermit> {
        if self.bindings.len() != 1 || next_run == self.run {
            return Err(Error::Invalid(
                "resume requires one selection and a new run".into(),
            ));
        }
        let finding = self.investigate_current(selection)?;
        if !finding.outcome().permits_closure() {
            return Err(Error::Reconciliation(format!(
                "investigation-blocks-resume: {:?}",
                finding.outcome()
            )));
        }
        let target = finding.target().clone();
        let mut registry = self.registry.borrow_mut();
        let view = registry.query(&target.registration)?;
        let disposition = if let Some(d) = view
            .recovery
            .dispositions
            .iter()
            .find(|d| d.target == target)
        {
            d.clone()
        } else {
            registry.dispose(
                &DispositionRequest {
                    target: target.clone(),
                    kind: DispositionKind::ConfirmedClosure,
                    evidence: RecoveryEvidence::Investigation {
                        report: RecoveryReport {
                            actor: actor.clone(),
                            scope: Name::new("host/execution-investigation").expect("literal"),
                            observed_at: finding.observed_at().clone(),
                            procedure: Name::new("linux/scoped-process-identity").expect("literal"),
                        },
                        finding: Box::new(finding),
                    },
                },
                authority,
            )?
        };
        // Fresh provider evidence is also required for a reissued, unconsumed request.
        let finding = registry.investigate(&target)?;
        let request = view
            .recovery
            .resume_requests
            .iter()
            .find(|r| r.request.disposition == disposition.id)
            .map(|r| r.request.clone())
            .unwrap_or_else(|| ResumeRequest {
                id: Id::new(uuid::Uuid::new_v4().to_string()).expect("UUID"),
                disposition: disposition.id,
                actor: actor.clone(),
                requested_at: finding.observed_at().clone(),
                next_run: next_run.clone(),
            });
        if request.next_run != next_run || request.actor != actor {
            return Err(Error::Reconciliation(
                "existing explicit resume names another run/actor".into(),
            ));
        }
        Ok(registry.request_investigated_resume(
            &target.registration,
            request,
            &finding,
            authority,
        )?)
    }

    /// Read-only views, keyed by current plan selection. Persisted observations
    /// never establish ownership or permission for this caller.
    pub fn registrations(&self) -> Result<BTreeMap<Name, View>> {
        self.bindings
            .iter()
            .map(|(selection, binding)| {
                Ok((
                    selection.clone(),
                    self.registry.borrow_mut().query(&binding.registration)?,
                ))
            })
            .collect()
    }
    /// Fresh, explicitly scoped snapshot over component self-report data. No
    /// evaluation is persisted/cached as permission, and query() remains read-only
    /// without probing. A later use requires a fresh operating-area judgment.
    fn use_snapshot(
        &mut self,
        selection: &Name,
        scope: &crate::use_assessment::UseScope,
    ) -> Result<UseSnapshot> {
        use crate::use_assessment::*;
        let binding = self
            .bindings
            .get(selection)
            .ok_or_else(|| Error::Invalid("use selection missing".into()))?;
        let view = self.registry.borrow_mut().query(&binding.registration)?;
        let state = self.supervisor.state()?;
        let authored = &self.uses[selection];
        let record = &state.records[selection];
        let subject = UseSubject {
            registration: binding.registration.clone(),
            registration_revision: view.registration.revision,
            program: binding.catalog.program.clone(),
            catalog_digest: binding.catalog.digest,
            run: self.run.clone(),
            selection: selection.clone(),
            instance: record.instance.clone(),
            pid: record.pid,
        };
        let assessment = if let Some(profile) = authored
            .readiness
            .as_ref()
            .and_then(|c| c.0.get(&scope.role))
        {
            let request =
                StatusObservationRequest::new(subject.clone(), scope.clone(), profile.endpoint);
            ReadinessAssessment::evaluate(
                &request,
                profile,
                self.supervisor
                    .observe_use_status(&request)
                    .unwrap_or_else(|error| StatusObservationResult::NotEvaluated {
                        condition: Name::new("readiness/observation-error").expect("literal"),
                        reason: format!(
                            "observation unavailable: {error}; no condition is inferred satisfied"
                        ),
                    }),
            )
        } else {
            ReadinessAssessment::unobserved(Some(scope.clone()),Some(subject.clone()),ConditionState::Unsupported,Name::new("catalog/readiness-profile").expect("literal"),"author has not declared supported readiness conditions for this intended use; metadata or process liveness cannot supply them".into())
        };
        Ok(UseSnapshot {
            view,
            state,
            subject,
            readiness: assessment,
        })
    }
    pub fn assess_use(
        &mut self,
        scope: crate::use_assessment::UseScope,
        port: &impl crate::use_assessment::WorkUsePort,
    ) -> Result<View> {
        use crate::use_assessment::*;
        let selection = self.single()?.selection.clone();
        let UseSnapshot {
            mut view,
            state,
            subject,
            readiness: assessment,
        } = self.use_snapshot(&selection, &scope)?;
        let single = self.single()?;
        let authored = &self.uses[&selection];
        let record = &state.records[&selection];
        let decision = if view.registration.registration.state == RegistrationState::Accepted
            && view.registration.registration.declaration.catalog == single.catalog
            && record.instance.is_some()
            && record.pid.is_some()
            && matches!(
                record.phase,
                Phase::Starting | Phase::ProcessReady | Phase::Unready
            ) {
            authored.decision_policy.as_ref().and_then(|policy| {
                authored
                    .decision_gate
                    .request(
                        crate::decision::Kind::WorkUse,
                        crate::decision::Owner {
                            registration: single.component.clone(),
                            revision: view.registration.revision,
                            program: single.catalog.program.clone(),
                            catalog: single.catalog.digest,
                        },
                        &scope.operating_area,
                        &scope.role,
                        serde_json::json!({"subject":subject,"configuration":state.plan_digest}),
                        policy,
                    )
                    .ok()
            })
        } else {
            None
        };
        let request = WorkUseRequest::new(scope, subject, assessment.clone(), decision);
        view.work_use_permission = WorkUseAssessment::assessed(&request, port.assess(&request));
        view.functional_readiness = assessment;
        Ok(view)
    }
    fn work_snapshot(
        &mut self,
        task: &crate::work_use::Task,
    ) -> Result<(
        crate::work_use::Input,
        crate::use_assessment::WorkUseRequest,
    )> {
        use crate::{use_assessment::*, work_use::*};
        let readiness_scope = UseScope {
            operating_area: task.operating_area.clone(),
            role: name(READINESS_ROLE),
        };
        let snapshot = self.use_snapshot(&task.selection, &readiness_scope)?;
        let authored = &self.uses[&task.selection];
        let record = &snapshot.state.records[&task.selection];
        if authored.effect != Effect::NonActuating
            || snapshot.state.stop_requested
            || record.phase != Phase::ProcessReady
        {
            return Err(invalid(
                "current-owned-ready-non-actuating-execution-required",
            ));
        }
        if snapshot.view.registration.registration.state != RegistrationState::Accepted
            || snapshot.view.registration.registration.declaration.catalog
                != self.bindings[&task.selection].catalog
        {
            return Err(invalid("current-registration-changed"));
        }
        let profile = authored
            .readiness
            .as_ref()
            .and_then(|r| r.0.get(&name(READINESS_ROLE)))
            .ok_or_else(|| invalid("support-summary-profile-unsupported"))?;
        let input = snapshot
            .readiness
            .support_gap_input(profile, snapshot.state.plan_digest)?;
        let policy = authored.decision_policy.as_ref().ok_or_else(|| {
            invalid("author-policy-absent; operating-area provider is not connected")
        })?;
        let scope = UseScope {
            operating_area: task.operating_area.clone(),
            role: name(ROLE),
        };
        let decision = authored
            .decision_gate
            .request(
                crate::decision::Kind::WorkUse,
                crate::decision::Owner {
                    registration: input.subject.registration.clone(),
                    revision: input.subject.registration_revision,
                    program: input.subject.program.clone(),
                    catalog: input.subject.catalog_digest,
                },
                &scope.operating_area,
                &scope.role,
                input.context(task)?,
                policy,
            )
            .map_err(|e| invalid(&e.to_string()))?;
        let request =
            WorkUseRequest::new(scope, snapshot.subject, snapshot.readiness, Some(decision));
        Ok((input, request))
    }
    /// Prepare a bounded support-gap task for an external issuer. This performs
    /// no work commit and consumes no permission. The prepare/use interval is
    /// explicit so changed conditions must be checked at the receiving boundary.
    pub fn prepare_work(
        &mut self,
        task: crate::work_use::Task,
        port: &impl crate::use_assessment::WorkUsePort,
    ) -> Result<crate::work_use::Prepared> {
        use crate::{use_assessment::*, work_use::*};
        self.registry
            .borrow_mut()
            .ensure_new_work(&task.operation)?;
        let (input, request) = self.work_snapshot(&task)?;
        match port.assess(&request) {
            WorkUseReply::Verified(proof) => {
                request
                    .decision_request()
                    .expect("work snapshot configured request")
                    .check(&proof)
                    .map_err(|e| invalid(&e.to_string()))?;
                Ok(Prepared {
                    context: digest("RX-WORK-USE-CONTEXT-v1", &input.context(&task)?)?,
                    task,
                    proof: *proof,
                })
            }
            other => {
                let assessment = WorkUseAssessment::assessed(&request, other);
                Err(invalid(&format!(
                    "external-judgment-{:?}: {}",
                    assessment.state(),
                    assessment
                        .conditions()
                        .iter()
                        .map(|c| format!("{}: {}", c.name, c.reason))
                        .collect::<Vec<_>>()
                        .join("; ")
                )))
            }
        }
    }
    /// Reobserve, then commit derived result and unique decision consumption
    /// atomically. A rollback spends neither. Query by operation ID resolves a
    /// lost response; duplicate submissions never execute another work result.
    pub fn commit_work(
        &mut self,
        prepared: &crate::work_use::Prepared,
    ) -> Result<crate::work_use::Report> {
        use crate::work_use::*;
        self.registry
            .borrow_mut()
            .ensure_new_work(&prepared.task.operation)?;
        let (input, request) = self.work_snapshot(&prepared.task)?;
        if digest("RX-WORK-USE-CONTEXT-v1", &input.context(&prepared.task)?)? != prepared.context {
            return Err(invalid("input-or-current-context-changed-since-judgment"));
        }
        let guard = request
            .decision_request()
            .expect("work snapshot configured request")
            .commit_guard(&prepared.proof)
            .map_err(|e| invalid(&e.to_string()))?;
        // guard remains alive through the physical transaction commit.
        Ok(self
            .registry
            .borrow_mut()
            .commit_work(&prepared.task, &input, &guard)?)
    }
    pub fn recorded_work(
        &self,
        selection: &Name,
        operation: &Id,
    ) -> Result<crate::work_use::Report> {
        let binding = self
            .bindings
            .get(selection)
            .ok_or_else(|| Error::Invalid("work selection missing".into()))?;
        Ok(self
            .registry
            .borrow_mut()
            .recorded_work(&binding.registration, operation)?)
    }
    pub fn record_verified_decision(
        &self,
        proof: &crate::decision::VerifiedDecision,
    ) -> Result<DecisionRecord> {
        self.single()?;
        Ok(self.registry.borrow_mut().record_verified_decision(proof)?)
    }
    pub fn record_verified_revocation(
        &self,
        proof: &crate::decision::VerifiedRevocation,
    ) -> Result<DecisionRevocationRecord> {
        self.single()?;
        Ok(self
            .registry
            .borrow_mut()
            .record_verified_revocation(proof)?)
    }
    pub fn recorded_decision(&self, decision: &Id) -> Result<DecisionRecord> {
        Ok(self
            .registry
            .borrow_mut()
            .recorded_decision(&self.single()?.component, decision)?)
    }
    pub fn history(&self) -> Result<Vec<rx_ports::StoredEvent>> {
        Ok(self
            .registry
            .borrow_mut()
            .history(&self.single()?.component)?)
    }
    pub fn retire(&self, expected: Counter) -> Result<VersionedRegistration> {
        Ok(self
            .registry
            .borrow_mut()
            .retire(&self.single()?.component, expected)?)
    }
    pub fn state(&mut self) -> Result<State> {
        self.supervisor.state()
    }
    pub fn execution_admission(&mut self) -> Result<BTreeMap<Name, execution::Status>> {
        self.supervisor.execution_admission()
    }
    pub fn request_stop(&mut self) -> Result<()> {
        self.supervisor.request_stop()
    }
    pub fn rearm_software(&mut self) -> Result<()> {
        self.supervisor.rearm_software()
    }
    pub fn tick(&mut self) -> Result<Status> {
        let outcome = self.supervisor.tick();
        // Even if the last supervisor commit failed, retain only what that store
        // actually confirms. Assignment remains unresolved if either store fails.
        self.synchronize()?;
        outcome
    }
    fn synchronize(&mut self) -> Result<()> {
        let state = self.supervisor.state()?;
        // Validate the whole composition before publishing any observation.
        let mut observations = Vec::new();
        for (selection, binding) in &self.bindings {
            let record = &state.records[selection];
            let Some(instance) = &record.instance else {
                continue;
            };
            let view = self.registry.borrow_mut().query(&binding.registration)?;
            let Some(execution) = view
                .executions
                .iter()
                .find(|e| e.binding.instance == *instance)
            else {
                if matches!(
                    record.phase,
                    Phase::Prepared | Phase::StartFailed | Phase::Skipped
                ) {
                    continue;
                }
                return Err(Error::Reconciliation(format!(
                    "{selection}: saved execution has no registration assignment; no adoption or inferred association"
                )));
            };
            if execution.binding.run != self.run
                || execution.binding.selection != *selection
                || execution.binding.catalog != binding.catalog
                || execution.binding.registration != binding.registration
            {
                return Err(Error::Reconciliation(format!(
                    "{selection}: saved execution belongs to another run/selection/catalog/registration"
                )));
            }
            let state = match record.phase {
                Phase::Exited => ExecutionState::Exited,
                Phase::StartFailed | Phase::Skipped => ExecutionState::NotStarted,
                Phase::Starting | Phase::ProcessReady | Phase::Unready | Phase::StopRequested => {
                    ExecutionState::Running
                }
                _ => ExecutionState::Unknown,
            };
            observations.push((execution.binding.clone(), Observation {
                state, pid: record.pid, exit_code: record.exit_code,
                process_identity: record.process_identity.clone(),
                detail: format!("supervisor phase {:?}; {}; functional readiness and work-use permission are not assessed",
                    record.phase, record.error.as_deref().unwrap_or("no additional lifecycle error")),
            }));
        }
        for (binding, observation) in observations {
            self.registry.borrow_mut().observe(&binding, observation)?;
        }
        Ok(())
    }
    pub fn into_parts(self) -> (S, B, A, Registry<R>) {
        let (store, backend, authority) = self.supervisor.into_parts();
        let RegisteredBackend {
            backend, registry, ..
        } = backend;
        drop(registry);
        let registry = Rc::try_unwrap(self.registry)
            .ok()
            .expect("private registry references released")
            .into_inner();
        (store, backend, authority, registry)
    }
}

// This adapter supplies observations only. The consumer's operation and result
// lifecycle remain in the repository-backed diagnostic module.
impl<S: Repository, B: Backend, A: LifecycleAuthority, R: Repository>
    crate::registration::diagnostic::Source for RegisteredSupervisor<S, B, A, R>
{
    fn observe(
        &mut self,
        request: &crate::registration::diagnostic::Probe,
    ) -> crate::registration::diagnostic::SourceReply {
        use crate::registration::diagnostic::*;
        let observed = (|| -> Result<SourceReply> {
            let registration = self.query()?.registration;
            if registration.registration.state != RegistrationState::Accepted {
                return Ok(SourceReply::Missing {
                    known_lost: true,
                    reason: "provider registration is retired".into(),
                });
            }
            let provider = RegistrationRef::from_registration(&registration);
            if provider != *request.provider() {
                return Ok(SourceReply::Missing { known_lost: true, reason: "provider registration/revision/catalog differs; replacement requires separate consumer-side judgment".into() });
            }
            let state = self.state()?;
            let record = &state.records[&self.single()?.selection];
            if matches!(
                record.phase,
                Phase::Exited | Phase::Skipped | Phase::StartFailed
            ) {
                return Ok(SourceReply::Missing {
                    known_lost: true,
                    reason: "owned provider execution has terminated or did not start".into(),
                });
            }
            let generation = record
                .instance
                .clone()
                .zip(record.pid)
                .map(|(instance, pid)| Generation {
                    run: self.run.clone(),
                    instance,
                    pid,
                    configuration: state.plan_digest,
                });
            let assessment = self
                .assess_use(
                    request.scope().clone(),
                    &crate::use_assessment::NoWorkUseProvider,
                )?
                .functional_readiness;
            Ok(SourceReply::Observed(Box::new(
                ProviderObservation::captured(request, provider, generation, assessment),
            )))
        })();
        observed.unwrap_or_else(|error| SourceReply::Missing {
            known_lost: false,
            reason: format!("provider observation unavailable: {error}"),
        })
    }
}
