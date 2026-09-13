# P-derived Frame input and source clock

`rx.bt-frame.v1` is the local typed IPC representation between the Rust executor client and C++ BT. It is not an API that calls frozen Protobuf after converting it to arbitrary JSON. Counters are canonical decimal strings, and optional identity/decision fields use this schema's null representation.

Rust authenticates/validates P data and produces a Packet with the same Context identity. C++ `decode_frame` checks the complete key set, types, UUID/Name/digest/uint64 values, closed enums, duplicates, array/size limits, and time bounds. Context also compares the result against node definitions, identity, history, and currentness. Rules governing arbitrary Script/BT builtins are unchanged.

Frame.source requires the same-host clock ID and P checked_at_ns and valid_until_ns. A trusted FrameClock is attached to the C++ Context. The Linux implementation uses `/proc/sys/kernel/random/boot_id` and CLOCK_BOOTTIME. The source clock is checked at publication, tick, and queue handoff; clock failure, mismatch, or expiry blocks new requests. A local steady deadline alone must not hide suspension.

If a frame expires temporarily, requests not yet handed off remain in the bounded queue and are delivered after a new valid frame and eligibility for the same Context are confirmed. Repeated ticks do not recreate requests already handed off. The producer/worker must durably record the received request's key before native submission. The S worker/journal is connected for finite operations; the complete operational daemon and remaining request kinds are future work.

Existing C++ synthetic in-process Frame tests may omit source_deadline. The actual IPC decoder requires source fields, and source-bound frames cannot be published to Context without FrameClock. This difference must not be used to bypass production input checks.

Additional validation covers queue blocking after source expiry and resumption with a new frame, clock identity changes, numeric/overflow/duplicate/unknown inputs, and actual Linux boottime queries. Restart Frames produced by the actual P → separate S read client were also fed into BT.CPP, confirming preservation of existing operations and the unauthorized state.

That integration test uses an explicit frozen simulation clock and a simulated cell. It does not validate real-time Frame file transport performance or physical devices, sensors, or protective functions. A fresh valid Frame does not replace actual operating authorization, native completion, resource handover, or process quality.

`rx-bt-request-fixture` is generated only in validation builds with RX_BUILD_TEST_HARNESS explicitly enabled. It ticks one Frame using a simulation clock and outputs typed request JSON. It is used in actual RPC integration tests for the S worker and is not a production launcher.
