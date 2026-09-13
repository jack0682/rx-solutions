//! Local historical inspection. Never opens a planner, emits a request or changes resolution.
pub(crate) mod store;
mod wal;
use super::*;
use crate::lifecycle::{StopPhase, StopRecord};
use std::collections::BTreeMap;
pub use store::FileDigest;

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptInspection {
    pub key: Id,
    pub logical: Logical,
    pub generation: Counter,
    pub context: Identity,
    pub body_digest: Digest,
    pub send: SendState,
    pub resolution: Resolution,
    pub digest: Digest,
    pub is_current: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StopInspection {
    pub revision: Counter,
    pub record: StopRecord,
    pub digest: Digest,
}
#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationInspection {
    pub revision: Counter,
    pub record: Observation,
    pub digest: Digest,
}
#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Inspection {
    pub scope: Scope,
    pub records_digest: Digest,
    pub files: Vec<FileDigest>,
    pub attempts: Vec<AttemptInspection>,
    pub stop: Option<StopInspection>,
    pub observations: Vec<ObservationInspection>,
}
pub(crate) fn value<T: DeserializeOwned>(document: &Document, schema: &str) -> Result<T> {
    if document.schema.as_str() != schema {
        return Err(StoreError::Integrity(
            "recovery document schema differs".into(),
        ));
    }
    canonical::decode_json(
        &canonical::bytes(&document.value).map_err(|e| StoreError::Integrity(e.to_string()))?,
    )
    .map_err(|e| StoreError::Integrity(e.to_string()))
}
fn failure<T>(message: &str) -> Result<T> {
    Err(StoreError::Integrity(message.into()))
}
fn basis_valid(b: &Basis) -> bool {
    b.sequence.0 > 0
        && b.run_revision.0 > 0
        && b.cell_revision.0 > 0
        && !b.checked_at.clock_id.is_empty()
        && b.checked_at.clock_id.len() <= 256
}
pub(crate) fn inspect(snapshot: &store::Snapshot, scope: &Scope) -> Result<Inspection> {
    let header = snapshot
        .records
        .get(&name("executor/header"))
        .ok_or_else(|| StoreError::Integrity("required executor header missing".into()))?;
    if header.revision != Counter(1) || decode::<Scope>(header, HEADER)? != *scope {
        return failure("required executor header scope/revision differs");
    }
    let mut attempts = BTreeMap::<Id, (Record, Entry)>::new();
    let mut indices = BTreeMap::<Name, Record>::new();
    let mut observed = BTreeMap::<Name, (Record, Observation)>::new();
    let mut stop = None;
    for row in snapshot.records.values() {
        match row.key.as_str() {
            "executor/header" | "executor/attachment-binding" => {}
            "executor-stop/current" => {
                let record: StopRecord = decode(row, "rx.executor-stop.v1")?;
                lifecycle::validate(scope, &record)?;
                stop = Some(StopInspection {
                    revision: row.revision,
                    digest: digest("RX-E-RECOVERY-STOP-v1", &record)?,
                    record,
                });
            }
            key_ if key_.starts_with("executor-attempt/") => {
                let entry: Entry = decode(row, ENTRY)?;
                validate_entry(scope, &entry)?;
                if !basis_valid(&entry.basis) || row.key != key("executor-attempt", &entry.key)? {
                    return failure("stored attempt key/basis differs");
                }
                if attempts
                    .insert(entry.key.clone(), (row.clone(), entry))
                    .is_some()
                {
                    return failure("duplicate request identity");
                }
            }
            key_ if key_.starts_with("executor-current/") => {
                indices.insert(row.key.clone(), row.clone());
            }
            key_ if key_.starts_with("executor-observed/") => {
                let observation: Observation = decode(row, "rx.executor-observation.v1")?;
                validate_observation(scope, &observation)?;
                if row.key != key("executor-observed", &observation.logical)? {
                    return failure("observed key differs");
                }
                observed.insert(row.key.clone(), (row.clone(), observation));
            }
            _ => return failure("unknown entity in run journal"),
        }
    }
    if attempts.len() > 1024 || observed.len() > 1024 {
        return Err(StoreError::Invalid(
            "RECOVERY_INSPECT_LIMIT_EXCEEDED".into(),
        ));
    }
    let mut groups = BTreeMap::<Name, Vec<&Entry>>::new();
    for (_, entry) in attempts.values() {
        groups
            .entry(key("executor-current", &entry.logical)?)
            .or_default()
            .push(entry);
    }
    if groups.len() != indices.len() {
        return failure("current index coverage differs");
    }
    for (key_, entries) in &mut groups {
        entries.sort_by_key(|entry| entry.generation);
        for (index, entry) in entries.iter().enumerate() {
            if entry.generation.0 != index as u64 + 1 {
                return failure("request generations are missing or duplicated");
            }
            if index > 0 {
                let old = entries[index - 1];
                if !matches!(
                    old.resolution,
                    Resolution::RevisionRejected | Resolution::CheckpointRejected { .. }
                ) || old.context != entry.context
                    || old.body.semantic_digest()? != entry.body.semantic_digest()?
                {
                    return failure("request generation replaced unconfirmed or different work");
                }
            }
        }
        let newest = entries.last().expect("nonempty request group");
        let row = indices
            .get(key_)
            .ok_or_else(|| StoreError::Integrity("current request index missing".into()))?;
        if row.revision != newest.generation
            || decode::<Id>(row, "rx.executor-current.v1")? != newest.key
        {
            return failure("current index is not the exact latest generation");
        }
    }
    for (_, observation) in observed.values() {
        let current = groups
            .get(&key("executor-current", &observation.logical)?)
            .and_then(|entries| entries.last())
            .copied();
        if let Some(entry) = current {
            observation_matches(entry, observation)?;
        }
    }
    audit_history(snapshot, scope, &attempts, &observed, stop.as_ref())?;
    let output = attempts
        .values()
        .map(|(_, entry)| {
            let newest = groups
                .get(&key("executor-current", &entry.logical)?)
                .and_then(|entries| entries.last())
                .expect("audited index");
            Ok(AttemptInspection {
                key: entry.key.clone(),
                logical: entry.logical.clone(),
                generation: entry.generation,
                context: entry.context.clone(),
                body_digest: entry.body_digest,
                send: entry.send,
                resolution: entry.resolution.clone(),
                digest: digest("RX-E-RECOVERY-ATTEMPT-v1", entry)?,
                is_current: newest.key == entry.key,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Inspection {
        scope: scope.clone(),
        records_digest: snapshot.digest()?,
        files: snapshot.files.clone(),
        attempts: output,
        stop,
        observations: observed
            .values()
            .map(|(row, record)| {
                Ok(ObservationInspection {
                    revision: row.revision,
                    record: record.clone(),
                    digest: digest("RX-E-RECOVERY-OBSERVATION-v1", record)?,
                })
            })
            .collect::<Result<_>>()?,
    })
}

fn validate_observation(scope: &Scope, observation: &Observation) -> Result<()> {
    if !basis_valid(&observation.basis)
        || observation.logical.visit.0 == 0
        || (observation.logical.stage != Stage::PauseRun && observation.logical.control.is_some())
    {
        return failure("incomplete or invalid observation basis");
    }
    match (&observation.logical.stage, &observation.target) {
        (Stage::BeginPart | Stage::CompletePart, ObservedTarget::Part { value })
            if value.run == scope.run
                && value.ordinal == observation.logical.visit
                && value.revision.0 > 0
                && (observation.logical.stage != Stage::CompletePart
                    || value.disposition
                        == rx_process_contract::execution::PartDisposition::ConfirmedCompleted) => {
        }
        (stage, ObservedTarget::Checkpoint { decision })
            if *stage == Stage::checkpoint(decision.action()) => {}
        (Stage::ReconcileOperation, ObservedTarget::Released { .. }) => {}
        (
            Stage::PauseRun,
            ObservedTarget::Run {
                run,
                revision,
                state,
            },
        ) if run == &scope.run && revision.0 > 0 && crate::lifecycle::restricted(*state) => {}
        (Stage::ResolveActivation, ObservedTarget::Activation { value })
            if value.node == observation.logical.node
                && value.visit == observation.logical.visit => {}
        (Stage::SubmitOperation, ObservedTarget::Operation { .. }) => {}
        _ => return failure("observation does not match run/logical scope"),
    }
    Ok(())
}
fn observation_matches(entry: &Entry, observation: &Observation) -> Result<()> {
    let valid = match (&entry.body, &observation.target) {
        (Body::BeginPart { .. }, ObservedTarget::Part { value }) => {
            !matches!(&entry.resolution, Resolution::Reply { response } if matches!(response.as_ref(), Response::Part(old) if old.id != value.id))
        }
        (Body::CompletePart { command, .. }, ObservedTarget::Part { value }) => {
            command.part == value.id
        }
        (
            Body::CommitCheckpoint { decision, .. },
            ObservedTarget::Checkpoint { decision: actual },
        ) => {
            !matches!(entry.resolution, Resolution::Reply { .. })
                || bytes(decision)? == bytes(actual)?
        }
        (
            Body::ReconcileOperation {
                operation,
                intent_digest,
                ..
            },
            ObservedTarget::Released {
                operation: actual,
                intent_digest: digest,
            },
        ) => operation == actual && intent_digest == digest,
        (Body::PauseRun { run, .. }, ObservedTarget::Run { run: actual, .. }) => run == actual,
        (Body::ResolveActivation { .. }, ObservedTarget::Activation { value }) => {
            !matches!(&entry.resolution, Resolution::Reply { response } if matches!(response.as_ref(), Response::Activation(old) if old.id != value.id))
        }
        (
            Body::SubmitOperation {
                activation, intent, ..
            },
            ObservedTarget::Operation {
                activation: actual,
                operation,
                intent_digest,
            },
        ) => {
            activation == actual
                && intent
                    .digest()
                    .map_err(|e| StoreError::Integrity(e.to_string()))?
                    == *intent_digest
                && !matches!(&entry.resolution, Resolution::Reply { response } if matches!(response.as_ref(), Response::Admission(old) if &old.operation != operation))
        }
        _ => false,
    };
    if !valid {
        return failure("observation contradicts stored request identity");
    }
    Ok(())
}
fn audit_history(
    snapshot: &store::Snapshot,
    scope: &Scope,
    attempts: &BTreeMap<Id, (Record, Entry)>,
    observations: &BTreeMap<Name, (Record, Observation)>,
    stop: Option<&StopInspection>,
) -> Result<()> {
    let mut entries = BTreeMap::<Id, (u64, Entry)>::new();
    let mut observed = BTreeMap::<Name, (u64, Observation)>::new();
    let mut stopped: Option<(u64, StopRecord)> = None;
    for event in &snapshot.events {
        let schema = event.document.schema.as_str();
        if schema == "rx.executor-observed.v1" {
            let next: Observation = value(&event.document, schema)?;
            validate_observation(scope, &next)?;
            let key_ = key("executor-observed", &next.logical)?;
            let count = observed.get(&key_).map_or(1, |(count, _)| count + 1);
            if let Some((_, old)) = observed.get(&key_)
                && (next.basis.sequence < old.basis.sequence || !same_observation(old, &next)?)
            {
                return failure("historical observation regressed or changed identity");
            }
            observed.insert(key_, (count, next));
        } else if matches!(
            schema,
            "rx.executor-stop-requested.v1" | "rx.executor-stop-changed.v1"
        ) {
            let next: StopRecord = value(&event.document, schema)?;
            lifecycle::validate(scope, &next)?;
            if let Some((_, old)) = &stopped {
                if schema != "rx.executor-stop-changed.v1" {
                    return failure("duplicate initial stop event");
                }
                lifecycle::validate_transition(old, &next)?;
            } else if schema != "rx.executor-stop-requested.v1"
                || next.phase != StopPhase::Pending
                || !next.attempts.is_empty()
                || next.observation.is_some()
            {
                return failure("stop history does not start at the original pending intent");
            }
            stopped = Some((stopped.as_ref().map_or(1, |(count, _)| count + 1), next));
        } else {
            let next: Entry = value(&event.document, schema)?;
            validate_entry(scope, &next)?;
            if !basis_valid(&next.basis) {
                return failure("historical request basis differs");
            }
            if let Some((_, old)) = entries.get(&next.key) {
                if bytes(&(
                    &old.key,
                    &old.logical,
                    old.generation,
                    &old.context,
                    &old.body,
                    old.body_digest,
                    &old.basis,
                ))? != bytes(&(
                    &next.key,
                    &next.logical,
                    next.generation,
                    &next.context,
                    &next.body,
                    next.body_digest,
                    &next.basis,
                ))? {
                    return failure("historical request identity changed");
                }
                let valid = match schema {
                    "rx.executor-request-entered.v1" => {
                        old.send == SendState::Prepared
                            && next.send == SendState::EmitEntered
                            && matches!(old.resolution, Resolution::Pending)
                            && matches!(next.resolution, Resolution::Pending)
                    }
                    "rx.executor-request-replied.v1" => {
                        old.send == SendState::EmitEntered
                            && matches!(old.resolution, Resolution::Pending)
                            && matches!(next.resolution, Resolution::Reply { .. })
                    }
                    "rx.executor-request-revision-rejected.v1" => {
                        old.send == SendState::EmitEntered
                            && matches!(old.resolution, Resolution::Pending)
                            && matches!(next.resolution, Resolution::RevisionRejected)
                            && next.logical.stage != Stage::ReconcileOperation
                            && !next.logical.stage.is_checkpoint()
                    }
                    "rx.executor-checkpoint-rejected.v1" => {
                        old.send == SendState::EmitEntered
                            && matches!(old.resolution, Resolution::Pending)
                            && matches!(next.resolution, Resolution::CheckpointRejected { .. })
                            && next.logical.stage.is_checkpoint()
                    }
                    "rx.executor-request-attention.v1" => {
                        old.send == next.send
                            && matches!(old.resolution, Resolution::Pending)
                            && matches!(next.resolution, Resolution::Attention { .. })
                    }
                    _ => false,
                };
                if !valid {
                    return failure("invalid request history transition");
                }
            } else if schema != "rx.executor-request-prepared.v1"
                || next.send != SendState::Prepared
                || !matches!(next.resolution, Resolution::Pending)
            {
                return failure("request history does not begin with preparation");
            }
            let count = entries.get(&next.key).map_or(1, |(count, _)| count + 1);
            entries.insert(next.key.clone(), (count, next));
        }
    }
    if entries.len() != attempts.len() || observed.len() != observations.len() {
        return failure("request/observation history coverage differs");
    }
    for (id, (row, entry)) in attempts {
        let (count, last) = entries
            .get(id)
            .ok_or_else(|| StoreError::Integrity("attempt history missing".into()))?;
        if row.revision != Counter(*count) || bytes(last)? != bytes(entry)? {
            return failure("attempt differs from immutable event history");
        }
    }
    for (id, (row, entry)) in observations {
        let (count, last) = observed
            .get(id)
            .ok_or_else(|| StoreError::Integrity("observation history missing".into()))?;
        if row.revision != Counter(*count) || bytes(last)? != bytes(entry)? {
            return failure("observation differs from immutable event history");
        }
    }
    match (stop, stopped) {
        (None, None) => {}
        (Some(saved), Some((count, last)))
            if saved.revision == Counter(count) && bytes(&saved.record)? == bytes(&last)? => {}
        _ => return failure("stop differs from immutable event history"),
    }
    Ok(())
}

fn same_observation(a: &Observation, b: &Observation) -> Result<bool> {
    Ok(match (&a.target, &b.target) {
        (ObservedTarget::Part { value: a }, ObservedTarget::Part { value: b }) => {
            a.id == b.id
                && a.run == b.run
                && a.ordinal == b.ordinal
                && (b.revision > a.revision || bytes(a)? == bytes(b)?)
        }
        (
            ObservedTarget::Checkpoint { decision: a },
            ObservedTarget::Checkpoint { decision: b },
        ) => bytes(a)? == bytes(b)?,
        (
            ObservedTarget::Released {
                operation: a,
                intent_digest: x,
            },
            ObservedTarget::Released {
                operation: b,
                intent_digest: y,
            },
        ) => a == b && x == y,
        (
            ObservedTarget::Run {
                run: a,
                revision: x,
                ..
            },
            ObservedTarget::Run {
                run: b,
                revision: y,
                ..
            },
        ) => a == b && y >= x,
        (ObservedTarget::Activation { value: a }, ObservedTarget::Activation { value: b }) => {
            a.id == b.id
        }
        (
            ObservedTarget::Operation {
                activation: a,
                operation: x,
                intent_digest: i,
            },
            ObservedTarget::Operation {
                activation: b,
                operation: y,
                intent_digest: j,
            },
        ) => a == b && x == y && i == j,
        _ => false,
    })
}
