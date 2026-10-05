# External process adapter packages

This SIMULATION-only provider implements the existing NativeAdapter interface on the
same rx-hostd binary. Host owns package verification, input membership, the command gate,
child/process group, receipt writer and resource-handover checks. A package supplies native
facts; it cannot approve an input, advance a Run, settle UNKNOWN or release a resource.
Existing Python and other builtin providers retain their behavior.

The installed rx-device-package tool provides:

```text
external-sdk OUT
external-program EXECUTABLE ARGUMENTS_JSON DEPENDENCIES_JSON OUT
external-assemble ASSEMBLY RECIPE OUT
request CANDIDATE KEY_ID OUT_FILE
seal CANDIDATE SIGNATURE POLICY OUT
external-register PACKAGE POLICY KEY OUT [BASE_REGISTRY]
```

SDK export creates a new directory with rx_external_adapter.py; no source checkout is
needed. Program preparation records exact bytes of an absolute executable and its declared
dependency files; arguments are an immutable JSON array. Native request data never selects
an executable, module, path or interpreter. Prepare these files in the target Linux
environment and install the verified closure read-only. Signing uses the existing external
signer boundary: request emits the bytes to sign and seal verifies the signature and policy.
An unsigned candidate is not registered or activated.

The assembly uses `rx.external-process-assembly.v1`: profile, program, common v2 template
catalog and native outcome table. The program descriptor is `rx.external-process-program.v1`.
Its artifact reference is the concrete ProgramGoal program pin. The profile declares commands
with the existing NodeContract parameter types/units/frames, typed source observations with
age/uncertainty bounds, and boolean source-to-condition mappings. The source profile and
native driver identities are signed with the existing DEVICE_REFERENCE package format.
No Python profile fields, common approval semantics or package ABI are redefined.

Registration produces a new immutable registry file and its SHA-256. It records availability
only, never writes a running Host's configuration, and refuses an existing registration key.
The Host selects a registered package through its ordinary pinned startup configuration:

```json
{
  "kind": "EXTERNAL_PROCESS_PACKAGE",
  "registry": {"path": "/config/host/registry.json", "sha256": "REGISTRY_SHA256"},
  "adapter": "example/counter"
}
```

An unknown key, invalid signature, changed bytes, undeclared command, or mismatched local
Host binding fails closed. Inspect and initialization do not execute adapter code. Selection
must still pass the existing configuration and qualification path. Host Prepare compares
received bytes against its own acknowledged v2 domain, exactly as for other v2 providers.

## Provider code

The exported Python helper is optional; other implementations can speak the same bounded
private protocol. It invokes a provider with three methods:

- `execute(envelope, correlation)` performs one finite native command, returning only
  `status_schema` and integer `status`. Correlation contains original operation/invocation
  and the saved v2 selection. It must never implement a workflow loop or retry an effect.
- `observe(sources)` is passive and returns exactly the requested declared samples. Use
  `sample(typed_value, acquired_at=..., uncertainty_ns=..., quality_good=...,
  origin_age_bounded=...)` with an actual acquisition timestamp. A cached value must retain
  its original age/quality; query time cannot turn it into a fresh measurement.
- `custody()` passively reports `no_pending_commands`, `control_available`, `support_stable`
  and `safe_to_drop` native facts. Host additionally checks its own owned process handles,
  outstanding invocation state and evidence; these flags cannot authorize handover.

The Host appends `execute`, `lookup` or `observe` and its owned native storage directory to
the pinned launch arguments. A bounded JSON request arrives on a private channel. Execute
returns a flushed, correlated entry frame followed by completion; passive modes never
invoke execute. No listener, shell command from a request or native fallback is provided.
Entry means durable acceptance of the exact finite request before I/O, not physical success.

The helper's create-once request/completion files are native evidence in the existing Host
native-storage namespace, not a second authority/recovery ledger. Completion survives lost
acknowledgements and lookup never reissues it. Existing request identities cannot be replaced.
Optional provider after_entry/after_completion hooks expose native boundaries for isolated
SIM provider testing; the Host has no device-specific fault or task switch.

The Host independently verifies response identity, exact dispatch digest, current device
generation, declared types, sample freshness and owned process exit/reaping. Native capture
is committed by the existing Host writer before post-commit acknowledgement clears pending
custody. Unknown or mismatched facts retain uncertainty. Restart alone is insufficient:
original completion and current custody evidence are both necessary. Cold Host recovery is
not implemented by this provider; reopening saved files does not recreate process ownership.

## F2 scope

The S2 example's separately committed acquire-support/withdraw split is scenario authoring,
not registry implementation cost. For this SIM, unclamp follows the same Part's settled,
successful acquire-support operation. **There is no fresh gripper support observation**;
ordering alone is weaker than a real cell requires. Keep that limitation in acceptance
receipts. A later registry-declared gripper observation and unclamp guard are parked.

Registration is not live replacement. Do not swap selected bytes, retarget a held operation,
discard its native facts or treat registration as a stop/handover certificate. Existing Host
lifecycle checks and retained operation custody still apply. Physical device operation,
hot replacement of held work and independent recovery state machines are outside this scope.
