# N1 handover: a compiled command prefix and its trace boundary

Measured 2026-09-15. **Software simulation only; NOT_COMMISSIONED.**
This fixture runs the real compiler and pure frontier planner with synthetic
operation inputs. It does not execute N1 through P, a Host, or a native device.
`OPEN-MOBILE-SUPPORT` and `OPEN-RECEIPT` remain **open**.

The case is [N1, sections 1–4](https://github.com/jack0682/rx_docs/blob/d39bac906681adb7eb75006f46f458eab11645ca/docs/20_first_handover_case.md).
The process is intentionally named `example/N1-command-prefix`, and its root
`command-prefix-boundary`: its endpoint is a command graph boundary.

## What is represented

| Authoring node | Host / target | Meaning assumed by the fixture |
|---|---|---|
| place | HOST-IN / IN-01 | Place PKG-01 onto the receiving tray |
| receiver-hold | HOST-OUT / OUT-01 | A finite command to establish receiver holding; no continuous support monitor |
| sender-release | HOST-IN / IN-01 | Release the sender gripper while alternate support is maintained |
| withdraw | HOST-IN / IN-01 | Withdraw the sender from BAY-01 |

[source.json](source.json) fixes that sequential order. [bindings.json](bindings.json)
selects the two Hosts, distinct controller targets and logical conflict resources.
The first three actions share `PKG-01/HG-01`; all four include `BAY-01`. The receiver
also includes `TRAY-01`. These are N1 case identifiers used as command-resource
names, not verified controller aliases or sensor-derived occupancy records.
The compiler rejects intersecting parallel resource sets across different Hosts.
It cannot discover a shared physical resource omitted from the bindings.

All profile/site digests and program/parameter digests are **placeholders** copied
from the existing [material-supply bindings](../material-supply.bindings.json).
`rx.development.placeholder.v1` marks the program references; repeated-byte
digests do not identify actual N1 artifacts. Empty calibration arrays provide no
calibration. `simulation/completed` and `simulation/cancel` are unresolved rule
names here. FINITE_ACTION bodies express intended command boundaries, not a
qualified capability. No ENSURE_STATE observation is invented to fill the gap.

The existing 5000 ms execution timeout and 1000 ms prepare validity are retained
as software fixture values, without physical timing claims or supporting
evidence. No new physical timing, load, pose or device specifications are supplied.

## Explicit boundaries outside the graph

[boundary.json](boundary.json) is an example annotation, not a new product/wire
contract. Its `outside_graph` list is also included in the generated trace:

1. **Retained material and BAY occupancy — NOT_EVALUATED.** At the N1 endpoint,
   OUT-01/TRAY-01 still supports PKG-01 and OUT-01 still occupies BAY-01. The fixture
   supplies no N1 MaterialState source, continuous support evaluation, physical
   alias resolution or occupancy admission adapter. A command `RELEASED` state
   does not clear physical support/occupancy or permit a conflicting new handover.
2. **Normal receipt and service aggregation — NOT_EVALUATED.** There is no
   authenticated RECIPIENT-01 attestation, validity procedure or service rule
   combining physical evidence, receipt and retained occupancy. Normal receipt is
   not represented by an Intervention node or a fake intervention clearance.

Both boundaries feed the unresolved service relation
[N2 R-11, section 5](https://github.com/jack0682/rx_docs/blob/d39bac906681adb7eb75006f46f458eab11645ca/docs/22_open_items_boundary.md#5-open-receipt%EC%9D%98-%EA%B8%B0%EC%A1%B4-%EA%B5%AC%EC%84%B1%EA%B3%BC-%EC%B6%94%EA%B0%80-%EC%9D%98%EB%AF%B8)
and [N3 section 6, final row](https://github.com/jack0682/rx_docs/blob/d39bac906681adb7eb75006f46f458eab11645ca/docs/23_handover_counterexamples.md#6-%EA%B7%9C%EB%B2%94%EB%A7%8C%EC%9C%BC%EB%A1%9C-%ED%99%95%EC%A0%95%ED%95%A0-%EC%88%98-%EC%97%86%EB%8A%94-%EB%B6%80%EB%B6%84%EC%9D%98-%EC%97%B0%EA%B2%B0).
**Even a `COMPLETED` command frontier is not N1 service completion, material
release, receipt validation or physical qualification.** The trace's release
evidence IDs and true release conditions are assumed synthetic inputs, not
observations that establish any of those claims.

## Reproduce the compilation and planning trace

Run from the repository root. Use a new output directory for each compilation:

```sh
./tools/cargo run --locked -p rx-process --bin rx-process-compile -- \
  examples/process/n1-handover/source.json \
  examples/process/n1-handover/bindings.json /tmp/n1-compiled
N1_TRACE_OUTPUT=/tmp/n1-trace.json ./tools/cargo test --locked \
  -p rx-process --test n1_handover
cmp /tmp/n1-trace.json examples/process/n1-handover/trace.json
```

The compiler writes `resolved.json`, `process.bt.xml`, and `compile-report.json`,
with `COMPILED_NOT_QUALIFIED`. Their digests are real content hashes of generated
software artifacts; they do not replace the placeholder profile/program hashes.
The compiler checks typed input/intent validity, reachable bounded graph shape,
binding resolution, stable node/source identities, and parallel resource
intersections. It does not load the named programs, validate rule semantics,
authenticate P, qualify a profile, or operate a device.

[The test](../../../runtime/rx-process/tests/n1_handover.rs) executes actual
compiler/frontier code. [trace.json](trace.json) records the synthetic ProgressView
and computed frontier for each step, with stable synthetic IDs, source digest and
resolved digest. `ProgressView.complete=true` means a complete input snapshot,
not a completed Run. No input snapshot in this file is an authenticated P view.

| N1 stage | What this run actually produces | What remains only a structural or missing record |
|---|---|---|
| Request receipt | No service-request storage call | CLIENT-01 request, durable Run, request identity/retrieval; the trace's Run UUID is synthetic |
| Operation admission | Planner candidate, then an explicitly constructed ADMITTED operation | P transaction, Activation binding, pending dispatch, mandate/permit and HOST_PREPARED are not exercised |
| Native entry and transfer | Synthetic `sent()` input, synthetic outcome/evidence IDs, computed frontier | Host SEND_ENTERED journal, invocation/result lookup, parcel identity, receiver support and sender separation are not observed |
| Command handover | Every SUCCEEDED+HELD node yields RUNNING, its node in `handovers`, and no successor candidate; assumed release then opens the next command | Actual residual-command, control-handover and continuous support evidence remain missing |
| Normal receipt | Explicit outside-graph NOT_EVALUATED record | HUMAN_ATTESTATION identity/authority/validity and its physical-evidence relationship |
| Command graph endpoint | All four assumed command handovers yield COMPLETED | R-11 service completion and retained material/BAY occupancy remain NOT_EVALUATED |
| Lost sender-release response | Original operation remains UNKNOWN/QUARANTINED, frontier BLOCKED, no new candidates over repeated planning reads | No packet was dropped, native effect executed, Host result queried or operator action authenticated |

### N3 response-loss correspondence and negative control

The `lost_response` trace follows
[N3 section 2.2, steps 1–6](https://github.com/jack0682/rx_docs/blob/d39bac906681adb7eb75006f46f458eab11645ca/docs/23_handover_counterexamples.md#22-%EC%A1%B0%EA%B1%B4%EB%B6%80-%EC%9C%A0%EB%B0%9C-%EC%8B%9C%ED%80%80%EC%8A%A4):
assumed sender-release entry, loss of continuity, preservation of the original
operation, and no successor/retry proposal. It starts after the first two command
handovers are assumed. Ten repeated planner reads cannot change that identity or
manufacture success. A separate injected investigation/abandonment conclusion
then sets UNRESOLVED and remains BLOCKED; elapsed time never supplies it. Actual
receipt/outbox lookup and physical evidence retrieval in N3 step 4 are untested.

The second test isolates the common support token in a deliberately parallel
receiver/sender variant: different Hosts/controllers still cause a compile error.
Removing that token makes this intentionally incomplete variant compilable. This
negative control demonstrates why compilation alone cannot verify physical alias
completeness or the runtime T1 reservation behavior discussed in N3 section 4.

## Declared input measurement: 8 filled, 69 not filled

[slots.json](slots.json) assigns each of the fixed 77 IDs in
[doc 21 section 2](https://github.com/jack0682/rx_docs/blob/d39bac906681adb7eb75006f46f458eab11645ca/docs/21_declaration_reuse_measurement.md#2-%EA%B3%A0%EC%A0%95-%EC%8A%AC%EB%A1%AF-%EB%AA%A9%EB%A1%9D)
exactly one status, using section 6's strict topic-completeness rule:

| Status | IDs | Count |
|---|---|---:|
| simulation_value | S07, S13, S20, S21, P20, P33, P39, P40 | 8 |
| placeholder_only; excluded from filled | S03, P16 | 2 |
| unfilled; includes partial topics and absent validation | Remaining IDs enumerated in slots.json | 67 |
| Total not filled | placeholder_only + unfilled | 69 |

This denominator measures input topics, not physical adequacy, labor or unique
decisions. P33 and P40 intentionally retain the same timeout in two fixed topics.
Concrete simulation identifiers, sequence/resource sets, open input bundles and
limits count; placeholder bytes and rule names do not. The selected measurement
inputs are source, bindings and boundary.json, not a union with all pre-existing
N1 prose or unrelated device fixtures. Thus participant-responsibility, human
intervention and attestation declarations from doc 20 (S14/S15/P49) are not
imported as newly filled values of this executable fixture. The planning trace
also does not fill device validation slots P51–P56 or installation validation S19.
The real compiled hashes fill neither device source-commit nor profile identity
slots. No inherited reuse coefficient or doc 21 label is recalculated.

```python
import json
from collections import Counter
from pathlib import Path
m = json.loads(Path("examples/process/n1-handover/slots.json").read_text())
expected = {f"S{i:02}" for i in range(1, 22)} | {f"P{i:02}" for i in range(1, 57)}
ids = [r["id"] for r in m["slots"]]
assert len(ids) == len(set(ids)) == 77 and set(ids) == expected
counts = Counter(r["status"] for r in m["slots"])
assert counts == {"simulation_value": 8, "placeholder_only": 2, "unfilled": 67}
print(dict(counts), "not_filled", 77 - counts["simulation_value"])
```

## Existing execution assets: measured baseline

Baseline source: rx-solutions `9c98da86960fe4412f3cea9340a050776184e899`.
The older process/example READMEs understate later runtime work. The actual
[resident service](../../../runtime/rx-executor/CELL_CLI.md),
[P decision/recovery integration](../../../runtime/rx-executor/DECISIONS_AND_RECOVERY.md)
and [persistent C++ engine](../../../native/executor/PERSISTENT_ENGINE.md) exist.
An absence of filenames containing `e2e` or `integration` is not evidence that
integration tests are absent.

Observed on macOS/arm64 and Docker Engine 29.7.2 (Linux/arm64):

| Command/probe | Observed result and scope |
|---|---|
| Compile existing material-supply with rx-process-compile | Success, three artifacts, COMPILED_NOT_QUALIFIED; no device execution |
| `./tools/cargo test --locked -p rx-process --tests` before this fixture | 11 tests passed: graph/source identity, package/semantic boundary, command release, uncertainty, decisions and conflicts |
| `./tools/cargo test --workspace --all-features --locked` before this fixture | 261 passed, 0 failed, 15 ignored; platform-gated tests may be absent from macOS builds |
| `./tools/cargo run --locked -p rx-executor --bin rx-executor-service -- --help` on macOS | Exit 2: `rx-executor-service requires Linux BOOTTIME; no substitute clock is used` |
| `python3 tools/test_bt_executor.py --evidence-dir OUTPUT` | First failed because protocol-validation image was missing; after building the repository Dockerfile below, passed real C++ BT nodes and compiled five-step material-supply with synthetic frames |

The recoverable Docker prerequisite and rerun are:

```sh
docker build -f docker/ProtocolValidation.Dockerfile -t rx-solutions:protocol-validation .
python3 tools/test_bt_executor.py --evidence-dir /tmp/n1-bt-baseline
```

BT baseline output: `PASS: RX BT nodes, unknown/revocation/handover, strict XML,
bounded requests, compiled five-step example`. The pinned BehaviorTree.CPP commit
is `6e469c6ba133aaa842dac9b096b41f2d33ee2b0e`. The test container has network disabled,
a read-only filesystem, dropped capabilities and no device access. This proves
that the existing C++ engine executes its synthetic tests and material-supply
graph. It does **not** execute this new four-node N1 graph or exercise P networking.
The built-in compiled-example test requires exactly five operations; its result
is not relabeled as an N1 test. N1's executed scope is the Rust compiler/frontier.

The [file-backed Host simulator](../../../runtime/rx-host/src/simulation.rs)
can emit `effects.jsonl` with operation/invocation IDs and simulated completion,
but ignores program body mechanics and reports synthetic support readiness. It
cannot supply N1 parcel/tray support, separation or receipt evidence. No N1 Host
deployment, validated P snapshot or real N1 evidence source was configured here.
The blocking boundary is those missing case inputs/adapters, not absence of an
execution engine. Building them or substituting `process=None` finite operations
for this graph is outside this measurement.

### Runtime test inventory at the baseline

Counts below are textual `#[test]`/`#[tokio::test...]` declarations, not executed
test counts or a coverage percentage. Cargo's platform/features/ignore rules
determine which run. Tests with integration behavior need not be named integration.

| Crate | Test declarations / files | Inspected paths and intended exercised boundary |
|---|---:|---|
| rx-process | 11 / 2 | tests/process.rs, package.rs: compiler/frontier and verified-package semantics |
| rx-executor | 104 / 19 | client/service/worker tests and tests/journal.rs, cell_cli.rs, recovery_inspect.rs: P mapping, journal, service lifecycle, pre-connect/recovery paths |
| rx-host | 97 / 12 | tests/gate.rs, process_crash.rs, mtls.rs, service.rs, guarded_status.rs, melsec.rs, ros_jtc.rs: gates, simulated/native adapter transport, durable recovery |
| rx-supervisor | 27 / 7 | tests/lifecycle.rs, os_process.rs, guarded.rs, service_catalog.rs: process ownership, ordering, readiness, authority gates |
| rx-service-status | 13 / 1 | src/tests.rs: status/identity/observation validation |
| rx-device-package | 23 / 2 | tests/authoring.rs, jtc_authoring.rs: typed device inputs and compilation |
| rx-process-package | 15 / 1 | tests/package.rs: package identity, contents and semantic provenance |
| rx-solution-catalog | 3 / 1 | tests/builtin.rs: catalog assembly and simulation boundaries |

These tests and historical P/BT integration records are not N1 physical
qualification. The unchanged contracts/SDK and unresolved case boundaries remain
the constraints for any later device-backed experiment.
