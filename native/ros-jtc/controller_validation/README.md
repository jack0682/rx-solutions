# N5: real ROS JTC software with GenericSystem mock hardware

Measured 2026-09-16 KST. The unchanged RX C++ bridge exchanges real ROS action
messages with `joint_trajectory_controller/JointTrajectoryController`, loaded by
the official `controller_manager` executable. The hardware plugin is explicitly
**`mock_components/GenericSystem`**, not a robot, sensor, dynamics simulator or
parcel-support model. Physical qualification is **NOT_PERFORMED**; the catalog
remains **SIMULATION_FIXTURE**; the case remains **NOT_COMMISSIONED**.
`OPEN-MOBILE-SUPPORT` and `OPEN-RECEIPT` remain open.

The principal result is acceptance, success/result retrieval and exact-UUID
cancellation against actual controller software with mock hardware. No Rust Host
Authority is installed, no P admission runs, and **none of N1's four operations
is executed**. N4's graph and its six artifacts are unchanged.

## Reproduce

From the repository root, with Docker and the existing pinned BehaviorTree.CPP
source prepared as described in [the product image instructions](../../../dependencies/NATIVE_IMAGE.md):

```sh
docker build --build-context btcpp=vendor/BehaviorTree.CPP \
  -f docker/Solutions.Dockerfile --target runtime \
  -t rx-solutions:runtime-draft .
python3 tools/test_solutions_image.py --image rx-solutions:runtime-draft \
  --evidence /tmp/n5-product-smoke.json
docker build --build-arg RX_BASE_IMAGE=rx-solutions:runtime-draft \
  -f docker/JtcControllerValidation.Dockerfile \
  -t rx-solutions:jtc-controller-validation .
python3 tools/test_jtc_controller.py --image rx-solutions:jtc-controller-validation \
  --evidence /tmp/n5-controller.json
./tools/cargo test --locked -p rx-host --test ros_jtc
```

The validation layer adds `ros-jazzy-controller-manager` and
`ros-jazzy-ros2controlcli`. It does not alter the product entrypoint or register an
Authority provider. [The runner](../../../tools/test_jtc_controller.py) inspects
the created container before starting it: user 10001:10001, network none,
read-only root, no-new-privileges, all capabilities dropped, no device or host
bind mounts. It records actual container exit status and removes only its own
test container. `/tmp` is disposable container storage. No ROS host installation,
hardware, privileged mode or DDS network between containers is required.

The runner writes raw evidence JSON and separate stdout/stderr logs. Success
means its assertions passed; timestamps, UUIDs, discovery retries, feedback count
and image IDs may differ. Compare protocol outcomes and input/version identity,
not byte equality with [the recorded run](observations.json). Both ROS base image
and product build sources follow existing pins, but APT snapshots are not pinned;
the actual package inventory is retained for each run.

### Inputs and provenance

- [controllers.yaml](controllers.yaml): one six-joint position JTC, configured
  update rate 100 Hz from the existing catalog. This rate is a setting, not a
  measured real-time guarantee.
- [mock.urdf](mock.urdf): deliberately nonphysical link/joint names and
  GenericSystem, with zero initial mock state. No geometry, payload, device
  dimensions or mechanical limits are specified.
- [goal.json](goal.json): the existing bridge test's six-joint 0.1 target, 0.1
  tolerances and 100 ms trajectory/goal tolerance. The cancellation probe uses
  its existing 0.3 target and a four-second software window. These are synthetic
  test parameters, not physical equipment limits.
- [run.py](run.py): starts the actual manager, publishes a latched robot
  description, observes messages and drives the bridge over private pipes. It
  implements **no ActionServer**. It retains request/reply bodies, feedback,
  controller-state samples, commands and logs.

`observations.json` includes SHA-256 of the executed inputs, bridge executable
and the JTC/GenericSystem libraries actually found in the manager's `/proc/.../maps`.
`list_hardware_components` also reports `mock_components/GenericSystem` as active.
This distinguishes the loaded official controller from
[the existing hand-written rclpy ActionServer](../test_bridge.py). These are real
software hashes; no N1 placeholder hash is replaced or promoted.

## Observed behavior

| Probe | Observed result | What it does not establish |
|---|---|---|
| Load/configure | Official manager loads arm_controller as inactive, then activates it; type and six position claims match the catalog | RX reservation/ownership, external ROS client exclusion, physical readiness |
| Inactive or malformed goal | Bridge returns REJECTED before SendGoal; reversed joint ordering is rejected too | These are bridge preflight rejections, not controller-native failure or rejection results |
| Send and duplicate IPC | SEND_RECORDED with accepted=true and the supplied UUID; repeated identical IPC returns the same saved receipt | Durable Host SEND_ENTERED or exactly-once physical effects |
| Success/result | RESULT_CAPTURED, ROS goal status 4, controller error code 0 | P Outcome/RESULT_RECORDED, physical trajectory or parcel transfer |
| Feedback/state | Actual JTC action feedback with the same UUID and JointTrajectoryControllerState samples | Sensor quality, support, acquisition-age bound or real-time performance |
| Exact cancellation | CANCEL_RESPONSE contains only the requested UUID and terminal_stop_proven=false; later RESULT_CAPTURED has status 5 | Cancel acknowledgment or CANCELED as physical stop/hold/support proof |
| Bridge restart | New bridge instance queries the same running controller's canceled goal; known_to_bridge=false; cancellation of that unowned UUID is rejected | Controller-reboot history retention, durable recovery authorization or inherited cancellation authority |
| Deactivate/shutdown | Manager deactivates JTC; bridge inspect no longer matches and sends are rejected; manager receives SIGINT and exits 0 | Physical stop, torque behavior, support transfer, normal Host release/drop permission |

Discovery may initially return RPC_UNKNOWN. Read-only inspect retries do not send
goals or convert missing responses into success. The test waits for actual service
availability and validates the resulting controller observation.

The observed logs include unavailable FIFO RT scheduling in this capability-free
container. Runs can report update-loop overruns. The recorded shutdown also
retains ROS `pal_statistics` context-invalid diagnostic messages around SIGINT,
even though the manager and container exit 0. These messages are not removed from
the artifact or represented as a physical shutdown guarantee. The controller stack
is exercised as shipped; no upstream controller behavior is patched.

## Four axes: measured layer and remaining work

| Axis | Observed in this slice | Still absent or fixture-only |
|---|---|---|
| Control authority | Controller-manager active state and exact claimed position interfaces; inactive preflight rejection; **controller_generation_known=false** in actual bridge replies | ROS active state is not RX command ownership or physical exclusivity. This test instantiates **no Authority provider**. The real Host default `UnavailableAuthority.snapshot()` returns Guard; its existing Rust regression passes with zero fake-transport sends. No actual generation/ownership/support provider or third-party ROS-client exclusion |
| Observation | Read-only manager observation, real action ACK/result/cancel messages, feedback and mock joint-state samples with timestamps/UUIDs | GenericSystem mirrors software commands. No actual sensor/source freshness, parcel identity, docking, support or separation observation; no validated Authority observation |
| Lifecycle | Actual load/configure inactive, activate, claimed interfaces, deactivate, bridge EOF and manager SIGINT/exit | Manually driven validation process only; product starts dormant diagnostics. No device lifecycle/torque validation, P-authorized lifecycle or Host handover/drop proof |
| Recovery | Bridge instance changes; same goal can be queried from a new bridge while controller remains alive; unowned cancellation is rejected | No controller restart/history-retention test, boot-generation provider, durable Host/P reconciliation in this path, or permission to replay an uncertain invocation |

The [Authority trait and default](../../../runtime/rx-host/src/ros_jtc/profile.rs)
and [adapter guards](../../../runtime/rx-host/src/ros_jtc/adapter.rs) are unchanged:

| Production provider obligation | Existing mechanism | N5 evidence and gap |
|---|---|---|
| Prevent generation reuse | AuthoritySnapshot carries controller_session; adapter compares it to dispatch context | Bridge explicitly reports generation unknown. A fresh bridge UUID identifies only a new client process. No provider derives a non-reused controller generation |
| Observation freshness | Snapshot.validate checks clock health, resources and age+uncertainty against profile maximum age | Default provider supplies no snapshot. Test Auth in Rust stamps an in-memory fixture with the test clock; N5's ROS samples are not wired into that provider. No sensor-age proof |
| Invalidate after protection | Adapter Protection.react first latches its local admission block, then invokes release-owned LocalProtection; guard/submit check that latch | The default provider's external callback is a no-op, but the adapter's local block still exists. No provider supplies and invalidates actual controller/ownership/support state after protection. N5's direct bridge test does not exercise that Host path |

The existing `missing_authority_or_changed_artifact_never_dispatches` test is a
real execution of the Rust guard with a fake transport; it is not a real-controller
Host run. [baseline.json](baseline.json) records this distinction and the 9 passed,
1 ignored macOS tests. Bridge IPC success therefore never becomes a claim that
Host admission passed. The [product JTC package boundary](../../../runtime/rx-host/JTC_PACKAGE.md)
still requires the missing execution Authority/lifecycle provider.

## N4 comparison by evidence kind

Compare [N4's command trace](../../../examples/process/n1-handover/README.md), not
individual N1 operations. `place`, `receiver-hold`, `sender-release` and `withdraw`
remain unexecuted; SIM-JTC-6DOF is a separate profile-level container experiment.

| Evidence kind | N4 | N5: real controller software + mock hardware |
|---|---|---|
| Service request / P admission | Synthetic Run and Operation inputs; no actual P transaction | Still absent; direct bridge IPC is not ADMITTED |
| Send boundary / goal acceptance | Synthetic sent()/operation state | Actual bridge entry timestamp, selected UUID and ROS SendGoal accepted response; bridge memory only, no durable Host journal |
| Native result retrieval | Injected SUCCEEDED or UNRESOLVED plus synthetic evidence IDs | Actual GetResult status/code for that controller goal, including lookup across bridge restart; no P outcome aggregation |
| Cancellation | No N1 native cancel performed | Actual exact-UUID CancelGoal response and separate CANCELED result |
| Current controller observation | No controller in N4 execution | Actual manager type/activation/claims and JTC feedback from GenericSystem |
| Material support, command release, ordinary receipt, service completion | Explicit outside-graph NOT_EVALUATED records | Still not evaluated: mock joint state provides none of these N1 facts |

## Input-topic measurement

The main measurement is **N5: 18 concrete container input topics, 59 unfilled or
partial, no placeholder-only inputs**. Values may be measured configuration or
software-test evidence; they are not physical qualification or a complete
BindingProfile. [measurement.json](measurement.json) gives source and scope for
every counted ID against [doc21's fixed 77 topics](https://github.com/jack0682/rx_docs/blob/3f685f1e81fa11a9598c1aa3acba89222c173b61/docs/21_declaration_reuse_measurement.md).

- N5 IDs: S04, S07, S21, P01, P04, P06, P07, P09, P11, P17, P19, P31, P33,
  P52, P53, P54, P55, P56.
- N4 retained IDs: S07, S13, S20, S21, P20, P33, P39, P40 (8).
- Newly corresponding relative to N4: S04, P01, P04, P06, P07, P09, P11, P17,
  P19, P31, P52, P53, P54, P55, P56 (15).
- Common topic IDs: S07, S21, P33 (3). **Their values and configurations differ**;
  common IDs do not establish equal or reusable values.

As a secondary **cross-artifact candidate-availability** count,
18 + 8 - 3 = **23/77**, leaving 54 without a complete value in this union. This
is not one installed baseline. Some counted values were already available in
older source/catalogs and are selected here; this is not a novelty or labor count.
**The inherited reuse coefficient remains unmeasured.** This follows doc21's
section 6 distinction between a union of available inputs and one valid installation.

Strict exclusions matter: controller claims are not an RX resource_set declaration
(P20); a goal's nominal duration is not RX execution_timeout_ms (P40); package
versions/hashes do not fill source commits (P05). A configured 100 Hz loop is not
an observation-age basis or timing evidence (P36/P42). ROS message/field existence
does not complete all success/failure/cancel semantics or physical stop/release
(P10/P18/P34/P35). This fixture is not a selected physical model, calibration,
facility program, authorized operating procedure or signed Host kind/body package.
Those partial/missing topics are excluded rather than filled with defaults.

```python
import json
from pathlib import Path
m = json.loads(Path("native/ros-jtc/controller_validation/measurement.json").read_text())
n5 = [r["id"] for r in m["n5_filled"]]
n4 = {r["id"] for r in json.loads(Path("examples/process/n1-handover/slots.json").read_text())["slots"]
      if r["status"] == "simulation_value"}
fixed = {f"S{i:02}" for i in range(1,22)} | {f"P{i:02}" for i in range(1,57)}
assert len(n5) == len(set(n5)) == 18 and set(n5) <= fixed
assert set(m["n5_unfilled_or_partial"]) == fixed - set(n5)
assert set(m["n4_filled"]) == n4 and len(n4) == 8
assert set(m["new_relative_to_n4"]) == set(n5) - n4
assert set(m["union_availability"]) == set(n5) | n4
print("N5", len(n5), "N4", len(n4), "new", len(set(n5)-n4),
      "overlap", len(set(n5)&n4), "union", len(set(n5)|n4))
```

## Baseline and interpretation sources

[baseline.json](baseline.json) records the current product image smoke PASS and
16 existing bridge-test cases with the **hand-written simulated ActionServer**.
The full product image built successfully on Docker 29.7.2 / Linux arm64. Its
installation audit checks executable ELF/linking and instantiates no hardware
plugin. The smoke test actively rejects control POST requests and checks that
controller-manager/BT processes are absent; it does not exercise JTC motion.
The product contains JTC/hardware-interface libraries but lacks the manager and
ros2controlcli packages. The separate validation layer fills those test dependencies.

The official [GenericSystem documentation](https://control.ros.org/jazzy/doc/ros2_control/hardware_interface/doc/mock_components_userdoc.html)
describes idealized command-to-state mirroring for offline tests. The
[controller manager interface](https://control.ros.org/jazzy/doc/ros2_control/controller_manager/doc/userdoc.html)
defines lifecycle, robot-description subscription and interface claims; the
[JTC interface](https://control.ros.org/jazzy/doc/ros2_controllers/joint_trajectory_controller/doc/userdoc.html)
distinguishes action goals/results from its topic interface. These explain the
test setup. Actual outcomes, package versions, raw diagnostics and limitations
come from the recorded execution, not the mere presence of those documents.
