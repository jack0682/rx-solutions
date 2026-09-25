# DHI foreign component ownership, PTY simulation only

`rx/dhi-pty-simulation` is declared as a release-owned recipe in the existing
supervisor catalog. Its direct-component path runs the original pinned
`dynamixel_hardware_interface` plugin in the actual ROS 2 controller_manager
against a fresh device-side PTY register model. Registered release admission for
this revision is not verified and remains authority-gated. This is not physical
DYNAMIXEL support, an operating-area work permission, exclusive ROS
administration, or a general supervisor adapter.

The three ownership layers are separate:

- DHI authors command bytes on the PTY slave (L1).
- controller_manager owns hardware/controller lifecycle and interfaces (L2).
- The existing supervisor owns the Python session, the session owns the native
  custody guardian, and the guardian owns controller_manager (L3, one parent per
  process). There is no Compose/s6 integration claim.

RX holds descriptor custody and mediates `openat` with a private Linux seccomp
notification listener. Source is unchanged upstream; syscall semantics are
changed: an eligible open receives an existing FD through atomic ADDFD/SEND.
The keeper never writes command bytes. A held child/pidfd, kernel notification
TID/TGID and executable identity bind grants; caller-supplied owner names do not.
The measured single-plugin lifecycle needs exactly two grants, for initial open
and the SDK baud-rate close/reopen. Extra grants are refused. The manager cannot
clear TIOCEXCL through its inherited filter.

Allocation starts with a locked fresh ptmx. RX sets slave mode0 before unlocking,
obtains the peer FD with TIOCGPTPEER, sets TIOCEXCL, then restores mode600. Ordinary
opens encounter EIO/EACCES during creation and EBUSY after custody. The completed
private master/pidfd handoff happens only after setup; there is no `openpty`
fallback. Existing tty adoption, physical paths and owner PIDs are not catalog
arguments. The recipe accepts only a typed port, in a Simulation plan with
restart0. Unsupported Linux/kernel features fail by name, without an unfenced
fallback. The guardian checks effective, permitted and inheritable capabilities with
`capget`, requires all three sets empty, and sets no-new-privileges before
allocation. This is an RX admission check, not an inference from container
options. Privileged outside actors remain outside the trusted-OS boundary. The process namespace is derived from its supervisor instance.

## Effect classification remains withheld

The competing-description counterexample disproved path-based containment: an
ordinary ROS description directed the unchanged plugin to another PTY before a
late startup refusal. R1 now classifies the kernel-resolved character resource
for every supported open surface. A non-custody character device is refused
before open; path spelling and ROS publisher identity do not grant authority.
P5 replayed that same external-first scene and observed zero new outside
requests/writes, resource refusal before the first grant, and exactly two
legitimate grants for initial open and baud reopen.

P6 covered absolute, symlink, relative/current-directory, `/proc/self/fd`,
dirfd-relative and `openat2` spellings. `open_by_handle_at`, io_uring setup and
pidfd descriptor import are refused by name. The complete channel and syscall
ledger is [endpoint-channels.json](endpoint-channels.json); entries not reachable
from the pinned component remain explicitly unsupported.

P8 observed actual manager threads continuing while a late process fork was
refused. Removing only that decision allowed a child to inherit the granted FD
and write one byte. Granted descriptors are also forced `FD_CLOEXEC`. The older
TGID membership check is secondary for forked new-open claims after this gate;
it is not reported as a separate proof of fork safety.

These direct-component observations support reconsidering `NonActuating` only
for the internally allocated fresh PTY model, but they do not authenticate the
registered catalog revision. The development release signing key was destroyed
after its original offline use, while signing custody and root rotation remain
unestablished. The catalog therefore retains `RequiresPlatformAuthority` and
reports `registered_release_admission_verified: false`; key re-establishment,
root rotation, SDK regeneration, old/new release rejection tests and revocation
tests belong in a separate release-governance slice. This classification must
not be inferred merely from simulation or copied to a real-device recipe. The
resource classifier assumes the trusted installed filesystem is stable between
stat and continuation; same-UID filesystem/ptrace mutation remains outside the
stated trusted-code/OS boundary.

The direct P5-P9 tools instantiate `session.py` against the image's actual
controller manager, unchanged DHI plugin, SDK, model and custody guardian. They
exercise the descriptor/resource mechanism before device I/O. They diverge from
registered operation before `rx-solutionsd` loads and verifies `release.json`,
constructs the release-owned program catalog, validates a resident plan and
spawns the program through the supervisor. Direct evidence therefore supports
the custody mechanism only; it does not establish signed catalog admission,
resident lifecycle integration, or release-origin acceptance for this revision.

If a later authenticated release classifies this recipe as `NonActuating`,
`OwnedNonActuatingExit` may settle the supervisor's owned OS Child only. It must
not clear the component's independently emitted residual, request/effect distinction,
or `stop_effect: UNCONFIRMED`. Every observed torque transition, diagnostic
response and session/model retirement emits an instance-bound timestamped record
into the supervisor's existing stdout log. Neither successful process exit nor
an upstream SUCCESS/Already enabled response rewrites those records. The model
can be retired while its last torque value is1, and that residual remains in the
log. The supervisor exposes `unconfirmed_component_stops`, keeps
`reconciliation_required: true`, and refuses an overall successful shutdown with
`DHI_MODEL_STOP_UNCONFIRMED` even when its owned OS child exited0. The lifecycle
test establishes that conditional behavior; it does not change the current
`RequiresPlatformAuthority` catalog gate. Component uncertainty would take
priority over the aggregate success verdict, and that future classification
must not be copied to a real-device recipe.

The upstream torque service accepts a request; actual writes occur elsewhere.
In the measured inactive case it replied success/Already enabled while register64
was0. Activation later changed it to1. Upstream `stop` does not check the return
of its disable operation. RX therefore reports the request reply separately from
the device-side model observation and always leaves stop effect UNCONFIRMED.

## Entry and boundaries

Select `rx/dhi-pty-simulation` with `parameters: {"port":"18089"}` in the existing
startup plan. The script requires the supervisor's instance environment and binds
HTTP only to127.0.0.1. `GET /status` and `/health` are diagnostic self-reports,
not functional readiness or work permission. `POST /diagnostic` accepts only:

- `{"action":"torque","enable":true}` (or false)
- `{"action":"activate"}`, `{"action":"deactivate"}`
- `{"action":"stop"}`: requests inactive hardware; it does not terminate the
  manager or certify an effect.

Use the supervisor's normal cooperative shutdown to end the session. Guardian
loss becomes ATTENTION and subsequent requests are refused. `restart` returns
`DHI_PREVIOUS_ENDPOINT_RETAINED`; an unproven owner claim returns
`DHI_OWNER_CLAIM_UNATTESTED`. They are different conditions. Recovery never adopts
an endpoint from a saved pathname. A new explicitly admitted session allocates
its own endpoint and cannot recover the original model's outcome.

This is not a sandbox against arbitrary installed same-UID code or privileged
OS tampering. The machine-readable limit is
`OUTSIDE_TRUST_BOUNDARY_CHMOD_PTRACE`. Serial exclusivity does not authenticate
ROS administrative clients. The trust boundary remains installed Rust and OS;
source assets match compiled content pins and runtime binaries match the existing
signed release inventory. Modified guardian bytes are a release content failure,
not a reason to skip custody. Dependency preparation checks exact archive hashes
and commits, and the image build rechecks source-content tree digests derived from the
approved original archives, not from the adjacent mutable source index;
upstream floating `main` references are not imported.

## Validation status

The development probes exercise actual Linux processes and an RX-owned PTY
simulation. They do not qualify physical devices. The release-image and independent
acceptance evidence are recorded separately; source presence or package builds do
not establish successful integration. `sdk_baseline_complete` and
`robotis_bundle_complete` remain false; ai_worker, ai_sapiens and open_manipulator
remain separate required follow-up integrations.
