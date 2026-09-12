//! One assigned run/visit service. P still owns starts, parts, outcomes and restart authority.
use crate::{
    Error, ValidatedSnapshot,
    clock::Clock,
    engine_process::{self, EngineProcess, Executable},
    frame::Identity,
    lifecycle::{StopPhase, StopReason},
    pending::{PendingRequests, StopState},
    worker::{StopController, Worker},
};
use rx_domain::types::{Counter, Id};
use rx_ports::Repository;
use rx_process_contract::execution::RunState;
use serde::{Deserialize, Serialize};
use std::{
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::watch;

#[tonic::async_trait]
pub trait PlannerFactory: Send {
    async fn spawn(
        &mut self,
        snapshot: &ValidatedSnapshot,
    ) -> Result<EngineProcess, engine_process::Error>;
}
pub struct PinnedPlanner(pub Executable);
#[tonic::async_trait]
impl PlannerFactory for PinnedPlanner {
    async fn spawn(
        &mut self,
        snapshot: &ValidatedSnapshot,
    ) -> Result<EngineProcess, engine_process::Error> {
        EngineProcess::spawn(
            Executable {
                path: self.0.path.clone(),
                sha256: self.0.sha256,
            },
            snapshot,
        )
        .await
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CoordinationMode {
    ManualVisit,
    #[default]
    SerialProduction,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub coordination: CoordinationMode,
    pub poll_ms: u64,
    pub communication_grace_ms: u64,
    pub stop_timeout_ms: u64,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            coordination: CoordinationMode::SerialProduction,
            poll_ms: 50,
            communication_grace_ms: 5000,
            stop_timeout_ms: 10000,
        }
    }
}
impl Options {
    fn validate(&self) -> Result<(), Error> {
        if !(10..=1000).contains(&self.poll_ms)
            || !(100..=60000).contains(&self.communication_grace_ms)
            || !(100..=60000).contains(&self.stop_timeout_ms)
        {
            return Err(Error::Invalid("service timing options out of range".into()));
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Phase {
    WaitingAssignment,
    WaitingPart,
    Running,
    GraphComplete,
    GraphFailed,
    Degraded,
    Stopping,
    Stopped,
    Attention,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Status {
    pub phase: Phase,
    pub session: Id,
    pub run: Id,
    pub visit: Counter,
    pub sequence: Option<Counter>,
    pub admission: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub phase: StopPhase,
    pub stop_id: Id,
    pub durability_fault: Option<String>,
    pub last_error: Option<String>,
    pub stop: crate::lifecycle::StopRecord,
}
/// Local planner process cleanup only; never Host stop or physical handover proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PlannerCleanup {
    /// This service never attempted to create a planner process.
    NotStarted,
    /// Every created planner completed the validated CLOSE/child-exit path.
    Confirmed,
    /// A spawn/cleanup failed or was cancelled while child ownership was uncertain.
    Unconfirmed,
}
impl PlannerCleanup {
    pub fn is_confirmed(self) -> bool {
        matches!(self, Self::NotStarted | Self::Confirmed)
    }
}
/// Ownership returned by a completed run loop. Reassignment requires checking cleanup
/// in addition to the caller's current P state, session and durable assignment record.
pub struct ServiceExit<R, F> {
    pub report: Report,
    pub worker: Worker<R>,
    pub factory: F,
    pub planner_cleanup: PlannerCleanup,
}
#[derive(Default)]
struct PlannerLifecycle {
    attempted: bool,
    outstanding: bool,
    failed: bool,
}
struct CleanupAttempt<'a> {
    lifecycle: &'a mut PlannerLifecycle,
    confirmed: bool,
}
impl Drop for CleanupAttempt<'_> {
    fn drop(&mut self) {
        if !self.confirmed {
            self.lifecycle.failed = true;
        }
    }
}
impl PlannerLifecycle {
    fn starting(&mut self) {
        self.attempted = true;
        // Record before awaiting spawn: cancellation can happen after a child exists
        // but before EngineProcess is returned to the service.
        self.outstanding = true;
    }
    fn spawn_failed(&mut self) {
        self.failed = true;
    }
    async fn finish<T>(
        &mut self,
        cleanup: impl Future<Output = Result<T, engine_process::Error>>,
    ) -> Result<T, engine_process::Error> {
        let mut attempt = CleanupAttempt {
            lifecycle: self,
            confirmed: false,
        };
        let result = cleanup.await;
        if result.is_ok() {
            attempt.lifecycle.outstanding = false;
            attempt.confirmed = true;
        }
        // Error or cancellation drops an unconfirmed attempt and latches failed.
        // Empty engine slots and unrelated last_error changes cannot clear it.
        result
    }
    fn result(&self) -> PlannerCleanup {
        if self.outstanding || self.failed {
            PlannerCleanup::Unconfirmed
        } else if self.attempted {
            PlannerCleanup::Confirmed
        } else {
            PlannerCleanup::NotStarted
        }
    }
}
#[cfg(feature = "test-harness")]
type StopHook = Arc<dyn Fn(&StopController) + Send + Sync>;
pub struct RunService<R, F> {
    worker: Worker<R>,
    factory: F,
    clock: Arc<dyn Clock>,
    visit: Counter,
    options: Options,
    context: Option<Identity>,
    last_context: Option<Identity>,
    engine: Option<EngineProcess>,
    planner_lifecycle: PlannerLifecycle,
    pending: Option<PendingRequests>,
    status: Status,
    #[cfg(feature = "test-harness")]
    stop_hook: Option<StopHook>,
}
impl<R: Repository, F: PlannerFactory> RunService<R, F> {
    pub fn new(
        mut worker: Worker<R>,
        factory: F,
        clock: Arc<dyn Clock>,
        visit: Counter,
        options: Options,
    ) -> Result<Self, Error> {
        options.validate()?;
        if visit.0 == 0 {
            return Err(Error::Invalid("positive visit required".into()));
        }
        let status = Status {
            phase: Phase::WaitingAssignment,
            session: worker.session_id(),
            run: worker.journal().scope().run.clone(),
            visit,
            sequence: None,
            admission: false,
        };
        Ok(Self {
            worker,
            factory,
            clock,
            visit,
            options,
            context: None,
            last_context: None,
            engine: None,
            planner_lifecycle: PlannerLifecycle::default(),
            pending: None,
            status,
            #[cfg(feature = "test-harness")]
            stop_hook: None,
        })
    }
    pub fn initial_status(&self) -> Status {
        self.status.clone()
    }
    #[cfg(feature = "test-harness")]
    pub fn with_stop_hook(mut self, hook: StopHook) -> Self {
        self.stop_hook = Some(hook);
        self
    }
    fn request_stop(&mut self, reason: StopReason) -> StopController {
        if let Some(pending) = &mut self.pending {
            pending.request_pause(Instant::now());
        }
        self.status.admission = false;
        self.status.phase = Phase::Stopping;
        let control = self.worker.request_stop(
            self.context.clone().or_else(|| self.last_context.clone()),
            reason,
            self.clock.now().ok(),
        );
        #[cfg(feature = "test-harness")]
        if let Some(hook) = &self.stop_hook {
            hook(&control);
        }
        control
    }
    pub async fn run(
        self,
        shutdown: watch::Receiver<bool>,
        updates: watch::Sender<Status>,
    ) -> Report {
        self.run_owned(shutdown, updates).await.report
    }
    /// Finish the existing stop flow and return the unchanged Client/journal/factory
    /// ownership for a resident caller. Cleanup confirmation is a separate result.
    pub async fn run_owned(
        mut self,
        mut shutdown: watch::Receiver<bool>,
        updates: watch::Sender<Status>,
    ) -> ServiceExit<R, F> {
        let mut stop = match self.worker.recovered_stop() {
            Ok(value) => value,
            Err(_) => Some(self.request_stop(StopReason::StoreFault)),
        };
        let mut last_good = Instant::now();
        let mut last_error = None;
        loop {
            if let Some(mut control) = stop.take() {
                self.status.phase = Phase::Stopping;
                self.status.admission = false;
                publish(&updates, &self.status);
                // Only the planner is closed. Host/controller lifetime is owned elsewhere.
                if let Some(engine) = self.engine.take()
                    && let Err(error) = self.planner_lifecycle.finish(engine.close()).await
                {
                    last_error = Some(error.to_string());
                }
                let deadline = tokio::time::Instant::now()
                    + Duration::from_millis(self.options.stop_timeout_ms);
                while control.record.phase == StopPhase::Pending {
                    let result = tokio::select! {
                        _=tokio::time::sleep_until(deadline)=>break,
                        result=self.worker.stop_once(&mut control)=>result,
                    };
                    if let Err(error) = result {
                        last_error = Some(error.to_string());
                    }
                    if control.record.phase == StopPhase::Pending {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
                self.status.phase = if matches!(
                    control.record.phase,
                    StopPhase::PauseObserved | StopPhase::Superseded
                ) {
                    Phase::Stopped
                } else {
                    Phase::Attention
                };
                publish(&updates, &self.status);
                return ServiceExit {
                    report: Report {
                        phase: control.record.phase,
                        stop_id: control.record.id.clone(),
                        durability_fault: control.durability_fault,
                        last_error,
                        stop: control.record.clone(),
                    },
                    planner_cleanup: self.planner_lifecycle.result(),
                    worker: self.worker,
                    factory: self.factory,
                };
            }
            if *shutdown.borrow() {
                stop = Some(self.request_stop(StopReason::Requested));
                continue;
            }
            let result = tokio::select! {
                biased;
                changed=shutdown.changed()=>{if changed.is_err() || *shutdown.borrow() {stop=Some(self.request_stop(StopReason::Requested));} continue;},
                result=self.cycle()=>result,
            };
            match result {
                Ok(Some(reason)) => stop = Some(self.request_stop(reason)),
                Ok(None) => last_good = Instant::now(),
                Err(error) => {
                    let retryable = transient(&error);
                    last_error = Some(error.to_string());
                    self.status.phase = Phase::Degraded;
                    self.status.admission = false;
                    if !retryable
                        || last_good.elapsed()
                            >= Duration::from_millis(self.options.communication_grace_ms)
                    {
                        stop = Some(self.request_stop(if retryable {
                            StopReason::StateUnavailable
                        } else {
                            StopReason::WorkerFault
                        }));
                    }
                }
            }
            publish(&updates, &self.status);
            if stop.is_none() {
                tokio::select! { _=tokio::time::sleep(Duration::from_millis(self.options.poll_ms))=>{}, changed=shutdown.changed()=>{if changed.is_err() || *shutdown.borrow(){stop=Some(self.request_stop(StopReason::Requested));}} }
            }
        }
    }
    async fn cycle(&mut self) -> Result<Option<StopReason>, Error> {
        if self.engine.is_none() && !self.planner_lifecycle.result().is_confirmed() {
            // A cancelled spawn or serial retirement may already have consumed the
            // local EngineProcess. Its absence cannot authorize another planner.
            return Ok(Some(StopReason::PlannerFault));
        }
        if self.options.coordination == CoordinationMode::SerialProduction {
            let view = self.worker.production_view().await?;
            if let Some(context) = self.context.as_ref().or(self.last_context.as_ref())
                && view.identity(context.visit) != *context
            {
                return Ok(Some(StopReason::ContextChanged));
            }
            let result = self
                .worker
                .coordinate(
                    &view,
                    self.context.as_ref().map(|c| c.visit),
                    self.status.phase == Phase::GraphComplete,
                )
                .await?;
            match result {
                crate::worker::Coordination::Waiting => {
                    return waiting_production(&mut self.status, self.context.is_some(), &view);
                }
                crate::worker::Coordination::Visit(visit) => {
                    self.visit = visit;
                    self.status.visit = visit;
                }
                crate::worker::Coordination::Retire(proof) => {
                    if let Some(engine) = self.engine.take() {
                        self.planner_lifecycle
                            .finish(engine.retire(proof))
                            .await
                            .map_err(|e| Error::Invalid(e.to_string()))?;
                    }
                    self.last_context = self.context.take();
                    self.pending = None;
                    self.status.phase = Phase::WaitingPart;
                    return Ok(None);
                }
                crate::worker::Coordination::Finished => return Ok(Some(StopReason::Completed)),
                crate::worker::Coordination::Attention => return Ok(Some(StopReason::WorkerFault)),
                crate::worker::Coordination::Deferred(outcome) => {
                    return Ok(
                        if matches!(
                            outcome,
                            crate::worker::Outcome::ContextChanged
                                | crate::worker::Outcome::Attention(_)
                                | crate::worker::Outcome::Unsupported(_)
                        ) {
                            Some(StopReason::WorkerFault)
                        } else {
                            None
                        },
                    );
                }
            }
        }
        let view = self.worker.inspect_run().await?;
        if view.state != RunState::Executing
            || view.executor_session.as_ref() != Some(&self.worker.session_id())
        {
            self.status.phase = Phase::WaitingAssignment;
            self.status.admission = false;
            return Ok(self.context.as_ref().map(|_| StopReason::ContextChanged));
        }
        let snapshot = match self.worker.snapshot(self.visit).await {
            Ok(value) => value,
            Err(Error::Rpc(status))
                if self.context.is_none()
                    && matches!(
                        status.code(),
                        tonic::Code::InvalidArgument | tonic::Code::FailedPrecondition
                    ) =>
            {
                self.status.phase = Phase::WaitingPart;
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        self.status.sequence = Some(snapshot.data().sequence);
        self.status.admission = snapshot.data().request_admission_allowed;
        if let Some(context) = &self.context {
            if snapshot.context_identity() != *context {
                return Ok(Some(StopReason::ContextChanged));
            }
        } else {
            self.worker.recover(&snapshot)?;
            self.planner_lifecycle.starting();
            let engine = match self.factory.spawn(&snapshot).await {
                Ok(engine) => engine,
                Err(error) => {
                    self.planner_lifecycle.spawn_failed();
                    return Err(Error::Invalid(error.to_string()));
                }
            };
            self.context = Some(snapshot.context_identity());
            self.pending = Some(PendingRequests::new(
                snapshot.context_identity(),
                snapshot.process().root.id.clone(),
            ));
            self.engine = Some(engine);
            return Ok(None); // Fetch a fresh view after process startup.
        }
        self.worker.recover(&snapshot)?;
        let reply = match self
            .engine
            .as_mut()
            .expect("initialized engine")
            .step(&snapshot)
            .await
        {
            Ok(value) => value,
            Err(engine_process::Error::Stale) => return Ok(None),
            Err(_) => return Ok(Some(StopReason::PlannerFault)),
        };
        self.status.phase = match reply.state {
            engine_process::State::Success => Phase::GraphComplete,
            engine_process::State::Failure => return Ok(Some(StopReason::WorkerFault)),
            engine_process::State::Fault => return Ok(Some(StopReason::PlannerFault)),
            _ => Phase::Running,
        };
        let pending = self.pending.as_mut().expect("initialized queue");
        pending.accept(reply.requests, Instant::now())?;
        if let Some(request) = pending.next(Instant::now()) {
            let result = self.worker.handle(request.clone()).await;
            pending.complete(&request, result, Instant::now())?;
        }
        Ok((pending.stop_state() != StopState::Running).then_some(StopReason::WorkerFault))
    }
}
fn waiting_production(
    status: &mut Status,
    has_context: bool,
    view: &crate::client::ValidatedProduction,
) -> Result<Option<StopReason>, Error> {
    // coordinate() records observations before constructing a CompletedVisit. If the read
    // expires during that work, Waiting requests a fresh read, not a context-change stop.
    if !view.is_current() {
        status.admission = false;
        return Err(Error::Expired);
    }
    status.admission = view.data().admission_allowed;
    if has_context && view.data().run.run.state != RunState::Executing {
        return Ok(Some(StopReason::ContextChanged));
    }
    if !has_context {
        status.phase = Phase::WaitingPart;
    }
    Ok(None)
}
fn transient(error: &Error) -> bool {
    matches!(error, Error::Expired)
        || matches!(error,Error::Rpc(status) if matches!(status.code(),tonic::Code::Unavailable|tonic::Code::DeadlineExceeded|tonic::Code::Cancelled|tonic::Code::ResourceExhausted|tonic::Code::Internal|tonic::Code::Unknown))
}

fn publish(sender: &watch::Sender<Status>, next: &Status) {
    sender.send_if_modified(|old| {
        if old == next {
            false
        } else {
            *old = next.clone();
            true
        }
    });
}

#[cfg(test)]
mod tests;
