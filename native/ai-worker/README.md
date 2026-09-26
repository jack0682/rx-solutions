# AI Worker L3 supervision boundary

RX pins AI Worker 2.2.7 at commit
`897ef342ff0e1b2ef59d6afbb5195b410d57b85f`. The upstream deployment has two
distinct supervisors: Docker Compose recreates the container with
`restart: always`, while s6 owns named service process groups inside it. RX does
not collapse those layers into one PID claim.

For this integration, an **L3 owner** is the authority allowed to create a
replacement root process tree for one stable service-generation identity. The
identity is `ai-worker/2.2.7/l3-simulation`; it is not a container ID, PID,
process group, ROS node or device-bus owner. RX holds an exclusive live-owner
lock and writes a durable stop fence before terminating its owned simulation
tree. A Compose incarnation can restart, but the release-owned entry gate sees
the fence, records `L3_FOREIGN_RESTART_AFTER_RX_STOP_REFUSED`, and refuses to
create another process tree.

That result is **blocked and recorded**, not merely observed. The negative
control removes only the stop-fence branch in a separately signed private image:
the real Compose `restart: always` loop then starts generation 2 under owner
`compose`. This demonstrates that the gate is load-bearing. The private mutation
never enters product source.

The simulation does not run ROS, an AI model, a camera or DYNAMIXEL. It exercises
only L3 process-tree ownership. The upstream physical image requests a privileged
container, `SYS_NICE`, `rtprio: 99`, ZED resource/settings mounts and `/dev`.
`l3_guard.py inspect zed` and `inspect rt` therefore refuse missing inputs as
`AI_WORKER_ZED_ASSETS_OR_DEVICE_UNAVAILABLE` and
`AI_WORKER_RT_AUTHORITY_UNAVAILABLE`; source installation cannot hide either
absence.

The upstream Dockerfiles and `.repos` use floating `jazzy`, `main`, base-image
tags and unhashed downloads. [dependencies.json](dependencies.json) replaces the
Git refs with exact commits for the RX source review. Base-image, s6 archive and
ZED installer content digests remain `NOT_ESTABLISHED`, so the upstream physical
image is not a qualified RX release input.

`dhi_compose_s6_reuse` remains `NOT_ESTABLISHED`. This mechanism is not claimed
for OpenMANIPULATOR; its s6/cyclo_manager topology is the next discriminating
test. Physical operation, product signing custody and offline revocation
freshness are not established, and `robotis_bundle_complete` remains false.
