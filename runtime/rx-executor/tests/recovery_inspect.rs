#![cfg(unix)]
use rx_domain::types::*;
use rx_executor::{
    assignment_journal::{self, AssignmentJournal, Identity, Phase, Preparation, ServiceScope},
    frame,
    journal::{Basis, Body, Journal, Logical, Scope, Stage},
    lifecycle::{StopReason, StopRecord},
    service_owner::ServiceOwner,
};
use rx_storage::SqliteRepository;
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{MetadataExt, symlink},
    path::{Path, PathBuf},
};

fn id(v: u64) -> Id {
    Id::new(format!("00000000-0000-4000-8000-{v:012}")).unwrap()
}
fn n(v: &str) -> Name {
    Name::new(v).unwrap()
}
fn d(v: u8) -> Digest {
    Digest::from_bytes([v; 32])
}
fn identity() -> Identity {
    Identity {
        journal: id(1),
        scope: ServiceScope {
            installation: id(2),
            store_generation: id(3),
            principal: n("executor/test"),
            release: d(4),
            cell: n("cell/test"),
            definition: d(5),
        },
    }
}
fn preparation() -> Preparation {
    let s = identity().scope;
    Preparation {
        id: id(10),
        run_journal: id(11),
        executor_session: id(12),
        epoch: Counter(1),
        scope: Scope {
            installation: s.installation,
            store_generation: s.store_generation,
            principal: s.principal,
            release: s.release,
            cell: s.cell,
            definition: s.definition,
            run: id(13),
            resolved_digest: d(6),
        },
        basis: Basis {
            runtime_boot: id(14),
            sequence: Counter(1),
            run_revision: Counter(1),
            cell_revision: Counter(1),
            checked_at: TimePoint {
                clock_id: "test-clock".into(),
                ticks_ns: Counter(100),
            },
        },
    }
}
fn root() -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = fs::canonicalize(directory.path()).unwrap();
    drop(ServiceOwner::acquire(&path.join("assignment.sqlite3")).unwrap());
    (directory, path)
}
fn initialized(root: &Path) -> AssignmentJournal<SqliteRepository> {
    AssignmentJournal::initialize_file(&root.join("assignment.sqlite3"), identity()).unwrap()
}
fn inspect(root: &Path) -> assignment_journal::recovery::Inspection {
    let _owner = ServiceOwner::acquire_existing(&root.join("assignment.sqlite3")).unwrap();
    assignment_journal::recovery::inspect_current(root, &identity(), d(9)).unwrap()
}
type FileState = (Vec<u8>, u64, i64, i64, u32);
fn files(root: &Path) -> BTreeMap<String, FileState> {
    fs::read_dir(root)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            let meta = entry.metadata().unwrap();
            (
                entry.file_name().to_str().unwrap().to_owned(),
                (
                    fs::read(entry.path()).unwrap(),
                    meta.len(),
                    meta.mtime(),
                    meta.mtime_nsec(),
                    meta.mode(),
                ),
            )
        })
        .collect()
}
fn current_run(
    root: &Path,
    assignment: &mut AssignmentJournal<SqliteRepository>,
) -> Journal<SqliteRepository> {
    let p = preparation();
    assignment.prepare(p.clone()).unwrap();
    let run = assignment.initialize_run_file(root, &p.id).unwrap();
    let (_, mut journal) = assignment.attach(run).unwrap();
    let context = frame::Identity {
        run: p.scope.run.clone(),
        executor_session: p.executor_session.clone(),
        resolved_digest: p.scope.resolved_digest,
        visit: Counter(1),
        epoch: p.epoch,
    };
    let entry = journal
        .prepare(
            Logical {
                visit: Counter(1),
                node: n("handover"),
                stage: Stage::ReconcileOperation,
                control: None,
            },
            context.clone(),
            Body::ReconcileOperation {
                run: p.scope.run.clone(),
                operation: id(20),
                intent_digest: d(7),
            },
            p.basis,
        )
        .unwrap();
    journal.enter(&entry.key).unwrap();
    journal
        .ensure_stop(&StopRecord::new(
            p.scope.run,
            p.executor_session,
            Some(context),
            StopReason::WorkerFault,
            None,
        ))
        .unwrap();
    journal
}

#[test]
fn current_pending_stop_is_inspected_twice_without_source_change_or_resolution() {
    let (_directory, root) = root();
    let mut assignment = initialized(&root);
    drop(current_run(&root, &mut assignment));
    drop(assignment);
    let before = files(&root);
    let a = inspect(&root);
    let b = inspect(&root);
    assert_eq!(
        serde_json::to_vec(&a).unwrap(),
        serde_json::to_vec(&b).unwrap()
    );
    assert_eq!(files(&root), before);
    assert_eq!(a.schema.as_str(), "rx.executor-recovery-inspection.v1");
    assert_eq!(a.attachment.as_ref().unwrap().phase, Phase::Attached);
    assert_eq!(a.attachment.as_ref().unwrap().run, preparation().scope.run);
    assert!(a.unresolved.pending_stop && a.unresolved.attachment_retained);
    assert_eq!(a.unresolved.pending_requests, Counter(1));
    let stop = &a
        .run_journal
        .as_ref()
        .unwrap()
        .stop
        .as_ref()
        .unwrap()
        .record;
    assert_eq!(stop.phase, rx_executor::lifecycle::StopPhase::Pending);
    assert_eq!(stop.origin_session, preparation().executor_session);
    assert!(stop.attempts.is_empty() && stop.observation.is_none());
    assert!(
        !a.network_accessed
            && !a.session_opened
            && !a.planner_started
            && !a.execution_authorized
            && !a.original_records_modified
    );
}

#[test]
fn preparing_never_initializes_or_attaches_and_creation_loss_is_rejected() {
    let (_directory, root) = root();
    let mut assignment = initialized(&root);
    assignment.prepare(preparation()).unwrap();
    drop(assignment);
    let before = files(&root);
    let report = inspect(&root);
    assert_eq!(files(&root), before);
    assert!(report.run_journal.is_none());
    assert_eq!(report.attachment.as_ref().unwrap().phase, Phase::Preparing);
    assert!(!report.attachment.as_ref().unwrap().creation_entered);
    let mut assignment =
        AssignmentJournal::open_file_required(&root.join("assignment.sqlite3"), identity())
            .unwrap();
    drop(
        assignment
            .initialize_run_file(&root, &preparation().id)
            .unwrap(),
    );
    drop(assignment);
    let before = files(&root);
    let report = inspect(&root);
    assert_eq!(files(&root), before);
    assert!(report.run_journal.is_some());
    assert_eq!(report.attachment.as_ref().unwrap().phase, Phase::Preparing);
    fs::remove_file(root.join(format!("run-{}.sqlite3", preparation().scope.run))).unwrap();
    let before = files(&root);
    assert!(assignment_journal::recovery::inspect_current(&root, &identity(), d(9)).is_err());
    assert_eq!(files(&root), before);
}

#[test]
fn crash_style_wal_copy_is_included_without_touching_the_retained_source() {
    let (_directory, original) = root();
    let mut assignment = initialized(&original);
    let requests = current_run(&original, &mut assignment);
    assert!(
        fs::metadata(original.join("assignment.sqlite3-wal"))
            .unwrap()
            .len()
            > 0
    );
    let (_copy_directory, copied) = root();
    // Test-only copy of committed files while these idle test repositories retain their WAL.
    for entry in fs::read_dir(&original).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), copied.join(entry.file_name())).unwrap();
    }
    let before = files(&copied);
    let report = inspect(&copied);
    assert!(report.unresolved.pending_stop);
    assert_eq!(report.unresolved.pending_requests, Counter(1));
    assert_eq!(files(&copied), before);
    drop(requests);
    drop(assignment);
}

#[test]
fn missing_header_wrong_identity_and_missing_writer_lock_are_never_initialized() {
    let (_directory, root) = root();
    let mut assignment = initialized(&root);
    drop(current_run(&root, &mut assignment));
    drop(assignment);
    let mut other = identity();
    other.journal = id(90);
    assert!(assignment_journal::recovery::inspect_current(&root, &other, d(9)).is_err());
    let path = root.join(format!("run-{}.sqlite3", preparation().scope.run));
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute("DELETE FROM entities WHERE key='executor/header'", [])
        .unwrap();
    drop(connection);
    let before = files(&root);
    assert!(assignment_journal::recovery::inspect_current(&root, &identity(), d(9)).is_err());
    assert_eq!(files(&root), before);
    fs::remove_file(root.join("assignment.writer.lock")).unwrap();
    let before = files(&root);
    assert!(assignment_journal::recovery::inspect_current(&root, &identity(), d(9)).is_err());
    assert_eq!(files(&root), before);
}

#[test]
fn owner_conflict_symlink_parent_and_size_limit_fail_without_source_writes() {
    let (_directory, root) = root();
    let assignment = initialized(&root);
    assert!(assignment_journal::recovery::inspect_current(&root, &identity(), d(9)).is_err());
    drop(assignment);
    let alias = root.join("alias");
    symlink(&root, &alias).unwrap();
    assert!(assignment_journal::recovery::inspect_current(&alias, &identity(), d(9)).is_err());
    fs::remove_file(alias).unwrap();
    let path = root.join("assignment.sqlite3");
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    let before = fs::metadata(&path).unwrap();
    let error =
        assignment_journal::recovery::inspect_current(&root, &identity(), d(9)).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("RECOVERY_INSPECT_LIMIT_EXCEEDED")
    );
    let after = fs::metadata(&path).unwrap();
    assert_eq!(before.mtime_nsec(), after.mtime_nsec());
    assert_eq!(before.len(), after.len());
}
