use super::Result;
use super::*;
use crate::{model::*, native::*};
use rx_domain::{host_snapshot::SourceObservation, intent::Intent};
use std::path::Path;

pub struct Builtin;
pub enum BuiltinAdapter<C: Clock> {
    File(crate::simulation::FileDevice<C>),
    Melsec(Box<crate::melsec::Melsec<C>>),
}
macro_rules! delegate {
    ($self:ident,$method:ident($($arg:expr),*)) => {
        match $self {Self::File(v)=>v.$method($($arg),*),Self::Melsec(v)=>v.$method($($arg),*)}
    };
}
impl<C: Clock> NativeAdapter for BuiltinAdapter<C> {
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
impl<C: Clock + 'static> AdapterFactory<C> for Builtin {
    type Adapter = BuiltinAdapter<C>;
    fn validate(&self, backend: &Backend, bindings: &[Binding]) -> Result<()> {
        match backend {
            Backend::FileSimulation
                if bindings
                    .iter()
                    .all(|b| b.environment == Environment::Simulation) =>
            {
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
            Backend::FileSimulation => Ok(None),
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
            _ => Err("unsupported backend metadata".into()),
        }
    }
    fn open_passive(&self, backend: &Backend, data: &Path, clock: C) -> Result<Self::Adapter> {
        match backend {
            Backend::JtcPackage { .. } => Err("JTC_CONTROL_PROVIDER_NOT_CONFIGURED: validated package cannot supply controller authority; no ROS process started".into()),
            Backend::FileSimulation => Ok(BuiltinAdapter::File(
                crate::simulation::FileDevice::open(data.join("device"), clock)?,
            )),
            Backend::MelsecPackage { .. } => {
                let p = device_package::load(backend)?;
                let descriptor: Installation =
                    canonical::decode_json(&fs::read(data.join("installation.json"))?)?;
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
                        &data.join("native-melsec"),
                        &identity,
                        p.profile,
                        clock,
                    )?,
                )))
            }
            _ => Err("unsupported backend".into()),
        }
    }
}
