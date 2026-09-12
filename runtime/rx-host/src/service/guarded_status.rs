//! Optional supervisor observations. Heartbeats describe this process, never P admission.
use rx_service_status::{GuardedState, Reporter};
use std::{sync::Arc, time::Duration};
use tokio::sync::{Mutex, watch};

type Result<T> = std::result::Result<T, String>;
pub(super) struct Heartbeat {
    reporter: Arc<Mutex<Reporter>>,
    pub(super) state: watch::Sender<GuardedState>,
    failure_sender: watch::Sender<Option<String>>,
    failure: watch::Receiver<Option<String>>,
    on_failure: Arc<dyn Fn() + Send + Sync>,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl Heartbeat {
    pub(super) fn start(
        mut reporter: Reporter,
        on_failure: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self> {
        reporter
            .publish(GuardedState::Starting)
            .map_err(|e| e.to_string())?;
        let reporter = Arc::new(Mutex::new(reporter));
        let (state, states) = watch::channel(GuardedState::Starting);
        let (failure_sender, failure) = watch::channel(None);
        let on_failure: Arc<dyn Fn() + Send + Sync> = Arc::new(on_failure);
        let target = reporter.clone();
        let failed = failure_sender.clone();
        let callback = on_failure.clone();
        let task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(200));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                // Read the latest state only after acquiring the sole writer lock.
                let mut writer = target.lock().await;
                let current = *states.borrow();
                if let Err(error) = writer.publish(current) {
                    failed.send_replace(Some(error.to_string()));
                    callback();
                    break;
                }
            }
        });
        Ok(Self {
            reporter,
            state,
            failure_sender,
            failure,
            on_failure,
            task: Some(task),
        })
    }
    pub(super) fn failure(&self) -> Option<String> {
        self.failure.borrow().clone()
    }
    pub(super) async fn publish(&mut self, state: GuardedState) -> Result<()> {
        self.state.send_replace(state);
        if let Some(error) = self.failure() {
            return Err(error);
        }
        let result = {
            let mut writer = self.reporter.lock().await;
            writer
                .publish(*self.state.borrow())
                .map(|_| ())
                .map_err(|e| e.to_string())
        };
        if let Err(error) = &result {
            self.failure_sender.send_replace(Some(error.clone()));
            (self.on_failure)();
        }
        result
    }
    pub(super) async fn finish(&mut self, state: GuardedState) -> Result<()> {
        // Join only the local writer task. This never cancels a service/native child.
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
        let prior_failure = self.failure();
        let state = if prior_failure.is_some() {
            GuardedState::Attention
        } else {
            state
        };
        self.state.send_replace(state);
        let result = self
            .reporter
            .lock()
            .await
            .publish(state)
            .map(|_| ())
            .map_err(|e| e.to_string());
        if let Err(error) = &result {
            self.failure_sender.send_replace(Some(error.clone()));
            (self.on_failure)();
        }
        result?;
        match prior_failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}
impl Drop for Heartbeat {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
