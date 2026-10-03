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

`run` reserves one Part, binds the selected actual object, obtains the explicit v2
start context, and submits the existing v2 Start. P repeats admission checks. It
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

This checkpoint does not establish N-Part operation or UNKNOWN reconciliation;
those remain separate user-run checkpoints.
