# Host evidence publication

`publication::Publisher` sends the evidence journal already stored by the Host to P. It has no device-call, grant/permit or production retransmission API. Native delivery from P→H is distinct from result retransmission from H→P.

## Storage and transmission

1. Within the same Host gate, read the journal ID, last event sequence, destination's durable ack cursor and next consecutive batch. Use `Repository::journal_head`, which does not construct a full entity snapshot.
2. Complete TLS and peer/base/cell negotiation. Reject unknown required features and responses from a different installation, definition, peer or boot. Honor both the negotiated record/byte limits and fixed upper limits of 128 records/1 MiB.
3. The first probe on a connection checks the current prefix. Subsequent transmission is consecutive from the saved cursor onward. Do not recreate sequence numbers or evidence IDs.
4. Compare the P ack's installation/store generation/view/journal/through and range, then save the cursor to the H DB. If the remote ack arrives but local persistence fails, resend the same evidence and let P deduplicate it.
5. Block ack regression within the same generation, through values beyond the local tail, partial-batch acks and missing/corrupt journals. Manage a new P store generation under a separate cursor key. Do not automatically change the restoration generation itself to match a remote value.

P and H cursors are separate. H's delivery journal sequence and evidence journal sequence are also different. An old ack in the same direction does not move the saved cursor backward. Original evidence is currently retained even after ack; retention/compaction are not implemented.

## Execution lifetime

`tick` is one bounded publication operation. `run` repeats it, using a normal idle check of 100 ms and transient-error backoff from 100 ms to at most 5 seconds. Even without evidence, once 1 second has elapsed since the last successful empty remote Publish probe, it sends an empty batch in the current session to check the session and accepted prefix. This interval uses a separate monotonic time and is not postponed by local Idle ticks. Probe failures do not update the last-success time, so retries do not turn into local Idle success and reset the existing backoff. When evidence is available, existing batch handling takes priority; immediately after connection, the existing empty prefix probe still runs first.

If Publish returns `Unauthenticated`, discard the evidence-path connection and repeat Session.Open, all Cell.Open calls and the prefix probe with the same pinned endpoint, destination, installation/store generation, release, Host boot and evidence journal. timeout/Unavailable back off while retaining the current connection and existing request/cursor semantics. Do not replace pins or journals based on remote responses. This reconnection restores evidence communication; it is not an operating HostRegistration rebind, qualification, Arm, grant, new Run start or native authority recovery.

Negotiation mismatches, loss of integrity/continuity and similar failures terminate in Blocked state. Timeouts and ack loss are handled by retransmitting the same already-stored evidence. Native control commands are not re-executed. Shutdown/watch sender termination closes the loop without marking unresolved device work complete.

Idle in StatusView means there is no evidence to transmit. It does not mean the actual device is healthy, the network is currently healthy or work is complete. A long block in the native gate can also delay journal reads, so local protection must not depend on this publisher.

A successful empty remote probe also reports its validated ACK cursor as `Published`. A subsequent local tick may show Idle again. This remote probe is separate from the daemon's software guarded heartbeat; neither is proof of P admission freshness or physical safety. One second is the probe interval under normal idle conditions, not a recovery-time guarantee including communication timeouts, backoff or Host gate waits.

P currently records a cursor revision and audit event even for an empty Publish. Native evidence sequence does not increase, but a one-second interval can produce approximately 86,400 audit records per idle Host per day. Long-term retention, capacity/load validation and probe transport optimization remain incomplete; this reconnection verification must not be used in their place.

P's current `platform_cursor` uses a cut in a separate control journal and the name `site-cell-control-v1`. H cursor keys also include view identity to avoid reusing old audit/base positions. The publisher's durable retransmission position is **producer journal+through_seq**; no consumer yet replays public events through the platform cursor. P's full wire mapping and snapshot/subscription policies must be completed before connecting the public Journal/Snapshot.

## Simulation tests

`test_seed_evidence` and `publisher` settings in `rx-host-sim-server` exist only in the test-harness feature. They seed a new test journal and send only to loopback P. A seed is not proof that a native effect occurred. Appending seeds to an existing journal is rejected.

- Host unit tests: rejection of incorrect destination/journal/range and ack regression, cursor preservation after H restart, and a separate cursor for a new store generation.
- Publisher unit tests: separation of 100 ms local ticks from 1-second remote probes, continued need for a probe after failure, empty batch/session/prefix preservation through actual loopback Publish, connection discard on Unauthenticated, and preservation of existing cursors on Unavailable/incorrect ACK. These tests isolate transport over an already negotiated connection; mTLS renegotiation is validated separately through actual P–H process tests.
- Separate P–H process tests: 131 source slots, first ack loss, retransmission with the same ID, metadata preservation, TLS registration checks and H restart/session invalidation.
- `PUBLISH` itself causes 0 device effects; native gate tests are a separate scope.
