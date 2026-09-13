# Executor branch/wait requests and recovery

Status: actual BT ResolveBranch/BeginWait requests are connected to the S journal, P candidate preparation and frozen CommitCheckpoint. This is not a completion specification for an entire resident service with branch/wait support or an operator recovery procedure.

## Validate prepared state

The worker validates run/session/epoch/recipe/visit and the actual resolved node in a new P snapshot. Branch uses CHECKPOINT_BRANCH; wait start and result use CHECKPOINT_START_WAIT and CHECKPOINT_CHECK_WAIT. Repeated BeginWait does not recreate an existing window.

The Client requests candidates with a separate executor-plan binding hash. WAITING must contain neither a candidate nor a time; repeated queries without a ready candidate are not accumulated in the mutation journal. ALREADY_APPLIED or a different revision is checked through a new snapshot.

For READY, read the current checkpoint and candidate artifacts separately and validate actual hash/size/schema and typed payloads. The candidate must be an exact successor of current state. It cannot change run metadata, budget, part lists, existing activation/slot or other visits' process state. Only the selected node's branch/window/wait result and its evidence/decision time are added. Empty/duplicate evidence, reuse of an existing decision/window ID, early timeout and incorrect window times are rejected.

P source clock, preparation validity and local elapsed time are all checked. S does not independently decide whether a condition is true/false or what a native outcome means. P rechecks current conditions and authority at actual commitment.

## Request and decision storage

| Record | Meaning |
|---|---|
| PREPARED | Retains full Checkpoint body, artifact reference, target, expected decision and key |
| EMIT_ENTERED/PENDING | Commits immediately before actual transmission. Application/response may remain unresolved |
| REPLY | Retains a P response matching original revision, recipe, session and entire Checkpoint |
| CHECKPOINT_REJECTED | Structured confirmation that P did not apply that atomic commit |
| Separate Checkpoint observation | Confirms a committed branch/window/result in the current P snapshot. Distinct from an RPC response |

Body and entry are recorded separately before the network call. An unresolved request for the same logical target may use only its original key/body. A newly available candidate does not replace an existing unresolved request.

If the executor confirms a decision already present in P, it retains that as an observation and proceeds. It does not overwrite the original PENDING with a successful response. An actual P decision can be recorded even when another candidate passed CAS first. A decision inconsistent with an already received response, or a change to a previously observed immutable decision, is an integrity error. A late response is also rejected if it contradicts an existing P observation.

## When preparation may be repeated

Only for CommitCheckpoint, check the combination of strict ErrorDetail in `rx-checkpoint-error-bin` and gRPC status. Only REVISION_CONFLICT/REFRESH+ABORTED and EXPIRED/RECONCILE+FAILED_PRECONDITION are definitive rejections. Duplicate/incorrect metadata, bare status, human-readable text or transport errors do not create a new key. This metadata does not replace standard grpc-status-details-bin.

After retaining a definitive rejection in the existing attempt, validate a new P snapshot/candidate and create the next generation/key. Previous body/key/rejection reasons are not deleted. This behavior is pinned to the [preparation binding](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/spec/executor-plan/v1/README.md); failures of other RPCs are not generalized to the same meaning.

## Restart

A new S boot opens the original journal and reads P's actual checkpoint. It observes each already committed branch/window/result without fabricating unreceived responses. If termination preceded checkpoint commitment, it does not invent a P decision. Execution authority withdrawn by P is not automatically resumed.

## Verification and remaining scope

Actual Linux BT.CPP → separate S process → P mTLS/SQLite tests comprise 9 existing and 9 new scenarios, 18 in total. New scenarios include true/false branches, wait success/repeated waiting/timeout, checkpoint response loss, termination immediately after entry/remote response, and candidate expiry with actual P clock followed by a new generation. After restart they check original key/response state, separate observations and operation counts. Journal tests verify prohibition of unresolved-body replacement, replacement after definitive expiry, observation of another candidate's P decision and rejection of contradictory late replies.

Fixture conditions/time and prior operator/part starts are simulation configuration. These tests do not control physical devices through the new branch/wait path. Existing separate Host integration tests verify simulated native outcome/handover paths.

Resident loop/process supervision, part coordination, intervention/clearance/cancellation/explicit restart, long-term journal/candidate retention/indexes/capacity, complete ErrorDetail transport, UI/product image and actual device verification remain outstanding. The 100 ms candidate lifetime and simulation test timing must not be used as guarantees of site response performance.

The branch/wait integration fixture was moved in a later stage to the [persistent BT engine and request queue](../../native/executor/PERSISTENT_ENGINE.md). It does not stand in for an actual operational daemon; it verifies same-process deduplication/wait retention and P pause during normal shutdown.
