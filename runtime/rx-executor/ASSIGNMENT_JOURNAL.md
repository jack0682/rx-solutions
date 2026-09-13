# Durable run attachment journal for the cell service

This module records which run journal S selected and with which P query context. **Connecting the resident CellService, CLI/BT and this journal remains future work.** The separately implemented P assignment query, Client and journal transitions alone do not create RunMandate, execution permission, actual completion/support or planner termination.

## Storage scope

`Identity` pins the service journal UUID and installation/store generation, principal/release and cell/definition. Deployment must retain the expected identity and provide it to `open_required`. A new process or missing file must not cause identity reissuance. phase74 `ServiceOwner` owns the service root at the process level; the caller must retain it while using the journal. The journal's single current attachment is a serial execution attachment for that root, not a platform-wide one-Run-per-cell constraint.

`Preparation` pins the attachment UUID, run journal UUID, existing exact `journal::Scope`, original executor session/epoch and P read basis. A request with the same id/Scope/basis retrieves the existing record; a different body under the same ID or reassignment of the same run under another attachment ID is rejected. Closed history is not reused as grounds for automatic restart of the same run.

The service DB stores an immutable header, current/count, attachment history, run→attachment index and immutable `creation-entered` marker for run creation. The marker binds to the exact original Preparation and remains revision 1. Required-open compares header revision/identity, history count, indexes, phase-specific revisions, current pointer, marker and completion-observation structure. It also rejects missing markers in Attached/Closed and markers belonging to other reservations. Queries and mutations perform these comparisons too. This does not provide anti-rollback proof against copying an entire consistent historical DB.

The service header schema is `rx.executor-attachment-header.v2`. v1 journals without creation markers are rejected by required-open without automatic promotion. This distinction prevents missing files in historical v1 Preparing state from being mistaken for “a new journal not yet created.” Existing phases and record revisions (Preparing 1, Attached 2, Closed 3) and run-index semantics are preserved.

The run DB stores existing `executor/header` and separate `executor/attachment-binding` in the same local transaction. The latter pins service identity, attachment and run journal identities and the full original Preparation. This marker is not automatically added to an existing legacy run journal. Explicit migration remains future work. Existing `Journal::open` implementation and request/stop schemas are unchanged.

## Transitions and return boundaries

| Call | Durable result | Failure/restoration |
|---|---|---|
| `initialize` / `initialize_file` | Save header/state to an explicitly empty new service store | Reject reuse of existing files, records or event history. After commit-response loss, check through required-open with the same identity |
| `prepare` | Save Preparing, current pointer, run index and transition event in one transaction | Reject another current Preparing/Attached. Repeating the entire original content returns the same revision/ID |
| `initialize_run` / `initialize_run_file` | First commit the current Preparing's creation-entered marker/event to the service DB, then save the new run header/binding | Reject re-entry to all initializers if the marker already exists. Retrieve an exact existing file only through required-open. Not yet Attached; does not return a Worker Journal |
| `open_run_required` / `open_run_file_required` | RunStore with exact run Scope and service/run-journal identity validated | Reject missing/tampered header/binding or a different root identity. Available for Preparing/Attached/Closed query/restoration |
| `attach` | Commit a validated RunStore from Preparing→Attached | Return the existing request Journal only after success. After commit-response loss, read Attached and reopen the same run store to recover the original transition |
| `close_completed` | Save Attached→Closed, original P COMPLETED observation, current=None and event in one transaction | Same completion observation returns original record. Different content conflicts. A late repeated old close does not clear a new current attachment |

Service and run journals are different DBs, not one transaction. File wrappers use **marker commit → create_new → private run-header initializer**. Generic initialize_run also commits the same marker first and then writes the run journal. File creation when a generic caller opens Repository is the caller's responsibility; the product file boundary uses file wrappers. If the marker commit response is lost, do not proceed to file creation. A future service **must not emit remote mutations/BT requests before Attached commit**. Current RunStore/Journal/Repository APIs are for trusted Rust composition, not a capability sandbox against malicious code in the same process.

`close_completed` reuses shared production View validation to compare exact installation/store generation/cell/definition/recipe/run/original executor session, revision, completed parts and exhausted budget. It does not convert PAUSED/RECOVERY_REQUIRED/ABANDONED or incomplete parts to Closed. A future Client/service must first verify network authentication/freshness of the stored View and actual planner retirement. This module does not prove raw data was actually issued by P. Stop intent, original EMIT_ENTERED/PENDING requests and absent replies remain unchanged after close. Observing P completion is not grounds for fabricating a lost RPC response.

## File loss and incomplete initialization

File paths are determined by the deployment root and reserved run UUID. P responses supply neither paths nor executables. `open_file_required` and `open_run_file_required` accept only existing nonempty regular files, reject symlinks/special files and check SQLite integrity. The root must be an owned local path. No separate filesystem sandbox is provided against an external process replacing an owned directory.

`recover_current_file` reports `NeedsInitialization` only for Preparing when **there is no creation-entered marker and no file**. It does not create anything automatically. With a marker present, a missing file is an error even in Preparing. If a file disappears after initializer success or header commit-response loss, a new journal cannot be created under the same identity. Existing empty files, DBs without headers and DBs without bindings also remain errors. There is no recovery API that deletes files or markers and initializes the same run again.

Failure after marker commit but before file creation leaves only the marker; initialization retries are then rejected even without a file. Failure after create_new but before header commit leaves an empty file or headerless SQLite file. Required-open and initializer retries both fail in this case as well. Preserve the original Preparing, marker and file; a separate installation-recovery decision is needed. First creation entry may be retried only if the marker transaction actually rolled back. If only the response after the first header commit is lost and the file remains, required-open can recover the original identity. Incomplete creation, file loss and response loss are not treated as equivalent.

## Capacity and incomplete connections

History is limited to **1,024 attachments per service journal**. Permanent operation rejects new run preparation at that limit. Existing-ID lookup/same-request recovery remain available. There is no automatic deletion, count reset or bypass through a new root/identity. Retention/archival, restart of existing runs and rebinding to new sessions require separate design and verification. Production Views and events retain the existing canonical/storage size limits.

The next connection is to bind the separately implemented P 0/1/AMBIGUOUS query and Client results to this journal, and apply service journal initialization, deployment identity pins and planner lifecycle to the existing CLI. Implementing this journal does not mean a resident assignment service or automatic acceptance of new runs is complete.

## Authored counterexample tests

Separate `tests/assignment_journal.rs` covers required-open for service/run headers, incorrect scope/identity, missing/empty/symlink files, rollback and post-commit response loss in Preparing→Attached→Closed, creation marker rollback/response loss/tampering, Preparing file loss after completed creation/header-response loss, failure between marker commit and file creation, rejection of incomplete initialization file reuse, rejection of automatic adoption of v1 headers/legacy run journals, incorrect completion observations, rejection of Closed reassignment, preservation of a new current attachment on a late close, original stop/unanswered request preservation and corrupt current pointers. It calls no actual P API, BT or device. Parent verification of the previous change is distinguished from new verification results for this revision; this revision awaits parent revalidation.
