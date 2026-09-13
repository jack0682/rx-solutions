# Host qualification acceptance and separate start procedure

Status: phase53 implementation draft. Connects Inspect/Accept/Lookup and the shared data model for optional `rx.host.qualification.v1`, S's atomic acceptance/final gate and the P transport client. Existing base/cell norms are unchanged. P's [durable issuance and global activation](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/QUALIFICATION_ACTIVATION.md) are connected.

## What is accepted

Authenticated P requests binding a reviewed qualification ID/revision to the current Host process context. The Request pins the following.

- Host ID, expected Host boot/delivery journal and current static Binding fingerprint.
- change/application digest, review ID/version/digest, decision revision and policy digest.
- Configuration digest and original process-context request/receipt sequence for every Host-managed cell.
- definition/envelope/environment, new qualification ID/revision, dependency hash and limitations ArtifactRef.
- An explicit subset of allowed intent digests/purposes.
- Current cell epoch/scope map, exact fence request and required block IDs.

Definitions, environments, intents and purposes outside the current Binding are not accepted. An empty accepted intent set allows no operations. All Host-managed cells must be included in the same request and negotiated in the same base/cell session. Cells must not be omitted or substituted with another cell's context/fence.

The Host does not independently rereview P's review documents. It uses the authentication boundary that establishes P as the authoritative issuer; actual documents, signatures, human review and global application decisions are P's responsibility. The current raw client is not an issuer implementing that responsibility.

## Prerequisites and storage boundary

Accept validates the current caller/Binding/boot/journal and full cohort. Each cell must be unarmed, and its current epoch/scope/blocks must match the corresponding fence receipt. It also compares the original process-context configuration/request/sequence/change/binding.

If PREPARED/SEND_ENTERED/NATIVE_ACCEPTED work remains anywhere in the Host, the result is NOT_ACCEPTED. The adapter's actual handover_snapshot must confirm no_pending_commands/control_available/support_stable and age+uncertainty within 100 ms. Unsupported, unknown or stale observations are not converted into success. Age is checked once more immediately before the transaction.

The following are recorded in one transaction.

| Record | Meaning |
|---|---|
| accepted-qualification | Per-cell qualification, narrowed allowed scope, original process context, Host/device session and epoch |
| qualification-request | Original Request/digest, sequence, ACCEPTED or NOT_ACCEPTED, reason and observations |
| qualification-slot | One request ID per `(P peer, review ID, review revision)` |
| qualification-identity/maximum | Immutable configuration/scope/review semantics of qualification ID/revision and prevention of revision regression |
| qualification-history | Immutable receipt history by sequence |

A qualification ID/revision must not be reused for a different configuration, limitations or review evidence. Replacement by another acceptance requires a higher cell epoch. The only repetition allowed in the same epoch is retrieval of the receipt for the original request ID/body. NOT_ACCEPTED does not overwrite an existing acceptance.

Acceptance neither clears blocks nor creates Arm/grant/permit, and calls neither native submit nor lookup. Receipt quiescence is an observation at acceptance time, not a guarantee of sustained physical stability.

## Errors, retries and restart

The same ID and normalized body return the original receipt. A different body under the same ID, or a different ID for the same review/version, conflicts. New review results require an explicit procedure with a new review/version/epoch.

An RPC error is not NOT_ACCEPTED. The sender must durably record the entire Request before transmission and Lookup the same ID after response loss. An empty Lookup is also not definitive proof of non-acceptance. P's durable qualification task connects original-request lookup with bounded retransmission that checks current authority.

Lookup returns the original receipt together with the current snapshot/accepted identities. `receipt_matches_current_host` reports agreement with Host boot/journal/Binding/process context/epoch and qualification records. It does not mean site equipment remains stable; `activation_authorized=false`. The client verifies that this derived value agrees with the actual payload.

Host restart preserves receipt/qualification history but does not use historical qualification as current authority in a new boot. Changing process context or cell/scope epoch also invalidates the old acceptance. Cached Accept/Arm retransmission does not restore Arm state.

## Subsequent Arm and native gate

A cell with process-context does not fall back to the old qualification in its startup Binding when no currently valid accepted-qualification exists. Qualification acceptance still requires a separate Arm afterward; the operation gate may proceed only after that Arm handles the designated blocks.

Final native entry performs the following checks in addition to existing guards.

- The Permit's qualification ID/revision matches the currently accepted qualification.
- Requested intent/purpose belongs to both the static Binding and the accepted subset.
- Host boot/journal/Binding/process context/epoch/scope match the acceptance record.
- The native guard's device session matches the device session in quiescence at acceptance.
- All existing checks pass: armed state, empty blocks, current grant/fence/expiry, permit and local conditions.

New actions are rejected even if the device session changes within the same Host process. Existing rechecks immediately before native entry of a prepared operation are retained. Unresolved outcomes of already started work are not converted to success/non-execution or resubmitted under a new qualification. The independent local protection port remains separate.

## RPC and implementation files

Applies the exact [wire binding](../../sdk/spec/host-qualification/v1/README.md) hash and limits of 1,000,000 bytes for canonical payloads/1 MiB for gRPC. Uses existing mTLS/base/cell sessions and strict decoders. Checks envelope reference hash/size/schema, request ID/cell and negotiation of the entire cohort. Response-loss/commit hooks from test-harness cannot be selected through product RPC.

- `gate/qualification.rs`: acceptance, history and current qualification checks.
- `gate/scopes.rs`, `gate/validation.rs`: connection to separate Arm and final native gate.
- `rpc/qualification.rs`: optional service.
- P `rx-host-client::qualification`: Inspect/Accept/Lookup and payload/current-claim checks.

The static Binding is unchanged; accepted qualifications are kept in a separate ledger. The Binding list is the startup scope. Current qualification must be queried through this service.

## Verification and remaining work

Scopes are distinguished in the [phase53 record](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase53_checks.json). Host tests cover no action from acceptance alone, simulated action with explicit Arm/new qualification, rejection of old qualification, purpose and empty intent scope, key/slot/qualification semantic conflicts, full-cohort atomicity, remaining native work, unsupported/stale quiescence and device session changes. SIGKILL immediately after commit checks receipt preservation and Arm rejection after restart.

P/S mTLS integration saves a Request **constructed by the test harness** from an actual approved P review view to a file before transmission, then checks Host commit-response loss, same-ID retrieval/retransmission and conflict rejection. This was the phase53 scope; subsequent phase54 extended it through actual P issuance/global activation and separate simulated work completion. This integration causes 0 native effects. Explicit simulated actions in separate Host unit tests are checked through an independent effect log.

P issuance/durable Host tasks, partial/unknown lookup, global activation and separate user start are connected through the [phase54 path](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/QUALIFICATION_ACTIVATION.md). Full cancellation/revocation, operational recovery and dedicated UI remain future work. Cancellation/revocation, upgrades/restoration, actual device-specific performance/protection validation and physical commissioning also remain outstanding. The first physical cell is NOT_COMMISSIONED.
