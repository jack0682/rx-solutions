//! Run-scoped stop intent, distinct from BT visits and from physical stop/handover.
use crate::{frame::Identity, journal::RunResponse};
use rx_domain::types::*;
use rx_process_contract::execution::RunState;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StopReason {
    Requested,
    Completed,
    PlannerFault,
    WorkerFault,
    StateUnavailable,
    ContextChanged,
    StoreFault,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StopPhase {
    Pending,
    PauseObserved,
    Superseded,
    Attention,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AttemptState {
    Prepared,
    Entered,
    Rejected,
    Replied,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StopAttempt {
    pub key: Id,
    pub session: Id,
    pub expected_run: Counter,
    pub state: AttemptState,
    pub response: Option<RunResponse>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StopRecord {
    pub id: Id,
    pub run: Id,
    pub origin_session: Id,
    pub origin_context: Option<Identity>,
    pub reason: StopReason,
    pub requested_at: Option<TimePoint>,
    pub phase: StopPhase,
    pub attempts: Vec<StopAttempt>,
    pub observation: Option<RunResponse>,
}
impl StopRecord {
    pub fn new(
        run: Id,
        session: Id,
        context: Option<Identity>,
        reason: StopReason,
        requested_at: Option<TimePoint>,
    ) -> Self {
        Self {
            id: Id::new(uuid::Uuid::new_v4().to_string()).expect("UUID"),
            run,
            origin_session: session,
            origin_context: context,
            reason,
            requested_at,
            phase: StopPhase::Pending,
            attempts: vec![],
            observation: None,
        }
    }
}
#[derive(Clone, Debug)]
pub struct VersionedStop {
    pub revision: Counter,
    pub record: StopRecord,
}
pub fn restricted(state: RunState) -> bool {
    matches!(
        state,
        RunState::Paused | RunState::RecoveryRequired | RunState::Completed | RunState::Abandoned
    )
}
