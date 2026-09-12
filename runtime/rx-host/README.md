# RX Host foundation

The Host owns native delivery facts and its final command gate. It does not assign platform outcomes.

## Structure

- gate/grants: durable maximum resource fences and expiring, session-bound grants.
- gate/scopes: cell/scope fence and Arm receipts with durable request identity.
- gate/dispatch: exact approved intent, permit identity, local guard, SEND_ENTERED and native entry under one gate.
- gate/receipts: receipt lookup, pre-send tombstones, native result recovery and evidence outbox.
- journal: separate delivery/evidence journal identities and sequence spaces.
- native: C++/ROS/SDK-facing adapter port and a separate local protection port.
- simulation: a file-backed device with an independent exclusive owner and effect log. It intentionally does not deduplicate native calls.

Known PREPARED work can only be rebound under a new validated permit; the previous permit ID is permanently retired. SEND_ENTERED is never re-submitted. Captured native status is evidence, not a platform SUCCEEDED result. A Host receipt does not claim RESULT_RECORDED before platform T2.

## Verification

Run from rx-solutions:

    ./tools/cargo test -p rx-host --features test-harness --locked
    ./tools/cargo clippy -p rx-host --all-targets --features test-harness --locked -- -D warnings

The test-harness feature builds a fixture binary. It is excluded from ordinary product builds. Tests kill that process at the durable-send/native boundaries, restart it, and count independent device-log effects. These are process-crash tests, not storage power-loss or machinery protection tests.

## Remaining integration

mTLS/gRPC and platform outbox/evidence exchange are connected. Complete native cancellation journals, automatic expiry/deadman monitoring, continuous-control streams, physical adapters and the full product image are pending. The independent protection-port test proves that the port remains callable during a blocked native submission; it does not prove a physical stop or support response.

SDK sources are exported and hash-pinned from rx-platform; rx-application is intentionally excluded. Edit the producer, regenerate the SDK and reverify. Do not modify the vendored sdk directory directly.

[Host 공정 구성 문맥과 receipt](PROCESS_CONFIGURATION.md)는 전체 관리 셀의 fence·현재 Binding·quiescence를 확인하고 공정 문맥을 영속 기록한다. 적용 뒤에는 unqualified gate를 유지하며 native 설정 변경/운전 자격을 주장하지 않는다.

[Host 자격 수용과 별도 시작](QUALIFICATION_ACCEPTANCE.md)은 exact cohort/문맥/세대·자격 의미를 원자적으로 보관한다. 수용 자체는 block 해제나 native 동작이 아니며 P 전역 활성화는 후속이다.

[제품 Host 실행파일·기동/종료](HOST_SERVICE.md)는 `/opt/rx/bin/rx-hostd`와 solutions image의 명시적 `host` mode를 제공한다. FILE_SIMULATION backend를 실제 Linux clock으로 실행하며 physical driver는 검증된 factory 등록 전 거부한다.
