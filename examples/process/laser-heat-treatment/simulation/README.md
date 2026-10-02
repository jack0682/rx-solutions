# File-only tending device program

This package supplies one Python entry point with the pick, load, clamp, door,
process, unload and place primitives used by the eight-node Workflow. It is a
**FILE_SIMULATION device model**, not a workflow engine, physical controller or
collision/functional-safety model. P/Executor determine the next operation; the
existing Host gate authorizes each invocation before this program is imported.

Input is the exact `rx.workflow-parameters.v1` artifact produced by the Workflow
compiler. The program consumes fixed quantities with explicit units and frames,
updates an independent device state and records applied parameters/observations.
It rejects unsupported primitives, nonconcrete values, frame/unit mismatches and
invalid device-state transitions. Clamp handover requires the explicit
`release_gripper` input; unload establishes simulated gripper support before
unclamping. Process completion waits for the resolved duration. No actual robot
kinematics, heating, contact mechanics or safety interlocks are modeled.

The prepared environment's signed `skill.json` supplies the simulation state
directory and initially available slots. The checked-in example has 24 simulated
slots. Site preparation must create an owned directory and set its exact absolute
path **before** preparing/pinning the environment. Runtime inputs cannot select a
filesystem path. `state.json` and `effects.jsonl` are device records separate from
the Host journal. They contain no operating authority. A failed/killed process
can leave an incomplete state, and this program never relabels it complete or
replays an original Host request. An already consumed slot is not reused silently.

Several fixed parameter artifacts can be packaged as one signed Python library:

```sh
rx-device-package python-library-assemble LIBRARY.json ENVIRONMENT.json RECIPE.json NEW_CANDIDATE
rx-device-package request NEW_CANDIDATE PUBLISHER_KEY_ID NEW_SIGNING_REQUEST.json
```

The external signing, policy verification, seal, review and Host binding steps
remain required. See the device-package README. The explicit new backend is
`PYTHON_SKILL_LIBRARY_PACKAGE`; the original single-program forms are unchanged.
The library contains at most 16 fixed programs in one environment, each with a
bounded object input. All programs share one Host support owner and uncertainty
set. A lost/unknown call blocks resource handover and other program dispatch.

Prechecks:

```sh
./tools/cargo test -p rx-host --test workflow_simulation --locked
./tools/cargo test -p rx-host --test python_skill --locked
./tools/cargo test -p rx-device-package --test python --locked
```

The first test resolves the real example data for ECC_51-14 and ECC_99-14 using the
shared resolver, compiles their values, executes all eight steps through actual
Host prepare/authorize and Python processes, checks independent device state and
applied widths/forces/poses, measures the 5/7-second process durations and verifies
that original-request reads add no effects. It uses explicit test Host identities
and a test clock; it does **not** establish registered P/mTLS/Executor operation,
Preview/publication, runtime actual-part binding, N-slot selection, transport-fault
recovery or final RC acceptance. Those remain M3 integration work.
