use super::*;
use crate::lifecycle::*;
const SCHEMA: &str = "rx.executor-stop.v1";
fn stop_key() -> Name {
    name("executor-stop/current")
}
impl<R: Repository> Journal<R> {
    pub fn stop_record(&mut self) -> Result<Option<VersionedStop>> {
        self.repository.transact(|tx| {
            tx.get(&stop_key())?
                .map(|row| {
                    let record: StopRecord = decode(&row, SCHEMA)?;
                    validate(&self.scope, &record)?;
                    Ok(VersionedStop {
                        revision: row.revision,
                        record,
                    })
                })
                .transpose()
        })
    }
    /// First intent wins; never erase it merely because a new process starts.
    pub fn ensure_stop(&mut self, candidate: &StopRecord) -> Result<VersionedStop> {
        validate(&self.scope, candidate)?;
        if candidate.phase != StopPhase::Pending
            || !candidate.attempts.is_empty()
            || candidate.observation.is_some()
        {
            return invalid("new stop must be pending");
        }
        self.repository.transact(|tx| {
            if let Some(row) = tx.get(&stop_key())? {
                let record: StopRecord = decode(&row, SCHEMA)?;
                validate(&self.scope, &record)?;
                return Ok(VersionedStop {
                    revision: row.revision,
                    record,
                });
            }
            let revision = tx
                .put(&stop_key(), None, &doc(SCHEMA, candidate)?)?
                .revision;
            event(tx, "rx.executor-stop-requested.v1", candidate)?;
            Ok(VersionedStop {
                revision,
                record: candidate.clone(),
            })
        })
    }
    pub fn save_stop(&mut self, expected: Counter, next: &StopRecord) -> Result<Counter> {
        validate(&self.scope, next)?;
        self.repository.transact(|tx| {
            let row = tx
                .get(&stop_key())?
                .ok_or(StoreError::Invalid("stop intent missing".into()))?;
            if row.revision != expected {
                return Err(StoreError::RevisionConflict(stop_key().to_string()));
            }
            let prior: StopRecord = decode(&row, SCHEMA)?;
            validate_transition(&prior, next)?;
            if bytes(&prior)? == bytes(next)? {
                return Ok(row.revision);
            }
            let revision = tx
                .put(&stop_key(), Some(expected), &doc(SCHEMA, next)?)?
                .revision;
            event(tx, "rx.executor-stop-changed.v1", next)?;
            Ok(revision)
        })
    }
}
fn validate(scope: &Scope, record: &StopRecord) -> Result<()> {
    if record.run != scope.run
        || record.attempts.len() > 64
        || record.origin_context.as_ref().is_some_and(|c| {
            c.run != scope.run
                || c.resolved_digest != scope.resolved_digest
                || c.executor_session != record.origin_session
                || c.epoch.0 == 0
        })
    {
        return invalid("stop scope differs");
    }
    let mut keys = std::collections::BTreeSet::new();
    for attempt in &record.attempts {
        if !keys.insert(&attempt.key)
            || attempt.expected_run.0 == 0
            || (attempt.state == AttemptState::Replied) != attempt.response.is_some()
        {
            return invalid("stop attempt invalid");
        }
        if let Some(response) = &attempt.response {
            validate_view(scope, response)?;
            if !restricted(response.state) || response.revision < attempt.expected_run {
                return invalid("pause reply did not restrict run");
            }
        }
    }
    if let Some(view) = &record.observation {
        validate_view(scope, view)?;
    }
    match record.phase {
        StopPhase::Pending if record.observation.is_some() => {
            return invalid("pending stop has final observation");
        }
        StopPhase::PauseObserved
            if record
                .observation
                .as_ref()
                .is_none_or(|v| !restricted(v.state)) =>
        {
            return invalid("pause requires P observation");
        }
        StopPhase::Superseded
            if record.observation.as_ref().is_none_or(|v| {
                v.executor_session
                    .as_ref()
                    .is_none_or(|s| s == &record.origin_session)
            }) =>
        {
            return invalid("superseded requires another executor session");
        }
        _ => {}
    }
    Ok(())
}
fn validate_view(scope: &Scope, view: &RunResponse) -> Result<()> {
    if view.run != scope.run
        || view.recipe != scope.resolved_digest
        || view.revision.0 == 0
        || view.checkpoint.run != view.run
        || view.checkpoint.revision != view.revision
    {
        return invalid("stop observation run differs");
    }
    Ok(())
}
fn validate_transition(old: &StopRecord, next: &StopRecord) -> Result<()> {
    if bytes(&(
        old.id.clone(),
        old.run.clone(),
        old.origin_session.clone(),
        &old.origin_context,
        old.reason,
        &old.requested_at,
    ))? != bytes(&(
        next.id.clone(),
        next.run.clone(),
        next.origin_session.clone(),
        &next.origin_context,
        next.reason,
        &next.requested_at,
    ))? || next.attempts.len() < old.attempts.len()
    {
        return invalid("stop identity/history changed");
    }
    if old.phase != StopPhase::Pending && bytes(old)? != bytes(next)? {
        return invalid("terminal stop cannot be reopened");
    }
    for (a, b) in old.attempts.iter().zip(&next.attempts) {
        if a.key != b.key || a.session != b.session || a.expected_run != b.expected_run {
            return invalid("stop attempt body changed");
        }
        if bytes(a)? == bytes(b)? {
            continue;
        }
        if !matches!(
            (a.state, b.state),
            (AttemptState::Prepared, AttemptState::Entered)
                | (AttemptState::Entered, AttemptState::Rejected)
                | (AttemptState::Entered, AttemptState::Replied)
        ) {
            return invalid("stop attempt regressed");
        }
    }
    if next.attempts.len() > old.attempts.len()
        && (next.attempts.len() != old.attempts.len() + 1
            || old
                .attempts
                .last()
                .is_some_and(|a| a.state != AttemptState::Rejected)
            || next.attempts.last().unwrap().state != AttemptState::Prepared)
    {
        return invalid("new stop key without confirmed rejection");
    }
    Ok(())
}
