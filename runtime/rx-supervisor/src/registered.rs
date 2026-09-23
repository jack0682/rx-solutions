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
    if let Some(requirements) = &program.execution_requirements {
        requirements.validate()?;
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
}
impl<B: Backend, R: Repository> RegisteredBackend<B, R> {
    fn begin(&mut self, launch: &Launch) -> std::result::Result<Binding, SpawnFailure> {
        let mut binding = self.binding.clone();
        binding.instance = launch.instance.clone();
        self.registry
            .borrow_mut()
            .assign(&binding)
            .map_err(|e| SpawnFailure::NotStarted(e.into()))?;
        Ok(binding)
    }
}
impl<B: Backend, R: Repository> Backend for RegisteredBackend<B, R> {
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
        mut registry: Registry<R>,
        component: Id,
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
        };
        this.synchronize()?;
        Ok(this)
    }
    pub fn query(&self) -> Result<View> {
        Ok(self.registry.borrow_mut().query(&self.component)?)
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
