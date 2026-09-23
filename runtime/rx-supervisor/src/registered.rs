//! One registered non-actuating component connected to the existing supervisor.
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
    binding: Binding,
    resume: Option<ResumePermit>,
}
impl<B: Backend, R: Repository> RegisteredBackend<B, R> {
    fn begin(&mut self, launch: &Launch) -> std::result::Result<Binding, SpawnFailure> {
        let mut binding = self.binding.clone();
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
        _: &Launch,
        _: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<u32, SpawnFailure> {
        Err(SpawnFailure::NotStarted(Error::Invalid(
            "registered execution requires explicit catalog execution requirements".into(),
        )))
    }
    fn spawn_with_requirements(
        &mut self,
        launch: &Launch,
        request: &execution::Request,
        authorize: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<execution::Decision, SpawnFailure> {
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
    component: Id,
    run: Id,
    selection: Name,
    catalog: CatalogReference,
    readiness: Option<crate::use_assessment::ReadinessContract>,
    decision_policy: Option<crate::decision::Policy>,
    decision_gate: crate::decision::Gate,
}
impl<S: Repository, B: Backend, A: LifecycleAuthority, R: Repository>
    RegisteredSupervisor<S, B, A, R>
{
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
        let readiness = program.functional_readiness.clone();
        let decision_policy = program.decision_policy.clone();
        let accepted = registry.query(&component)?.registration;
        if accepted.registration.declaration.catalog != catalog {
            return Err(Error::Invalid("program/catalog declaration differs from accepted registration; review an explicit revision, preserve prior records".into()));
        }
        let run = plan.id.clone();
        let selection = process.id.clone();
        let registry = Rc::new(RefCell::new(registry));
        let backend = RegisteredBackend {
            backend,
            registry: registry.clone(),
            resume,
            binding: Binding {
                registration: component.clone(),
                registration_revision: accepted.revision,
                catalog: catalog.clone(),
                run: run.clone(),
                selection: selection.clone(),
                // Replaced by the supervisor-generated execution identity before assignment.
                instance: component.clone(),
            },
        };
        let supervisor = Supervisor::open(store, backend, authority, plan, programs, support)?;
        let mut this = Self {
            supervisor,
            registry,
            component,
            run,
            selection,
            catalog,
            readiness,
            decision_policy,
            decision_gate: crate::decision::Gate::new(),
        };
        this.synchronize()?;
        Ok(this)
    }
    pub fn query(&self) -> Result<View> {
        Ok(self.registry.borrow_mut().query(&self.component)?)
    }
    /// Fresh, explicitly scoped snapshot over component self-report data. No
    /// evaluation is persisted/cached as permission, and query() remains read-only
    /// without probing. A later use requires a fresh operating-area judgment.
    pub fn assess_use(
        &mut self,
        scope: crate::use_assessment::UseScope,
        port: &impl crate::use_assessment::WorkUsePort,
    ) -> Result<View> {
        use crate::use_assessment::*;
        let mut view = self.query()?;
        let state = self.supervisor.state()?;
        let record = &state.records[&self.selection];
        let subject = UseSubject {
            registration: self.component.clone(),
            registration_revision: view.registration.revision,
            program: self.catalog.program.clone(),
            catalog_digest: self.catalog.digest,
            run: self.run.clone(),
            selection: self.selection.clone(),
            instance: record.instance.clone(),
            pid: record.pid,
        };
        let assessment = if let Some(profile) =
            self.readiness.as_ref().and_then(|c| c.0.get(&scope.role))
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
        let decision = if view.registration.registration.state == RegistrationState::Accepted
            && view.registration.registration.declaration.catalog == self.catalog
            && record.instance.is_some()
            && record.pid.is_some()
            && matches!(
                record.phase,
                Phase::Starting | Phase::ProcessReady | Phase::Unready
            ) {
            self.decision_policy.as_ref().and_then(|policy| {
                self.decision_gate
                    .request(
                        crate::decision::Kind::WorkUse,
                        crate::decision::Owner {
                            registration: self.component.clone(),
                            revision: view.registration.revision,
                            program: self.catalog.program.clone(),
                            catalog: self.catalog.digest,
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
    pub fn record_verified_decision(
        &self,
        proof: &crate::decision::VerifiedDecision,
    ) -> Result<DecisionRecord> {
        Ok(self.registry.borrow_mut().record_verified_decision(proof)?)
    }
    pub fn record_verified_revocation(
        &self,
        proof: &crate::decision::VerifiedRevocation,
    ) -> Result<DecisionRevocationRecord> {
        Ok(self
            .registry
            .borrow_mut()
            .record_verified_revocation(proof)?)
    }
    pub fn recorded_decision(&self, decision: &Id) -> Result<DecisionRecord> {
        Ok(self
            .registry
            .borrow_mut()
            .recorded_decision(&self.component, decision)?)
    }
    pub fn history(&self) -> Result<Vec<rx_ports::StoredEvent>> {
        Ok(self.registry.borrow_mut().history(&self.component)?)
    }
    pub fn retire(&self, expected: Counter) -> Result<VersionedRegistration> {
        Ok(self
            .registry
            .borrow_mut()
            .retire(&self.component, expected)?)
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
        let record = self.supervisor.state()?.records[&self.selection].clone();
        let Some(instance) = &record.instance else {
            return Ok(());
        };
        let view = self.query()?;
        let Some(execution) = view
            .executions
            .iter()
            .find(|e| e.binding.instance == *instance)
        else {
            if matches!(
                record.phase,
                Phase::Prepared | Phase::StartFailed | Phase::Skipped
            ) {
                return Ok(());
            }
            return Err(Error::Reconciliation("saved execution has no registration assignment; no adoption or inferred association".into()));
        };
        if execution.binding.run != self.run
            || execution.binding.selection != self.selection
            || execution.binding.catalog != self.catalog
        {
            return Err(Error::Reconciliation(
                "saved execution belongs to another run/selection/catalog".into(),
            ));
        }
        let state = match record.phase {
            Phase::Exited => ExecutionState::Exited,
            Phase::StartFailed | Phase::Skipped => ExecutionState::NotStarted,
            Phase::Starting | Phase::ProcessReady | Phase::Unready | Phase::StopRequested => {
                ExecutionState::Running
            }
            _ => ExecutionState::Unknown,
        };
        let detail = format!(
            "supervisor phase {:?}; {}; functional readiness and work-use permission are not assessed",
            record.phase,
            record
                .error
                .as_deref()
                .unwrap_or("no additional lifecycle error")
        );
        self.registry.borrow_mut().observe(
            &execution.binding,
            Observation {
                state,
                pid: record.pid,
                exit_code: record.exit_code,
                detail,
            },
        )?;
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
            let record = &state.records[&self.selection];
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
