# Durable requests and finite-operation worker

Status: actual C++ BT requests are connected through the S journal to P ResolveActivation/SubmitOperation. PauseExecutor and RequestHandover requests also use durable boundaries. The [branch/wait worker and checkpoint requests/recovery](DECISIONS_AND_RECOVERY.md) are connected as well. The complete operational daemon, part coordinator and intervention worker remain future work.

## State ownership

S's journal records “which request was sent with which key/body.” It does not take ownership of P's run/operation/permit decisions or Host native outcomes. It uses S-only SQLite storage and single-writer ownership without accessing the P DB.

The journal header is pinned to installation/store generation, principal/release, cell/definition and run/resolved digest. The same journal is not silently reused for a different store generation, run or recipe. Logical actions are distinguished by visit/node/stage. ResolveActivation, SubmitOperation and ReconcileOperation use the same operation mapping; PauseRun adds a session/epoch control identity.

Each attempt preserves its key, generation, original context(session/epoch), full typed body and digest, and P read basis. The current pointer is an index to the latest attempt; previous attempts are not deleted. Request bodies validate intent normalization and IDs/relationships/CAS; they do not execute arbitrary text.

## Durable boundaries

| State | Meaning | Reconnection handling |
|---|---|---|
| PREPARED | Key/body stored, but network entry not yet recorded | Retrieve the same request. Do not overwrite it with a new body |
| EMIT_ENTERED + PENDING | Entry record committed before network call. Actual transmission/response may remain unresolved | In the same context, use only original key/body. No arbitrary new key |
| EMIT_ENTERED + REPLY | Strictly validated P response durably recorded | Do not change the response or emit again |
| REVISION_REJECTED | Transaction rejection confirmed by ABORTED from the corresponding Resolve/Submit/Pause RPC | After a new snapshot, allow a new CAS/body, new key and generation with the same meaning |
| ATTENTION | Inputs/authority/responses/integrity or other state require reconciliation | Do not automatically turn it into a new request |

The worker order is prepare commit → enter commit → RPC → reply commit. A local write failure prevents the next step. Death after Entered commit does not arbitrarily revert to Prepared, even if the actual call had not begun. If local reply storage fails after P responds, retain the original key and unresolved state.

The machine signal currently permitting rebase is gRPC ABORTED from atomic Resolve/Submit/Pause RPCs. ReconcileOperation has no CAS and cannot create a new key from ABORTED. Keys are not changed by interpreting reason/detail strings. This rule must not be generalized to other APIs that can record facts before returning a business CAS error. KEY_CONFLICT, unsupported/authority errors and invalid responses are not automatically rewritten in the same way.

## Observations and responses differ

If activation/operation exists in the current P snapshot, the worker validates and reuses its ID and intent/slot relationship. This is recorded separately as `ObservedTarget` and P read basis. It does not fabricate an absent RPC reply or overwrite original EMIT_ENTERED/PENDING with success.

Repeated queries of an identical mapping are not journaled on every tick. They update only when reconfirmed in a new P runtime or when mapping content changes; changes to operation/activation identity itself are integrity errors. Native success/failure and resource handover for work continue to be read from P's current state.

## Worker processing

1. Read a fresh validated current P snapshot.
2. Compare C++ request context/run/session/recipe/visit/epoch and node kind/argument/timeout with the actual resolved process. Do not use C++-supplied arguments as native intents.
3. If work is already mapped, record the observation and return its existing ID.
4. If current admission and frontier allow it, prepare and enter the activation request in the journal, then send it to P.
5. After T3, read a new snapshot and recheck run revision, current authority and mapping.
6. Build the T1 body from the validated binding and submit through the same journal boundaries. Root clock expiry or context change blocks new transmission.

If a mapping previously acknowledged by P disappears from a complete new snapshot, retain it for storage/restoration reconciliation. Do not create a new ID based only on that response. Unanswered requests whose context changed are not automatically reissued either. A newly eligible P state must not be confused with confirmation of an old request's result.

SubmitOperation is connected to finite-operation handling; RequestHandover to durable queries and release observations. ResolveBranch/BeginWait are connected to candidate validation, CommitCheckpoint and P decision observation. RequestIntervention worker remains Unsupported. PauseExecutor connects to current P PauseRun using a journal key with separate session/epoch control identity. The parent runner must not consume it as successful completion. These methods and actual restart/clearance integration remain part of the overall goal.

## Tests

- Failures immediately before prepare/enter/reply transactions and response loss after commit; storage reopen and same key/body preservation.
- Rejection of unresolved request body replacement; new CAS/key allowed only after confirmed ABORTED; previous attempt preservation.
- P observations do not forge RPC responses; unchanged mapping deduplication; rejection of different store generation/identity.
- Actual C++ BT request producer → separate S worker → P mTLS/SQLite handling.
- Normal processing; forced termination immediately after Submit network entry and immediately after P Submit response but before local reply commit.
- Loss of P Resolve/Submit responses after their respective commits, followed by recovery of existing mappings and confirmation that only one operation remains.
- A new-boot S recovery process preserves original keys/network-uncertain state and creates no new work without authority.

The C++ request producer and process hooks are validation/test-harness-only; Docker image digest is pinned and pull/network/device access is not allowed. Operator start, part coordination and qualification/Host preparation are explicit simulation composition. Worker integration tests validate P admission/Pause and handover requests/release observations. Host evidence in handover tests is a synthetic fixture; separate Host tests validate P→H/native/query/handover. The complete resident BT daemon has not yet been delivered.

Pause authority, fences and outcome limitations follow the [P PauseRun specification](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/EXECUTOR_PAUSE.md). Existing operation logical keys retain their prior canonical form because optional control identity is absent.

## Handover requests and release observations

When actual BT emits RequestHandover after a valid successful result, the worker stores a ReconcileOperation body/key and then requests a query from P. The RPC response is ReconciliationAccepted, distinct from actual RELEASED. An already accepted query is not reissued for each repeated BT request.

When a new P snapshot confirms RELEASED, a separate ObservedTarget::Released is stored. If the query response was lost, original PENDING is preserved. Restart recover also records that release observation without fabricating a successful request response. Adding 9 branch/wait scenarios to the existing 9 covering actual BT requests, normal responses, response loss and restart brings the wire fixture to 18 scenarios.

Passing P plan ATTENTION details to current Frame/UI and operator requery remain future work. Detailed handling and restrictions follow the [P query/handover specification](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/RECONCILIATION.md).

CHECKPOINT_REJECTED for checkpoint requests is distinct from generic REVISION_REJECTED. Only confirmed structured atomic rejection allows rereading current P state and creating a new candidate/key. Follow [branch/wait and recovery](DECISIONS_AND_RECOVERY.md) for the detailed procedure.
