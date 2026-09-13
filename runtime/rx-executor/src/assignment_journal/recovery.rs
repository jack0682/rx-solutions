//! Offline inspection of the original current attachment. No journal initialization or attach.
use super::*;
pub use crate::journal::recovery::store::validate_path;
use crate::journal::recovery::{self as requests, store::Snapshot};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttachmentInspection {
    pub id: Id,
    pub phase: Phase,
    pub revision: Counter,
    pub run: Id,
    pub executor_session: Id,
    pub preparation_digest: Digest,
    pub preparation: Preparation,
    pub creation_entered: bool,
    pub run_file: PathBuf,
}
#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Unresolved {
    pub attachment_retained: bool,
    pub pending_stop: bool,
    pub attention_stop: bool,
    pub pending_requests: Counter,
    pub attention_requests: Counter,
}
#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Inspection {
    pub schema: Name,
    pub service_root: PathBuf,
    pub configuration_digest: Digest,
    pub service: Identity,
    pub assignment_digest: Digest,
    pub assignment_files: Vec<requests::FileDigest>,
    pub attachment: Option<AttachmentInspection>,
    pub run_journal: Option<requests::Inspection>,
    pub unresolved: Unresolved,
    pub execution_authorized: bool,
    pub network_accessed: bool,
    pub session_opened: bool,
    pub planner_started: bool,
    pub current_p_state_observed: bool,
    pub original_records_modified: bool,
    pub inspection_digest: Digest,
}

/// Caller retains the existing ServiceOwner throughout configuration validation and this read.
pub fn inspect_current(
    root: &Path,
    expected: &Identity,
    configuration_digest: Digest,
) -> Result<Inspection> {
    let mut source = Snapshot::open(&root.join("assignment.sqlite3"))?;
    let state = audit(&mut source, expected)?;
    for row in source.records.values() {
        if !matches!(row.key.as_str(), "attachment/header" | "attachment/state")
            && !row.key.as_str().starts_with(PREFIX)
            && !row.key.as_str().starts_with("attachment/by-run/")
            && !row.key.as_str().starts_with(CREATION_PREFIX)
        {
            return integrity("unknown entity in assignment journal");
        }
    }
    audit_history(&source)?;
    let mut attachment = None;
    let mut run_journal = None;
    let mut run_source = None;
    let mut missing_run = None;
    if let Some(id) = state.current {
        let current = read(&mut source, &id)?;
        let p = current.value.preparation;
        let entered = creation_entered(&mut source, &p)?;
        let path = root.join(format!("run-{}.sqlite3", p.scope.run));
        match fs::symlink_metadata(&path) {
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && current.value.phase == Phase::Preparing
                    && !entered =>
            {
                missing_run = Some(path.clone());
                // A reservation before creation has no request journal to initialize during inspection.
                for path in [
                    path.with_extension("writer.lock"),
                    PathBuf::from(format!("{}-wal", path.display())),
                    PathBuf::from(format!("{}-shm", path.display())),
                ] {
                    match fs::symlink_metadata(path) {
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        _ => {
                            return integrity("orphan sidecar for uninitialized Preparing journal");
                        }
                    }
                }
            }
            Err(error) => {
                return integrity(&format!("required current run file unavailable: {error}"));
            }
            Ok(metadata) => {
                if !entered || !metadata.file_type().is_file() || metadata.len() == 0 {
                    return integrity("required current run file/creation entry differs");
                }
                let mut run = Snapshot::open(&path)?;
                run_store::verify_recovery(&mut run, expected, &p)?;
                run_journal = Some(requests::inspect(&run, &p.scope)?);
                run_source = Some(run);
            }
        }
        attachment = Some(AttachmentInspection {
            id: p.id.clone(),
            phase: current.value.phase,
            revision: current.revision,
            run: p.scope.run.clone(),
            executor_session: p.executor_session.clone(),
            preparation_digest: canonical::digest("RX-E-RECOVERY-PREPARATION-v1", &p)
                .map_err(|e| StoreError::Integrity(e.to_string()))?,
            preparation: p,
            creation_entered: entered,
            run_file: path,
        });
    }
    let stop = run_journal.as_ref().and_then(|run| run.stop.as_ref());
    let mut unresolved = Unresolved {
        attachment_retained: attachment.is_some(),
        pending_stop: stop.is_some_and(|s| s.record.phase == crate::lifecycle::StopPhase::Pending),
        attention_stop: stop
            .is_some_and(|s| s.record.phase == crate::lifecycle::StopPhase::Attention),
        pending_requests: Counter(0),
        attention_requests: Counter(0),
    };
    if let Some(run) = &run_journal {
        for entry in &run.attempts {
            if matches!(entry.resolution, crate::journal::Resolution::Pending) {
                unresolved.pending_requests.0 += 1;
            }
            if matches!(
                entry.resolution,
                crate::journal::Resolution::Attention { .. }
            ) {
                unresolved.attention_requests.0 += 1;
            }
        }
    }
    let assignment_digest = source.digest()?;
    let inspection_digest = canonical::digest(
        "RX-E-RECOVERY-INSPECTION-v1",
        &(
            root,
            configuration_digest,
            expected,
            assignment_digest,
            &source.files,
            &attachment,
            &run_journal,
            &unresolved,
        ),
    )
    .map_err(|e| StoreError::Integrity(e.to_string()))?;
    if let Some(run) = &mut run_source {
        run.verify_sources()?;
    }
    if let Some(path) = missing_run {
        for source in [
            path.clone(),
            path.with_extension("writer.lock"),
            PathBuf::from(format!("{}-wal", path.display())),
            PathBuf::from(format!("{}-shm", path.display())),
        ] {
            match fs::symlink_metadata(source) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                _ => return integrity("uninitialized run source appeared during inspection"),
            }
        }
    }
    source.verify_sources()?;
    Ok(Inspection {
        schema: name("rx.executor-recovery-inspection.v1"),
        service_root: root.into(),
        configuration_digest,
        service: expected.clone(),
        assignment_digest,
        assignment_files: source.files.clone(),
        attachment,
        run_journal,
        unresolved,
        execution_authorized: false,
        network_accessed: false,
        session_opened: false,
        planner_started: false,
        current_p_state_observed: false,
        original_records_modified: false,
        inspection_digest,
    })
}

fn audit_history(source: &Snapshot) -> Result<()> {
    let mut transitions = BTreeMap::<Id, Attachment>::new();
    let mut creations = BTreeSet::new();
    for event in &source.events {
        match event.document.schema.as_str() {
            "rx.executor-attachment-transition.v1" => {
                let next: Attachment =
                    requests::value(&event.document, "rx.executor-attachment-transition.v1")?;
                let id = &next.preparation.id;
                let expected = match transitions.get(id) {
                    None => Phase::Preparing,
                    Some(old) if old.phase == Phase::Preparing && creations.contains(id) => {
                        Phase::Attached
                    }
                    Some(old) if old.phase == Phase::Attached => Phase::Closed,
                    _ => return integrity("invalid attachment history transition"),
                };
                if next.phase != expected
                    || transitions.get(id).is_some_and(|old| {
                        !same(&old.preparation, &next.preparation).unwrap_or(false)
                    })
                {
                    return integrity("attachment history identity/phase differs");
                }
                transitions.insert(id.clone(), next);
            }
            CREATION => {
                let preparation: Preparation = requests::value(&event.document, CREATION)?;
                let Some(old) = transitions.get(&preparation.id) else {
                    return integrity("creation history without preparation");
                };
                if old.phase != Phase::Preparing
                    || !same(&old.preparation, &preparation)?
                    || !creations.insert(preparation.id.clone())
                {
                    return integrity("repeated or foreign run creation history");
                }
            }
            _ => return integrity("unknown assignment history event"),
        }
    }
    let rows: Vec<_> = source
        .records
        .values()
        .filter(|row| row.key.as_str().starts_with(PREFIX))
        .collect();
    if rows.len() != transitions.len() {
        return integrity("assignment history coverage differs");
    }
    for row in rows {
        let attachment: Attachment = decode(row, RECORD)?;
        let Some(last) = transitions.get(&attachment.preparation.id) else {
            return integrity("attachment history missing");
        };
        if !same(last, &attachment)?
            || creations.contains(&attachment.preparation.id)
                != source
                    .records
                    .contains_key(&creation_key(&attachment.preparation.id))
        {
            return integrity("attachment differs from immutable event history");
        }
    }
    Ok(())
}
