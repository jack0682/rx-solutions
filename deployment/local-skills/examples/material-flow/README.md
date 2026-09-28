# Material flow as four registered skills

This is a **logical software model**, shaped like rx_poc main's pick-and-place
flow. It does not observe or control a robot, fixture or material. Its purpose is
to exercise server-side composition, per-execution data lineage and failures.

From the installed bundle's example directory:

```sh
rx skill add ./acquire
rx skill add ./load
rx skill add ./release
rx skill add ./unload
rx process add .
rx process run material-flow --input '{"part":"part-one","present":true}'
rx process run material-flow --input '{"part":"missing","present":false}'
rx process runs
rx metrics
```

The first call returns `part-one` at `output-bin`. The second fails inside acquire
and leaves the later three skills NOT_STARTED. No readiness, request polling,
authority, journal or result-transfer nodes are authored in the process.

`process.json` uses RX's existing ProcessSource structure and its validator. Each
Operation binding identifies an immutable registered skill version and business
input references. The server pins package digests. INPUT reads this process's
input, OUTPUT reads a named preceding skill's validated output, and LITERAL binds
a constant. Forward references and incompatible port types are rejected.

The current LOCAL_SIM draft executes Sequence/Operation only. It refuses other
control kinds rather than flattening their semantics. Full P/Host/Executor
integration, resource/physical handover, branch/parallel execution and operator
reconciliation remain separate incomplete work in the overall framework draft.
