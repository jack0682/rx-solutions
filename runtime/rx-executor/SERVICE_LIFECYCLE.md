# Execution service and retained stop intent

`rx-executor-service` runs a P Client, S journal, persistent BT process and pending queue for a specified run/visit on Linux. It waits until P authorizes the run for the current service session and the corresponding material attempt exists. Software startup does not replace operator admission, material budget consumption or actual equipment startup.

## Current execution scope

The service is pinned to one run. ManualVisit handles only the designated visit; default SerialProduction switches visits in order according to P parts/budget. It reads current P state, supplies it to BT in the same context and handles one ordinary worker request at a time. During communication gaps it creates no new BT Frames; if configured grace is exceeded or the planner/worker fails, it starts stopping. Context changes are not automatically adopted as new execution authority.

BT graph completion alone is not part/run completion. SerialProduction uses the [material coordinator](PRODUCTION_COORDINATOR.md) to confirm completion with P and admit the next material. ManualVisit waits at GraphComplete for a separate coordinator. Explicit restart procedures remain future work. Graph failure stops new ordinary requests and requests P pause.

## Stop is a run lifecycle record

Stop intent is retained in `rx.executor-stop.v1` at `executor-stop/current`. This record does not impersonate a BT node/visit request. It contains run, initial request ID/session/optional execution context/time, reason, stage, pause attempts and P observations. A pre-start state without a part or BT root can also be stopped.

| Stage | Meaning |
|---|---|
| PENDING | Stop intent stored, but P's restricted state not yet confirmed |
| PAUSE_OBSERVED | Authenticated P PAUSED/RECOVERY_REQUIRED/COMPLETED/ABANDONED confirmed |
| SUPERSEDED | P confirmed the run bound to another executor session. Do not pause the new owner under the old intent |
| ATTENTION | Authority/generation/response or other reconciliation required. Not represented as complete |

The initial intent does not change, and a new service start does not erase it. Additional pause attempts while Pending preserve key/session/expected revision. PREPARED is stored, then ENTERED, then the request is sent to P. Response loss remains ENTERED; even if the next P query confirms a restricted state, no absent RPC response is fabricated.

A new attempt/key is created only after confirmed revision rejection by current atomic PauseRun. Previous attempts/history are preserved. Ordinary transport errors do not change body/key. Attempt history is limited to 64 entries; limits on new attempts are distinct from recovery of existing unresolved attempts.

## Shutdown and restart

Store stop intent first, freeze ordinary planning, then close only the planner child. Communication with P continues until pause confirmation. Host/device drivers/torque-control processes are not coupled to shutdown of this service. P pause observation is not confirmation of physical stop, material support release or resource handover.

The service attempts to confirm P's restricted state within a bounded time. If P cannot be reached, it leaves PENDING and exits with an attention result. On program restart it reads and handles that intent first without starting a new BT. Even a completed stop record is not automatically erased. Reoperating the same run requires subsequent explicit restart/lifecycle rebind. Normal serial material transitions retire only the existing planner after P completion confirmation, without generating run-pause.

Planner freeze and P pause are attempted even if local storage fails. In that case, key/body are retained in process memory and durability_fault is reported. Even if P confirms a restricted state, local recording failure is not hidden as success. This path is not claimed to preserve durable stop intent.

## Executable

The Linux CLI accepts one JSON configuration file. It contains P URI/server name, CA/client certificate/key paths, PeerPin, run/visit, S journal path, immutable BT binary path/SHA-256 and poll/grace/stop options. The endpoint uses TLS and clock_id must match the actual local Linux boot clock. peer_boot is newly generated once per process instead of taken from config. SIGINT/SIGTERM become shutdown requests; unsupported operating systems do not use a substitute clock to run.

Immediately after parsing configuration and before P connection/Session.Open, the CLI acquires `.rx-executor-service.lock` ownership in the configured journal's parent directory. Scope is **one process per configured service/journal root**; valid services in different roots can run independently. It is not a policy limiting Run count across an entire cell. Different run journals in the same root are also treated as duplicate startups that would replace the existing P peer and are rejected before connection. Directory aliases are normalized to the actual root; symlinks/special files at existing lock paths are rejected.

Ownership is retained throughout connection, journal open, service operation and stop confirmation. On normal return, error return or unwind, the owner's Drop explicitly unlocks before closing the file handle. Therefore a descriptor briefly inherited during a concurrent child's fork→exec does not delay normal release. Forced termination relies on kernel handle cleanup for release. The lock file is not deleted because other processes may be waiting on the same inode. This lock does not replace existing run Scope checks, request/stop journals or authority revocation for a new boot. The pre-connect probe in `test-harness` builds records only how many independent CLI processes enter the connection stage and immediately fails. Default product builds contain neither probe environment-variable handling nor that path.

Status is output as JSON only when it changes. A stop result with PENDING/ATTENTION or durability_fault causes abnormal CLI exit. Configuration/startup failures before connection/authentication grant no execution permission. Deployment tools that provide the journal parent directory, accounts/certificates, P service and read-only release remain a separate implementation scope.

Default coordination is SERIAL_PRODUCTION. MANUAL_VISIT is for individual visits/setup. Default poll is 50 ms, communication grace 5 seconds and P pause confirmation window 10 seconds. Configurable ranges are validated in code. Planner CLOSE has a separate maximum of 2 seconds. These are service handling policies, not equipment stop deadlines or site performance guarantees. Model-specific support/stop requirements must be met separately by Host and site acceptance specifications.

## Verification

The following 6 scenarios were added using an actual service loop, separate S process, persistent C++ engine and P mTLS/SQLite: normal stop, stop before execution assignment, pause response loss, process termination immediately after intent storage, local journal failure and P pause nonresponse. Together with 18 existing cases, this totals 24.

New-boot recovery checks 0 BT starts, original intent/attempt preservation, absence of lost responses, Pending retention on P nonresponse, and P pause attempts/fault reporting during storage failure. Journal tests check rollback/commit-response loss, key/history immutability and prohibition of automatic resume from stop records. Conditions, clock and initial operator/part start in service tests are simulation. These results must not be broadened to validation of the entire actual CLI signal-to-P path, product supervisor or physical equipment shutdown acceptance.

The serial part coordinator is connected. Remaining work includes parallel materials/physical genealogy, explicit restart/rebind of the same run, intervention/clearance/cancel and continuous control, full P/Host/S supervision, two product images, device stacks, installation/restoration/acceptance and the full UI.
