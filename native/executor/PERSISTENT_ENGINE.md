# Persistent BT engine and private-pipe contract v1

`rx-bt-engine` is an executable that retains BT memory for one context. It ticks once only when the Rust bridge provides validated P state. It does not provide networking, ROS/native drivers, arbitrary plugin loading, or independent operation completion decisions. The default executable uses Linux BOOTTIME and accepts no clock override argument.

## Commands and responses

Input is newline-terminated JSON. A command is limited to 4MiB; each resolved graph, XML document, and Frame is limited to 1,000,000 bytes. Duplicate keys, unknown fields, excessive nesting, and truncated packets are rejected. Request sequence starts at 1 and increases by exactly 1; the context cannot be reset.

| command | Additional fields | Meaning |
|---|---|---|
| INITIALIZE | identity, resolved, xml | Construct the tree from the validated declarative graph/generated XML. Do not tick; there must be no requests |
| STEP | frame | Publish/tick after checking the same context/revision and clock bounds |
| HALT | None | Latch the stop on new proposals and issue a separate PauseExecutor proposal |
| CLOSE | None | After HALT, flush the response and exit the process |

Common fields are schema=`rx.bt-command.v1`, sequence (base-10 string), and command. Responses contain schema=`rx.bt-reply.v1`, sequence, state, requests, and fault. State is READY/RUNNING/SUCCESS/FAILURE/STALE/HALTED/FAULT. Fault is present only in FAULT, and diagnostic strings are bounded. A response contains at most 32 requests.

Interrupted input/EOF terminates the planner. Malformed input, context errors, and integrity errors return FAULT and PauseExecutor where possible, then exit. An expired frame for the same context is ignored as STALE while waiting for a fresh frame. Future timestamps, a different clock, or a different execution generation are not treated as ordinary delay. STEP is forbidden after HALT.

## Responsibility boundary

Rust validates the P endpoint, artifact hash/size/schema, and resolved semantics, and generates XML. C++ receives a private pipe from its trusted parent and checks graph shape, Name/Counter values, the XML whitelist, Frame, context, and state continuity. This does not claim that C++ recalculates the resolved JSON's JCS digest or authenticates P itself. Returned requests are validated again by the Rust worker and P.

Committed wait outcomes are immutable, as are operation/branch identities. Undelivered proposals already resolved by P are removed from the C++ internal queue to make room for new proposals. Requests already handed off are not repeatedly sent by reticking.

SUCCESS is the BT state computed from that P view. It does not replace P's part/run completion record, native stopping, or completed resource handover. CLOSE/process exit is likewise not device controller shutdown.

## Rust bridge and queue

`EngineProcess::spawn` verifies an absolute-path regular file and its SHA-256 and executes it without a shell or site-supplied argv. Deployment must provide that binary and its dependent libraries as an immutable/read-only release. The hash check must not be overstated as TOCTOU protection on a mutable filesystem. The child does not inherit the parent's environment or credentials.

The bridge checks sequence, response size, fields, state, and request context. Pipe I/O is limited to 2 seconds, and there is no automatic restart after an error. Dropping the bridge in the parent terminates the planner child. This behavior must not terminate Host/device driver/torque control processes.

PendingRequests manages at most 32 ordinary proposals and a priority pause state. Incomplete requests remain pending across new frames. In particular, confirmation that BeginWait started does not remove the request; it continues checking until the wait outcome. Repeated requests are coalesced, and body changes are rejected. Transient RPC errors use 100ms–5-second backoff, while state is rechecked at short intervals. The Instant used for backoff is not used to decide physical clock or authorization expiry.

The queue is volatile; the existing S journal owns durable mutation records. Pause can be requested with priority even when the ordinary queue is full or an operation is in progress. PauseObserved means that P's restricted state was observed, not that physical stopping was confirmed. Errors/unsupported proposals stop new ordinary proposals and request pause. A later phase added the [single-run/visit execution service, signal integration, and durable stop intent](../../runtime/rx-executor/SERVICE_LIFECYCLE.md). The complete process supervisor and part coordinator remain future work.

## Validation scope

- 9 actual C++/private-pipe cases check initialization without actions, request deduplication across reticks, latched halt, accepting a fresh frame after expiry, a different context, duplicate/truncated/oversized JSON, changed wait outcomes, and production clock protection.
- Of the existing 18 S/P integration cases, the 9 branch/wait cases now use one persistent C++ process and PendingRequests. BeginWait is proposed only once, and Rust retains it until the outcome. Normal fixture shutdown confirms P pause. The original journal state is also preserved in shutdown/response-loss fixtures.
- Linux-only checks cover rejection of an incorrect production binary digest, actual BOOTTIME, repeated handling in the same PID, no operation proposals from an unauthorized view, and planner CLOSE. Input P state is synthetic restoration data, not a physical device test.

`rx-bt-engine-fixture`, which accepts a test clock file, is built only under RX_BUILD_TEST_HARNESS. The production executable has no such argument. This validation must not be expanded into claims of completed validation for the two product images, all resident services, or site timing/qualification.
