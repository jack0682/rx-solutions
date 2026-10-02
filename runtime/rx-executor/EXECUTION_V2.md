# Explicit execution v2

Set the existing service `options.coordination` to `SERIAL_EXECUTION_V2` for a
published `rx.execution-plan.v2` Run. Default `SERIAL_PRODUCTION` retains the v1
path. The v2 profile accepts only the already specified finite sequence. It uses
the shared verified P frontier to request its single eligible node; it does not
install a v1 recipe or send the template to the v1 BT planner. A wrong plan version
or unsupported service fails closed. This profile adds no rule language or
cell-specific state machine.

The current registered Executor explicitly negotiates `rx.executor.execution.v2`.
P still owns actual-object binding, slots, qualification, start/arm/mandate, node
admission, resources, permits and completion. Snapshot2 keeps the immutable Plan2
identity separately from the graph instantiated with this Part's parameters. The
client checks peer/session/installation/store/runtime scope, exact journal Plan
identity, complete operation mappings, monotonic positions and the existing 100 ms
shared-clock bound. A read never authorizes an effect.

BeginPart2 and SubmitNode2 use explicit local journal body/response variants. The
same existing journal transaction records the full request and key before emission.
Response loss keeps ENTERED and retries only that original body/key. The new records
require storage reader 10; no v1 fallback is attempted. Observations may recover the
original Part/operation without fabricating a missing RPC response. The reply must
match pinned Run, Part, actual object, slot, candidate, report, parameter, Intent,
mandate, Host and epoch; a late reply cannot replace an already observed identity.

The serial coordinator retains its existing Part lifecycle. P's production read
waits for an explicit current actual-object binding before enabling a new Part.
The worker requests ordinary original-operation reconciliation for required handover,
and requests completion only after re-reading the v2 frontier. P requires actual
outcome/release evidence before consuming that slot. UNKNOWN/blocked work cannot
advance by template materialization. Stop intent and P pause remain durable; the
v2 stop path reads v2 context rather than silently invoking a v1 snapshot.

Current checks include real SQLite reopen/commit-loss controls, foreign response
refusals, and a plaintext gRPC fixture that loses the first SubmitNode reply and
observes the same key after reopening the journal. These fixtures are not enrolled-P
mTLS, native Host effects, frozen-v1 injection or M3 acceptance. Those integration
checks remain required. Explicit no-effect retry, new Host and UI integration follow
the approved first P/Executor gate; this mode does not silently retry an effect.

```sh
cargo test -p rx-executor --all-features --locked
cargo clippy -p rx-executor --all-targets --all-features --locked -- -D warnings
```
