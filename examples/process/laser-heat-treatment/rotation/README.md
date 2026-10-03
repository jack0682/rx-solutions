# S2 rotation alignment and tending package data

These files carry the F0 S2 example into explicit execution-v2. `definitions.json`
adds angle/tolerance properties, a part model and a fixture model/instance.
`task.json` declares `rotate-align`: the angle is part nominal angle plus fixture
offset (90 + 5 = 95 degrees), and tolerance is the minimum of their limits
(min(0.8, 0.5) = 0.5 degrees). `compose.py` inserts this task between load and clamp
in the existing workflow. No Task-specific core branch is required.

```sh
python3 compose.py ../../../definitions/0f-laser-simulation/workflow.json \
  task.json /absolute/new-workflow.json
```

Run the composer with the actual path to the base workflow if working elsewhere.
Apply base/M2 definitions before these additions, retaining their definition
receipts. Use the common definition and workflow CLI to resolve and save the model.

`skill.py` accepts only the common `rx.workflow-parameters.v2` envelope. One call
performs one primitive on the file simulation. It has no workflow or N-Part loop.
The existing Python environment/package tools pin `skill.py` and `skill.json`;
prepare the configured simulation state directory before operating. A rotation
records requested/measured angle and tolerance; clamp requires successful alignment.
The common input selection is preserved in each effect record.

This is SIMULATION only. The prepared Python environment, signed template catalog,
publication and actual object binding are required by the
[execution path](../../../../deployment/local-skills/EXECUTION.md).
