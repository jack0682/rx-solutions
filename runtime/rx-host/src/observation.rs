//! Explicit read-only provider surface. A sample is evidence, never execution permission.
use crate::{model::*, native::*};
use rx_domain::{host_snapshot::SourceObservation, intent::Intent, types::*};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::Arc};
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ValueType {
    Boolean,
    Integer,
    Real,
    Symbol,
    Reals,
}
impl ValueType {
    pub fn matches(&self, value: &TypedValue) -> bool {
        matches!(
            (self, value),
            (Self::Boolean, TypedValue::Boolean(_))
                | (Self::Integer, TypedValue::Integer(_))
                | (Self::Real, TypedValue::Real(_))
                | (Self::Symbol, TypedValue::Symbol(_))
                | (Self::Reals, TypedValue::Reals(_))
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub id: Name,
    pub schema: Name,
    pub unit: Name,
    pub value_type: ValueType,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    pub schema: Name,
    pub sources: Vec<Source>,
}
impl Declaration {
    pub fn validate(&self) -> Result<()> {
        if self.schema.as_str() != "rx.observation-only-binding.v1"
            || self.sources.is_empty()
            || self.sources.len() > 128
            || self
                .sources
                .iter()
                .map(|s| &s.id)
                .collect::<BTreeSet<_>>()
                .len()
                != self.sources.len()
            || self
                .sources
                .iter()
                .any(|s| s.id.as_str().starts_with("handover/"))
        {
            return Err(HostError::Invalid(
                "observation-only declaration differs".into(),
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct StopObservation {
    pub generation: Id,
    pub observed_at: TimePoint,
    pub uncertainty_ns: Counter,
    /// The provider has stopped its read activity and its owned IO can be dropped.
    /// This says nothing about a downstream actuator's physical support.
    pub stopped: bool,
}
pub trait Provider: Send {
    fn environment(&self) -> Environment;
    fn protection(&self) -> Arc<dyn LocalProtection>;
    fn observe(&self, cell: &Name, sources: &[Name]) -> Result<Vec<SourceObservation>>;
    fn stop(&mut self) -> Result<()>;
    fn stop_observation(&self) -> Result<StopObservation>;
}
/// Compile-time facade: external source authors implement no submit/lookup/control hooks.
pub struct Adapter<P: Provider> {
    provider: P,
    stop_requested: bool,
    stop_accepted: bool,
}
impl<P: Provider> Adapter<P> {
    pub fn new(provider: P) -> Self {
        Self {
            provider,
            stop_requested: false,
            stop_accepted: false,
        }
    }
}
impl<P: Provider> NativeAdapter for Adapter<P> {
    fn environment(&self) -> Environment {
        self.provider.environment()
    }
    fn protection(&self) -> Arc<dyn LocalProtection> {
        self.provider.protection()
    }
    fn observe_sources(&self, cell: &Name, sources: &[Name]) -> Result<Vec<SourceObservation>> {
        if self.stop_requested {
            return Err(HostError::Guard);
        }
        self.provider.observe(cell, sources)
    }
    fn guard(&self, _: &Intent, _: &TimePoint) -> Result<Guard> {
        Err(HostError::Forbidden)
    }
    fn can_handover(&self, _: &[Name]) -> bool {
        false
    }
    fn submit(&mut self, _: &Id, _: &Id, _: &Intent) -> Result<NativeCapture> {
        Err(HostError::Forbidden)
    }
    fn lookup(&mut self, _: &Id, _: &Id) -> Result<Option<NativeCapture>> {
        Err(HostError::Forbidden)
    }
    fn prepare_shutdown(&mut self, resources: &[Name]) -> Result<()> {
        if !resources.is_empty() {
            return Err(HostError::Forbidden);
        }
        self.stop_requested = true;
        if !self.stop_accepted {
            self.provider.stop()?;
            self.stop_accepted = true;
        }
        Ok(())
    }
    fn shutdown_snapshot(&self, resources: &[Name]) -> Result<NativeShutdown> {
        if !resources.is_empty() {
            return Err(HostError::Forbidden);
        }
        let state = self.provider.stop_observation()?;
        Ok(NativeShutdown {
            device_session: state.generation,
            observed_at: state.observed_at,
            uncertainty_ns: state.uncertainty_ns,
            resources: vec![],
            no_pending_commands: true,
            safe_to_drop: state.stopped,
        })
    }
}
