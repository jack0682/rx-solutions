# Deliverable Host executable and process lifetime

Status: phase58 implementation draft. Provides `rx-hostd drivers`, `rx-hostd inspect|init|run CONFIG` and the `host` entrypoint in the same solutions image. The product config has no test-only clock, response-loss or seed options. Builtin backends are FILE_SIMULATION and signed MELSEC_PACKAGE. Follow the [device package startup specification](DEVICE_PACKAGE_STARTUP.md); unregistered generic drivers are rejected before startup.

## Startup and storage ownership

The startup JSON contains installation/release/Host identities, bind address, separate data/runtime paths, pinned Binding files and TLS material, allowed P certificate fingerprints, backend and optional publisher. It accepts no arbitrary executable/argv or library paths. It validates policy/file pins, schemas, sizes, peers, environment and TLS material.

init creates new SQLite Host journals and an installation descriptor without opening an adapter. It completes them in a temporary installation directory before publication and must not be repeated on an existing installation. The descriptor binds the Host/installation/Binding/backend identities to the delivery/evidence journal identities.

run first compares the descriptor with the existing DB/Host metadata/cell generation. It does not replace missing DBs or metadata with new ones. It checks the runtime lock and existing SQLite/device owner locks. This is not a stored-PID adoption feature. It does not overwrite a status file as a new owner if that file belongs to another Host/installation or is a symlink.

The product clock uses the Linux kernel boot UUID and CLOCK_BOOTTIME. Time values cannot be set through the CLI. run on other operating systems is not yet supported; inspect validates inputs. The typed factory/clock in `run_with` are library composition ports for separate implementations and tests; the site JSON exposes no clock override.

## Adapter selection

The release-owned AdapterFactory validates metadata and creates the adapter through `open_passive`. This function must not cause native motion, torque or mode changes, and closing the connection before first admission must have no physical control effect. If actual driver initialization causes an action, that part must move to a separately authorized lifecycle operation.

Builtin currently registers FileDevice and the Melsec adapter from a validated DEVICE_REFERENCE package. VALIDATED_DRIVER profile/digest values produce an explicit unsupported error. External device SDKs, ROS and models are added through separately validated configurations; drivers must not be launched collectively without individual validation or allowed physical effects in constructors/destructors. Physical operation requires acceptance of the current process context and qualification, followed by a separate Arm. The generic driver factory and full lifecycle authority remain future work.

## Readiness and execution

Startup begins with a new Host boot and inactive Arm. It retains the existing mTLS/base/cell negotiation, P grant/fence/qualification/start/permit and native guard checks. SOFTWARE_READY_UNARMED in status means that the RPC process is ready. Restart alone does not restore qualification, Arm or native commands.

Host data consists of delivery facts and the evidence journal; it does not replicate P's outcome/resource/operating authority ledger. When a publisher is configured, it uses the existing ordering and ack cursor and retries transient transport errors with bounded backoff. A definitive publisher failure or RPC owner failure starts the admission shutdown procedure.

## Normal shutdown and adapter retention

A stop request first lowers the atomic admission latch. It does not wait for DB locks or ongoing native calls. New grants, renewals, Arm, prepare, authorize, configuration and qualification acceptance are rejected. Evidence/receipt queries and fences/reconciliation that increase restrictions remain available at boundaries with no physical effects. The latch is checked again immediately before native entry.

The native `shutdown_snapshot` is separate from the existing handover_snapshot. A current support_stable=true does not establish that a destructor or connection close is safe. Shutdown proof must explicitly cover the complete resource scope, current device session/time/uncertainty, absence of residual commands and **safe_to_drop**. The default port is unsupported and does not permit exit.

Normal shutdown retains the same native owner and rechecks the evidence. If native/gate does not respond, it waits for the existing inspection handle instead of continually spawning inspection threads. A timeout alone does not drop or kill the controller. Final drop evidence is checked again after transport/publisher drain. Unexpected loss of the library owner closes admission and notifies the independent protection port. Forced process/runtime termination and power loss cannot be prevented by this normal shutdown procedure; they require validated external protection.

Prepared work or work with uncertain entry is not erased from the journals or converted into success/non-execution. FILE_SIMULATION has no residual asynchronous native queue or physical support, so it can exit while retaining history once independent safe-to-drop is confirmed. Unsent evidence remains even after a configured publisher's drain time expires. Without a publisher, retained evidence is not considered fully published.

## Status and deployment

`host-status.json` contains installation/Host/instance/Host boot/endpoint/clock, admission, publisher state and the stop snapshot. It distinguishes STOP_WAITING_FOR_ADAPTER, STOP_STATE_UNAVAILABLE, STOP_DRAINING_EVIDENCE, STOPPED and STOPPED_WITH_RECONCILIATION_REQUIRED. Neither ready nor stop output creates physical qualification. A status update failure also blocks new admission.

`rx-hostd` is copied to `/opt/rx/bin` in the image runtime and included in the runtime inventory. Host mode enters the binary directly before running any ROS setup script. The default diagnostics mode is unchanged. The [deployment templates](../../examples/deployment/host/README.md) describe the existing solutions image, separate volumes, non-root/read-only operation, minimum privileges and manual startup.

The supervisor's control lifecycle authority and validated driver recipes are not yet connected. Drivers are not all classified as simple NonActuating recipes for automatic startup or restart.

## Verification scope

Actual results are retained in the [phase55 record](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase55_checks.json). Generic composition tests check no adapter open during initialization, ownership by the current process, normal shutdown, rejection of missing journals, unsupported backends and tampered/unknown config, and retention of the native owner before drop permission. Gate tests cover the stop latch, preservation of unresolved work, and the distinction between stable handover and drop permission.

Product binary/image tests check the Linux clock, mTLS readiness, duplicate owner rejection, SIGTERM/restart and zero native effects. Actual work integration between the product Host and P uses rx-hostd and the Linux kernel clock. It checks qualification issuance, activation, separate start, one simulated native action, evidence/handover and Run completion; it also checks that retained evidence is not assumed to have been fully published during shutdown. Response loss is injected by the test client after receiving the actual RPC response and before delivering it to the P writer; no product server fault feature is used. Product config has no fault-injection/manual-clock options. Simulation tests do not validate actual robot/PLC, load, stop or support safety. The first physical cell is NOT_COMMISSIONED.


The MELSEC state-ensuring adapter library and native journal/simulated Host tests have been added. Follow the [implementation scope and publication prerequisites](MELSEC_ADAPTER.md). phase58 connected the signed-package factory, atomic initialization of native identity and mandatory current-qualification checks for physical bindings. This does not mark field qualification complete.
# Host binding change inspection addition

The read-only phase71 `inspect-binding-change` follows the [current/proposed configuration comparison](HOST_BINDING_INSPECTION.md). A configuration match grants no Host installation/execution authority and does not bypass the installation identity checks in existing `init` and `run`.
