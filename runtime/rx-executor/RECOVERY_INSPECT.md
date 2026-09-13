# Offline inspection of unfinished executor journals

`rx-executor-service cell recovery-inspect CONFIG` reads existing executor service journals and outputs deterministic JSON. It performs no actual P state query, current-session registration, completion of original cleanup requests, attachment adoption, device operation or planner execution. It is a separate command from existing `cell init`, `cell run` and legacy CONFIG invocation.

## Inputs and preservation of originals

It validates deployment configuration, installation marker, service scope and existing owned files. It does not compete with an owning service or SQLite writer. It does not create absent roots/owners/journals/headers or migrate schemas. Symlinks, special files, empty required DBs, different installations/configurations, tampered records and exceeded limits fail.

It does not open the source DB through a normal storage initializer. After acquiring the original writer lock, it reads DB/WAL/SHM into bounded temporary copies and performs SQLite validation on the copies. It does not use immutable-main-only reads that ignore WAL commits. Source file inode/time/content are rechecked, and existing locks remain held until completion. It writes no repairs/commits, new stops/requests or new events to originals.

It validates the assignment header, current pointer, Preparing/Attached/Closed and event history, and the connection between run creation markers and actual run headers. If Preparing has not entered creation and the actual Run file is absent, that state is reported unchanged. Missing required files after creation entry are not filled with new ones. Original request key/body/context/send/resolution and original stop state/revision/digest are reported separately.

## Output meaning

The schema is `rx.executor-recovery-inspection.v1`. It contains service scope and configuration/assignment digests, original attachment/Run journal/stop/attempt/observation records, unfinished-work totals and an inspection digest. No new time or PID is added, so repeated inspections of the same originals produce identical output.

`execution_authorized`, `network_accessed`, `session_opened`, `planner_started`, `current_p_state_observed` and `original_records_modified` are all false. A visible PENDING stop means the original request remains. The new reader's non-execution state is not evidence of historical planner termination. Successful inspection is not retroactively attributed as normal termination of the original service.

After full validation, it emits one JSON object to stdout. Failure emits no success JSON and exits abnormally. Inspection output is not evidence of current device state, physical support, participants/isolation or new production authority.

## Verification and follow-up

Local journal counterexamples, Linux actual CLI counterexamples and acceptance involving two network-none reads after ATTENTION/PENDING/exit1 of the actual product E are distinguished. Acceptance compares all original E data file contents/mtime/mode/size, unchanged stop/attachment, deterministic output and unchanged P/H authority/native calls. Test execution results and exact sources/images are managed in per-phase evidence in the documentation repository.

Subsequent collect that registers a new current E session is not yet provided. Such registration is a separate authority change affecting P epoch/blocks and requires ordering/CAS with Host recovery approval. Retrieval of original unfinished request outcomes, UNRESOLVED investigation disposition, resource handover and explicit RestartRun are separate implementation boundaries.

Before SQLite reads WAL, its structure, salts, cumulative checksum and complete commit end are checked. Even reused old-salt tails or incomplete/uncommitted tails accepted by normal SQLite are rejected here with `RECOVERY_WAL_TAIL_UNPROVEN`, without declaring corruption. Temporary SHM retains the original digest but is discarded from the copy SQLite reads; its index is rebuilt from the validated WAL. Original SHM is unchanged. Absent/zero-byte/valid header-only WAL means no current frames and does not prove a historical committed head. Detecting deletion of an entire WAL, truncation to a valid historical prefix or coordinated DB/WAL rollback requires a separate trusted historical reference. See the [official SQLite format](https://www.sqlite.org/fileformat2.html#walformat) and [WAL index/recovery explanation](https://www.sqlite.org/walformat.html).
