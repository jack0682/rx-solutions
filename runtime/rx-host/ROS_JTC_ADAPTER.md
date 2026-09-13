# ROS JTC NativeAdapter — durable work, control authority and bridge ownership

2026-09-12. `ros_jtc::Jtc` connects to the existing Host NativeAdapter. The actual Rust Host → private pipe → C++ ROS bridge → simulated ActionServer path is implemented. **Product startup factory registration and an actual controller authority provider are still incomplete.** This library must not be promoted to operating suitability for the first site.

## 1. Components and responsibilities

| Component | Responsibility |
|---|---|
| Profile / TrajectoryAsset | Model/controller and joint ordering matching the support table, original trajectory byte hash/size, site/calibration/resource and Intent binding |
| Process | Release-owned executable pin, private directory copy, config, bounded stdin/stdout, bridge instance/clock/sequence validation and child reaping |
| Jtc native journal | Pre-send operation/invocation/intent/artifact/controller-session records, retention of ACK/result/capture/disputes and prevention of re-execution |
| Authority | Independent evidence of current controller generation, exclusive control authority, external goals, conditions, support and client shutdown permission |
| Host | Current qualification/epoch/grant/permit, final device generation/expiry checks, delivery/evidence and global reconciliation |

Authority is a release-owned Rust trait, not a `safe=true` field in site JSON. The default `UnavailableAuthority` provides no snapshot and rejects actions. At this stage, the Authority implementation is a test fixture owning the simulated controller. An actual provider must implement prevention of generation reuse, observation freshness and state revocation after protection requests.

## 2. Profiles and original goals

Only FiniteAction/Trajectory is currently accepted. Intent target/profile/site/calibration/resource, trajectory ArtifactRef/joint_group/tool and completion/cancellation rules must match exactly. Goal canonical bytes must match the declared SHA-256/size, and the artifact schema is `rx.ros-jtc.goal.v1`. An altered trajectory must not be sent under the same reference.

Joint, point, finite-value and time/tolerance validation is aligned with the [ROS bridge specification](../../native/ros-jtc/README.md). Limits are 512 KiB per goal and 16 trajectories per profile. These are input/resource limits, not validation of actual robot geometry, joint limits, collisions or calibration. Actual Profile/Envelope qualification must separately verify suitability.

## 3. Bridge process and communication

The executable is supplied by release composition, not by an operation or site JSON. The original regular file is acquired within a 16 MiB limit, hash-checked, copied to an owner-only temporary directory and executed. Subsequent changes to the original file do not change the already selected executable bytes. Config is also written in that directory. It runs with one fixed argument without a shell.

Environment variables are limited to an allowlist of ROS/library paths and discovery/log settings; the default environment is cleared. This environment is also trusted release input. A production resolver checking the actual library-path supply chain and ROS peer security policy remains future work.

Unix nonblocking pipes and poll bound transmission and reception. Replies/commands are limited to 1 MiB, with exact schema, bridge instance, sequence and current boot-clock validation. Partial lines, oversized content, different instances and I/O timeouts fault the pipe without automatic restart/retransmission. stdout and stdin are exclusive to this child.

Because the copied executable is run, the runtime directory's filesystem must permit execution. Linux tests use `--tmpfs /tmp:rw,exec`. The initial noexec tmpfs test failed with execution denied; no fallback was added to bypass this by executing the original file. Actual deployment must explicitly define the private runtime mount policy.

## 4. Controller generation and authorization validity

ROS bridge instance and controller session are different. The same controller may survive bridge restart, or the controller may change while the bridge remains. NativeCapture device_session uses the controller session verified by Authority. The bridge's `controller_generation_known=false` is not converted into controller identity evidence.

The guard checks an independent Authority snapshot and current ROS controller/type/claim/service state. Controller sessions before and after those checks must match. It also checks readiness, conditions and observation age.

The Host passes the last guard's device_session and **the earlier of permit expiry and guard expiry** in NativeDispatch. Jtc rechecks that session/expiry even after journal commit and passes it through expires_at_ns in the C++ send request. If intermediate queries/storage take too long or the generation changes, no goal is sent. The default implementation rejects physical adapters that do not handle this context. Existing Melsec also connects the same context to its final readiness check.

Current device-session checks end at Rust's final Authority snapshot. C++/standard ROS SendGoal accepts no field for verifying controller session. This code alone is not claimed to prevent controller replacement between that check and actual ROS reception. A production Authority/manager must close and validate this gap through generation-nonreusing endpoints, lifecycle/fencing or equivalent measures. Until then, actual device support is incomplete.

This is a dispatch admission boundary. It does not mean a trajectory already received by the controller physically stops at expiry; actual stop/hold conditions are separate.

## 5. Native journal and restart

A native journal is initialized only in a new directory, with existing identity/profile pinned. open rejects missing/empty DBs, different journals/profiles, count/index inconsistencies and invalid captures. Host and native journals are separate and do not form a shared transaction with the device.

Before transmission, operation/invocation, complete Intent and digest, profile/goal digest, controller session, bridge instance and entry time are recorded atomically. One native pending slot prevents bypass using another operation ID. A different body under the same ID conflicts; repeated submit of the same request returns only its original record.

ACK is retained as an acceptance fact. Acceptance alone does not record success in Host evidence. Rejection by the action server is a separate `rx.ros-jtc.goal-rejected.v1` native fact. Completion results distinguish succeeded/canceled/aborted schemas and controller codes. Contradictory ROS success and nonzero controller error preserve the raw reply and dispute without creating a capture. A later clean result does not automatically overwrite an existing dispute.

After reply loss or restart, only the original invocation's result is queried. A new result is connected only if Authority confirms the current controller session matches the original record. A different generation retains None/unresolved state. An already durable capture is retrieved with its original session and receive time. No new send is created to infer a result.

The current native journal has a 10,000-entry limit and no retention/cleanup or manual dispute recovery API. No mechanism is provided to bypass unresolved/disputed work using a new ID or restart.

## 6. Handover and normal shutdown

Action completion is not support or shutdown permission. Handover returns absence of pending work and independent control/support evidence. Normal shutdown also requires no native pending work, no-external-goals, support-stable and client-drop-allowed. Unresolved or disputed entries retain ownership without starting child close. A separate recovery/responsibility handover path does not yet exist.

Call `prepare_shutdown` after closing Host admission. When conditions are met, initiate bridge stdin EOF and repeatedly check actual child exit. If the child is still alive, return Busy and retain the owner. A timeout alone does not kill it. `shutdown_snapshot.safe_to_drop` is true only after child exit and while current Authority conditions still match.

Unexpected adapter destruction invokes an independent protection callback. Process closes pipes and hands the child and executable directory to a reaper; it does not automatically send goal cancel, driver stop or torque off. Neither the protection callback nor child exit itself is interpreted as physical stop. Machinery behavior after forced termination of the entire parent or power loss requires separate validation.

## 7. Verification scope and next connections

Tests cover executable pins/original changes, pipe timeout/oversize/instance mismatch, missing journals/capture corruption, unresolved restart/new-ID bypass rejection, controller generation/control authority/support/child exit and preservation of contradictory results. Separate child processes are terminated after entry/send/capture to check that no additional native effects occur.

Linux integration uses actual SystemClock, a copied-and-executed C++ bridge, actual ROS action/service messages and a simulated controller. It checks Host prepare/authorize→one goal with the original UUID→reconcile/evidence→handover/shutdown. Authority is an explicit fixture here, not proof for an actual robot.

Actual execution results and source/image hashes follow the [phase61 verification record](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase61_checks.json). Production Authority, ROS controller lifecycle/identity, device package authoring/factory registration, durable Host coordination/recovery of native cancel, and calibration/support/acceptance of actual ROS devices remain outstanding. The first physical cell is NOT_COMMISSIONED.

## 8. phase62 · Platform outcome interpretation integration

`Profile::outcome_table()` returns a shared data table bound to the profile digest and completion rule. Supplying this table and success postconditions to the platform's `CompletionRule::NativeOutcomes` allows success/failure/cancellation decisions in P's existing evidence transaction. Actual table generation and native capture interpretation were checked with a simulated adapter.

It distinguishes success schema/code0, canceled schema/known code and aborted schema/known code. Goal rejection is FAILED and is not converted to pre-native-transmission NOT_EXECUTED. Unknown combinations retain only the originals without drawing a conclusion. A resolver that automatically delivers this return value into a signed device package/P configuration remains future work. Calling the generation function alone creates no installation, approval or operating permission.

Exact mappings, compatibility and counterexamples follow the [core outcome specification](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/NATIVE_OUTCOMES.md); actual test scope follows the [phase62 verification record](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase62_checks.json).

phase63 connected [Template/Site package authoring, verification and Host registration](JTC_PACKAGE.md). It recalculates signed assembly to compare profile/operations/outcomes and supports JTC_PACKAGE inspection and journal initialization. The current product factory lacks an execution Authority/lifecycle provider, so run is rejected before creating a ROS client. This advances package authoring/metadata registration among the phase61/62 follow-up items above; it does not complete actual control authority, physical validation or automated P configuration import.
