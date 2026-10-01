//! Source investigation without reopening a manager, replaying a grant or rewriting history.
use super::*;
use crate::{
    model::{Effect, GuardedExit, Phase},
    process_identity::InvestigationOutcome,
    registration::{ExecutionState, ObservationRef, Registry},
    supervisor::{persisted_state, unconfirmed_component_stops},
};
use rx_storage::SqliteRepository;
use serde::Serialize;
use std::{collections::BTreeMap, path::Path};

#[derive(Serialize)]
#[serde(tag = "basis", rename_all = "SCREAMING_SNAKE_CASE")]
enum Finding {
    RecordedDirectChildExit {
        pid: u32,
        exit_code: Option<i32>,
    },
    RecordedNotStarted,
    UnconsumedBeforeStart,
    ScopedInvestigation {
        outcome: InvestigationOutcome,
        observed_at: TimePoint,
        evidence_digest: Digest,
    },
    Unresolved {
        reason: String,
    },
}
#[derive(Serialize)]
struct NodeInspection {
    instance: Id,
    phase: Phase,
    original: Option<ObservationRef>,
    recorded_outcome: Option<ExecutionState>,
    finding: Finding,
    residuals: Vec<String>,
}
/// Only the authenticated client and actual local records construct this diagnostic.
/// Serialized output is not a grant, live process handle or recovery approval.
/// ```compile_fail
/// use rx_supervisor::resident_execution::SourceInspection;
/// let _: SourceInspection = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Serialize)]
pub struct SourceInspection {
    schema: &'static str,
    assignment: Id,
    grant: Id,
    original_peer: data::Peer,
    current_peer: data::Peer,
    preparation_digest: Digest,
    supervisor_revision: Counter,
    supervisor_digest: Digest,
    inspected_at: TimePoint,
    start_window_expired: bool,
    nodes: BTreeMap<Name, NodeInspection>,
    operating_permission: &'static str,
    limitations: Vec<&'static str>,
}

fn existing(path: &Path, directory: bool) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink()
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        return Err(invalid("original execution path has unexpected type"));
    }
    Ok(())
}

pub(super) fn inspect(
    assignment: data::Assignment,
    peer: &data::Peer,
    catalog: Catalog,
    registry: &mut Registry<SqliteRepository>,
    clock: &dyn Clock,
) -> Result<SourceInspection> {
    let grant = assignment
        .grant
        .as_ref()
        .ok_or_else(|| invalid("original grant absent"))?;
    let content = assignment
        .content
        .as_ref()
        .ok_or_else(|| invalid("original preparation absent"))?;
    if grant.assignment != assignment.intent.id
        || grant.intent_digest != assignment.intent.digest().map_err(invalid)?
        || grant.preparation_digest != content.preparation.digest().map_err(invalid)?
        || grant.peer != content.preparation.peer
        || peer.registry != grant.peer.registry
        || peer.principal != grant.peer.principal
        || peer.installation != grant.peer.installation
        || peer.store_generation != grant.peer.store_generation
        || content.basis != data::ContentBasis::EnrolledSupervisorVerifiedRelease
    {
        return Err(invalid(
            "original execution and current source context differ",
        ));
    }
    let mut prepared = Prepared::reconstruct(assignment.intent, &grant.peer, catalog)?;
    prepared.offer.legacy_source = content.preparation.legacy_source.clone();
    if prepared.offer != content.preparation {
        return Err(invalid(
            "original content differs from actual verified release",
        ));
    }
    let records = registry.platform_recovery_records(
        &prepared.intent,
        grant,
        prepared.offer.legacy_source.as_ref(),
    )?;
    let run = registry.platform_run_directory(&prepared.intent.id)?;
    existing(
        run.parent().ok_or_else(|| invalid("run parent missing"))?,
        true,
    )?;
    existing(&run, true)?;
    let path = run.join("supervisor.db");
    existing(&path, false)?;
    let lock = path.with_extension("writer.lock");
    match std::fs::symlink_metadata(&lock) {
        Ok(_) => existing(&lock, false)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    // Opening acquires the original store's writer lock. Never construct Supervisor::open here.
    let mut store = SqliteRepository::open(&path)?;
    let (supervisor_revision, state) = persisted_state(&mut store)?;
    if state.plan != prepared.intent.run
        || state.plan_digest != prepared.offer.plan_digest
        || state.records.keys().ne(prepared.intent.nodes.keys())
    {
        return Err(invalid(
            "original supervisor journal differs from assignment",
        ));
    }
    let now = clock.now()?;
    if now.clock_id != grant.issued_at.clock_id
        || now.clock_id != grant.valid_until.clock_id
        || now.ticks_ns < grant.issued_at.ticks_ns
        || grant.valid_until.ticks_ns <= grant.issued_at.ticks_ns
    {
        return Err(invalid("original grant clock cannot be compared"));
    }
    let start_window_expired = now.ticks_ns >= grant.valid_until.ticks_ns;
    let residual_stops = unconfirmed_component_stops(&prepared.plan, &state);
    let mut nodes = BTreeMap::new();
    for (selection, node) in &prepared.intent.nodes {
        let record = &state.records[selection];
        if record.id != *selection
            || record.instance.as_ref() != Some(&node.instance)
            || record.pid == Some(0)
        {
            return Err(invalid("original supervisor instance differs"));
        }
        let program = &prepared.catalog.programs[&node.registration.declaration.catalog.program];
        let mut residuals = Vec::new();
        if !start_window_expired {
            residuals.push("original start window remains open".into());
        }
        if program.effect != Effect::NonActuating {
            residuals.push(
                "process evidence cannot settle this program's external effect contract".into(),
            );
        }
        if let Some(reason) = residual_stops.get(selection) {
            residuals.push(reason.clone());
        }
        if matches!(
            record.guarded_exit,
            Some(GuardedExit::Unconfirmed { .. })
                | Some(GuardedExit::Confirmed {
                    reconciliation_required: true,
                    ..
                })
        ) {
            residuals.push("guarded shutdown has unresolved obligations".into());
        }
        let execution = records[selection].as_ref();
        let original = execution
            .map(|_| registry.recovery_target(&node.registration.id, &node.instance))
            .transpose()?;
        let recorded_outcome = execution.map(|e| e.last_observed.state);
        let finding = match execution {
            None if matches!(
                record.phase,
                Phase::Prepared | Phase::Skipped | Phase::StartFailed
            ) && record.pid.is_none()
                && record.exit_code.is_none()
                && record.process_identity.is_none()
                && start_window_expired =>
            {
                Finding::UnconsumedBeforeStart
            }
            None => Finding::Unresolved {
                reason:
                    "no matching durable consumption; journal alone cannot establish non-execution"
                        .into(),
            },
            Some(execution) => {
                let observed = &execution.last_observed;
                if observed.pid != record.pid
                    || observed.process_identity != record.process_identity
                {
                    Finding::Unresolved { reason: "registry and supervisor observations differ; retain both original records".into() }
                } else if record.phase == Phase::Exited
                    && observed.state == ExecutionState::Exited
                    && record.exit_code == observed.exit_code
                    && record.pid.is_some()
                {
                    residuals.push(
                        "direct child exit does not assess descendants or external resources"
                            .into(),
                    );
                    Finding::RecordedDirectChildExit {
                        pid: record.pid.expect("checked"),
                        exit_code: record.exit_code,
                    }
                } else if matches!(record.phase, Phase::StartFailed | Phase::Skipped)
                    && observed.state == ExecutionState::NotStarted
                    && record.pid.is_none()
                    && record.exit_code.is_none()
                    && record.process_identity.is_none()
                {
                    Finding::RecordedNotStarted
                } else if matches!(
                    observed.state,
                    ExecutionState::Exited | ExecutionState::NotStarted
                ) {
                    Finding::Unresolved {
                        reason: "terminal source history conflicts with supervisor journal".into(),
                    }
                } else {
                    let investigation =
                        registry.investigate(original.as_ref().expect("execution reference"))?;
                    residuals.push("scoped process investigation preserves the unresolved past outcome and does not assess descendants or resources".into());
                    Finding::ScopedInvestigation {
                        outcome: investigation.outcome().clone(),
                        observed_at: investigation.observed_at().clone(),
                        evidence_digest: canonical::digest(
                            "RX-PROCESS-INVESTIGATION-v1",
                            &investigation,
                        )
                        .map_err(invalid)?,
                    }
                }
            }
        };
        nodes.insert(
            selection.clone(),
            NodeInspection {
                instance: node.instance.clone(),
                phase: record.phase,
                original,
                recorded_outcome,
                finding,
                residuals,
            },
        );
    }
    let supervisor_digest =
        canonical::digest("RX-RESIDENT-RECOVERY-SUPERVISOR-v1", &state).map_err(invalid)?;
    store.close()?;
    Ok(SourceInspection {
        schema: "rx.resident-execution-source-inspection.v1",
        assignment: prepared.intent.id,
        grant: grant.id.clone(),
        original_peer: grant.peer.clone(),
        current_peer: peer.clone(),
        preparation_digest: grant.preparation_digest,
        supervisor_revision,
        supervisor_digest,
        inspected_at: now,
        start_window_expired,
        nodes,
        operating_permission: "NOT_GRANTED; ORIGINAL_CLAIMS_AND_OUTCOMES_UNCHANGED",
        limitations: vec![
            "Source investigation only; P owner reconciliation and a new assignment are still required",
            "Stored records rely on the installed verifier and local OS/storage; whole-state rollback is not detected",
            "No process adoption, termination, grant restoration or original outbox rewrite",
        ],
    })
}
