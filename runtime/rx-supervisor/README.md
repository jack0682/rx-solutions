# Solutions process management draft

## Catalog-owned execution requirements (F1)

`Program.execution_requirements` optionally declares named upper bounds, reserved capacity,
exclusive access or shared access. CPU quantities are millicores and memory quantities are bytes.
`None` preserves legacy serialization/startup; an explicit empty `Requirements` map is a
different declaration and digest. `NotRequired` and `Unknown` are distinct: unknown requests
never authorize startup. Zero capacity is not a way to disable a requirement.

Site `Process` inputs cannot replace the catalog declaration. Unknown fields and parameters
are rejected, and the existing plan digest pins the selected program including its requirements.
Public Rust authors add `execution_requirements: None` to legacy literals; this is an additive
Rust field, not source compatibility for unchanged struct literals. The initial catalog and
release image remain trusted inputs; F1 does not introduce a signed generic catalog format.

The `Backend::spawn_with_requirements` port owns whole-bundle admission/application and
process creation together, after the existing durable spawn intent. It must recheck lifecycle
authorization immediately before effects. `Rejected`/`NotStarted` promise no remaining
application, reservation or child; uncertainty must use the existing `Uncertain` channel.
Only a complete request-bound receipt can produce admitted status. Invalid/stale receipts
remain UNKNOWN and cannot trigger automatic retry. Existing lifecycle authority and
`RequiresPlatformAuthority` behavior for undeclared programs is unchanged.

The default backend implements **no cgroup v2, rlimit, CPU/memory reservation or device
access policy on any OS**. Every required policy is rejected before exec, with named reasons.
Empty/NotRequired-only declarations can use the existing spawn path with `NoRequirements`
evidence. Positive policy application in the F1 tests is explicitly **simulated**.

Status exposes `execution_admission` with `requested`, `application` and
`not_applied_reasons`. `Supervisor::execution_admission()` reads these observations
without admission, spawn, readiness or authority evaluation; `tick()` includes the same
map in its status. Pending/NotApplied with no reason means no attempt has been assessed.
`ReportedAtStart` is the backend's observation for one instance,
not continuous enforcement monitoring. Simulation and future host reports have different
evidence tags. After reopening, absent current-owner receipts are Unconfirmed; no current
resource ownership is inferred from stored process records. **A configured resource limit
does not establish a worst-case response time.** Computing-resource reclamation does not
establish collaboration-resource or physical handover.

The `execution_admission` integration tests cover whole-bundle positive/negative combinations,
site/catalog downgrade attempts, reservation/access semantics in a model, stale evidence,
lost authorization, and real non-actuating no-policy/rejection paths. Linux enforcement,
rollback under partial OS application, resource recovery and physical qualification remain
unimplemented. A future enforcing backend must validate those responsibilities before use.
This contract covers Supervisor-managed Program startup. Separate metadata-only initializers,
direct backend/OS callers and hostile changes to trusted host code are not globally sandboxed.

The following sections describe the original process-management baseline; F1 adds the
bounded requirement/diagnostic path above, not general resident-framework completion.

2026-09-11. Added `rx-supervisor` and `rx-solutionsd`. They record startup, observation and shutdown of selected programs in a separate S store, distinguishing process liveness from RX operating readiness. The product recipe at this stage is **one read-only status service**; lifecycle authority integration for actual device drivers/controllers remains future work.

## Repository and product boundary

The management module resides in `rx-solutions` and is included in the same image. The default catalog contains simulated device declarations; external device dependencies are added during individual integration. Program startup does not collectively launch all models. It checks that the selected profile ID exists in the catalog but does not thereby declare control readiness or physical validation for that model.

P run/operation/qualification/dispatch permits are not replicated into this store. Actual start/stop of control processes must connect to existing P/Host authority and validated site procedures. `LifecycleAuthority` is the port for that connection; the product default `SoftwareOnly` rejects programs with control effects. Test authority is used only with explicit simulated backends.

## Plans and program recipes

Site plans specify process ID, release program ID, allowed parameters, dependencies, startup/shutdown timeouts and restart limit/backoff. Site inputs do not supply executable/script paths, arbitrary argv, environment variables or effect classifications. The current `rx/status-http` recipe accepts only a fixed Python interpreter, RX status script and restricted bind/port.

Programs and effect classifications belong to the release-owned catalog. Executable and related-file SHA-256 values are checked. The plan digest binds not only the original plan but selected programs' actual execution definitions/file digests/effect classifications and selected device profiles. A changed program is not silently applied to the same saved plan.

Startup dependencies form a DAG of at most 32 programs. Missing/duplicate/cyclic dependencies and disallowed arguments/profiles are rejected. Startup/shutdown timeouts are currently limited to 100–30000 ms and software restart limits to 0–3. These are management software input ranges, not universally safe values for all controllers. Programs with control effects permit only automatic restart limit 0.

Integrity between program-file verification and actual exec assumes a read-only release image and trusted host OS. This stage does not claim atomic fexecve for generic writable executable paths or defenses against hostile host administrators.

## Durable state and OS execution boundary

| State | Meaning |
|---|---|
| PENDING | Startup intent not yet recorded |
| PREPARED | Fixed instance ID and startup intent recorded |
| SPAWN_ENTERED | OS execution-boundary entry recorded first. This alone does not confirm process creation |
| STARTING | Current owner acquired an actual Child handle/PID |
| PROCESS_READY | Designated process probe confirmed. Not Control prepared or qualification |
| UNREADY | Startup confirmation deadline exceeded. Process may still be alive |
| STOP_REQUESTED | Authorized stop intent recorded. Actual exit remains separate |
| EXITED | Current owner observed actual OS exit |
| SKIPPED | Not started before stop request |
| START_FAILED | Failure in which backend confirmed execution did not occur |
| UNKNOWN | Execution/ownership continuity unconfirmed. Re-execution and signaling stored PIDs prohibited |

Uses a dedicated SQLite store and single-writer lock. A store with existing entities/events for other purposes is not claimed as a new supervisor store. Programs are not executed inside transaction callbacks.

PREPARED and SPAWN_ENTERED are committed first, authority is rechecked, and then the OS backend is entered. After file verification and argument preparation, the backend rechecks authority immediately before exec. Even if state persistence fails after actual startup, records are supplemented only through the Child handle held by the same live owner; no respawn occurs.

If SPAWN_ENTERED was stored but the execution-entry response is unclear, or the manager restarts and loses its handle, the state is UNKNOWN. Backend “not executed” and “execution unknown” are also distinguished. A new owner does not adopt or terminate an existing process from a stored PID alone.

## Readiness, dependencies and shutdown

A generic alive probe confirms only process liveness. The current HTTP status service passes the generated instance ID in the child environment and verifies the same ID in responses. Another process responding on the same port does not count as ready. Dependencies start after satisfying the process probe; that probe is not used as a physical equipment condition.

Stop requests first latch in memory as well, preventing continued new process startup after persistence failure. Dependent processes are cleaned up before providers. Authority is checked before/after shutdown, and control-effect programs are retained without permission. A timeout alone does not force termination of a control process.

Non-actuating programs may be force-terminated after timeout following authorized normal shutdown. The OS backend verifies its owned Child handle before signaling. It cannot read another owner's PID file and kill that process. stdout/stderr are per-instance files; parent termination of a control process is not used as an input-EOF shutdown command.

If persisting observed EXITED fails, the same Child's exit observation can be recorded again. Once EXITED is definitively persisted, exited handles and timers are reclaimed so repeated normal reactivation does not continually accumulate memory objects. Unexpected exit without normal stop intent remains an error; only non-actuating programs receive configured additional starts and backoff. Control program reactivation requires a separate procedure. A process still alive after startup-probe timeout is not automatically converted to success.

## Invocation and current image paths

Default image entrypoint `serve`/`inspect` retain existing read-only diagnostics. Select the new management path explicitly as follows.

```text
/opt/rx/entrypoint.sh supervise /config/solutions-startup.json
/opt/rx/bin/rx-solutionsd inspect /config/solutions-startup.json
```

The [example plan](../../examples/deployment/solutions-startup.json) references `SIM-JTC-6DOF` but starts only the read-only status HTTP program. It starts no controller/robot Host; control_prepared and physical_shutdown_assessed are false.

state_subdirectory is a restricted relative path under `/var/lib/rx-solutions`. Directory and DB/lock symlinks are rejected. P DB must not be supplied as a volume. Command arguments/program paths and state paths undergo different validation.

Normal shutdown records are not automatically erased by the next `run`. Only when all processes are in confirmed terminal states and the full plan is non-actuating may explicit `rx-solutionsd activate CONFIG` prepare the software plan again. A plan left UNKNOWN after forced termination cannot be reactivated through this path either.

## Persistent component registration (F2)

`registration::Registry` owns a separate `Repository` and remains usable without
a plan, supervisor or execution. It records registration UUIDs, accepted catalog
references, revision-CAS changes, retirement and execution history. The module
does not import `Supervisor`, `Plan` or `State`. `registered::RegisteredSupervisor`
connects one non-actuating component to the existing supervisor and F1 backend.
This first placement in the same crate does not settle the final platform owner
of component management.

Registration accepts content; it does not grant lifecycle authority, functional
readiness or permission to use the component for work. Query without a scoped
assessment leaves both axes explicitly NOT_EVALUATED and treats saved execution
observations as history, not proof of present process/resource ownership. The accepted reference
pins the program ID and complete catalog Program digest; registration never
stores a mutable authoritative copy of F1 requirements.

Each new assignment is durably recorded before external effects. Retirement
blocks new assignments without stopping existing children or deleting their
records. Revision conflicts, changed catalog references and unresolved prior
assignments fail closed. The registration and supervisor stores are separate:
failure between them can leave an unresolved assignment. No distributed commit,
automatic recovery disposition, PID adoption or replay is claimed. Historical
assignments retain their original catalog and registration revision.

The `rx/status-http` author recipe now declares an explicit empty F1 requirement
set. Its `NoRequirements` receipt means no resource policy needed enforcement.
This changes the catalog/plan digest. Reopening a supervisor store pinned to the
old undeclared recipe is refused with a reason; the old records remain readable
and are not migrated or deleted. Preserve the old release/store, review execution
and unresolved state, and decide a reviewed transition before creating a new
plan. Creating a fresh store is not a recovery procedure for an unknown child.

Run the storage and simulated-backend regressions with an isolated target:

```sh
cargo test -p rx-supervisor --test registration --locked --target-dir /tmp/rx-registration-check -- --nocapture
```

The executable passage uses the actual existing service from a validated runtime
image and compiles the current supervisor test in a Linux builder. It checks the
installed service hash against current source, records immutable image IDs and
keeps commands, outputs and databases in a fresh evidence directory:

```sh
python3 tools/registration_passage.py --image RX_RUNTIME_IMAGE --evidence /tmp/rx-registration-passage-new
```

The `registration_passage` integration tests are opt-in because they need the
validated `/opt/rx` installation. The procedure asserts registration with no
execution, F1 admission, instance-correlated HTTP output, owned-child exit and
reopen in another manager process. It additionally exercises abnormal child
exit and manager-object recreation while a real child remains alive, labeling
that last probe separately. Independent functional/physical qualification,
positive work-use permission, positive binding acceptance, multi-host operation and Linux
resource enforcement remain unsupported. F3, F4 and F5 below extend this same procedure.

## Explicit software recovery disposition (F3)

The registration recovery API records a disposition separately from the original
execution observation. Its reference pins registration, instance, revision and
the entire original observation digest. That observation and earlier history
remain unchanged; later attempts to overwrite a disposed observation fail.

The first positive provider is `OsProcesses::observe_recovery_exit`: it observes
actual exit through an owned, non-actuating direct Child and retires that handle.
The opaque `OwnedExit` has neither public fields/constructor nor Deserialize.
Stored dispositions retain only its reference and digest, never a resurrectable
evidence token. The original instance and saved PID must match. Timeout, idle,
absence and elapsed time alone cannot produce a confirmed disposition.

`RecoveryAuthority` defaults to denying disposition and resume. Bind a custom
implementation to authenticated local policy; actor strings do not authenticate
callers. The type boundary does not establish the truth of arbitrary investigation
reports or defend against a hostile host administrator. Direct-child exit does
not establish descendant termination, resource recovery or physical outcomes.

`ConfirmedClosure` and `UnableToResolve` remain distinct. Both retain an unresolved
past outcome, make no resource-recovery claim and leave work-use permission
unsupported. A confirmed disposition alone does not enable a new assignment.
An explicit, authorized `ResumeRequest` binds one new run. Its permit is consumed
atomically with one new execution assignment. A failed transaction consumes
neither. Identical, still-unconsumed request delivery may be reauthorized; changed
or consumed requests and retained-token replay fail.

Use `RegisteredSupervisor::open_with_resume` with a fresh execution store/run and
zero automatic restart budget. It never resets the old UNKNOWN or reuses its
instance. Lifecycle authority, registration revision/catalog and F1 admission
are checked anew. No prior F1 receipt or work permission is restored. New query
fields show dispositions, requests, consumption and the recovery gate separately.

An unable-to-resolve disposition remains blocking because an external investigation
provider is not implemented here. This is not a design claim that an unresolved
past must block resources forever. A future authorized recovery procedure must
establish relevant current conditions, residual commands, control authority and
physical support/handover where applicable, then connect its evidence to the
original execution. Total manager-process/Child-handle loss, a missing original
PID binding and follow-up adjudication of an unable disposition are unsupported.
These provider limits also appear in query and passage output.

The same `tools/registration_passage.py` procedure now continues the real Linux
lost-manager-ownership scene through owned exit, separate disposition, rejection
of implicit assignment, explicit new-instance startup, normal exit and two replay
rejections. The retained old backend supplies the real exit evidence. This is
manager-object/store recreation, not recovery of a dead process's lost handle.
`tests/recovery.rs` adds real software-child, SQLite and injected-commit-failure
regressions. Independent functional qualification, positive work-use permission, positive binding acceptance,
resource enforcement, multi-host operation and physical qualification remain out
of scope. Older writers do not know the new disposition/frozen-observation rules;
semantic downgrade on a recovered registry is not supported.

## Reported readiness and work-use judgment (F4)

Author-owned `Program.functional_readiness` declares named intended-use profiles.
`RegisteredSupervisor::assess_use(UseScope, WorkUsePort)` compares a fresh component
self-report with the selected profile. `AliveOnly` remains unchanged and never
supplies functional evidence. The first provider supports owned non-actuating
HttpStatus programs. There is no new mandatory Backend method; other backends
default to an unsupported observation source.

`diagnostics/support-summary` compares the instance, schema and reported count
fields. `diagnostics/operator-connected` additionally requires CONNECTED, while
the current release literally reports NOT_CONNECTED. That failure is a static
release declaration, not a failed live connectivity probe. Counts are startup
audit/catalog derived; the instance comes from the process environment. Each
condition names its provenance. A response timestamp is not a new measurement
timestamp for those startup values. SATISFIED means only the declared self-report
conditions matched, not that actual functional operation, calibration, physical
readiness or business suitability was independently established.

The readiness axis distinguishes NOT_EVALUATED, NOT_MET, UNSUPPORTED and SATISFIED
with named conditions. Observations bind a unique assessment request to the
registration/catalog/run/instance/PID/scope/endpoint. Wrong-instance responses and
replayed observations do not satisfy conditions. Assessment performs no start,
stop, admission, disposition or persistent history write. Results are snapshots;
plain `query()` does not probe or restore a cached readiness judgment.

Work-use authority belongs to operating-area task judgment. The host has no
Granted variant, positive credential constructor or issuer. `NoWorkUseProvider`
reports UNSUPPORTED with a named provider-connection condition. An actual adapter's
reported DENIED decision has its own reference and named reasons; it is distinct
from an unconnected or not-yet-evaluated provider. Positive operating-area provider
connection/verification is unsupported, not a design claim of permanent refusal.
This is not HTTP access-control or a business-work dispatcher.

The two `View` fields now contain typed assessments rather than strings. This is
an explicit local query API shape change, not unchanged string-client compatibility.
Existing assertions now require named NOT_EVALUATED conditions on standalone/new
owner queries. Program literals need the optional declaration field; None is
omitted from serialization and retains prior digest inputs. The builtin declaration
changes its catalog/plan digest, so old accepted pins are refused for review,
without migration, weakening or deletion of unresolved records. Shared SDK,
protocol/normative documents and manifests are unchanged.

The same Linux passage includes ProcessReady with operator conditions NOT_MET and
readonly conditions SATISFIED with work-use provider UNSUPPORTED. Scope limitations
remain explicit: actual device operations, collaborative resource binding, general multi-hop dependency
execution, resource enforcement, multiple hosts and external recovery providers are
not supplied by this assessment.

### Non-actuating exit race found during regression

The pre-change backend could report a failed signal even though the same owned
Child had exited by the time the signal command returned. For NonActuating only,
the backend now rechecks that positive owned-exit evidence after a failed signal.
It does not relabel the failed signal as delivered. Successful signal acceptance,
confirmed owned exit, and live/unconfirmed outcomes are tested separately. Control
effects retain their previous errors, and timeout/force policies remain unchanged.
Neither a successful signal nor this narrow correction proves descendant shutdown,
physical stop or resource handover.


## Diagnostic dependency consumption (F5)

`registration::diagnostic::Consumer` is a repository-backed framework operation,
not a new executable or an alias for process liveness. Its author-owned `Catalog`
is pinned by a separate F2 registration. Provider and consumer may use separate
repositories; there is no cross-store atomicity claim. The source adapter in
`RegisteredSupervisor` supplies actual current F4 HTTP observations. The consumer
module has no dependency on the supervisor, process plan/state or stop APIs.

The lifecycle is `assign` (Assigned), `begin` (Running), optional `poll` and
`finish` (Completed with an immutable diagnostic result). Actual report samples
and provenance produce the result body/digest. Result creation and run completion
commit atomically, with control history. This is DIAGNOSTIC_ONLY; work use remains
UNSUPPORTED. An independent operation produces a real authored catalog summary.

Dependency declarations distinguish Undeclared, Unknown, Independent and Required.
Required profiles own their temporal interval, provider role and interpretation:

- `snapshot-after-preparation`: capture during assignment, then consume that
  captured input. Provider exit alone blocks the next assignment, not completion
  of the current run or historical diagnostic consumption.
- `snapshot-at-generation`: assignment/begin prepare the result envelope; obtain
  current input at finish. Provider loss withholds generation, not earlier results.
- `current-report-collection`: capture at assignment/begin/poll/finish. Current
  consumption also requires the same generation/configuration and report digest.

These are explicit caller-driven checkpoints, not an autonomous watcher or proof
of continuous availability between samples. `inspect` reports new assignment,
ongoing performance and result consumption separately, using named F4 conditions.
`recorded_result` reads intact history; `consume_result` additionally gates current
diagnostic consumption. It never turns a historical result into work permission.
Retirement or changed current input can withhold consumption without rewriting
old results. Repeated completion cannot overwrite a result.

A tracked relationship pins both registrations/revisions/catalogs, scope and
profile, then the first actual provider execution/configuration. It exists even
with zero provider executions, but is not an accepted work binding. Each opaque
Probe/ProviderObservation binds a fresh nonce to the relationship, diagnostic run,
phase and source. Another registration, a restarted execution, changed settings
or a replayed response cannot inherit it. Persisted samples are inert history,
not deserializable current-observation capabilities.

Initial and replacement `BindingJudgment` requests are distinct. Their reply has
no positive variant; default provider absence is UNSUPPORTED, and a reported
DENIED response has its own decision reference. The host issues neither binding
acceptance nor work permission. Same-format replacement does not modify old
relationships/runs/results. Explicitly tracking a new diagnostic relationship
creates a new identity; it does not transfer prior work or accept a work binding.

Existing Program literals/catalog digests, F1-F4 contracts and all four plan-local
`depends_on` behaviors (validation/topology, startup gating, loss reporting and
reverse shutdown blocking) remain unchanged. Local diagnostic documents are
additive; older code is not claimed to enforce their semantics. There is no
automatic deletion/expiry: limits are 64 relationships and 512 runs per consumer,
and 16 samples per run with a reserved finish slot. Archival, cancellation,
automatic scheduling, multi-hop graphs and positive provider integrations remain
outside this implementation. Existing control-effect signal-race limitations
also remain.

The same real Linux passage now has six workers and 24 observed stages. Its added
segment uses the installed report producer, a registered framework consumer,
actual persisted results, scoped loss reactions, same-format replacement rejection
and completion of unrelated catalog-summary work. These observations do not prove
physical operation, collaborative resources or Linux resource enforcement.

## Further connections

Tests check failures before/after startup commit, persistence failure after actual spawn, unknown backend results, supervisor restart, loss of shutdown authority, rejection of force termination for control processes, stop latch during storage failure, exit-observation repersistence, dependency ordering and bounded software restart. A separate actual non-actuating HTTP child also checks instance-correlated readiness, incorrect responses/file tampering and owned-process shutdown.

Release recipes, device/network permissions and P/Host lifecycle permit validation for actual ROS/driver/BT processes, native shutdown/support handover evidence, process adoption/proof of absence after restart and the full installation/update supervisor remain incomplete. Prepared ports or simulated authority do not count as substitutes for that validation. Current state output is stdout/storage; management command/status integration with the P operations UI also remains future work. The first physical cell is NOT_COMMISSIONED.
