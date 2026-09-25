# DYNAMIXEL protocol 2 Ping simulation

The product Host implements exactly one previously reserved `VALIDATED_DRIVER` profile:
`rx/dynamixel-protocol2-ping-simulation-v1`, with endpoint `simulation/dynamixel/id-1`.
`rx-hostd drivers dynamixel` prints its source identity, exact artifact bodies and scope.
Every other profile/source identity or endpoint is refused before opening a device.
In particular, `/dev/ttyUSB0` produces `DXL_REAL_ENDPOINT_UNSUPPORTED`.

This is real Linux execution against a **fictional simulated transport/model**, using the
unmodified official DYNAMIXEL SDK 4.1.0 protocol 2 packet implementation. It is not a physical
DYNAMIXEL connection, equipment qualification, motion capability or safety assessment.
The RX model number is 65500, device ID 1, firmware 1, with fixed 250ms modeled response
latency. It is not a claim that a ROBOTIS model has these characteristics.

## Execution and ownership

Installed rxclcpp/rxclpy submit through `Session.Open`, `Cell.Open`, and
`Cell.SubmitOperation`. Neither library contains this driver, a serial implementation,
helper executable, device lock or authorization policy. Base `Operation.Submit` is not
used. G3/G4 supervisor work-use has no gRPC ingress and is not proxied by these clients.

The Host checks the exact finite-action target, profile digest, program and parameter
ArtifactRefs, completion rule and cancellation declaration. Arbitrary finite programs
cannot be interpreted as Ping. Existing cell admission, Host grants/permits and durable
SEND_ENTERED remain ahead of native dispatch. Passive inspect/init/open/readiness do not
Ping. Software readiness is not equipment readiness or permission to perform work.

One native instance UUID is created inside the Host's unpublished installation staging
directory and bound in both `installation.json` and the native SQLite metadata. It is
subordinate to this local Host installation and its journals. Equal model bytes do not
identify the same instance. Native database identity mismatch and a second live writer
are refused. This is not a second F7 supervisor Registry, distributed identity authority,
or detection of wholesale copying/rollback of all installation records.

The Host starts one external `rx-dynamixel-ping` per newly admitted invocation, using an
inherited anonymous Unix socket. There is no listener to which another client can connect.
Linux socket peer credentials and the parent's actual executable inode/device are also
required to reject a caller-created socket and a second independently callable helper.
These controls close different paths; neither is an optional reinforcement. A private
negative-control build disabling only the parent executable comparison allowed an
unauthorized fake-socket caller to complete Ping, and the same refusal test failed.
The unchanged product helper rejects direct exec, caller-created sockets and forged argv0. The fixed installed parent
is `/opt/rx/bin/rx-hostd`; environment variables and caller claims cannot name another parent.
The check applies to a live parent/channel, not historical PID-only recovery. Parent death
closes the Host channel; Linux parent-death SIGKILL also bounds the helper's lifetime.

This is the existing **trusted installed Rust binaries and OS** boundary, not a sandbox
against root, same-UID ptrace/FD theft, malicious recompilation or replacing the trusted
installation. A separately compiled/copied model does not address the selected instance.
The helper links no Linux/macOS/Windows serial PortHandler; its transport accepts only the
exact ID1 protocol2 Ping frame and exposes no register-write, torque or motion entrypoint.

## Content, evidence and recovery

The build source digest covers Host source, helper source, upstream pins, build inputs and
SDK source-lock. The helper executable and descriptor must also appear in the G2 signed
release inventory authenticated by the existing compiled development root. The Host
verifies that inventory and its contents, then rechecks the fixed helper byte pin before
spawn. No configurable executable path or self-signed helper identity is accepted.
Trust in installation stability between the byte check and fixed-path execution remains
explicit; offline revocation freshness and whole-state rollback detection are unchanged.

The native journal records the invocation **before** starting the helper. The helper itself
writes an invocation audit entry to an inherited file before calling the SDK. A confirmed
capture contains the actual SDK result, model and TX/RX bytes. Host capture is a native
fact; the platform applies the reviewed completion rule. The artifact is historical
observation, never current work permission or physical proof.

A duplicate receipt lookup never starts a helper. Missing native capture remains UNKNOWN;
either another lookup or a Host restart cannot silently Ping again. Pending uncertainty
refuses new dispatch and release/handover. Killing a helper or closing a client does not
prove an operation succeeded or was canceled. Physical stopping is not claimed.

## Compatibility and remaining work

The previously unsupported `VALIDATED_DRIVER` configuration now requires `endpoint`.
Previously supported backend configuration and protobuf wire contracts are unchanged.
The new adapter adds an additive `DYNAMIXEL` native-installation arm and its own journal
namespace; existing records are not migrated or rewritten. Unknown profiles remain refused;
this does not implement a generic driver registry or dynamic plugin loader.

The image installs the helper, upstream license and descriptor. Build it through
`docker/Solutions.Dockerfile`; its inventory still needs the existing external development
release signing step before this adapter can be selected. Actual product custody and
physical deployment remain separate approvals and validation.

`tools/test_dynamixel_helper.py` exercises direct Python/C++ caller and endpoint refusals.
The platform's `tools/test_clients.py --adapter-descriptor <drivers-output>` uses explicit
S materials before signing/compilation and actual public commissioning. Its external OS
observer kills the helper during modeled response latency for the UNKNOWN scene; there
is no product fault-injection argument. Installed language consumers test lost receipt,
same-request replay, cross-language replay on the same live Host instance and later queries.
These tests do not establish an entire material workflow, physical release, completed G5 SDK
baseline or completed five-product ROBOTIS bundle.
