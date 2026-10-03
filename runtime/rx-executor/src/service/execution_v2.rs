use super::*;
use crate::worker::{Coordination, Outcome, RequestBasis};
impl<R: Repository, F: PlannerFactory> RunService<R, F> {
    pub(super) async fn execution_cycle(&mut self) -> Result<Option<StopReason>, Error> {
        self.worker.negotiate_execution().await?;
        let view = self.worker.production_view().await?;
        if view.data().resolved.schema_id.as_str() != rx_process_contract::execution_v2::PLAN_SCHEMA
        {
            return Err(Error::Invalid("explicit v2 mode requires Plan2".into()));
        }
        if let Some(context) = self.context.as_ref().or(self.last_context.as_ref())
            && view.identity(context.visit) != *context
        {
            return Ok(Some(StopReason::ContextChanged));
        }
        match self
            .worker
            .coordinate(&view, self.context.as_ref().map(|c| c.visit), true)
            .await?
        {
            Coordination::Waiting => {
                return waiting_production(&mut self.status, self.context.is_some(), &view);
            }
            Coordination::Finished => return Ok(Some(StopReason::Completed)),
            Coordination::Attention => return Ok(Some(StopReason::WorkerFault)),
            Coordination::Retire(_) => {
                self.last_context = self.context.take();
                self.status.phase = Phase::WaitingPart;
                return Ok(None);
            }
            Coordination::Deferred(v) => return outcome(v),
            Coordination::Visit(v) => {
                self.visit = v;
                self.status.visit = v;
            }
        }
        let snapshot = self.worker.execution_snapshot(self.visit).await?;
        let identity = snapshot.context_identity();
        if self.context.as_ref().is_some_and(|c| c != &identity) {
            return Ok(Some(StopReason::ContextChanged));
        }
        self.context = Some(identity);
        self.status.sequence = Some(snapshot.data().context.sequence);
        self.status.admission = snapshot.data().context.request_admission_allowed;
        self.status.phase = match snapshot.frontier().state {
            rx_process_contract::frontier::State::Completed => Phase::GraphComplete,
            rx_process_contract::frontier::State::Failed => Phase::GraphFailed,
            rx_process_contract::frontier::State::Blocked => Phase::Attention,
            _ => Phase::Running,
        };
        outcome(self.worker.handle_execution_snapshot(&snapshot).await?)
    }
}
fn outcome(value: Outcome) -> Result<Option<StopReason>, Error> {
    if matches!(value, Outcome::RefreshRequired) {
        return Err(Error::Expired);
    }
    Ok(match value {
        Outcome::ContextChanged => Some(StopReason::ContextChanged),
        Outcome::Attention(_) | Outcome::Unsupported(_) => Some(StopReason::WorkerFault),
        _ => None,
    })
}
