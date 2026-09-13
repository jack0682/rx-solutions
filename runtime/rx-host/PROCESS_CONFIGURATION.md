# Host process context checks and durable receipts

2026-09-12. The new optional `rx.host.configuration.v1` is a contract for recording process configuration context in the Host. **It is not a receipt for applying PLC/robot/driver configuration or programs.** It accepts a configuration digest specified by P within the intent, condition, definition/envelope/environment scope provided by existing Host Bindings, and remains unqualified for operation.

## Scope and states

- `APPLIED_UNQUALIFIED`: the process-context records for this Host were recorded in one transaction. This does not mean the entire P ChangeRecord was applied.
- `NOT_APPLIED`: rejection for the same request, such as an unsupported binding change, remaining Host work or unverifiable quiescence, was durably recorded. Some cells are not applied first.
- RPC/storage error: do not infer either of the above. Retain the sent ID/body and check through Lookup. Absence of a receipt in Lookup is also not general proof that the previous request did not reach the execution boundary.

Effects are distinguished as `INSTALLED`, `ALREADY_PRESENT` and `NONE`. ALREADY_PRESENT may be recorded when a new request expects and confirms the same target context already present. Simple retransmission of the same request returns its original sequence/receipt.

## Request binding

The request binds its ID to the change/preparation slot, plan digest, Host ID/expected boot/journal, current Binding fingerprint, and each managed cell's fence/epoch/scope and before/after configuration digests. It must include all cells managed by the Host; partial cohorts are rejected. For RPC use, all such cells must have negotiated the cell contract in the current base session.

The Binding fingerprint includes normalization/sorting of intents in the current approved Binding and its scope/condition sets. Acceptance is denied if necessary intents/conditions are absent from the actual approved scope managed by the Host, or if definition/envelope/environment differ. This API cannot install new low-level bindings/drivers.

`expected_context` is the current Host process-context digest read through Inspect, or absence (null). A Host that did not know the previous context is not treated as having independently verified P's before_configuration. The initial before hash is a plan reference from authenticated P; the actual Host comparison conditions are expected_context and approved Binding/fence. Once context exists, its expected digest must match exactly.

Different semantics under the same request ID, reapplication of the same change/preparation slot under another ID, and incorrect boot/journal/fence are rejected. Current Platform peer/session authentication is checked even before returning a cached receipt.

## Checks before application

1. Own the Host command gate lock and validate the current caller.
2. Validate all currently managed cells, the Binding fingerprint and scope structure.
3. Confirm each cell's current blocked state and the boot/journal/epoch/scope/block set of its exact fence receipt. Do not apply to an already Armed cell.
4. If PREPARED/SEND_ENTERED/NATIVE_ACCEPTED work remains anywhere in the Host, record non-application. Do not convert existing work/outcomes to success or delete them. Proper voiding/reconciliation of prepared work must use the existing contracts.
5. Read no-pending/control-available/support-stable state for the Host's entire resource set through the adapter's read-only handover_snapshot. The default implementation cannot confirm it and therefore cannot pass. Observation age+uncertainty must be within 100 ms.
6. Record context, slot, receipt, delivery sequence and history in one SQLite transaction. Any recording failure rolls everything back. This procedure calls neither native submit/lookup nor driver configure.

The receipt's `recorded_at` is the check time immediately before the transaction. It does not guarantee that physical state persists afterward. Immediately before application, P must recheck the currency, affected scope and work/support disposition it requires. This must not be broadened into signed physical safety validation or proof of equipment stop.

## Gate after application and restart

If a process-context record exists without current [qualification acceptance](QUALIFICATION_ACCEPTANCE.md), Arm and native entry checks reject it. Process restart, a new grant or retransmission of an existing Arm request does not restore operating qualification. Host qualification acceptance is connected; P's global qualification activation is a subsequent step.

Lookup preserves the original receipt's boot/journal/sequence and returns the current Host snapshot alongside it. `context_matches_current_host` is a metadata judgment comparing current boot/binding/epoch/scope and the request/sequence/digest of the last applied context. It is neither a new physical quiescence observation nor execution authority. This value is false for a receipt from a previous boot. `activation_authorized` is always false.

Even if the response is lost or the process terminates after commit, retrieve the receipt with the same request. If the current context must be confirmed after Host restart, use current Inspect/fence/expected_context and a new change preparation slot; do not merely rename an old receipt and submit it as a current ack.

## Transport

Inspect/Apply/Lookup have been added to the same mTLS server. They reuse frozen base Session/CellCall and check the new binding hash and JSON artifact hash/size. All payloads undergo strict JSON/Protobuf checks and a 1,000,000-byte limit. All counters use the existing exact integer representation.

P's HostClient validates current Host identity, response hash/schema/size, request digest and metadata currency judgments. Direct API users **must durably retain the request before transmission.** This transport helper does not itself replace approval, the send boundary or mixed-configuration coordination in P's business ledger.

## Verification and remaining connections

Tests include retransmission of the same request, rejection of different intents/slots, current caller checks, full cohort/partial rejection, absent/stale quiescence, remaining prepared work, commit rollback, and SIGKILL/restart/Arm rejection immediately after commit. Separate S simulation server and P client mTLS tests inject normal response loss after commit and verify recovery of the original receipt and its historical classification after restart. Test hooks and response-loss injection cannot be configured by external requests, and the product image has test-harness disabled.

P's process-context coordinator/receipt aggregation, actual configuration selection replacement and revalidation review are connected. Global qualification activation and cancellation/restoration are not yet connected. No actual equipment was controlled and NativeAdapter physical support was not validated.
