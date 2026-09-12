use super::*;
use crate::lifecycle::*;
#[derive(Debug)]
pub struct StopController {
    pub record: StopRecord,
    revision: Option<Counter>,
    pub durability_fault: Option<String>,
}
impl<R: Repository> Worker<R> {
    pub fn session_id(&self) -> Id {
        Id::new(self.client.session_id()).expect("validated session")
    }
    pub async fn inspect_run(&mut self) -> Result<RunResponse, Error> {
        let view = self.client.inspect_run(&self.journal.scope().run).await?;
        if view.recipe != self.journal.scope().resolved_digest {
            return Err(Error::Invalid("service recipe changed".into()));
        }
        Ok(view)
    }
    pub fn recovered_stop(&mut self) -> Result<Option<StopController>, Error> {
        Ok(self.journal.stop_record()?.map(|v| StopController {
            record: v.record,
            revision: Some(v.revision),
            durability_fault: None,
        }))
    }
    /// The caller freezes normal planning before this call. Storage failure does not prevent pause.
    pub fn request_stop(
        &mut self,
        context: Option<crate::frame::Identity>,
        reason: StopReason,
        time: Option<TimePoint>,
    ) -> StopController {
        let candidate = StopRecord::new(
            self.journal.scope().run.clone(),
            self.session_id(),
            context,
            reason,
            time,
        );
        match self.journal.ensure_stop(&candidate) {
            Ok(v) => StopController {
                record: v.record,
                revision: Some(v.revision),
                durability_fault: None,
            },
            Err(error) => StopController {
                record: candidate,
                revision: None,
                durability_fault: Some(error.to_string()),
            },
        }
    }
    fn persist_stop(&mut self, control: &mut StopController) {
        if let Some(revision) = control.revision {
            match self.journal.save_stop(revision, &control.record) {
                Ok(next) => control.revision = Some(next),
                Err(error) => {
                    control.revision = None;
                    control.durability_fault = Some(error.to_string());
                }
            }
        }
    }
    pub async fn stop_once(&mut self, control: &mut StopController) -> Result<StopPhase, Error> {
        if control.record.phase != StopPhase::Pending {
            return Ok(control.record.phase);
        }
        let view = self.inspect_run().await?;
        if restricted(view.state) {
            control.record.observation = Some(view);
            control.record.phase = StopPhase::PauseObserved;
            self.persist_stop(control);
            return Ok(control.record.phase);
        }
        if view
            .executor_session
            .as_ref()
            .is_some_and(|s| s != &control.record.origin_session)
        {
            control.record.observation = Some(view);
            control.record.phase = StopPhase::Superseded;
            self.persist_stop(control);
            return Ok(control.record.phase);
        }
        if let Some(origin) = &control.record.origin_context
            && view.state == rx_process_contract::execution::RunState::Executing
        {
            let snapshot = self.snapshot(origin.visit).await?;
            if snapshot.context_identity() != *origin {
                control.record.phase = StopPhase::Attention;
                control.record.observation = Some(view);
                self.persist_stop(control);
                return Ok(control.record.phase);
            }
        }
        let session = self.session_id();
        if control.record.origin_session != session
            || (control.record.attempts.len() >= 64
                && control
                    .record
                    .attempts
                    .last()
                    .is_some_and(|a| a.state == AttemptState::Rejected))
        {
            control.record.phase = StopPhase::Attention;
            self.persist_stop(control);
            return Ok(control.record.phase);
        }
        if control
            .record
            .attempts
            .last()
            .is_none_or(|a| a.state == AttemptState::Rejected)
        {
            control.record.attempts.push(StopAttempt {
                key: Id::new(uuid::Uuid::new_v4().to_string()).expect("UUID"),
                session: session.clone(),
                expected_run: view.revision,
                state: AttemptState::Prepared,
                response: None,
            });
            self.persist_stop(control);
        }
        let attempt = control
            .record
            .attempts
            .last_mut()
            .expect("prepared stop attempt");
        if attempt.session != session {
            control.record.phase = StopPhase::Attention;
            self.persist_stop(control);
            return Ok(control.record.phase);
        }
        if attempt.state == AttemptState::Prepared {
            attempt.state = AttemptState::Entered;
            self.persist_stop(control);
        }
        let attempt = control
            .record
            .attempts
            .last()
            .expect("stop attempt")
            .clone();
        match self
            .client
            .pause_lifecycle(&attempt.key, &control.record.run, attempt.expected_run)
            .await
        {
            Ok(response) => {
                let attempt = control.record.attempts.last_mut().expect("same attempt");
                attempt.state = AttemptState::Replied;
                attempt.response = Some(response.clone());
                self.persist_stop(control);
                control.record.phase = StopPhase::PauseObserved;
                control.record.observation = Some(response);
                self.persist_stop(control);
            }
            Err(Error::Rpc(status)) if status.code() == tonic::Code::Aborted => {
                control
                    .record
                    .attempts
                    .last_mut()
                    .expect("same attempt")
                    .state = AttemptState::Rejected;
                self.persist_stop(control);
            }
            Err(Error::Rpc(status))
                if matches!(
                    status.code(),
                    tonic::Code::Unavailable
                        | tonic::Code::DeadlineExceeded
                        | tonic::Code::Cancelled
                        | tonic::Code::Internal
                        | tonic::Code::Unknown
                        | tonic::Code::ResourceExhausted
                ) =>
            {
                return Err(Error::Rpc(status));
            }
            Err(error) => {
                control.record.phase = StopPhase::Attention;
                self.persist_stop(control);
                return Err(error);
            }
        }
        Ok(control.record.phase)
    }
}
