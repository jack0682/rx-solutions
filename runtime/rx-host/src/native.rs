use crate::model::*;
use rx_domain::{intent::Intent, types::Id};
use std::sync::Arc;

/// Must not share the Host gate or depend on a successful database transaction.
pub trait LocalProtection: Send + Sync {
    fn react(&self, incident: ProtectionIncident);
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct NativeShutdown {
    pub device_session: Id,
    pub observed_at: rx_domain::types::TimePoint,
    pub uncertainty_ns: rx_domain::types::Counter,
    pub resources: Vec<rx_domain::types::Name>,
    pub no_pending_commands: bool,
    pub safe_to_drop: bool,
}
#[derive(Clone, Debug)]
pub struct NativeDispatch {
    pub device_session: Id,
    pub expires_at: rx_domain::types::TimePoint,
}
pub trait NativeAdapter: Send {
    /// Called only after Host admission is closed. May begin passive bridge shutdown once support is proven.
    fn prepare_shutdown(&mut self, _resources: &[rx_domain::types::Name]) -> Result<()> {
        Ok(())
    }
    /// Stronger than stable support: dropping this adapter must not remove required support.
    fn shutdown_snapshot(&self, _resources: &[rx_domain::types::Name]) -> Result<NativeShutdown> {
        Err(HostError::Guard)
    }
    fn environment(&self) -> Environment;
    fn protection(&self) -> Arc<dyn LocalProtection>;
    /// Read-only, explicitly supported native sources. Default cannot synthesize readiness.
    fn observe_sources(
        &self,
        _cell: &rx_domain::types::Name,
        _sources: &[rx_domain::types::Name],
    ) -> Result<Vec<rx_domain::host_snapshot::SourceObservation>> {
        Err(HostError::Guard)
    }
    fn guard(&self, intent: &Intent, now: &rx_domain::types::TimePoint) -> Result<Guard>;
    /// Confirms no residual native command and required physical support/handover.
    fn can_handover(&self, resources: &[rx_domain::types::Name]) -> bool;
    /// Fresh physical/current-state evidence. Unsupported bindings must not synthesize it.
    fn handover_snapshot(&self, _resources: &[rx_domain::types::Name]) -> Result<LocalHandover> {
        Err(HostError::Guard)
    }
    /// Submit exactly once. Return a captured native fact, not a platform outcome.
    fn submit(&mut self, operation: &Id, invocation: &Id, intent: &Intent)
    -> Result<NativeCapture>;
    /// Final Host device session/expiry; network adapters carry this through preflight to dispatch.
    fn submit_with_context(
        &mut self,
        op: &Id,
        inv: &Id,
        intent: &Intent,
        _context: &NativeDispatch,
    ) -> Result<NativeCapture> {
        if self.environment() == Environment::Physical {
            return Err(HostError::Guard);
        }
        self.submit(op, inv, intent)
    }
    /// Read-only lookup; it must not re-submit the original command.
    fn lookup(&mut self, operation: &Id, invocation: &Id) -> Result<Option<NativeCapture>>;
}
/// Tests can stop a real process at the two non-atomic database/native boundaries.
/// No hook is configured from an external request.
pub trait BoundaryHook: Send + Sync {
    fn before_qualification_commit(&self) -> std::result::Result<(), String> {
        Ok(())
    }
    fn after_qualification_commit(&self) {}
    fn before_configuration_commit(&self) -> std::result::Result<(), String> {
        Ok(())
    }
    fn after_configuration_commit(&self) {}
    fn after_send_commit(&self) {}
    fn after_native_entry(&self) {}
}
pub struct NoHooks;
impl BoundaryHook for NoHooks {}
