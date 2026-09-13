# Resident cell service

`cell_service::CellService` owns one required-open assignment journal, one pinned cell,
one existing authenticated Client, and one planner factory. The caller must retain
`ServiceOwner` for the root and validate its required journal before `Client::connect`.
The service never opens another session between Runs.

`new(client, assignments, root, factory, clock, service::Options)` accepts only
`SerialProduction`. `initial_status()` seeds a watch channel; `run(shutdown, updates)`
returns `cell_service::Report { status, last_run }`. Status distinguishes `IDLE`,
`ARMING`, `RUNNING`, `ATTENTION`, and `STOPPED`, and embeds the active RunService
status. Timing reuses the existing bounded poll/grace/stop options. Transient idle
read failures back off to at most one second; shutdown remains responsive.

With no durable attachment, NONE waits and a current pending/ARMING start is only
observed. AMBIGUOUS, a foreign/expired start, a restricted Run or changed configuration
returns Attention. No UUID/revision ordering selects a winner. A SINGLE EXECUTING
production candidate must also pass a fresh authenticated Production.Inspect with
current admission, caller, configuration digest, epoch, recipe and mandate.

The service commits the exact Preparation once, including its two new UUIDs and
original Scope/Basis. Existing Preparing or Attached always fixes that original Run.
Recovery uses creation-entered and required-open run binding verification. A missing
file after creation-entered, a partial header, or a different identity cannot become
a new journal. Attached commits before a Worker or planner receives the journal.
Old session/runtime/epoch, PAUSED/RECOVERY_REQUIRED and existing stop intent require
attention; this service does not implement automatic same-Run restart or rebind.

The attached Run uses existing `RunService::run_owned` and serial production handling.
The same Client, factory and request journal return on exit. An independent Run B
appearing in discovery does not replace or revoke attached A: A is read directly.
Only StopReason::Completed, PauseObserved with the actual COMPLETED observation,
confirmed planner cleanup, no durability fault, the exact saved stop, and a fresh
exact P COMPLETED production view permit the assignment to close. The run repository
binding is verified again before closure. Original request and stop histories remain.
The same Client then waits for the next Run.

Idle shutdown creates no stop intent and sends no PauseRun. Active shutdown delegates
to the existing durable stop flow. A successful requested stop returns Stopped while
retaining its attachment; incomplete stop, fault or unconfirmed planner cleanup returns
Attention and cannot advance. A completion read/closure failure also retains the
attachment and requires attention; there is no cleanup inferred from a stored status.
Shutdown while validating a previous attachment or observing a known ARMING start
also returns Attention with the Run retained, without inventing a PauseRun.

Unit coverage exercises discovery and fresh-cut rejection, exact Preparing recovery,
creation-entered file loss, old-context rejection, structured completion evidence, and
SQLite preservation across A closure and B preparation. Integration acceptance must
also exercise real authenticated reads and planner execution through the product CLI.
This module adds no scheduler, StartRun mutation, P database access, hardware control,
or physical safety proof.

[Offline recovery inspection](RECOVERY_INSPECT.md), which neither creates/changes existing service journals nor connects to the network, is available through `cell recovery-inspect CONFIG`. It preserves original PENDING/ATTENTION, attachment and request records without current P queries or operating resumption.
