# RX process sources and deterministic compilation

This is S-owned ProcessSource, resolved-tree and next-work-candidate computation. It contains neither device I/O nor P's operating authority/outcome ledger. It proposes subsequent requests based on P-validated results and checkpoints.

## Source representation

`ProcessSource` consists of a process ID, entry flow, condition definitions and a unique node graph per flow. Each flow is a tree; reuse is represented through distinct Call nodes.

| Node | Meaning |
|---|---|
| Sequence | Proceed to the next step after the previous step completes and required resource handover is confirmed |
| ParallelAll | Parallel paths with separate actual resource sets. All must complete for success |
| Branch | One path selected by a condition decision persisted in P |
| Repeat | Expand into distinct execution positions for an explicit finite count |
| Call | Instantiate a subflow separately at each call location |
| Operation | Reference a Host/Intent binding provided by the site resolver |
| Wait | Condition with explicit deadline. Proceeds by P's wait decision |
| Intervention | Procedure reference and intervention wait. Requires an authorized P continuation to proceed |

Duplicate sources/flows/nodes, missing/unreachable/cyclic/shared nodes, recursive calls, empty control nodes and exceeded repetition/depth/expansion limits are rejected. Condition grammar also checks size, empty groups and inverted ranges.

## Compilation identity

Repetitions/calls expand into unique node IDs and SourceLocations containing instance paths. Identical original node IDs and call/repetition paths retain identity even if tree-definition serialization order changes. Flow/node lists are normalized in the source digest; Sequence child order is preserved as meaningful execution order.

The resolved digest binds original identity to actual normalized Host/Intent bindings. Overlapping resource unions across parallel paths are rejected. This uses resolved resource names even when two differently named logical functions share the same controller. Correct consolidation of aliases by the site resolver requires separate validation.

`compile_package` reads only immutable Process entries from VerifiedPackage and checks that OperationSubmit requests used by bindings are declared in package permissions. An invalid process source does not pass even with a valid signature. This permission check grants no site/Host native grant.

## Frontier computation

Input ProgressView must be **an authenticated, validated complete P run view**. Evidence IDs or booleans arbitrarily created by local UI/BT must not be trusted as this input. This is currently a pure planning API; its P wire/checkpoint adapter remains future work.

- UNKNOWN/DISPUTED/UNRESOLVED remain BLOCKED. They are not converted to FAILURE for automatic fallback/retry.
- SUCCEEDED alone does not start the next sequence step; required resource release is awaited.
- Without a branch decision, only a decision-request candidate is produced. Reading current sensor values does not commit a branch in local memory.
- History in an unselected branch, later-step history missing earlier steps, different resolved digests, partial views and incorrect node/operation correlations are rejected.
- Parallel failure/uncertainty suppresses new admission candidates while preserving work already in progress. Native cancel success is not inferred.
- Wait timeout is an explicit P decision, not physical completion. Intervention clearance must also bind to actual P authorization.

Frontier COMPLETED means structural completion in this planning view. It does not directly record part CONFIRMED_COMPLETED, conforming output or operating resumption.

## BT XML and current limitations

Generates BT.CPP format4 XML. It uses the [official XML format](https://behaviortree.dev/docs/tutorial-basics/tutorial_07_multiple_xml/), but **requires dedicated registration of RXSequence/RXParallelAll/RXBranch/RXOperation/RXWait/RXIntervention nodes**. It emits no arbitrary Script/include, generic RetryUntilSuccessful or transformation downgrading UNKNOWN to FAILURE.

Actual BT.CPP factory registration of the six RX C++ nodes and execution tests with synthetic P views are implemented in the [native executor](../../native/executor/README.md). Actual P client, durable requests/branches/checkpoint recovery are not yet connected. XML and unit execution alone are not claimed to complete the product operating path.

The common model/frontier uses P-owned rx-process-contract SDK. P validates branch/wait/checkpoint and activation/Submit eligibility in graph configurations through actual transactions. External RPC/checkpoint artifacts, C++ client and full intervention/restart integration remain incomplete. A graph must not be arbitrarily flattened into existing process=None finite configuration for operation.

## CLI and examples

```text
rx-process-compile SOURCE.json BINDINGS.json NEW_OUTPUT_DIRECTORY
```

Creates resolved.json, process.bt.xml and compile-report.json in a new directory. Existing outputs are not overwritten. The result is COMPILED_NOT_QUALIFIED and executes no device/process.

`examples/process` is an **unverified example** representing five material-supply steps. It is not actual robot/PLC signal, program, calibration, gripper or fixture binding. Placeholder artifacts must be replaced with actual deployment material and validated.
