# RX executor client and Frame boundary

The current implementation provides authenticated restoration/current-state reads from P, C++ BT Frames and a [durable request journal and finite-operation worker](JOURNAL_AND_WORKER.md). [Branch/wait, checkpoint commit and observation recovery](DECISIONS_AND_RECOVERY.md) are also connected. The complete daemon/part coordinator and intervention/restart coordination remain future work.

- Client.connect uses the deployment-specified TLS endpoint/CA, service credentials, installation/store generation/release/shared clock/cell definition. It does not use browser sessions. Each actual process start must issue a new peer_boot; the same boot may be retained only for transport reconnects within the same process.
- Client.restore combines frozen GetRun with run-scoped artifact queries to recover original activation/slot/intent. Historical EXECUTING is not used as current authority.
- Client.snapshot validates the optional executor-read binding hash, canonical payload hash/size/schema, P identity/order/clock, actual resolved process and shared validation. It does not import P's internal DB or rx-application.
- Only the client can construct ValidatedSnapshot, whose internal data is read-only. Frame is compared with an explicitly selected Context identity. New epochs/sessions/digests are not automatically adopted.
- Clock is a trusted same-host adapter. LinuxBoottime uses the kernel boot ID and CLOCK_BOOTTIME. The clock interval before/after the request, P absolute expiry and local request-send deadline are checked together.
- Frame is data for proposing RPC requests. It is not a native permit and does not replace P/H gates. SourceDeadline is also passed to C++ to prevent stale Frame use after suspend/clock changes.

`rx-executor-read-fixture` is test-harness-only. It operates only with an explicit loopback simulation clock and performs no native/work submission. A new peer connection may revoke previous sessions/authority in P. Fixture results contain checkpoint/snapshot/resolved/Frame/XML generated through an actual TLS connection to P.

The current S client's restoration material is a shared DTO, not a writer granting execution permission. Payload bytes are not signatures; trust comes from the pinned endpoint, current authentication/cell authority and structural validation. The artifact API offers neither generic URL fetching nor arbitrary file access.

C++ decoder/Context and SourceDeadline requirements follow the [native executor](../../native/executor/FRAME_BOUNDARY.md). Linux checks and mock-clock tests must not be broadened into physical robot performance or site qualification claims.

The [persistent BT engine and Rust private-pipe/request queue](../../native/executor/PERSISTENT_ENGINE.md) are connected. A prepared tree is repeatedly processed in one process, and pending requests are retained through their results. The complete daemon/supervision, part coordinator and durable shutdown intent remain future work. P gRPC channels have separate 2-second connect and RPC limits; transport timeout is not used as evidence of non-application.

The [Run/visit execution service and retained stop intent](SERVICE_LIFECYCLE.md) are connected. They provide a Linux CLI, assignment waiting, continuous processing, communication grace, a separate stop journal and resume blocking after restart. Full deployment supervision, part coordination and explicit restart/rebind of the same run remain future work.

[Serial material coordination](PRODUCTION_COORDINATOR.md) is connected as the service's default mode. P validates material admission/completion, and original IDs and budget consumption are preserved through response loss. ManualVisit remains available through separate configuration.

[Offline recovery inspection](RECOVERY_INSPECT.md), which neither creates/changes existing service journals nor connects to the network, is available through `cell recovery-inspect CONFIG`. It preserves original PENDING/ATTENTION, attachment and request records without current P queries or operating resumption.
