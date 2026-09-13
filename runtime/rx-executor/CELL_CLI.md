# Resident cell executor CLI

Existing `rx-executor-service CONFIG` retains per-Run configuration and manual visit/serial production behavior. New commands are explicit `cell init CONFIG` and `cell run CONFIG` in the same product executable. The Linux BOOTTIME product boundary is preserved; other operating systems do not operate with a substitute clock.

```text
rx-executor-service /absolute/legacy-run.json
rx-executor-service cell init /absolute/cell.json
rx-executor-service cell run /absolute/cell.json
```

## New configuration

The new schema is `rx.executor-cell-service.v1`. CONFIG must be an absolute-path regular JSON file at most 65,536 bytes; duplicate/unknown fields are rejected.

| Field | Contents |
|---|---|
| `service_root` | Absolute directory owned by this service. Does not change per run/visit. |
| `expected_service` | Existing AssignmentJournal `Identity`: pinned journal UUID and installation/store generation, principal/release, cell/definition Scope. Deployment selects it once before initialization and retains it. |
| `platform` | `uri`, `server_name`, `ca`, `certificate`, `key`. URI is HTTPS; each file has a `{path, sha256}` pin. |
| `engine` | Release-owned fixed BT executable specified by `{path, sha256}`. |
| `options` | Optional `poll_ms`, `communication_grace_ms`, `stop_timeout_ms`. New cell mode is fixed to SerialProduction and accepts no arbitrary coordination. |

The new config accepts no fixed `run/visit`, `peer_boot/clock_id`, arbitrary argv/launch or shell. CellService checks current P start relationships and the journal's run; existing SerialProduction retrieves visits from P part state. PeerPin's fixed fields come from expected_service.scope, peer_boot is a new UUID per process, and clock_id comes from actual LinuxBoottime.

Configuration and file paths are local deployment inputs. No API accepts this CONFIG, file paths or engine through UI/P requests. Certificate, CA and private key are each limited to 1 MiB; engine has the same 128 MiB maximum as existing engine validation. Symlinks, special files and pin mismatches are rejected. Private keys require owner-only permissions; engines require execute permission. Private key bytes are not printed. EngineProcess also repeats the existing hash check when actually starting the planner.

## init and configuration pin

`init` acquires root ownership through ServiceOwner and normalizes the actual root path. Any file other than `.rx-executor-service.lock` is treated as an existing or partial installation and rejected. It creates an owner-only root directory and validates TLS/engine file pins. This stage neither connects to P nor executes the engine.

First, it creates `cell-installation.json` with create_new and fsyncs it. This file retains the schema, normalized configuration digest, actual root path and expected_service Identity. The digest covers the entire CellConfig with default options filled in, using the `RX-EXECUTOR-CELL-CONFIG-v1` domain. Next, it creates `assignment.sqlite3` through existing AssignmentJournal::initialize_file and fsyncs the directory. Successful output is the pinned installation descriptor.

These two files are not claimed to form one transaction. If DB initialization fails or the process disappears after manifest creation, the partial installation is preserved. Subsequent init does not overwrite it, and run does not automatically create missing/corrupt DBs. If only the header commit response was lost and both files are valid, run's required-open can verify the original identity. Key rotation, engine/release/endpoint/option changes and root moves change the configuration pin and are not silently adopted. Procedures for such installation changes/recovery are a separate scope.

## run and shutdown

Startup order is **root owner → manifest/config/root/Identity comparison → AssignmentJournal required-open → required current run-file validation → TLS/engine pins → actual clock/new peer boot → Session.Open → CellService**. If Preparing has creation-entered but its file has disappeared, or an Attached file is missing, startup rejects before registering the P peer. Initial Preparing without a marker is passed only as a reservation and does not bypass subsequent current-P-authority checks in CellService.

CellService keeps the same Client/session and queries start relationships from Idle. After an attached run ends, it receives the Client and planner factory back and waits for the next run without repeating Session.Open per run. AMBIGUOUS candidates, historical sessions/processes, pause/recovery and existing stop intent are not converted into automatic Run selection/restart.

SIGINT/SIGTERM feed watch shutdown. During Idle observation, it exits Stopped without fabricated run/stop records. With an actual active RunService, it follows existing durable stop boundaries and planner cleanup. Preparing/unresolved states are not hidden as simple idle success; Attention is an abnormal exit. A signal before initial connection completes returns a startup-interrupted error. CLI exit does not prove physical Host/native stop or support handover.

Status JSON reports current CellService `Idle/Arming/Running/Attention/Stopped`, session, optional run/attachment, this process's completed_runs, active RunService status and detail. watch retains only the latest status and emits changes; the final Report is also printed. Successful CLI exit means final CellService Stopped; Attention or startup errors fail.

Default poll is 50 ms (allowed 10–1,000 ms), communication grace 5 seconds (100–60,000 ms) and stop timeout 10 seconds (100–60,000 ms). CellService transient-query-error backoff doubles from the current poll up to 1,000 ms and resets after a valid query. Client connect/RPC limits are each 2 seconds; engine CLOSE retains its existing 2-second limit. These values are software processing limits, not physical stop deadlines. Authentication/session/clock changes are not automatically adopted through simple retry.

## Deployment and test scope

Provide the service root on a persistent writable volume, and CONFIG, authentication material and the release engine as managed read-only inputs outside the root. Other services use different roots. UI StartRun is a typed request to P, not a request to create an S process/configuration file. Do not assume the current supervisor release catalog has registered this CLI. Subsequent registration must use fixed program/executable/config arguments with `RequiresPlatformAuthority` and restart_limit=0, without exposing generic site executable/path arguments.

`tests/cell_cli.rs` covers init and pre-connect counterexamples through the actual product entrypoint on Linux+test-harness. It checks valid/default-option normalization, duplicates/aliases, partial installations, missing/corrupt headers, Preparing/Attached file loss, config/asset/root changes, forbidden fields/time limits and file policy. The test-harness before-connect probe records and stops before actual TLS/server/BT. Failure injection immediately after init manifest creation is also test-harness-only. Neither test path exists in default product builds. These tests are distinct from actual P–S/signal acceptance to be performed by the parent.
