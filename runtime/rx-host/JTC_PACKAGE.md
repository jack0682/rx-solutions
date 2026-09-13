# ROS JTC device packages and Host registration

phase63. Authors and validates Template/Site inputs for device catalog configurations using a position JointTrajectoryController as DEVICE_REFERENCE v2 packages, and connects them to Host initialization. The JTC execution provider in the current product factory is not connected. Inspection and journal preparation are available, but `run` fails with `JTC_CONTROL_PROVIDER_NOT_CONFIGURED` before creating a ROS client.

## Authoring inputs and outputs

| Input | Contents |
|---|---|
| Template | Catalog SHA, support ID, controller, logical resource/condition roles, operation slots, joint groups, tool roles, execution/prepare timeouts and Authority observation validity period |
| Site | Template digest, installation/cell/target/site configuration, actual resource/condition names, calibration/tool artifacts, ROS namespace/manager/domain, communication limits and per-slot Goals |
| Recipe | Package name/version/publisher and Linux/Jazzy target |

Template has no site coordinates, namespace or actual resource names. Site must correspond exactly to Template's operation and role lists. Missing/extra slots, duplicate actual resource/condition aliases, incorrect joint ordering and execution timeouts shorter than goal duration are rejected. Semantically irrelevant ordering, such as Template action order and calibration lists, is normalized, while trajectory joint/point ordering is preserved.

Current calibration material requires 1–16 `rx.robot-calibration.v1` artifacts, and each tool role requires a `rx.tool-definition.v1` ArtifactRef. Each artifact is limited to 1 MiB, with at most 32 unique assets overall. Contradictory schemas/sizes for an identical digest are rejected. File bytes/digest/size agreement does not establish physical suitability of the content or completed calibration. Commissioning is incomplete without actual material from the first site.

Trajectory ArtifactRefs are calculated from original per-slot Goals. Current assembly produces the following 7 payloads. All are data; they contain no executables, ROS launch files, environment variables or arbitrary DLL paths.

| File | Role |
|---|---|
| family.json | Model/support ID and environment verified in the catalog |
| profile.json | Actual JTC Profile checked by the Host |
| adapter.json | Release JTC implementation name and source digest |
| authoring/assembly.json | Original Template/Site inputs |
| operations.json | Exact per-slot Intent, time limits and profile/site/tool/calibration/resource references |
| outcomes.json | Native outcome mapping bound to the same profile digest |
| device-catalog.json | phase64 common operation declarations and signed source document references; for import/query by P |

The publication has 9 files including manifest/signature, and total payload size is currently limited to 2 MiB. The decoder still reads phase63 packages with 6 payloads, but they contain no common declaration query data. The total package limit may constrain content before the Profile's individual Goal limit does. Exceeding the limit returns an error requiring splitting or scope redesign; assembly does not omit some goals.

## Authoring and inspection flow

Existing `rx-device-package` commands are reused. Template/assembly schemas distinguish MELSEC from JTC; unknown schemas are rejected. Existing MELSEC APIs and `driver-identity` output are preserved.

```text
rx-device-package driver-identity jtc
rx-device-package template-digest TEMPLATE
rx-device-package assemble TEMPLATE SITE RECIPE CANDIDATE
rx-device-package request CANDIDATE KEY_ID REQUEST_FILE
# An external signer signs the request's original message bytes
rx-device-package seal CANDIDATE SIGNATURE POLICY PACKAGE
rx-device-package inspect PACKAGE POLICY
```

The product CLI has no private-key handling or external transmission. External signing rules follow the [authoring tool](../rx-device-package/README.md). Rereading a candidate also compares the original assembly with the manifest/payloads, so edited intermediate files cannot simply be sealed unchanged.

After common signature, publisher, permission and content hash checks, the inspector interprets immutable bytes owned by the verifier. It reassembles the assembly and compares family/profile/operations/outcomes again. Even a valid signer's signature is rejected if it covers an incorrect model, timeout, outcome table or different release descriptor. The external calibration/tool asset list must match the assembly originals exactly.

The JTC target is Linux/Jazzy. The Host additionally checks the current CPU architecture, base/cell/package ABI, pinned policy and selected manifest digest. The source descriptor is based on build sources including Host/MC sources, shared SDK lock, JTC C++ bridge layer, catalog and native source lock. It is not the same as an actual binary/dynamic library signature or physical controller identity, and does not replace release verification.

## Host configuration, initialization and execution boundary

Specify `JTC_PACKAGE` with package directory/manifest_digest/pinned policy as the Host backend. Only the same installation/cell/environment/condition list is allowed, and each allowed Intent in the Host must match the exact Intent digest in operations.json. A subset of package operations may be selected, but operations with changed semantics such as altered timeouts cannot be inserted.

`rx-hostd drivers jtc` outputs the descriptor expected by the product executable. `inspect` checks content and bindings and shows `control_provider=NOT_CONFIGURED`, activation=false and native_processes_started=0. `init` creates Host journals, native-jtc/native.sqlite3 and journal identity/manifest in a private staging directory, then publishes them atomically. It does not overwrite existing installations.

Builtin currently has no JTC provider. `run` fails because no product Authority/lifecycle/fencing provider exists; it does not create READY, Arm or a new ROS client. Adding `safe=true` or an arbitrary execution path to Site cannot bypass this boundary. This does not mean provider implementation is complete.

A release-owned provider must be connected next. It must verify the actual controller generation, exclusive control authority, independent protection/material support, generation-specific endpoints and lifecycle. It must not infer that the same controller persists from process restart or bridge READY alone. Automation that connects operations/outcomes to actual cell configuration through P package import/review also remains future work.

phase64 connected [P's common operation declaration import, retention and query](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/DEVICE_CATALOG.md). It checks original-source correlation and the current installation/cell/environment, but does not perform manufacturer validation, review approval or actual configuration application. Compatibility between an old image's exact source descriptor and a new package is not inferred.

## Verification and limitations

Tests cover Template reuse/normalization, original→candidate→external test signature→inspection, rejection of signed-but-inconsistent payloads, asset tampering, exact Intent matching, Host metadata initialization and rejection of run without a provider. Container verification uses the actual authoring executable in the same image; the test signer is separate test code on the host. Exact execution results follow the [phase63 record](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase63_checks.json).

This stage does not constitute support acceptance for actual devices, grippers or leader/base/policy paths. Data declarations are not converted into execution authority. The [JTC adapter](ROS_JTC_ADAPTER.md), [outcome mapping](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/NATIVE_OUTCOMES.md), two-image boundary, vendor-neutral support declarations and first physical cell NOT_COMMISSIONED remain in force.

## Transition to a neutral catalog

From 2026-09-14, JTC schema/family/driver identifiers and digest domains use `rx.ros-jtc.*` and `RX-ROS-JTC-*`. The new catalog is an explicit simulation fixture. Packages signed with earlier identifiers are not automatically compatible or reinterpreted. Template/Site digests must be recalculated and package assembly, signing and verification repeated. The current fixture permits only the SIMULATION environment; source evidence and commissioning must be checked separately when adding actual devices.
