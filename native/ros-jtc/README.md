# ROS JTC ROS integration layer

2026-09-12. `rx-ros-jtc-bridge` selects a position JointTrajectoryController from the ROS support table and sends ROS 2 action goals, queries results, and cancels exact goals. C++/ROS dependencies belong in `rx-solutions`. ROS is not added to the Platform core.

**This is currently a communication bridge.** Its executable is included in the product image, and phase61 connected the [Rust NativeAdapter and durable ledger](../../runtime/rx-host/ROS_JTC_ADAPTER.md) at library level. The product startup factory and actual Authority provider are not yet connected. It does not run at default startup, and this does not claim that the Host's durable native journal, qualification, grant/permit, handover, or normal shutdown authority currently extend to this bridge. It also has no code that starts or stops physical ROS devices, device drivers, or controller_manager.

## 1. Default model and controller selection

The [default support table](../../catalogs/device-support.v1.json) is included at build time, and its source SHA-256 is checked against configuration. The support ID/model/controller type and joint order are selected from that table. Configuration cannot arbitrarily override the joint list or plugin type.

The current default catalog contains newly written simulation declarations. It permits the two position JTC profiles `SIM-JTC-6DOF` and `SIM-JTC-7DOF`; leader, impedance, and gripper-only actions are rejection-test cases. It inherits no observation or validation evidence for actual vendor models.

Among MANIPULATOR/FOLLOWER/MOBILE_BASE roles, only plugin `joint_trajectory_controller/JointTrajectoryController` with a position command interface is accepted. Selecting a controller declaration does not grant execution authority, and the Host rejects using SIMULATION_FIXTURE for a physical device environment.

## 2. Startup configuration

```json
{
  "schema": "rx.ros-jtc-bridge.v1",
  "catalog_sha256": "<current catalog file SHA-256>",
  "support_id": "SIM-JTC-6DOF",
  "controller": "arm_controller",
  "namespace": "/cell_robot",
  "controller_manager": "/cell_robot/controller_manager",
  "domain_id": 171,
  "timeout_ms": 150,
  "capacity": 32
}
```

This is a format example, not actual site values. Replace the angle-bracketed catalog hash. The ROS domain is explicit, and namespace/manager are absolute ROS names. The action name is generated as namespace + selected controller + `/follow_joint_trajectory`. Commands do not accept topic remapping, arbitrary service names, or ROS global arguments. No parameter service/event publisher is opened.

Startup only creates ROS clients. It does not invoke hardware plugins, driver configure/activate, controller switches, init_position, or torque enable/disable. ROS discovery/network configuration is not itself authority validation; ROS peer/network access control for field deployment remains future work. Tests use explicit Fast DDS/localhost settings and a network-none container.

## 3. IPC with the parent

IPC uses private stdin/stdout pipes with one JSON object per line. Input is limited to 1MiB and depth 32; duplicate keys, extra/missing fields, and invalid UUIDs/counters are rejected. Stdout contains reply JSON; stderr contains diagnostics. At READY, the parent receives the bridge instance UUID and Linux boot-based clock ID/ticks. The UUID changes for each bridge process.

Each request contains schema, bridge_instance, an increasing sequence (decimal string), expires_at_ns (an absolute value on the same CLOCK_BOOTTIME), command, and body. The maximum admission window is 1 second. ROS waits are limited to the smaller of the configured timeout (at most 1 second) and the remaining request time. This is a software timing boundary, not a guarantee of hard real-time behavior or actual motor stop time.

| command | body | Meaning |
|---|---|---|
| inspect | Empty object | Observe controller state, exact claimed interfaces, and service availability |
| send | operation UUID, invocation UUID, goal | Send a new finite trajectory goal consistent with the catalog |
| result | invocation UUID | Query only the result for that ROS goal UUID |
| cancel | invocation UUID | Request cancellation only of that single UUID known to this bridge |

Results contain bridge_instance, sequence, clock_id/ticks_ns, state, value, and fault. `REJECTED` means rejection of the IPC request, not evidence that an earlier native operation did not execute. If a read RPC provides no response, the result is `RPC_UNKNOWN`; it is not converted into native failure/completion.

## 4. Trajectory representation and preflight

A goal contains exactly joints, points, path_tolerance, goal_tolerance, and goal_time_ns. Points contain positions/velocities/accelerations/time_ns. The joint list and order must match the complete catalog declaration; partial goals are not accepted. The position array covers every joint; velocity/acceleration arrays are either complete or empty. Values must be finite with absolute magnitude at most 1e6; this numerical bound is not a mechanical joint limit.

There may be at most 1024 points; time_from_start must be greater than 0 and strictly increasing, with the final time no greater than 1 hour. Per-joint path/goal tolerances explicitly include name and position/velocity/acceleration and accept only positive values. ROS tolerances of 0 (default) or -1 (disabled) are not used at this stage. goal_time_tolerance must be greater than 0 and at most 60 seconds. All units, joint limits, velocity/acceleration, collision, mechanical, and calibration checks must be added in the actual Profile/Envelope.

Immediately before sending, ListControllers verifies that the selected controller is active and that its type and set of position claimed interfaces match exactly. A controller with more joints does not pass on the basis of a partial match. Chained controllers are rejected.

This query is not evidence of the controller server's boot identity or cryptographic proof that it is the same action server. It also does not prove anything about goals from other ROS clients or actual material support. The response explicitly states `controller_generation_known=false` and `physical_readiness_proven=false`.

## 5. Admission, results, and cancellation

ROS actions use a client-selected UUID for each goal, and acceptance and terminal results are separate responses. The result cache depends on server configuration/lifetime. A cancellation response is also distinct from terminal canceled state. [ROS 2 action design](https://design.ros2.org/articles/actions.html).

The bridge passes the caller's invocation UUID unchanged into SendGoal. It does not let the action client generate another arbitrary UUID and report it afterward. Before the send boundary, operation/invocation/body are recorded in process memory; repeating the same invocation/body returns the original receipt. A different body or another invocation for the same operation is rejected.

If this bridge knows an unresolved goal, it does not preempt it with a new goal. Stored goals cannot exceed the configured capacity (at most 512), and total body storage cannot exceed 16MiB. **These records are not durable.** The external Host must first durably store SEND_ENTERED and the original UUID. After a bridge restart, receiving a new READY does not permit resending an earlier operation. The current durable Host integration scope follows the NativeAdapter specification above. Production controller authority and factory integration remain future work.

A lost/timed-out SendGoal response remains SEND_UNKNOWN and is not automatically resent. A result for the same UUID may be queried later, but the admission fault latch is not automatically cleared. There is currently no explicit latch-clear/recovery command. Restarting the bridge must not bypass the block; a new start must be decided through the future Host's durable ledger and recovery/revalidation procedure. Ordinary results retain both the ROS goal status and controller error_code/error_string. Status UNKNOWN with error_code0 is not interpreted as success; SUCCEEDED accompanied by an error code also preserves both facts. Deciding the global Outcome is not this bridge's role. The actual receipt time of ACK/result/cancel is stored as captured_at_ns in value; cached responses retain the original timestamp. The outer reply ticks_ns is the response transmission time and must not be used as the time of a new device observation.

Cancellation sends only a known nonzero UUID with timestamp0. Cancel-all and bulk cancellation up to an earlier timestamp cannot be expressed. Responses containing another goal UUID are also rejected. Cancellation records for the same goal remain in memory; repeated cancellations are not resent. CANCEL_UNKNOWN is also preserved and latches new sends. `terminal_stop_proven` in a cancellation acceptance response is false, and a separate result is required. A terminal action result is still not evidence of physical stopping or material handover.

Timed-out rclcpp pending requests are removed so that repeated result queries do not keep accumulating unfinished futures. This removal does not cancel device effects already in progress.

## 6. Shutdown and subsequent integration

Stdin EOF closes clients/context. It does not automatically cancel goals, stop controllers, or turn off torque. This process does not own device/robot drivers and therefore does not invoke their destructors. Goals already in progress may continue in the controller. Parent loss, SIGTERM, and forced termination must not be interpreted as machine stopping or normal handover.

The current Rust NativeAdapter connects the durable journal and artifact/dispatch context. Remaining work includes production controller generation and authority providers, package/factory and qualification/permit configuration, actual observations/final error and handover/drop proof, and normal shutdown/parent-loss handling. Gripper, leader, base, and policy paths and physical validation for each actual device also remain outstanding.

## 7. Validation

Tests use a simulated rclpy ActionServer and actual C++ ROS service/action clients. They check rejection of joints/timing/tolerances/extra fields, inactive or incorrect claim sets, specified UUIDs and duplicates/races, result UNKNOWN/abort/contradictions, exact cancellation and separation of cancellation responses from terminal state, preflight deadlines, IPC generations/sequences/duplicate keys, lost responses, and late facts. Startup selection and joint sets for the two current simulation JTC declarations are compared without sending goals at this stage.

These tests do not operate actual ROS drivers or physical devices. The C++ bridge is included in the product S image, but default process management mode does not start it automatically. Original earlier validation records remain in Git history. Tests of the new neutral fixture and image are confirmed through separate execution results.
