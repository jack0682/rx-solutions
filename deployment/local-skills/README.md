# RX local skills — 0.3.0-rc.1 developer preview

Install a local RX service, register a Python skill and execute it without building
Rust or ROS. The server records immutable skill versions, execution identities,
results and unknown outcomes using the RX application, state writer, SQLite store
and Operation model. Python code runs in a separate solutions worker container.

This release supports **LOCAL_SIM Python computations only**. It does not install
or commission the physical Platform/Host/Executor stack, control robots, claim
physical handover, or complete the general skill-to-device integration. Only
register code from trusted authors. Resource limits and container separation are
not a hostile-code sandbox or a multitenant boundary.

## Install, register, use

Prerequisites: Python 3.11+, curl and a running Linux Docker engine. Linux amd64
and arm64 release bundles use the Docker engine architecture. Docker Desktop on
macOS can use the same Linux containers. No administrator access or global Python
package installation is performed by this installer.

```sh
curl -fsSL https://github.com/jack0682/rx-solutions/releases/download/v0.3.0-rc.1/install.sh | sh
~/.local/bin/rx skill add --example add
~/.local/bin/rx run add --input '{"a":2,"b":3}'
~/.local/bin/rx ui
```

The result contains `output: {"sum": 5}` and `operation.outcome: "SUCCEEDED"`.
The dashboard shows registered skills, execution counts, measured duration and
inspectable records. Run `rx token` to obtain the local dashboard token; paste it
into the password field. The dashboard keeps it in page memory only.
Add `~/.local/bin` to PATH to use the short `rx` command. The installer does not
edit shell startup files. Use `--port 8767` for a different loopback port.

```sh
~/.local/bin/rx skill new ./my-skill
# Edit my-skill/skill.py and skill.json.
~/.local/bin/rx skill add ./my-skill
~/.local/bin/rx run my-skill --input @request.json
~/.local/bin/rx skill list
~/.local/bin/rx runs
~/.local/bin/rx result ORIGINAL_REQUEST_UUID
```

A skill implements `main(inputs)` and returns a JSON object. `skill.json` declares
name, immutable numeric three-component version, `environment: "LOCAL_SIM"`,
input/output field maps and a 100–60000 ms deadline. Every declared field is
required; extra top-level fields are rejected. Types: string, integer, number,
boolean, object, array. Nested schemas and third-party Python dependencies are
not supported in this preview. Python's standard library is available.

The code and manifest are transferred as immutable bytes. Editing the same
name/version is rejected; choose a new version and select it with `--version`.
There are no device mounts, host source mounts or Docker sockets in runtime
containers. The worker uses an internal network. Only the server publishes a port,
bound to host 127.0.0.1. Client and worker credentials are distinct.

## Requests, faults and state

The CLI prints and durably saves the request UUID before admission. Reuse
`--request-id UUID` with the exact original inputs after a lost reply; never create
a new ID to retry an unknown execution. A repeated request returns its original
Operation. CLI exit 2 means execution did not reach confirmed success (including
an unresolved or still-running result); inspect the returned state.

Timeout, worker loss and server restart preserve UNKNOWN/UNRESOLVED instead of
automatically replaying work. Captured worker result receipts survive transport
loss and are retried unchanged. A result returned after server continuity loss is
not used to silently clear the quarantine. Such runs remain inspectable; this
preview does not include a human reconciliation/clearance workflow.

This is a serial executor. Worker-measured duration excludes queue wait; wall-clock
timestamps are diagnostic and not safety timing. Successful computation does not
assert physical resource release. History is bounded to 128 skill versions and
1000 runs; reaching a limit refuses new records rather than deleting old ones.

```sh
~/.local/bin/rx status
~/.local/bin/rx logs
~/.local/bin/rx down
~/.local/bin/rx up
```

`down` preserves all Docker volumes and local request records. Reinstalling the
same version reuses the same installation. Other versions require an explicit
migration; no automatic database adoption or reset occurs. This preview does not
provide an automated uninstaller or a destructive reset command.

Local credentials, installation identity and saved client requests are stored in
`~/.local/share/rx-skills`; the authoritative server records and worker receipt
journal are in distinct installation-labelled Docker volumes. Runtime files are
under `~/.local/lib/rx-skills/VERSION/ARCH`. Keep these together when backing up an
installation. `RX_SKILLS_HOME` selects a separate installation directory.

## Release and validation

The GitHub release provides `install.sh`, architecture-specific bundles,
`CHECKSUMS.sha256`, a signed checksum file, source revisions and acceptance results.
The installer verifies bundle and image checksums and the loaded image ID. The
checksum download uses the pinned GitHub HTTPS release; it is not an independent
publisher-key trust system. The optional checksum signature can be verified with
the publisher's separately trusted OpenPGP key.

Bundle creation uses `tools/build_skill_release.py`; acceptance uses
`tools/test_skill_install.py` from the source checkout. Tests cover a fresh
installation, independently authored Python code, idempotency, discarded admission
responses, schema failures, timeouts, process death, persistence and physical-scope
refusal. Acceptance is specific to these local software computations.
