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

The generic Backend default rejects required policies before exec, with named reasons.
`OsProcesses` now overrides that boundary for the narrow Linux address-space ceiling
described in F8 below. CPU/memory reservation, aggregate memory and device policies remain unsupported.
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
lost authorization, and real non-actuating no-policy/rejection paths. F8 below adds one
actual Linux policy and gated rollback. General resource recovery, multi-policy rollback
and physical qualification remain unimplemented.
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

F2 introduced an explicit empty F1 requirement set for `rx/status-http`. F8 now
replaces that author declaration with a 256 MiB virtual address-space ceiling.
An empty set still means `NoRequirements` for other explicitly unbounded recipes.
Both declaration changes alter the catalog/plan digest. Reopening a supervisor store pinned to the
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
actual operating-area work/binding decisions, multi-host operation and general capacity
enforcement remain unsupported. F3 through F8 below extend this same procedure.

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

An unable-to-resolve disposition remains blocking; follow-up adjudication of that
immutable disposition is not implemented. The separate F9 kernel investigation
below supports total manager loss only with a recorded scoped birth identity.
Missing original birth identity cannot be retroactively supplied from a PID.

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
positive credential constructor or issuer. F6 below adds a proof-bearing reply. `NoWorkUseProvider`
reports UNSUPPORTED with a named provider-connection condition. An actual adapter's
reported DENIED decision has its own reference and named reasons; it is distinct
from an unconnected or not-yet-evaluated provider. Positive operating-area provider
service connection is unsupported, not a design claim of permanent refusal.
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

Initial and replacement `BindingJudgment` requests are distinct. A positive reply
requires the externally verified F6 proof below; default provider absence is UNSUPPORTED, and a reported
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
automatic scheduling, multi-hop graphs and actual operating-area provider connections remain
outside this implementation. Existing control-effect signal-race limitations
also remain.

The F5 segment has six workers and 24 observed stages including its baseline. Its added
segment uses the installed report producer, a registered framework consumer,
actual persisted results, scoped loss reactions, same-format replacement rejection
and completion of unrelated catalog-summary work. These observations do not prove
physical operation, collaborative resources or general Linux resource enforcement.

## Verified external decisions (F6)

F4 work use and F5 initial/replacement acceptance can now receive a
`Verified(Box<VerifiedDecision>)` reply. There is no boolean grant constructor.
The display state VERIFIED records verification at that assessment; it is not a
live credential. Only the receiving path checks a live proof against its sealed
current request. Work use, initial binding and replacement binding have distinct
signed kinds and cannot be substituted for one another. Signature success does
not promote readiness, start execution, rewrite results or apply a replacement.

Optional author-owned `decision_policy` fields in Program and the diagnostic
Catalog declare issuer/key, exact operating area/roles/kinds and maximum lifetime.
They have no Deserialize, site/env loader or arbitrary-key public verifier. Policy
is included in the registered catalog digest, and current revision/catalog is
checked before creating a sealed request. Changed keys cannot reuse an accepted
pin. None is omitted, preserving default catalog/plan digest inputs. Rust Program
literals need `decision_policy: None`; diagnostic Catalog literals likewise need
the optional field. Positive enum variants affect exhaustive matches. F5
AcceptanceAssessment now uses read-only accessors instead of mutable public fields.
The shared SDK, normative documents, wire/proto and manifests remain unchanged.

This boundary trusts author Rust and trusted registration callers. It protects
against untrusted site/decision input, not arbitrary author code changes, registry
misuse, OS/image replacement or malicious in-process code. An authenticated,
immutable release root is a named unsupported requirement for that stronger model.
**Every default production catalog has no anchors, so the shipped defaults cannot
approve anything.** Actual operating-area service/RPC integration is unsupported.

A receiver creates a fresh epoch and challenge nonce with the complete target,
policy fingerprint and local `Instant`. External signing covers an explicit
APPROVE verdict, challenge, decision ID and TTL. Verification reuses the vendored
`rx_package::verify_detached_message` (strict Ed25519); production code never signs
or reads a private key. Policy TTL is positive and capped at 60000ms; each issuer
can require a shorter maximum. The deadline starts at challenge creation, not
receipt/import time. Delayed import, reimport, cloning and decision-ID reuse
cannot extend it. Remote wall-clock expiry is not an input.

Live proofs have private fields and no Deserialize. Clones share a bounded
512-entry receiver ledger, original deadline and signed revocation state. Dropping
the owner invalidates all retained requests/challenges/proofs; reopening creates
a new epoch. Signed exact-decision revocation blocks subsequent receiving checks
and reimports. There is no automatic renewal or remote revocation feed.

Explicit `record_verified_decision` and `record_verified_revocation` calls append
inert references/digests to the registration repository. They do not make ordinary
assessment write history, change old results, or restore a live lease from stored
JSON. Old positive display snapshots remain historical; they must not be treated
as current permission after expiry/revocation. This is not a physical command gate
or a claim of atomic authorization through subsequent device dispatch.

The same Linux passage has seven workers and 32 observed stages. Its F6 segment
uses deliberately authored test catalogs and fresh ephemeral keys in a separate
OpenSSL process. The private keys never enter production APIs or commits. Tests
require an OpenSSL 3 CLI supporting Ed25519; the fixture uses
[`pkeyutl -sign -rawin`](https://docs.openssl.org/3.0/man1/openssl-pkeyutl/).
These positives prove that the test issuer held its key and signed the exact
statement, not that a real operating area approved work or that the decision was
correct. Actual device operation, collaborative resources, general Linux enforcement,
external recovery investigation, the control-effect signal issue and multi-host
operation remain outside this verification boundary.

## Resident registration ownership

`rx-solutionsd run|activate` now opens `RegisteredSupervisor::open_resident`.
One Supervisor still owns the entire plan and its OS children. One Registry
binds a separate registration UUID to each process selection before OS effects.
The existing startup schema and single binary remain unchanged. The state
directory adds exactly one SQLite store, `registration.db`, with its writer lock.
Registration and the initial selection index commit in one transaction.

The saved index is a lookup, not authority. It does not derive identity from a
plan ID, PID, port or display label. Missing/duplicate mappings, retired
registrations and changed author catalog digests are refused without rewriting
their histories. Selection-set changes require explicit review; there is no
automatic replacement/migration API. A pre-registration supervisor database
cannot be adopted merely by creating the new store. Preserve its records for
reconciliation; deleting them to obtain a clean start is not a recovery method.

Startup emits `rx.resident-reconciliation.v1` with each registration, execution
observations, supervisor state and execution admission. Lifecycle changes also
emit `rx.resident-registration-observation.v1`. After manager loss, saved PIDs
remain historical and unresolved executions remain UNKNOWN. `activate` cannot
erase that uncertainty. Normal stopped software can still be explicitly rearmed.

The resident constructor supports the existing multi-process and guarded
lifecycle path. Legacy guarded programs keep `execution_requirements=None` and
`LegacyNotDeclared`; they do not acquire an F1 receipt or an empty declaration.
Dependency stop order, cooperative-stop confirmation and no forced termination
of guarded services remain in the same Supervisor/OS backend. `Launch.selection`
is an in-memory Rust field, not a persisted/shared-contract schema change.

The original `open`/`open_with_resume` keep their single non-actuating component
and explicit-requirements restrictions. Resident mode intentionally refuses the
single-component assessment/decision APIs, even for a one-process resident plan.
Its ordinary lifecycle path does not infer work permission or perform F5
consumption/replacement. F9 recovery and F10 report work are separate explicit
paths described below. Default decision anchors remain absent, and the original
single-component assessment APIs stay restricted. No registration network API is added.

`tools/resident_registration_passage.py --image IMAGE --evidence FRESH_DIRECTORY`
builds the candidate daemon and runs it at `/test/rx-solutionsd` against the
unchanged validated runtime image. Two real read-only status services demonstrate
registration after the launcher exits, normal reopen, explicit activation and
UNKNOWN after a forced test-container crash. This does not install or publish a
new release image. The original 32-stage library passage remains separate.
Guarded composition regressions exercise a fake backend; this new daemon passage
does not qualify actual Host/Executor integration or physical equipment.

## Linux address-space enforcement (F8)

`AddressSpaceBytes` is a **per-process virtual address-space usage ceiling**. It
is not RSS, physical RAM, reserved capacity or a guarantee that an allocation is
available. Linux RLIMIT_AS constrains virtual mappings; it is not an aggregate
process-group limit. Descendants inherit limits but are not collectively budgeted.
CPU, physical-memory and access/reservation requirements remain named unsupported
conditions. A mixed bundle is refused before any application; no supported subset
is silently admitted. Non-Linux backends refuse the requirement rather than skip it.

The existing status recipe now declares 268435456 bytes (256 MiB). In the measured
runtime, startup `VmPeak` was 52219904 bytes, approximately 49.8 MiB; the selected
ceiling is 5.14 times that observation and the service remained healthy under it.
This is a scoped engineering choice, not a universal workload/availability claim.
Changing this declaration changes the author catalog digest: an existing F7
registration is refused with `resident catalog digest changed`. Preserve and
review the old records; there is no automatic migration or weaker fallback.

The Linux backend initially supports only author-verified `/usr/bin/python3`
non-actuating programs. A fixed embedded bootstrap waits on a private inherited
socket. The bootstrap uses Python isolated mode with bytecode writes disabled,
so current-directory and user-site modules cannot run before limit application.
One live Child owns the same PID throughout bootstrap and target exec.
The parent applies soft and hard RLIMIT_AS through safe `rustix::process::prlimit`,
reads `/proc/PID/limits`, then rechecks registration and lifecycle authorization
before sending EXEC. After EOF it independently checks target argv, the limits and
the owned Child. EOF alone (including a killed gate) cannot mint a receipt.
No project unsafe-code exception, new daemon, installed helper, network API or
container memory/ulimit setting is introduced.

The `LINUX_RLIMIT` receipt contains a private-field `KernelObservation`, tied to
the whole request. It records a parent observation after exec, not an assertion
that the operating policy is correct. Public HostReport/Simulation tags are not
promoted to this kernel-observation type. Stored `resources.last_observed` is a
separate inert DTO; it cannot be deserialized into a KernelObservation or Receipt.
`ReportedAtStart` remains historical, never a continuous monitor.

If application, parent observation or authorization fails before EXEC, the gated
child must be killed and its exit confirmed before whole-bundle rejection. An
explicit exec error is likewise rolled back. Unconfirmed rollback or uncertain
exec entry retains the Child and reports `Uncertain`; it is never relabeled
NotStarted. Tests exercise a real EPERM after lowering the hard ceiling, failure
of observation after application, authority change, exec failure and a gate killed
in the exec window. **Multiple kernel policies' partial application is not
established by these tests.**

Supervisor records preserve `resources` history with distinct lifetime states.
Owner loss makes it UNCONFIRMED and does not restore a receipt after restart.
Confirmed direct-child exit is explicitly `DIRECT_CHILD_EXITED_DESCENDANTS_UNASSESSED`;
it is not proof that descendants or collaborative resources were reclaimed.
`capacity_reservation` is NONE_CREATED only where this backend established no
reservation; unresolved attempts remain NOT_ESTABLISHED. `physical_handover` is
NOT_ASSESSED. Existing stores may omit the additive history field; older writers
are not claimed to understand it.

The actual daemon and the separate supervisor-process observer are exercised by:

```sh
python3 tools/resource_enforcement_passage.py --image RX_RUNTIME_IMAGE \
  --baseline-daemon /absolute/path/to/F7/rx-solutionsd \
  --evidence /absolute/path/to/fresh-resource-evidence
```

Supply a Linux F7 daemon built from contribution
`9fa53afe308f0df89b09d2fa01e92a2e753181cb` for the old-catalog refusal comparison.
The procedure runs the existing resident passage, observes current daemon and
child limits from a separate process, checks unchanged container resource
options, and preserves the old registration/execution entities on digest refusal.
Its explicitly authored 64 MiB test child is launched by the real RX backend and
fails a 128 MiB mmap with ENOMEM. This is address-space enforcement evidence, not
physical memory pressure or physical-equipment qualification. The existing library
passage retains its 32 stages; its old empty-requirement assertion is now strengthened
to the actual Linux receipt and exact soft/hard limits, and its Owned test wrapper
forwards requirement-bearing starts to the real OS implementation.

Kernel semantics: [Linux getrlimit/prlimit manual](https://man7.org/linux/man-pages/man2/getrlimit.2.html).
Delegation boundary: [Linux cgroup v2 documentation](https://docs.kernel.org/admin-guide/cgroup-v2.html).
The runtime probe measured a read-only cgroup mount and EROFS on child creation;
that excluded cgroup v2 for this deployment posture. It is not a claim that
cgroup v2 cannot enforce limits under a properly delegated host configuration.

## Total manager loss: scoped process investigation (F9)

`rx-solutionsd investigate CONFIG` opens the recorded run, preserves UNKNOWN and
reports what the Linux kernel can establish about the original **direct process**.
It does not adopt, signal, reap or replay a saved PID. Alive and unverifiable
findings do not automatically create an immutable UnableToResolve disposition.

The real OS backend captures birth identity while it still owns an unreaped Child.
The opaque `OwnedProcessIdentity` is bound to the complete launch request; the
supervisor commits its inert stored form with PID and Starting state, then copies
it to the matching registration execution. Capture or persistence failure leaves
no usable cold-recovery evidence. No post-restart backfill is performed.

The context includes boot ID, observer PID/time/user/mount namespace identities,
UID/EUID, validated proc mount/view, PID-1 start ticks and time-namespace offsets.
Each context is read twice coherently and checked around the process observation.
A namespace-inode match alone is insufficient: sequential containers actually
reused all observed namespace inodes. The initially failing counterexample is
retained in the F9 evidence. An init-birth match raises confidence; it is **not**
a cryptographic or permanent unique namespace UUID. The lifetime inference is
that simultaneously live namespaces cannot share the kernel inode and reuse
occurs only after the prior namespace is gone. This inference is not a positive
observation of the saved process and must not bypass the scope guard.

A mismatch means `saved-namespace-not-observable-in-current-scope`: observations
of this namespace's PID cannot describe the saved namespace. **Replacing the
container leaves the old record permanently Unverifiable through this kernel
path.** Crossing namespace lifetimes requires another evidence provider, outside
this scope. Likewise, pre-F9 records without stored birth identity are permanently
unsupported by this provider, not temporarily waiting for a PID backfill.

Within a matched context, `pidfd_open` ESRCH establishes scoped kernel absence;
otherwise two proc stat reads compare the original start ticks. A different start
time means the PID was reused; a matching live process blocks resume. A same-tick
identity collision conservatively blocks rather than proves closure. Zombies,
process-read races, scope-read races, missing/hidden proc data, permission errors
and unsupported platforms are distinct Unverifiable findings. The observer pidfd
never enters the owned Child map and is never signalled.

`Registry::investigate(ObservationRef)` reads the original record itself. Its
private-field `ProcessInvestigation` has no deserializer or public constructor.
Legacy string Investigation references remain readable history only; they cannot
be submitted as new evidence or converted into this type. Stored typed summaries
also cannot restore current evidence. This trusts the existing journal and OS;
it does not cryptographically authenticate truth against malicious DB/OS authors.

Explicit `rx-solutionsd resume CURRENT_CONFIG NEXT_CONFIG` initially supports one
`rx/status-http` selection, the same state root and identical reviewed plan except
for a new run ID, with zero automatic restart budget. The status recipe creates
no subprocesses. OriginalNotRunning evidence permits ConfirmedClosure followed by
a fresh investigation and the existing F3 one-shot ResumeRequest/Permit. Original
UNKNOWN and PastOutcome::Unresolved remain unchanged. No past outcome, descendant
shutdown, resource handover, physical result or work permission is recovered.

There remains exactly one root `registration.db`. Original `supervisor.db` is
preserved. Explicit resume creates **`runs/<run-id>/supervisor.db`**, a new execution
journal kind justified by F3's fresh-run/store isolation requirement. Ordinary
`run` selects only the original or an already existing matching run store; an
unknown run ID is refused without creating a new run store. A crash after creating
an incomplete new run store fails closed; automatic journal repair is not supplied.
The registry's single writer lock spans the old and new execution stores.

```sh
python3 tools/manager_loss_passage.py --image RX_RUNTIME_IMAGE --builder RUST_BUILDER \
  --evidence /absolute/path/to/fresh-manager-evidence
# Separate counterexample fixture, not the product security posture:
python3 tools/manager_loss_passage.py --image RX_RUNTIME_IMAGE --builder RUST_BUILDER \
  --pid-reuse-fixture --evidence /absolute/path/to/fresh-reuse-evidence
```

The ordinary passage uses UID10001, no capabilities, read-only root and an
external PID-1 fixture observer to retain the namespace while killing the actual
manager. A pre-loss observer pidfd is used only for fixture cleanup. The separate
reuse fixture enables only CAP_CHECKPOINT_RESTORE with seccomp unconfined and UID0
inside a private PID namespace. It actually reuses the RX child's PID through
clone3 and keeps the unrelated replacement alive while RX investigates/resumes.
That privilege creates a counterexample; it is not RX's recovery mechanism.
The product posture's clone3 set_tid probe returned ENOSYS.

Kernel sources: [proc stat starttime](https://man7.org/linux/man-pages/man5/proc_pid_stat.5.html),
[pidfd_open](https://man7.org/linux/man-pages/man2/pidfd_open.2.html),
[namespace identity/lifetime](https://man7.org/linux/man-pages/man7/namespaces.7.html),
[PID namespaces](https://man7.org/linux/man-pages/man7/pid_namespaces.7.html), and
[time namespaces](https://man7.org/linux/man-pages/man7/time_namespaces.7.html).
The stored optional birth field is additive; old records remain readable without
it, but older writers are not claimed to understand new evidence. The Rust
Investigation input now requires typed provider evidence; the former string API
is intentionally source-incompatible. No shared SDK, wire contract, new daemon,
new service or second production binary is introduced.

## Enforced support-gap work commitment (F10)

A work judgment now guards a concrete, bounded non-actuating operation:
**support-gap report commitment**. Given required native-package/profile counts
and current reported counts, the result contains observed, required and
`shortfall = max(required - observed, 0)` for each. Reported profiles 4 versus
required 6 produces shortfall 2; changing the requirement to 4 produces 0.
These calculations describe the supplied self-report, not equipment qualification
or an operating-area policy judgment. The output is a work result, not a grant receipt.

F5 diagnostic generation/consumption remains DIAGNOSTIC_ONLY with work use
UNSUPPORTED. Measurement found no existing work action there or in the status
service (POST returns CONTROL_NOT_EXPOSED). Gating those reads would change their
meaning. Instead, the new operation commits a distinct result and decision
consumption to the **existing registration repository**. Its CAS and atomic
control events provide one completion boundary without another writable output
mount, database, service, process program or daemon.

`RegisteredSupervisor::prepare_work(Task, WorkUsePort)` obtains fresh self-report
readiness and asks the external provider about the exact work context. Only a
verified external F6 decision can construct the private, non-deserializable
`work_use::Prepared`. The operation ID, demands, input digest, current registration
and revision, program/catalog, run/instance, configuration and authored readiness
meaning are additional bindings. F6 issuer, kind, context, policy, epoch, monotonic
TTL and revocation checks remain intact. The fixed work role is
`work/support-gap-report`; its observation profile is `diagnostics/support-summary`.
Caller task data cannot replace either role, the catalog or its issuer policy.

`commit_work(&Prepared)` reobserves and rejects changed input/readiness/context,
then rechecks current registration/execution inside the transaction. It writes the
derived result under the operation ID and a unique decision-consumption record
using CAS, together with their control events. **The logical cut is the last live
decision check in a successful atomic transaction.** This is the claimed use and
completion point; it is not a claim about the later commit-IO completion clock.
A failed transaction has no completed work or consumed authorization. A still-live
prepared proof may be retried only through all current checks again.

The SQLite Immediate transaction holds its write lock from the transaction's
start, closing registration/revision and consumption-key changes underneath it.
A private receiving helper retains the F6 revocation-ledger mutex across the
**entire transact call**, including physical commit. A concurrent revocation waits
and takes effect for the next receiving check; it does not retroactively rewrite
a committed result. Tests also cover a waiting revocation after transaction
rollback, where a subsequent retry is refused as revoked.

Exactly two timing residuals remain:

1. A monotonic TTL may expire during commit IO **after the logical cut**. A
   successful transaction still retains its result; it asserts validity at the
   logical cut, not at IO completion. A delayed-commit test exercises this case.
2. The HTTP self-report and SQL commit are not physically atomic. Counts retain
   their own observation timestamp, execution instance and F4 ReportOrigin. They
   do not claim continuous or commit-time truth about an external world.

Duplicates return revision conflict without another result or permission
consumption. A lost result response is resolved by `recorded_work(selection, id)`;
resubmission cannot execute the same operation again. An injected rollback after
both writes leaves both absent. An injected error **after successful commit** is
response loss, not rollback, and the operation ID recovers its durable result.
Receiver restart invalidates retained proofs while historical results remain.

`work_use::Report` is a deserializable, inert history DTO. It records observation
provenance and labels signature verification separately from operating-area policy:
`EXTERNAL_SIGNATURE_AND_CONTEXT_VERIFIED_AT_LOGICAL_CUT`,
`NOT_EVALUATED_BY_HOST; PRODUCTION_PROVIDER_NOT_CONNECTED`, and
`NONE; HISTORICAL_WORK_RESULT_ONLY` for current permission. Neither a report, a
stored F6 reference nor a past WorkUseAssessment can become Prepared. The host
adds no signing key, positive issuer or site-configurable anchor.

The same daemon optionally accepts one task as `rx-solutionsd run CONFIG WORK_TASK`:

```json
{
  "operation": "f07f98af-8b41-4af7-a5f5-5e95337b5f59",
  "selection": "status",
  "operating_area": "example/area",
  "required_native_packages": "1000",
  "required_support_profiles": "6"
}
```

This mode evaluates the task once after the selected status process leaves its
startup phases and emits `rx.work-use-result.v1`. Shipping catalogs and their
digests are unchanged and still contain **no decision anchors**. Consequently
shipping work is denied with a named reason while ordinary daemon startup and
diagnostic reads remain available. There is no production operating-area provider
connection. Positive tests use a separately authored catalog and an external
OpenSSL test issuer; they are not actual operating approvals.

```sh
python3 tools/work_use_passage.py --image RX_RUNTIME_IMAGE --builder RUST_BUILDER \
  --evidence /absolute/path/to/fresh-work-evidence
```

The actual runtime passage runs the shared receiving code with installed release
counts, confirms derived output under the test issuer, changes conditions between
judgment and commit, and exercises expiry/revocation/duplicates/response loss/store
failure. A separate observer confirms the shipping daemon is healthy after work
denial and has zero result/consumption rows. The unchanged library32, resident,
resource and manager-loss passages remain separate regression evidence.

This adds local Rust task/result APIs and registration document schemas. It does
not change Program serialization, shipping catalogs, shared SDK, wire/proto,
physical-operation permission or F5 diagnostic meaning. Existing observers of
historical decisions still do not hold a current authorization capability.

## Explicit dependency replacement (F11)

The diagnostic `Consumer` application API can explicitly apply an externally
approved replacement. `prepare_replacement(ReplacementIntent, BindingJudgment)`
verifies approval but changes no route. A separate
`apply_replacement(&PreparedReplacement, Source)` reobserves the candidate and
atomically commits an immutable binding version, one active route, an application
receipt and decision consumption in the existing registration repository.
`assign_current(root, Source)` resolves that route; it rechecks the route inside
the assignment transaction, so an observation racing a switch cannot publish an
assignment to the superseded version.

Runs and results keep their binding IDs. Every advance still reads its binding,
but a sealed binding cannot change. Old runs can therefore finish against A while
new runs use B. There is no drain barrier: these are non-actuating diagnostic input
relationships, not exclusive physical or process ownership. There is one route
for future assignments per relationship and one version per run. Replacement does
not kill, adopt or transfer a provider process. Returning to A requires a fresh
approval and explicit application with another new binding ID.

Approval binds the application ID, current route/revision, old binding/revision,
new binding ID and proposed registration/generation in addition to all F6 context,
issuer, receiver epoch, TTL and revocation checks. The old binding must already
have a pinned generation. A changed consumer revision cannot reinterpret that
binding; historical receipts remain readable. Shipping catalogs have no anchors,
so replacement defaults to named refusal while ordinary diagnostic work remains
available. The existing verified-acceptance reason still correctly says it is not
automatic replacement or work-use permission.

One SQLite Immediate transaction commits the route CAS, immutable memberships,
receipt and unique consumption key. Concurrent applies from the same revision
cannot both win. Partial writes roll back; a lost successful reply is resolved by
`recorded_replacement(application)`, never replayed as new authority. Restart
recovers either the old route or the committed new route and inert history, not
source ownership or a live approval. PreparedReplacement is opaque and cannot be
deserialized or made from assessment/receipt DTOs. Stored receipts explicitly say
historical application only, no process adoption, physical handover unassessed,
operating-area policy not evaluated by the host, and work_use Unsupported.

The logical cut is the final live decision check in a successful transaction.
SQLite serializes route/registration/consumption changes; the F6 ledger mutex is
held through physical commit and blocks interleaving revocation. Two timing
residuals remain: TTL can expire after the cut during commit IO, and the fresh HTTP
observation is as of its own timestamp/instance, not atomic with the SQL commit.

Inspection, routing and receipts emit `CheckpointPolicy`. There is no timer or
bounded detection delay. Preparation-only captures at ASSIGN and reuses that
input; result-generation captures at FINISH; continuous observes at explicit
ASSIGN/BEGIN/POLL/FINISH calls. INSPECT explicitly observes dependent providers,
and APPLY probes its candidate. At the next required observation, known loss or
changed identity/report is NOT_MET and unavailable evidence is NOT_EVALUATED.
Affected transitions/result consumption are withheld; history is preserved.
Unrelated diagnostic work can continue. No automatic stop, retry or replacement
and no continuous monitoring service is added.

```sh
python3 tools/dependency_replacement_passage.py --image RX_RUNTIME_IMAGE \
  --builder RUST_BUILDER --evidence /absolute/path/to/fresh-replacement-evidence
```

This exercises the application API with real Linux HTTP processes, an external
test signer, concurrent applications, partial staging rollback, reply loss and
actual manager SIGKILL before/after commit. It also switches a route while a run
is running and shows that run still consumes A while a new run consumes B.
The initial mutable-row counterexample is retained as a test intervention, not a
replacement API. There is no new resident routing CLI or production operating-area
provider integration. The five existing passages remain distinct regression
checks. This adds four registration document schemas and additive Inspection
output, but changes no Program/catalog/Run/TrackedBinding layout, SDK or wire API.
Older writers do not enforce these routes; downgrade writing is unsupported.

## Open items and enforced support limits (F12)

Every resident command verifies the installed status script and device catalog
against source bytes embedded in the compiled Rust binary. Matching a changed
file to a forged adjacent inventory no longer suffices: mismatched inventory pins
produce `release/source-pin-mismatch`, and mismatched installed bytes are refused.
The inventory is a consistency index, **not an authenticated root**. The emitted
`release_boundary` trusts the installed Rust binaries and OS. Interpreter and
Host/Executor binary digests still rely on inventory consistency within that
explicit trust boundary. Authenticated immutable provenance remains the named
`AUTHENTICATED_RELEASE_ORIGIN` successor, not a completed feature or an item
silently excluded from the startup path.

Supervisor, Host and Executor CLI failures caused by the pinned SDK's writer-lock
acquisition emit `rx.support-refusal.v1` with condition
`storage/exclusive-writer-not-established`, decision REFUSED and owner_identity
NOT_ESTABLISHED. The Host's separate runtime-owner refusal is named
`host/runtime-ownership-unavailable`. A failed lock acquisition does not identify
another owner. No automatic retry, lock-file deletion or PID termination is added.
The diagnostic adapter narrowly recognizes the current SDK's legacy error prefix;
it does not change the SDK or reinterpret unrelated I/O failures.

The historical Host flake is **not fixed**. Actual Linux parallel stress reproduced
Host and store lock failures; macOS 50 rounds did not. Excluding the old crash-child
spawner as a measurement control produced 50 clean rounds, while the unmodified
parallel test suite remains in normal regression runs. Actual Command pre-exec
measurement with the unmodified storage implementation shows an inherited file
description can retain a lock after parent repository drop until exec. In contrast,
the real resident status children retained no DB/writer-lock descriptors after
exec, and a parent observer reacquired both locks after manager SIGKILL while those
children remained. Do not claim a lasting product-child leak from the pre-exec
counterexample. A controlled explicit-unlock prototype is evidence only, not
product code. `STORAGE_LOCK_LIFETIME` must address the platform-owned guard/error
and regenerated SDK separately; these observations do not attribute the single F9
CI failure with certainty.

Guarded services are an admitted shipping software path: the daemon uses
GuardedServices, not SoftwareOnly. Failed TERM response is not completion; the
owned child and stop request remain. A later current-instance final report plus
owned exit0 can confirm cooperative shutdown; missing final report refuses that
conclusion even when all_exited is true. Force remains refused. Real Linux tests
exercise these distinctions with explicit delivery-failure injection, without
claiming to reproduce every natural signal race or physical shutdown.

Source trust, lock refusals, future-schema refusal, initializer replay refusal,
actual descriptor observations and guarded stop boundaries are reproducible with:

```sh
python3 tools/support_limits_passage.py --image RX_RUNTIME_IMAGE \
  --builder RUST_BUILDER --evidence /absolute/path/to/fresh-support-evidence
```

The output intentionally says `ENFORCEMENT_PASS_LOCK_LIFETIME_UNRESOLVED` and
retains every stress outcome; an observed failure is not replaced by a later clean
run. F10's post-cut TTL expiry and HTTP/SQL non-atomicity remain permitted temporal
semantics, not refusal scenes. F9 scope/legacy-identity refusals and F11 explicit
checkpoint/no-monitor semantics remain unchanged. Diagnostic inspection no longer
contains pre-F8/F9/F11 global "unsupported" statements that contradict those
narrower implemented paths. None of these outputs creates current authority,
physical qualification, multi-host support or operating-area provider integration.

## Further connections

Tests check failures before/after startup commit, persistence failure after actual spawn, unknown backend results, supervisor restart, loss of shutdown authority, rejection of force termination for control processes, stop latch during storage failure, exit-observation repersistence, dependency ordering and bounded software restart. A separate actual non-actuating HTTP child also checks instance-correlated readiness, incorrect responses/file tampering and owned-process shutdown.

Release recipes, device/network permissions and P/Host lifecycle permit validation for actual ROS/driver/BT processes, native shutdown/support handover evidence, process adoption and the full installation/update supervisor remain incomplete. The F9 section defines the narrower supported kernel proof of direct-process absence after restart. Prepared ports or simulated authority do not count as substitutes for that validation. Current state output is stdout/storage; management command/status integration with the P operations UI also remains future work. The first physical cell is NOT_COMMISSIONED.

## Storage ownership lifetime (G1)

G1 supersedes the normal-close lock-lifetime limitation in the historical F12
section. The regenerated SDK closes SQLite explicitly before releasing the shared
file description. A fork child's copy therefore no longer delays legitimate
normal close/drop. A live writer still refuses another writer, and an inherited
repository or borrowed transaction cannot execute SQLite operations, commit,
rollback or unlock its creator's ownership. Runtime creator checks enforce this;
a non-cloneable Rust type alone cannot prevent fork copying it.

The Host runtime guard uses the same source-owned `ExclusiveFileLock` and retains
its previous complete service/maintenance scope. The F12 string parser is removed:
`rx.support-refusal.v1` now derives its condition from typed `StoreError::Ownership`.
Acquisition failure, contention, foreign-process use, unconfirmed connection close
and unconfirmed release are distinct local errors. No wire/document schema change,
retry, lock-file deletion, force termination or process adoption is introduced.

Close failure retains connection and this process's lock descriptor for process
lifetime, without an active quarantine service. SIGKILL/abort cannot execute Drop.
A pre-exec inherited description can therefore still block reopening until exec
or exit. The named condition remains `storage/exclusive-writer-not-established`
and its recovery text explicitly names that abrupt-loss window. G1 does not erase
that refusal or establish an old operation's outcome.

The platform's `tools/storage_lock_passage.py --solutions SOLUTIONS --image IMAGE
--builder BUILDER --evidence FRESH` runs opposing release/refusal scenes and all
predetermined Host stress rounds without exclusions. Its separate single-thread
fork fixture is measurement code, not a shipped daemon. Existing F12 passage
labels and historical evidence remain historical; G1's source-origin guard,
actual overlap scenes and compatible commits are the current lifetime evidence.

## Authenticated development release checkpoint (G2)

The compiled root lives in platform `rx-package::release::root`, exported into the
SDK. `release_programs` and `add_guarded_services` authenticate signed metadata and
all inventory bytes before deriving recipes; the daemon shares the same opaque
verified object between both projections. The inventory cannot introduce keys or
substitute expected content. F12's two compiled source-content pins remain.

`rx-solutionsd init/run/activate/investigate/resume` record the signed revocation
checkpoint and admit the release before initializers, registration or execution.
The installation-wide floor is `/var/lib/rx-solutions/release.db`, independent of
plan `state_subdirectory`. This adds one local store, no daemon, service or network
endpoint. G1's writer lock and transaction apply. The store closes after the
admission checkpoint; it is not a background revocation monitor or global runtime
ownership lock. Inspection authenticates captured content but neither advances
nor certifies the durable floor. Its report is not permission.

The six public refusal conditions distinguish unsigned, invalid signature, unknown
key, revoked release, rollback and content mismatch. `rx.release-refusal.v1`
reports their names; malformed input and state failures remain distinct. An old
unsigned image is refused by the new supervisor. Build the ordinary image first,
sign its final inventory offline with platform `tools/sign_release.py`, and create
a derived image adding only `manifests/release.json` and `manifests/revocations.json`.
Do not copy the signing key into either image or its build context. Existing
passages accept this signed image through their unchanged `--image` input.

The public key is **development-only**. Product release custody and rotation are
**NOT_ESTABLISHED**. The verifier and OS remain trusted installed binaries, not
self-authenticated artifacts. Whole-state rollback/deletion is **NOT_DETECTED**;
the floor protects ordinary restart with intact retained state. Offline revocation
freshness is **NOT_ESTABLISHED**. `release_boundary()` reports all four limits.
Checks are `EXPLICIT_CALLER_DRIVEN_CHECKPOINTS`, with
`NO_TIMER_OR_BACKGROUND_MONITOR`; trusted installation stability remains required
between checking bytes and use. Existing per-launch file hashes remain in force.
Direct OS/Host/Python invocations and custom trusted Rust compositions are not
sandboxed by a supervisor checkpoint. This is release-recipe authentication, not
operating-area work authority, host attestation or physical qualification.
