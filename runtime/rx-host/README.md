# RX Host foundation

The Host owns native delivery facts and its final command gate. It does not assign platform outcomes.

## Structure

- gate/grants: durable maximum resource fences and expiring, session-bound grants.
- gate/scopes: cell/scope fence and Arm receipts with durable request identity.
- gate/dispatch: exact approved intent, permit identity, local guard, SEND_ENTERED and native entry under one gate.
- gate/receipts: receipt lookup, pre-send tombstones, native result recovery and evidence outbox.
- journal: separate delivery/evidence journal identities and sequence spaces.
- native: C++/ROS/SDK-facing adapter port and a separate local protection port.
- simulation: a file-backed device with an independent exclusive owner and effect log. It intentionally does not deduplicate native calls.

Known PREPARED work can only be rebound under a new validated permit; the previous permit ID is permanently retired. SEND_ENTERED is never re-submitted. Captured native status is evidence, not a platform SUCCEEDED result. A Host receipt does not claim RESULT_RECORDED before platform T2.

## Verification

Run from rx-solutions:

    ./tools/cargo test -p rx-host --features test-harness --locked
    ./tools/cargo clippy -p rx-host --all-targets --features test-harness --locked -- -D warnings

The test-harness feature builds a fixture binary. It is excluded from ordinary product builds. Tests kill that process at the durable-send/native boundaries, restart it, and count independent device-log effects. These are process-crash tests, not storage power-loss or machinery protection tests.

## Remaining integration

mTLS/gRPC and platform outbox/evidence exchange are connected. Complete native cancellation journals, automatic expiry/deadman monitoring, continuous-control streams, physical adapters and the full product image are pending. The independent protection-port test proves that the port remains callable during a blocked native submission; it does not prove a physical stop or support response.

SDK sources are exported and hash-pinned from rx-platform; rx-application is intentionally excluded. Edit the producer, regenerate the SDK and reverify. Do not modify the vendored sdk directory directly.

[Host process configuration context and receipts](PROCESS_CONFIGURATION.md) check the fences, current Bindings and quiescence of every managed cell and durably record the process context. After application, the gate remains unqualified; this does not claim native configuration changes or operating qualification.

[Host qualification acceptance and separate start](QUALIFICATION_ACCEPTANCE.md) atomically retain the exact cohort, context, generation and qualification semantics. Acceptance itself neither clears a block nor performs a native action; global activation by P is a subsequent step.

[Product Host executable and startup/shutdown](HOST_SERVICE.md) provide `/opt/rx/bin/rx-hostd` and the explicit `host` mode of the solutions image. The FILE_SIMULATION backend runs with the actual Linux clock; physical drivers are rejected until registered through a validated factory.
