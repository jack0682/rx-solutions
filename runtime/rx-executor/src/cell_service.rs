//! Resident ownership of one durable Run attachment under one pinned cell/session.
//! Discovery observes P starts; it never starts work or supplies admission authority.
mod policy;

use crate::{
    Client, Error,
    assignment_journal::{AssignmentJournal, Preparation, RecoveredFile},
    clock::Clock,
    journal::Journal,
    lifecycle::{StopPhase, StopReason},
    service::{self, CoordinationMode, PlannerFactory, RunService},
    worker::Worker,
};
use rx_domain::types::{Counter, Id};
use rx_storage::SqliteRepository;
use serde::Serialize;
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::watch;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Phase {
    Idle,
    Arming,
    Running,
    Attention,
    Stopped,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Status {
    pub phase: Phase,
    pub session: Id,
    pub run: Option<Id>,
    pub attachment: Option<Id>,
    pub completed_runs: Counter,
    pub active: Option<service::Status>,
    pub detail: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub status: Status,
    /// The original durable stop report is retained even after normal attachment closure.
    pub last_run: Option<service::Report>,
}

/// The caller holds ServiceOwner and required-opens the journal before opening Client.
/// Client and factory move into each RunService and return unchanged on its owned exit.
pub struct CellService<F> {
    client: Option<Client>,
    assignments: AssignmentJournal<SqliteRepository>,
    root: PathBuf,
    factory: Option<F>,
    clock: Arc<dyn Clock>,
    options: service::Options,
    status: Status,
    last_run: Option<service::Report>,
}
impl<F: PlannerFactory> CellService<F> {
    pub fn new(
        client: Client,
        mut assignments: AssignmentJournal<SqliteRepository>,
        root: PathBuf,
        factory: F,
        clock: Arc<dyn Clock>,
        options: service::Options,
    ) -> Result<Self, Error> {
        policy::check_options(&options)?;
        policy::check_pin(assignments.identity(), client.peer_pin())?;
        if !root.is_dir() {
            return Err(Error::Invalid("existing service root required".into()));
        }
        let current = assignments.current()?;
        let status = Status {
            phase: Phase::Idle,
            session: Id::new(client.session_id()).map_err(|e| Error::Invalid(e.to_string()))?,
            run: current
                .as_ref()
                .map(|a| a.value.preparation.scope.run.clone()),
            attachment: current.as_ref().map(|a| a.value.preparation.id.clone()),
            completed_runs: Counter(0),
            active: None,
            detail: None,
        };
        Ok(Self {
            client: Some(client),
            assignments,
            root,
            factory: Some(factory),
            clock,
            options,
            status,
            last_run: None,
        })
    }
    pub fn initial_status(&self) -> Status {
        self.status.clone()
    }

    pub async fn run(
        mut self,
        mut shutdown: watch::Receiver<bool>,
        updates: watch::Sender<Status>,
    ) -> Report {
        let mut last_good = tokio::time::Instant::now();
        let mut delay_ms = self.options.poll_ms;
        loop {
            // There is no synthetic Run/stop intent while merely observing discovery.
            if *shutdown.borrow() || shutdown.has_changed().is_err() {
                policy::observation_shutdown(&mut self.status);
                break;
            }
            let polled = tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        policy::observation_shutdown(&mut self.status);
                        break;
                    }
                    continue;
                },
                result = self.poll() => result,
            };
            match polled {
                Ok(Some((preparation, journal))) => {
                    if let Err(error) = self
                        .run_attachment(preparation, journal, shutdown.clone(), &updates)
                        .await
                    {
                        self.attention(error.to_string());
                        break;
                    }
                    if self.status.phase != Phase::Idle {
                        break;
                    }
                    last_good = tokio::time::Instant::now();
                    delay_ms = self.options.poll_ms;
                }
                Ok(None) => {
                    last_good = tokio::time::Instant::now();
                    delay_ms = self.options.poll_ms;
                    self.status.detail = None;
                }
                Err(error) => {
                    self.status.detail = Some(error.to_string());
                    if !policy::transient(&error)
                        || last_good.elapsed()
                            >= Duration::from_millis(self.options.communication_grace_ms)
                    {
                        self.status.phase = Phase::Attention;
                        break;
                    }
                    delay_ms = delay_ms.saturating_mul(2).min(1000);
                }
            }
            publish(&updates, &self.status);
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(delay_ms)) => {},
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        policy::observation_shutdown(&mut self.status);
                        break;
                    }
                },
            }
        }
        publish(&updates, &self.status);
        Report {
            status: self.status,
            last_run: self.last_run,
        }
    }

    async fn poll(&mut self) -> Result<Option<(Preparation, Journal<SqliteRepository>)>, Error> {
        // An existing attachment fixes the Run even when discovery now says NONE or B.
        if let Some(current) = self.assignments.current()? {
            let preparation = current.value.preparation;
            self.status.run = Some(preparation.scope.run.clone());
            self.status.attachment = Some(preparation.id.clone());
            let view = self
                .client
                .as_mut()
                .expect("idle client")
                .production_view(&preparation.scope.run)
                .await?;
            if !view.is_current() {
                return Err(Error::Expired);
            }
            policy::check_existing(&preparation, view.data())?;
            return self.attach_current(preparation).map(Some);
        }
        let discovery = self
            .client
            .as_mut()
            .expect("idle client")
            .assignment_view()
            .await?;
        if !discovery.is_current() {
            return Err(Error::Expired);
        }
        match policy::discover(discovery.data())? {
            policy::Discovery::Idle => {
                self.status.phase = Phase::Idle;
                self.status.run = None;
                self.status.attachment = None;
                Ok(None)
            }
            policy::Discovery::Arming(run) => {
                self.status.phase = Phase::Arming;
                self.status.run = Some(run);
                Ok(None)
            }
            policy::Discovery::Executing(run) => {
                self.status.run = Some(run.clone());
                let view = self
                    .client
                    .as_mut()
                    .expect("idle client")
                    .production_view(&run)
                    .await?;
                if !discovery.is_current() || !view.is_current() {
                    return Err(Error::Expired);
                }
                let preparation =
                    policy::prepare(self.assignments.identity(), discovery.data(), view.data())?;
                // Generate identities once, preserve the exact Preparation before creating any file.
                self.assignments.prepare(preparation.clone())?;
                self.status.run = Some(preparation.scope.run.clone());
                self.status.attachment = Some(preparation.id.clone());
                self.attach_current(preparation).map(Some)
            }
        }
    }

    fn attach_current(
        &mut self,
        preparation: Preparation,
    ) -> Result<(Preparation, Journal<SqliteRepository>), Error> {
        let recovered = self
            .assignments
            .recover_current_file(&self.root)?
            .ok_or_else(|| Error::Invalid("current attachment disappeared".into()))?;
        let run = match recovered {
            RecoveredFile::NeedsInitialization(record) => {
                policy::same_preparation(&preparation, &record.value.preparation)?;
                self.assignments
                    .initialize_run_file(&self.root, &preparation.id)?
            }
            RecoveredFile::Existing { attachment, run } => {
                policy::same_preparation(&preparation, &attachment.value.preparation)?;
                *run
            }
        };
        // The journal becomes usable only after Attached commits. Required-open verifies
        // creation-entered and the old header; missing/partial files are never reinitialized.
        let (_, mut journal) = self.assignments.attach(run)?;
        if journal.stop_record()?.is_some() {
            return Err(Error::Invalid(
                "existing stop intent requires attention; automatic resume is unsupported".into(),
            ));
        }
        Ok((preparation, journal))
    }

    async fn run_attachment(
        &mut self,
        preparation: Preparation,
        journal: Journal<SqliteRepository>,
        shutdown: watch::Receiver<bool>,
        updates: &watch::Sender<Status>,
    ) -> Result<(), Error> {
        let worker = Worker::new(self.client.take().expect("idle client"), journal)?;
        let run = RunService::new(
            worker,
            self.factory.take().expect("idle factory"),
            self.clock.clone(),
            Counter(1),
            self.options.clone(),
        )?;
        let (run_updates, mut run_status) = watch::channel(run.initial_status());
        self.status.phase = Phase::Running;
        self.status.detail = None;
        self.status.active = Some(run_status.borrow().clone());
        publish(updates, &self.status);
        let finished = run.run_owned(shutdown, run_updates);
        tokio::pin!(finished);
        let mut observing = true;
        let exit = loop {
            tokio::select! {
                result = &mut finished => break result,
                changed = run_status.changed(), if observing => {
                    if changed.is_err() { observing = false; }
                    else { self.status.active = Some(run_status.borrow_and_update().clone()); publish(updates, &self.status); }
                },
            }
        };
        self.status.active = Some(run_status.borrow().clone());
        let (client, mut journal) = exit.worker.into_parts();
        self.client = Some(client);
        self.factory = Some(exit.factory);
        self.last_run = Some(exit.report.clone());
        if exit.report.stop.reason != StopReason::Completed {
            self.status.phase = if exit.report.stop.reason == StopReason::Requested
                && exit.report.phase == StopPhase::PauseObserved
                && exit.report.durability_fault.is_none()
                && exit.planner_cleanup.is_confirmed()
            {
                Phase::Stopped
            } else {
                Phase::Attention
            };
            self.status.detail = exit.report.last_error;
            return Ok(());
        }
        policy::check_exit(
            &preparation,
            &exit.report,
            exit.planner_cleanup,
            journal.stop_record()?.as_ref(),
        )?;
        let view = self
            .client
            .as_mut()
            .expect("returned client")
            .production_view(&preparation.scope.run)
            .await?;
        if !view.is_current() {
            return Err(Error::Expired);
        }
        policy::check_completion(&preparation, view.data())?;
        let mut run = self
            .assignments
            .open_run_required(&preparation.id, journal.into_repository())?;
        if !view.is_current() {
            return Err(Error::Expired);
        }
        self.assignments
            .close_completed(&mut run, view.data().clone())?;
        self.status.completed_runs = Counter(self.status.completed_runs.0 + 1);
        self.status.phase = Phase::Idle;
        self.status.run = None;
        self.status.attachment = None;
        self.status.active = None;
        self.status.detail = None;
        Ok(())
    }
    fn attention(&mut self, detail: String) {
        self.status.phase = Phase::Attention;
        self.status.detail = Some(detail);
    }
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
