#![cfg(unix)]
//! Only disposable test stores/copies are used. A corrupt retained WAL must not
//! turn committed attachment/request history into a successful empty inspection.
use rx_domain::types::*;
use rx_executor::{
    assignment_journal::{self, AssignmentJournal, Identity, Phase, Preparation, ServiceScope},
    frame,
    journal::{Basis, Body, Logical, Resolution, Scope, SendState, Stage},
    lifecycle::{StopPhase, StopReason, StopRecord},
    service_owner::ServiceOwner,
};
use rx_storage::SqliteRepository;
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

fn id(value: u64) -> Id {
    Id::new(format!("00000000-0000-4000-8000-{value:012}")).unwrap()
}
fn name(value: &str) -> Name {
    Name::new(value).unwrap()
}
fn digest(value: u8) -> Digest {
    Digest::from_bytes([value; 32])
}
fn service() -> Identity {
    Identity {
        journal: id(1),
        scope: ServiceScope {
            installation: id(2),
            store_generation: id(3),
            principal: name("executor/wal-test"),
            release: digest(4),
            cell: name("cell/wal-test"),
            definition: digest(5),
        },
    }
}
fn preparation() -> Preparation {
    let scope = service().scope;
    Preparation {
        id: id(10),
        run_journal: id(11),
        executor_session: id(12),
        epoch: Counter(1),
        scope: Scope {
            installation: scope.installation,
            store_generation: scope.store_generation,
            principal: scope.principal,
            release: scope.release,
            cell: scope.cell,
            definition: scope.definition,
            run: id(13),
            resolved_digest: digest(6),
        },
        basis: Basis {
            runtime_boot: id(14),
            sequence: Counter(1),
            run_revision: Counter(1),
            cell_revision: Counter(1),
            checked_at: TimePoint {
                clock_id: "test/wal-corruption".into(),
                ticks_ns: Counter(100),
            },
        },
    }
}
fn directory() -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = fs::canonicalize(directory.path()).unwrap();
    (directory, path)
}
fn copy_without_shm(original: &Path, destination: &Path) {
    for entry in fs::read_dir(original).unwrap() {
        let entry = entry.unwrap();
        assert!(entry.file_type().unwrap().is_file());
        if entry.file_name().to_string_lossy().ends_with("-shm") {
            continue;
        }
        fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
    }
}
type FileState = (Vec<u8>, u64, i64, i64, u32);
fn files(root: &Path) -> BTreeMap<String, FileState> {
    fs::read_dir(root)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            let metadata = entry.metadata().unwrap();
            (
                entry.file_name().to_str().unwrap().to_owned(),
                (
                    fs::read(entry.path()).unwrap(),
                    metadata.len(),
                    metadata.mtime(),
                    metadata.mtime_nsec(),
                    metadata.mode(),
                ),
            )
        })
        .collect()
}
fn inspect(root: &Path) -> rx_ports::Result<assignment_journal::recovery::Inspection> {
    let _owner = ServiceOwner::acquire_existing(&root.join("assignment.sqlite3"))
        .map_err(|error| rx_ports::StoreError::Unavailable(error.to_string()))?;
    assignment_journal::recovery::inspect_current(root, &service(), digest(9))
}

#[test]
fn invalid_assignment_wal_header_cannot_hide_committed_attachment_and_pending_requests() {
    let (_original_directory, original) = directory();
    let assignment_path = original.join("assignment.sqlite3");
    drop(ServiceOwner::acquire(&assignment_path).unwrap());
    // Closing the only initial connection checkpoints the valid schema/header and
    // current=None state. All subsequent attachment commits will be WAL-only.
    drop(AssignmentJournal::initialize_file(&assignment_path, service()).unwrap());
    let checkpointed_main = fs::read(&assignment_path).unwrap();
    assert!(!checkpointed_main.is_empty());
    let mut assignment =
        AssignmentJournal::<SqliteRepository>::open_file_required(&assignment_path, service())
            .unwrap();
    let preparation = preparation();
    assignment.prepare(preparation.clone()).unwrap();
    let run = assignment
        .initialize_run_file(&original, &preparation.id)
        .unwrap();
    let (_, mut requests) = assignment.attach(run).unwrap();
    let context = frame::Identity {
        run: preparation.scope.run.clone(),
        executor_session: preparation.executor_session.clone(),
        resolved_digest: preparation.scope.resolved_digest,
        visit: Counter(1),
        epoch: preparation.epoch,
    };
    let request = requests
        .prepare(
            Logical {
                visit: Counter(1),
                node: name("handover"),
                stage: Stage::ReconcileOperation,
                control: None,
            },
            context.clone(),
            Body::ReconcileOperation {
                run: preparation.scope.run.clone(),
                operation: id(20),
                intent_digest: digest(7),
            },
            preparation.basis.clone(),
        )
        .unwrap();
    requests.enter(&request.key).unwrap();
    let stop = StopRecord::new(
        preparation.scope.run.clone(),
        preparation.executor_session.clone(),
        Some(context),
        StopReason::WorkerFault,
        None,
    );
    requests.ensure_stop(&stop).unwrap();
    assert_eq!(
        fs::read(&assignment_path).unwrap(),
        checkpointed_main,
        "fixture must keep the committed attachment out of the initial main DB"
    );
    let wal_name = "assignment.sqlite3-wal";
    let retained_wal = fs::read(original.join(wal_name)).unwrap();
    assert!(
        retained_wal.len() > 32,
        "fixture requires a nonempty committed WAL"
    );
    assert_eq!(
        u32::from_be_bytes(retained_wal[..4].try_into().unwrap()) & 0xffff_fffe,
        0x377f_0682
    );

    let (_positive_directory, positive) = directory();
    let (_corrupt_directory, corrupt) = directory();
    // These are copies of idle committed test stores. The live test writer locks
    // remain on the original inodes; no original P/H or service records are opened.
    copy_without_shm(&original, &positive);
    copy_without_shm(&original, &corrupt);
    assert!(!positive.join("assignment.sqlite3-shm").exists());
    assert!(!corrupt.join("assignment.sqlite3-shm").exists());

    let before_positive = files(&positive);
    let recovered = inspect(&positive).unwrap();
    assert_eq!(files(&positive), before_positive);
    assert_eq!(
        recovered.attachment.as_ref().unwrap().phase,
        Phase::Attached
    );
    assert_eq!(
        recovered.attachment.as_ref().unwrap().run,
        preparation.scope.run
    );
    assert!(recovered.unresolved.attachment_retained && recovered.unresolved.pending_stop);
    assert_eq!(recovered.unresolved.pending_requests, Counter(1));
    let journal = recovered.run_journal.as_ref().unwrap();
    assert_eq!(journal.attempts.len(), 1);
    assert_eq!(journal.attempts[0].key, request.key);
    assert_eq!(journal.attempts[0].send, SendState::EmitEntered);
    assert!(matches!(
        journal.attempts[0].resolution,
        Resolution::Pending
    ));
    assert_eq!(journal.stop.as_ref().unwrap().record.id, stop.id);
    assert_eq!(
        journal.stop.as_ref().unwrap().record.phase,
        StopPhase::Pending
    );

    // Exactly one byte of the disposable copied WAL magic is corrupted. Without
    // SHM, SQLite may discard this WAL and return the valid checkpointed main DB.
    let mut damaged = retained_wal;
    damaged[0] ^= 1;
    fs::write(corrupt.join(wal_name), damaged).unwrap();
    let before_corrupt = files(&corrupt);
    let result = inspect(&corrupt);
    assert_eq!(files(&corrupt), before_corrupt);
    assert!(
        result.is_err(),
        "corrupt retained WAL was accepted: attachment={:?}, pending_stop={:?}, pending_requests={:?}; committed pending history must not become idle",
        result.as_ref().ok().map(|report| &report.attachment),
        result
            .as_ref()
            .ok()
            .map(|report| report.unresolved.pending_stop),
        result
            .as_ref()
            .ok()
            .map(|report| report.unresolved.pending_requests)
    );
    drop(requests);
    drop(assignment);
}

#[test]
fn stale_committed_shm_cannot_hide_later_complete_wal_commits_or_change_the_original_copy() {
    let (_original_directory, original) = directory();
    let assignment_path = original.join("assignment.sqlite3");
    drop(ServiceOwner::acquire(&assignment_path).unwrap());
    drop(AssignmentJournal::initialize_file(&assignment_path, service()).unwrap());
    let checkpointed_main = fs::read(&assignment_path).unwrap();
    let mut assignment =
        AssignmentJournal::<SqliteRepository>::open_file_required(&assignment_path, service())
            .unwrap();
    let preparation = preparation();
    let reserved = assignment.prepare(preparation.clone()).unwrap();
    assert_eq!(reserved.value.phase, Phase::Preparing);
    // This is a genuine index from the earlier completed Preparing transaction,
    // captured before either the creation marker or Attached commit exists.
    let shm_name = "assignment.sqlite3-shm";
    let earlier_shm = fs::read(original.join(shm_name)).unwrap();
    let earlier_wal = fs::read(original.join("assignment.sqlite3-wal")).unwrap();
    assert!(!earlier_shm.is_empty() && earlier_wal.len() > 32);

    let run = assignment
        .initialize_run_file(&original, &preparation.id)
        .unwrap();
    let (_, mut requests) = assignment.attach(run).unwrap();
    let context = frame::Identity {
        run: preparation.scope.run.clone(),
        executor_session: preparation.executor_session.clone(),
        resolved_digest: preparation.scope.resolved_digest,
        visit: Counter(1),
        epoch: preparation.epoch,
    };
    let request = requests
        .prepare(
            Logical {
                visit: Counter(1),
                node: name("handover"),
                stage: Stage::ReconcileOperation,
                control: None,
            },
            context.clone(),
            Body::ReconcileOperation {
                run: preparation.scope.run.clone(),
                operation: id(20),
                intent_digest: digest(7),
            },
            preparation.basis.clone(),
        )
        .unwrap();
    requests.enter(&request.key).unwrap();
    let stop = StopRecord::new(
        preparation.scope.run.clone(),
        preparation.executor_session.clone(),
        Some(context),
        StopReason::WorkerFault,
        None,
    );
    requests.ensure_stop(&stop).unwrap();
    assert_eq!(fs::read(&assignment_path).unwrap(), checkpointed_main);
    let later_wal = fs::read(original.join("assignment.sqlite3-wal")).unwrap();
    assert!(later_wal.len() > earlier_wal.len());
    assert_eq!(
        &later_wal[..32],
        &earlier_wal[..32],
        "fixture must remain in one WAL generation"
    );
    assert_ne!(fs::read(original.join(shm_name)).unwrap(), earlier_shm);

    let (_copy_directory, copied) = directory();
    copy_without_shm(&original, &copied);
    fs::write(copied.join(shm_name), &earlier_shm).unwrap();
    let before = files(&copied);
    // The supplied source SHM is intentionally stale but remains untouched.
    // SQLite must reconstruct only its private copied index from the valid WAL.
    let first = inspect(&copied).unwrap();
    let second = inspect(&copied).unwrap();
    assert_eq!(files(&copied), before);
    assert_eq!(fs::read(copied.join(shm_name)).unwrap(), earlier_shm);
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
    let attachment = first.attachment.as_ref().unwrap();
    assert_eq!(attachment.phase, Phase::Attached);
    assert_eq!(attachment.run, preparation.scope.run);
    assert!(attachment.creation_entered);
    assert!(first.unresolved.attachment_retained && first.unresolved.pending_stop);
    assert_eq!(first.unresolved.pending_requests, Counter(1));
    let journal = first.run_journal.as_ref().unwrap();
    assert_eq!(journal.attempts.len(), 1);
    assert_eq!(journal.attempts[0].key, request.key);
    assert_eq!(journal.attempts[0].send, SendState::EmitEntered);
    assert!(matches!(
        journal.attempts[0].resolution,
        Resolution::Pending
    ));
    assert_eq!(journal.stop.as_ref().unwrap().record.id, stop.id);
    assert_eq!(
        journal.stop.as_ref().unwrap().record.phase,
        StopPhase::Pending
    );
    assert!(!first.network_accessed && !first.session_opened && !first.planner_started);
    drop(requests);
    drop(assignment);
}
