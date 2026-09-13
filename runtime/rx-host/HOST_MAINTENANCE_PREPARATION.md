# Durable records for Host change preparation

phase72. The current implementation covers shutdown evidence, change preparation, lookup and preparation cancellation. Selected configuration-file replacement, native effects, restart/qualification issuance and P ReleaseManager integration remain future work.

## Journal records instead of a shutdown file

After obtaining the runtime owner and existing DB writer, the product Host records STARTING before opening the listener/native adapter. A new attempt number makes the previous STOPPED record no longer current. Even if an old `host-status.json` remains after a later listener bind or other startup failure, that file is not used as shutdown evidence.

After closing RPC and publisher and rechecking that the current adapter may be dropped, the service records a StopSeal if there is no service error. It contains the startup attempt, Host boot, installation identity, full Configuration digest actually executed, delivery/evidence journals and sequences, Host operational record digest and actual shutdown snapshot. Change preparation is rejected if unresolved Host operations remain.

Installation identity retains its existing backend/bindings-oriented meaning. A separate full startup-configuration digest links STARTING through StopSeal, preventing bypass of comparison checks by changing values such as release/TLS/bind in both CURRENT_CONFIG and PROPOSED_CONFIG. Independent review found and corrected this counterexample, and comparison tests against the actual shutdown configuration were added.

Lifecycle and maintenance history are DB records in separate namespaces. Service events are not mixed into the native evidence journal, and neither the native evidence publisher's sequence nor input format changes. Observations in the shutdown snapshot are facts from that time; they do not guarantee current physical support or new control authority.

## Preparation, lookup and cancellation

```text
rx-hostd prepare-binding-change PLAN CURRENT_CONFIG PROPOSED_CONFIG REQUEST_ID
rx-hostd lookup-binding-preparation CURRENT_CONFIG REQUEST_ID
rx-hostd cancel-binding-preparation CURRENT_CONFIG REQUEST_ID
```

Preparation must first pass the existing read-only binding inspection. It then obtains the same runtime lock and existing DB writer as the service and compares the installation descriptor, journal identity, latest StopSeal and current operational records/journal tail. It does not initialize a missing DB and cannot prepare concurrently with a live service.

Request semantics are bound to the plan, current/proposed identities and proposed configuration. Reusing the request ID with the same meaning retrieves its recorded current state; different meaning causes a conflict. While PREPARED exists, other preparations and normal Host startup are blocked. Lookup can retrieve the durable state even if candidate files have disappeared.

Active preparation, request state and original preparation history are recorded in one SQLite transaction. Preparation cancellation also atomically records request/active state and separate cancellation history. Calling prepare again does not revive a cancelled request. Cancellation releases PREPARED before any native/configuration application effect; it is not robot-operation cancellation or application rollback.

Execution authority for the current local tool follows the installation-file/journal owner's access rights. A management protocol authenticating P-approved changes, registered terminals and ReleaseManager delegation is not yet connected. These records therefore must not be used as P's Host configuration application receipts or physical execution permits. installation_changed and activation_authorized remain false.

## Older programs and installation data

New installation descriptors use `rx.host-installation.v2` and `maintenance_protocol=rx.host-maintenance.v1`. This distinction prevents an older service's v1 schema/strict decoder from starting a new installation without knowing about preparation records. Actual deliverable rollback testing with a previous binary was not performed at this stage.

The new service retains ordinary execution for existing v1 descriptors, but disallows maintenance preparation. It does not automatically modify v1 descriptors or create new journals. Explicit migration and rollback of existing installations remain future work; no arbitrary upgrade was run on site data.

## Verification scope

The following were checked using the actual product service, FileSimulation and SQLite reconnection.

- Preparation rejected before first execution and while the service runs; successful preparation after actual shutdown.
- Retrieval of the same request, rejection of changed semantics/competing requests, startup blocked during preparation and execution allowed after cancellation.
- Rejection of reuse of an old STOPPED file left after startup failure.
- Preparation rejected when operational DB records change after shutdown.
- Immediate termination of a separate test process before/after commit, confirming rollback or durable retrieval of the same request.
- Maintenance preparation rejected without automatically promoting v1 installations.

Process-termination injection is exposed only in test-harness and cannot be configured through product CLI external inputs. These results do not prove native device configuration changes, PLC actions, sustained physical shutdown or full P/Host replacement/restoration acceptance.
