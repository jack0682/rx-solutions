use super::Result;
use super::*;
use crate::{model::*, native::*};
use rx_domain::{host_snapshot::SourceObservation, intent::Intent};
use std::path::Path;

const JTC_NOT_CONFIGURED: &str = "JTC_CONTROL_PROVIDER_NOT_CONFIGURED: validated package cannot supply controller authority; no ROS process started";

pub struct Builtin;
pub enum BuiltinAdapter<C: Clock> {
    #[cfg(unix)]
    External(Box<crate::external_process::External<C>>),
    #[cfg(unix)]
    Python(Box<crate::python_skill::PythonSkill<crate::simulation::FileDevice<C>, C>>),
    File(crate::simulation::FileDevice<C>),
    Dynamixel(Box<crate::dynamixel::Dynamixel<C>>),
    Melsec(Box<crate::melsec::Melsec<C>>),
}
macro_rules! delegate {
    ($self:ident,$method:ident($($arg:expr),*)) => {
        match $self {#[cfg(unix)] Self::External(v)=>v.$method($($arg),*), #[cfg(unix)] Self::Python(v)=>v.$method($($arg),*), Self::File(v)=>v.$method($($arg),*),Self::Melsec(v)=>v.$method($($arg),*),Self::Dynamixel(v)=>v.$method($($arg),*)}
    };
}
impl<C: Clock> NativeAdapter for BuiltinAdapter<C> {
    fn begin_with_context(
        &mut self,
        op: &Id,
        inv: &Id,
        intent: &Intent,
        ctx: &NativeDispatch,
    ) -> crate::Result<NativeSubmission> {
        delegate!(self, begin_with_context(op, inv, intent, ctx))
    }
    fn completed(&mut self) -> crate::Result<Vec<NativeCompletion>> {
        delegate!(self, completed())
    }
    fn acknowledge_completion(&mut self, op: &Id) {
        delegate!(self, acknowledge_completion(op))
    }

    fn submit_with_context(
        &mut self,
        op: &Id,
        inv: &Id,
        intent: &Intent,
        ctx: &NativeDispatch,
    ) -> crate::Result<NativeCapture> {
        delegate!(self, submit_with_context(op, inv, intent, ctx))
    }
    fn prepare_shutdown(&mut self, r: &[Name]) -> crate::Result<()> {
        delegate!(self, prepare_shutdown(r))
    }
    fn environment(&self) -> Environment {
        delegate!(self, environment())
    }
    fn protection(&self) -> Arc<dyn LocalProtection> {
        delegate!(self, protection())
    }
    fn observe_sources(
        &self,
        cell: &Name,
        sources: &[Name],
    ) -> crate::Result<Vec<SourceObservation>> {
        delegate!(self, observe_sources(cell, sources))
    }
    fn guard(&self, intent: &Intent, now: &TimePoint) -> crate::Result<Guard> {
        delegate!(self, guard(intent, now))
    }
    fn guard_with_input(
        &self,
        intent: &Intent,
        now: &TimePoint,
        input: Option<&BoundInput>,
    ) -> crate::Result<Guard> {
        delegate!(self, guard_with_input(intent, now, input))
    }
    fn lookup_with_input(
        &mut self,
        operation: &Id,
        invocation: &Id,
        intent: &Intent,
        input: Option<&BoundInput>,
    ) -> crate::Result<Option<NativeCapture>> {
        delegate!(
            self,
            lookup_with_input(operation, invocation, intent, input)
        )
    }
    fn can_handover(&self, resources: &[Name]) -> bool {
        delegate!(self, can_handover(resources))
    }
    fn handover_snapshot(&self, resources: &[Name]) -> crate::Result<LocalHandover> {
        delegate!(self, handover_snapshot(resources))
    }
    fn shutdown_snapshot(&self, resources: &[Name]) -> crate::Result<NativeShutdown> {
        delegate!(self, shutdown_snapshot(resources))
    }
    fn submit(
        &mut self,
        op: &Id,
        invocation: &Id,
        intent: &Intent,
    ) -> crate::Result<NativeCapture> {
        delegate!(self, submit(op, invocation, intent))
    }
    fn lookup(&mut self, op: &Id, invocation: &Id) -> crate::Result<Option<NativeCapture>> {
        delegate!(self, lookup(op, invocation))
    }
}
impl<C: Clock + Clone + 'static> AdapterFactory<C> for Builtin {
    type Adapter = BuiltinAdapter<C>;
    fn execution_templates(
        &self,
        backend: &Backend,
    ) -> Result<Vec<crate::execution_package::Templates>> {
        #[cfg(unix)]
        if matches!(backend, Backend::PythonExecutionPackage { .. }) {
            return Ok(vec![python_execution_package::load(backend)?.1.templates]);
        }
        #[cfg(unix)]
        if let Backend::ExternalProcessPackage { registry, adapter } = backend {
            return Ok(vec![external_package::load(registry, adapter)?.1.templates]);
        }
        Ok(vec![])
    }
    fn validate(&self, backend: &Backend, bindings: &[Binding]) -> Result<()> {
        match backend {
            #[cfg(unix)]
            Backend::ExternalProcessPackage { registry, adapter } => {
                external_package::load(registry, adapter)?
                    .1
                    .validate_bindings(bindings)
            }
            #[cfg(unix)]
            Backend::PythonExecutionPackage { .. } => {
                python_execution_package::load(backend)?
                    .1
                    .validate_bindings(bindings)?;
                python_skill::release()?;
                Ok(())
            }

            #[cfg(unix)]
            Backend::PythonSkillLibraryPackage { .. } => {
                let (_, library) = python_library::load(backend)?;
                library.validate_bindings(bindings)?;
                python_skill::release()?;
                Ok(())
            }
            #[cfg(unix)]
            Backend::PythonSkillSimulation { .. } | Backend::PythonSkillPackage { .. } => {
                let (_, registration) = python_skill::load(backend)?;
                registration.validate_bindings(bindings)?;
                python_skill::release()?;
                Ok(())
            }
            Backend::FileSimulation
                if bindings
                    .iter()
                    .all(|b| b.environment == Environment::Simulation) =>
            {
                Ok(())
            }
            Backend::ValidatedDriver { .. } => {
                crate::dynamixel::profile::validate(backend, bindings)?;
                crate::dynamixel::profile::executable_pin(Path::new("/opt/rx"))?;
                Ok(())
            }
            Backend::MelsecPackage { .. } => {
                device_package::load(backend)?.validate_bindings(bindings)
            }
            Backend::JtcPackage { .. } => jtc_package::load(backend)?.validate_bindings(bindings),
            _ => Err("no matching release-owned backend for these bindings".into()),
        }
    }
    fn initialize_metadata(
        &self,
        backend: &Backend,
        data: &Path,
    ) -> Result<Option<NativeInstallation>> {
        match backend {
            #[cfg(unix)]
            Backend::ExternalProcessPackage { registry, adapter } => {
                let (registration_digest, package) = external_package::load(registry, adapter)?;
                let device_session =
                    crate::external_process::initialize(&data.join("native-external"))?;
                Ok(Some(NativeInstallation::ExternalProcess {
                    registration_digest,
                    program_digest: package.profile.program.sha256,
                    device_session,
                }))
            }
            #[cfg(unix)]
            Backend::PythonExecutionPackage { .. } => {
                let (registration_digest, p) = python_execution_package::load(backend)?;
                std::fs::create_dir(data.join("native-python"))?;
                Ok(Some(NativeInstallation::PythonExecution {
                    registration_digest,
                    environment_digest: p.profile.environment_digest,
                }))
            }

            #[cfg(unix)]
            Backend::PythonSkillLibraryPackage { .. } => {
                let (registration_digest, library) = python_library::load(backend)?;
                std::fs::create_dir(data.join("native-python"))?;
                Ok(Some(NativeInstallation::PythonSkillLibrary {
                    registration_digest,
                    environment_digest: library.environment_digest,
                }))
            }
            #[cfg(unix)]
            Backend::PythonSkillSimulation { .. } | Backend::PythonSkillPackage { .. } => {
                let (registration_digest, registration) = python_skill::load(backend)?;
                std::fs::create_dir(data.join("native-python"))?;
                Ok(Some(NativeInstallation::PythonSkill {
                    registration_digest,
                    environment_digest: registration.environment_digest,
                }))
            }
            Backend::FileSimulation => Ok(None),
            Backend::ValidatedDriver { .. } => Ok(Some(NativeInstallation::Dynamixel {
                identity: crate::dynamixel::initialize(&data.join("native-dynamixel"))?,
            })),
            Backend::JtcPackage { .. } => {
                let p = jtc_package::load(backend)?;
                let identity =
                    crate::ros_jtc::initialize_journal(&data.join("native-jtc"), &p.profile)?;
                Ok(Some(NativeInstallation::Jtc {
                    identity,
                    manifest_digest: p.manifest_digest,
                }))
            }
            Backend::MelsecPackage { .. } => {
                let p = device_package::load(backend)?;
                let identity = crate::melsec::Melsec::<C>::initialize(
                    &data.join("native-melsec"),
                    &p.profile,
                )?;
                Ok(Some(NativeInstallation::Melsec {
                    identity,
                    manifest_digest: p.manifest_digest,
                }))
            }
        }
    }
    fn open_passive(&self, backend: &Backend, data: &Path, clock: C) -> Result<Self::Adapter> {
        // An unconfigured controller provider is refused regardless of installation state.
        if let Backend::JtcPackage { .. } = backend {
            return Err(JTC_NOT_CONFIGURED.into());
        }
        let descriptor = read_installation(data)?;
        let native_root = native_storage_root(data, &descriptor)?;
        match backend {
            #[cfg(unix)]
            Backend::ExternalProcessPackage { registry, adapter } => {
                let (registration_digest, package) = external_package::load(registry, adapter)?;
                let Some(NativeInstallation::ExternalProcess {
                    registration_digest: stored,
                    program_digest,
                    device_session,
                }) = descriptor.native
                else {
                    return Err("external registry installation metadata missing".into());
                };
                if registration_digest != stored || program_digest != package.profile.program.sha256
                {
                    return Err("external selected package differs from installed identity".into());
                }
                let native = crate::external_process::External::open(
                    package.profile,
                    package.program,
                    native_root.join("native-external"),
                    clock,
                )?;
                if native.device_session() != &device_session {
                    return Err("external device generation differs".into());
                }
                Ok(BuiltinAdapter::External(Box::new(native)))
            }
            #[cfg(unix)]
            Backend::PythonExecutionPackage { .. } => {
                let (registration_digest, p) = python_execution_package::load(backend)?;
                let Some(NativeInstallation::PythonExecution {
                    registration_digest: stored,
                    environment_digest,
                }) = descriptor.native
                else {
                    return Err("Python execution installation metadata missing".into());
                };
                if stored != registration_digest
                    || environment_digest != p.profile.environment_digest
                {
                    return Err("Python execution installation differs".into());
                }
                Ok(BuiltinAdapter::Python(Box::new(
                    crate::python_skill::PythonSkill::open_execution(
                        python_skill::release()?,
                        p.profile,
                        native_root.join("native-python"),
                        crate::simulation::FileDevice::open(
                            native_root.join("device"),
                            clock.clone(),
                        )?,
                        clock,
                    )?,
                )))
            }
            #[cfg(unix)]
            Backend::PythonSkillLibraryPackage { .. } => {
                let (digest, library) = python_library::load(backend)?;
                let Some(NativeInstallation::PythonSkillLibrary {
                    registration_digest,
                    environment_digest,
                }) = descriptor.native
                else {
                    return Err("Python library native installation identity missing".into());
                };
                if digest != registration_digest
                    || library.environment_digest != environment_digest
                    || library.installation != descriptor.installation
                {
                    return Err("Python library changed after Host initialization".into());
                }
                let support = crate::simulation::FileDevice::open(
                    native_root.join("python-support"),
                    clock.clone(),
                )?;
                Ok(BuiltinAdapter::Python(Box::new(
                    crate::python_skill::PythonSkill::open_library(
                        python_skill::release()?,
                        library.programs(),
                        native_root.join("native-python"),
                        support,
                        clock,
                    )?,
                )))
            }
            #[cfg(unix)]
            Backend::PythonSkillSimulation { .. } | Backend::PythonSkillPackage { .. } => {
                let (digest, registration) = python_skill::load(backend)?;
                let installation = descriptor;
                let Some(NativeInstallation::PythonSkill {
                    registration_digest,
                    environment_digest,
                }) = installation.native
                else {
                    return Err("Python native installation identity missing".into());
                };
                if digest != registration_digest
                    || registration.environment_digest != environment_digest
                    || registration.installation != installation.installation
                {
                    return Err("Python registration changed after Host initialization".into());
                }
                let support = crate::simulation::FileDevice::open(
                    native_root.join("python-support"),
                    clock.clone(),
                )?;
                Ok(BuiltinAdapter::Python(Box::new(
                    crate::python_skill::PythonSkill::open(
                        python_skill::release()?,
                        registration.program(),
                        native_root.join("native-python"),
                        support,
                        clock,
                    )?,
                )))
            }
            Backend::JtcPackage { .. } => Err(JTC_NOT_CONFIGURED.into()),
            Backend::ValidatedDriver { .. } => {
                let installation = descriptor;
                let Some(NativeInstallation::Dynamixel { identity }) = installation.native else {
                    return Err("DXL_NATIVE_IDENTITY_MISSING".into());
                };
                let pin = crate::dynamixel::profile::executable_pin(Path::new("/opt/rx"))?;
                Ok(BuiltinAdapter::Dynamixel(Box::new(
                    crate::dynamixel::Dynamixel::open(
                        &native_root.join("native-dynamixel"),
                        identity,
                        pin,
                        clock,
                    )?,
                )))
            }
            Backend::FileSimulation => Ok(BuiltinAdapter::File(
                crate::simulation::FileDevice::open(native_root.join("device"), clock)?,
            )),
            Backend::MelsecPackage { .. } => {
                let p = device_package::load(backend)?;
                let Some(NativeInstallation::Melsec {
                    identity,
                    manifest_digest,
                }) = descriptor.native
                else {
                    return Err("native installation identity absent".into());
                };
                if p.profile.installation != descriptor.installation
                    || p.manifest_digest != manifest_digest
                {
                    return Err("native package/installation changed".into());
                }
                Ok(BuiltinAdapter::Melsec(Box::new(
                    crate::melsec::Melsec::open(
                        &native_root.join("native-melsec"),
                        &identity,
                        p.profile,
                        clock,
                    )?,
                )))
            }
        }
    }
}
