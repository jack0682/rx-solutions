# Published workflow execution (checkpoint 1)

The `rx execution` commands use a registered P terminal, the resident Executor,
and the existing Host gate and Python runner. The checkpoint covers one Part of
the package-defined S2 nine-step workflow on SIMULATION devices. An installed and
qualified publication, initialized slot pools, and an actual object instance are
required. Publication or material export alone does not qualify a cell.

Use the connection file described in [RUNTIME.md](RUNTIME.md). Select the saved
publication and an exact object reference (`catalog`, `id`, `revision`, `digest`):

```sh
python3 deployment/local-skills/rx execution \
  --connection /absolute/operator.json --state-dir /absolute/request-journal \
  run /absolute/publication.json --object /absolute/object.json \
  --request-id CANONICAL_UUID --output /absolute/new-receipt.json
python3 deployment/local-skills/rx execution \
  --connection /absolute/operator.json --state-dir /absolute/request-journal \
  inspect P_RUN_UUID --reports
```

`run` reserves one Run for the ordered `--object` arguments. An optional `--count`
must match their number; one object preserves the original one-Part command.
The CLI binds the first actual object, obtains the explicit v2 start context,
and submits the existing v2 Start. While waiting, it supplies the next object only
after P reports the preceding Part CONFIRMED_COMPLETED. P rechecks current authority
and references for every binding/admission. If the CLI exits, the existing Executor
waits for the next object; repeat the original command/request ID to continue. P repeats admission checks. It
never falls back to legacy Start. Keep the printed request UUID: after a lost reply,
repeat the same command and UUID with a new output path or without `--output`.
The saved request cannot be replaced by another publication or object. Creating a
new request is not a recovery procedure. `inspect` sends no execution command.

The receipt names the workflow and environment and includes P's Run, Part and
operation states, immutable operation bindings, and the approved report whose
bytes match the operation's report reference. Reports describe selected inputs;
operation completion and handover are separate records. A non-completed Run exits
with code 2; input/transport failures exit with code 1. Existing output paths are
refused before login or mutation.

## Installation and authoring boundary

The authoring commands `preview INPUT`, `publish INPUT`, and `export PREVIEW DIRECTORY`
use P's saved versions. `export` retains exact canonical policy, input closure and
report index bytes and writes `host-material.json`, split into pinned files within
the existing per-file limit. The installer supplies those files as
`execution_materials` in Host startup configuration. Paths are installation-owned;
Prepare cannot choose them. Export is not a qualification acknowledgement.

A signed `rx.python-execution-assembly.v2` combines a Python-only profile with the
common execution template catalog. The package CLI assembles, signs externally,
verifies and reassembles it through the existing DEVICE_REFERENCE boundary:

```sh
rx-device-package python-execution-assemble ASSEMBLY ENVIRONMENT RECIPE NEW_CANDIDATE
rx-device-package request NEW_CANDIDATE KEY_ID NEW_SIGNING_REQUEST
# An external signer signs the exact request; runtime images receive no private signer.
rx-device-package seal NEW_CANDIDATE SIGNATURE POLICY NEW_PACKAGE
```

Use Host backend `PYTHON_EXECUTION_PACKAGE`. In the Executor's cell startup
configuration, explicit `"execution_v2": true` declares the existing v2 protocol
before P can admit the first Run. The omitted/default false setting preserves
legacy configuration serialization. Unsupported v2 negotiation prevents startup;
it does not enable a legacy execution fallback.

Host verifies the complete approved domain, associates its own durable v2
qualification acknowledgement with that domain, and independently compares actual
Prepare parameter bytes with the approved member. Python receives the common
verified envelope; its profile only pins environment and program artifacts. The
[common contract](https://github.com/jack0682/rx_docs/blob/2f5526e40a458b7bffb7af0b6ba6d9ce22e0e4c0/docs/contracts/workflow-execution/v2/host-input-membership.md)
owns input/binding/membership semantics, including for future external providers.

For CP2, supply three distinct object references in ordinal order:

```sh
python3 deployment/local-skills/rx execution \
  --connection /absolute/operator.json --state-dir /absolute/request-journal \
  run /absolute/publication.json --count 3 \
  --object /absolute/object-1.json --object /absolute/object-2.json \
  --object /absolute/object-3.json --request-id CANONICAL_UUID --wait-seconds 180
```

The execution-v2 runner supplies RX_HOST_OPERATION_ID and RX_HOST_INVOCATION_ID
from its existing private request solely for log correlation. They are not approved
parameters or admission/recovery authority. The S2 SIM effects log records them
before returning, so each effect can be joined to the product Run's operation and
invocation records. Parameter bytes and main(inputs) remain unchanged. A repeated
effect remains visible; this logging does not suppress it or certify completion.

Checkpoint 3 exercises the separate result-loss observation and continuation path below.

## Result-loss observation and continuation

For the isolated SIM result-loss fixture, add `--until-unknown` to `execution run`.
It returns P's receipt immediately when an operation has execution_knowledge UNKNOWN,
with the usual incomplete-run exit code 2. This changes only CLI waiting; it does not
change admission, create uncertainty, stop the Host or settle an operation.

Each work entry includes current P-owned resource records and its original
reconciliation request, if any. `slot_pools` contains current pool generations and
this Run's slot holds. These are observations, not new authority; resource state is
current while operation/report references preserve their historical identity.

After restoring transmission, repeat the same `run` command, ordered object references
and request ID without `--until-unknown`, using a new output path. This recovers the
original Run/Start and continues supplying objects after confirmed Part completion.
Already emitted native operations are queried/reconciled through the existing path;
they are not resubmitted. `inspect --reports` reopens those same records.

The [SIM result-loss link](../simulation/result-link/README.md) implements the
checkpoint fault outside the Host. Mixed ECC model data is in
[rotation/mixed-models.json](../../examples/process/laser-heat-treatment/rotation/mixed-models.json).
