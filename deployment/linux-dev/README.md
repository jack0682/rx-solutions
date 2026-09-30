# RX Linux development installation bundle

This installer provides a **FILE_SIMULATION development environment** on a
Linux Docker engine. It includes Platform, Host, the BT Executor, the operator UI,
the supervisor executable, package tools, the runtime Python client, and the
pinned Platform/Solutions source snapshots. It does not install physical drivers,
the full ROBOTIS/ROS development bundle, or a production signing service.
The supervisor is included for development; the validated session runs P, Host
and Executor as separate containers, not under supervisor management.

Prerequisites: Ubuntu/Linux with Python 3.11+ and a running Linux Docker engine.
The installer performs no sudo operations and does not install host prerequisites.
The binary bundle loads included images without network access or a compiler.
The source-only variant additionally requires Git, Docker BuildKit and internet
access for its first build. `bundle.json` identifies which variant is supplied.
The bundle manifest selects amd64 or arm64. Cross-architecture execution needs
Docker emulation; it does not establish performance on a native target PC.

## Install and verify

Extract the archive into a new directory, then run:

```sh
cd rx-linux-dev
./rx-dev doctor
./install.sh
./rx-dev verify
```

Installation loads included images or builds source-pinned development images.
It does not start robots or
automatically start a session. `INSTALLED_NOT_RUNTIME_VERIFIED` means the image
builds completed; only a successful `verify` establishes the reported runtime
scene. In the source-build variant, base images and sources are pinned, but
external APT repositories are not snapshot-pinned. Binary bundles contain exact
image IDs and content hashes so installation does not repeat those downloads.

`verify` creates a new isolated session. It generates fresh test TLS identities
and signing fixtures, compiles a process, uses public review/activation APIs, then
runs the real Platform/Host/Executor against a file device. It tests lost-response
recovery and compares authoritative operation records with independent simulated
effects. Test approval uses separate generated accounts, not independent humans.
Afterwards it requests cooperative shutdown. An incomplete stop is an error,
never a successful physical shutdown claim. UI assets are built and served;
this command exercises the CLI/API and does not run browser interaction tests.

## Interactive session

```sh
./rx-dev demo
# In another terminal, using the session UUID printed above:
./rx-dev status
./rx-dev logs --session SESSION_UUID
./rx-dev stop --session SESSION_UUID
```

`demo` first performs the same verification, then stays in the foreground with
the isolated cell available. Ctrl+C requests the same cooperative stop. Every
invocation creates a new explicitly identified simulation session; it never
reinitializes or resumes an old ledger. Hardware and host device mounts are absent.
The API/UI port is random and bound only to host loopback.

The UI requires the session's test terminal client certificate. Its origin,
account passwords and certificate paths are in `private/final/browser-fixture.json`
inside the printed session directory. These are private development credentials:
do not publish them, import the disposable CA into system-wide trust, or reuse them
on real installations. Use a separate test browser profile with this client
certificate. The existing runtime client can use the generated connection at
`private/runtime-client/connection.json`; invoke `private/installed-client/rx runtime
--connection PATH skills` to inspect approved work.

## Data and limitations

The default state directory is `~/.local/share/rx-linux-dev`; every command accepts
`--state-dir PATH`. Each session retains its evidence, private configuration,
container records and Docker volumes. `stop` never removes data or force-kills a
process. Do not run Docker volume pruning while these records are needed. There
is intentionally no automatic update, cross-version state adoption or factory
reset. Archive the state directory **and** its recorded Docker volumes for a full
development snapshot; an archive alone does not prove runtime-restorable backup.

If the session runner is interrupted by SIGKILL or the host dies, `status` reports
recorded containers; it does not claim their operations finished. `stop` can also
request cooperative shutdown of recorded, ownership-checked containers after
runner loss. Resuming an interrupted operation still requires investigation and
is not automated by this preview.

Bundle checksums detect accidental content changes, not publisher identity. No
production keys are included. The installer preserves a different bundle's state
and refuses to overwrite it; select a new state directory instead.
