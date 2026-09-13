# 지속 BT 엔진과 private-pipe 계약 v1

`rx-bt-engine`은 한 context의 BT 메모리를 유지하는 실행 파일이다. Rust 연결부가 검증된 P 상태를 전달할 때만 한 번 tick한다. 네트워크, ROS/native driver, 임의 plugin 로딩이나 독립적인 작업 완료 판단은 제공하지 않는다. 기본 실행 파일은 Linux BOOTTIME을 사용하며 시계 변경 인자를 받지 않는다.

## 명령과 응답

입력은 줄바꿈으로 끝나는 JSON이다. 한 명령은 최대 4MiB, resolved graph/XML/Frame 각각은 최대 1,000,000 bytes다. 중복 key·알 수 없는 field·과도한 중첩·잘린 패킷을 거부한다. 요청 sequence는 처음 1부터 정확히 1씩 증가하고 context를 재설정할 수 없다.

| command | 추가 필드 | 의미 |
|---|---|---|
| INITIALIZE | identity, resolved, xml | 검증된 선언형 graph/생성 XML로 트리를 구성. tick하지 않으며 요청은 없어야 함 |
| STEP | frame | 동일 context/revision·시계 범위를 확인한 후 publish/tick |
| HALT | 없음 | 새 제안 중단을 고정하고 별도 PauseExecutor 제안 |
| CLOSE | 없음 | HALT 후 응답을 flush하고 프로세스 종료 |

공통 필드는 schema=`rx.bt-command.v1`, sequence(10진 string), command다. 응답은 schema=`rx.bt-reply.v1`, sequence, state, requests, fault를 가진다. state는 READY/RUNNING/SUCCESS/FAILURE/STALE/HALTED/FAULT다. fault는 FAULT에서만 존재하고 진단 문자열은 제한한다. 응답의 요청은 최대 32개다.

입력 중단/EOF는 planner를 종료한다. malformed/context/무결성 오류는 FAULT와 가능한 PauseExecutor를 반환한 뒤 종료한다. 만료된 같은 context의 frame은 STALE로 무시하고 새 frame을 기다린다. 미래 시각/다른 clock·실행 세대는 단순 지연으로 취급하지 않는다. HALT 뒤 STEP은 금지한다.

## 책임 경계

Rust가 P endpoint·artifact hash/size/schema·resolved 의미를 검증하고 XML을 생성한다. C++는 신뢰된 부모의 private pipe를 받아 graph shape/Name/Counter·XML whitelist·Frame·context 및 상태 연속성을 확인한다. C++ 자체가 resolved JSON의 JCS digest를 다시 계산하거나 P 인증을 수행한다고 주장하지 않는다. 반환 요청은 Rust worker와 P에서 다시 검증한다.

동일 operation/branch뿐 아니라 확정된 wait 결과도 변경할 수 없다. P가 이미 해결한 미전달 제안은 C++ 내부 큐에서 제거하여 새 제안의 공간을 확보한다. 이미 전달한 요청은 재tick으로 반복 송신하지 않는다.

SUCCESS는 해당 P view로 계산한 BT 상태다. P의 part/run 완료 기록, native 정지 또는 자원 인계 완료를 대신하지 않는다. CLOSE/프로세스 종료 역시 장비 제어기의 종료가 아니다.

## Rust 연결부와 큐

`EngineProcess::spawn`은 절대 경로의 일반 파일과 SHA-256을 확인하고 shell/현장 argv 없이 실행한다. 배포는 해당 binary와 의존 라이브러리를 immutable/read-only release로 제공해야 한다. 해시 검사를 수정 가능한 파일시스템의 TOCTOU 방어로 과장하지 않는다. 자식에게 부모 환경/credential을 전달하지 않는다.

연결부는 sequence/응답 크기·필드·상태·요청 context를 확인한다. pipe I/O는 2초로 제한하고 오류 후 자동으로 재시작하지 않는다. 부모가 bridge를 폐기하면 planner 자식을 종료한다. 이 동작으로 Host/장비 드라이버/토크 제어 프로세스를 종료해서는 안 된다.

PendingRequests는 최대 32개의 일반 제안과 우선 pause 상태를 관리한다. 완료 전 요청은 새 프레임이 오더라도 유지한다. 특히 BeginWait의 시작 확정은 요청을 없애지 않으며, wait 결과까지 계속 확인한다. 반복 요청은 합치고 body 변경을 거부한다. 일시적 RPC 오류는 100ms–5초 backoff, 상태 재확인은 짧은 간격으로 진행한다. backoff용 Instant는 물리 시계/허가 만료 판단에 쓰지 않는다.

큐는 volatile이며 durable mutation 기록은 기존 S journal이 담당한다. pause는 일반 큐가 가득 차거나 작업이 진행 중이어도 우선 요청할 수 있다. PauseObserved는 P의 제한 상태를 확인했다는 뜻이며 물리 정지 확인이 아니다. 오류/지원하지 않는 제안은 새 일반 제안을 중단하고 pause를 요청한다. 후속 단계에서 [한 run/visit 실행 서비스·signal 연결·durable stop intent](../../runtime/rx-executor/SERVICE_LIFECYCLE.md)를 추가했다. 전체 프로세스 supervisor와 part coordinator는 후속이다.

## 검증 범위

- 실제 C++/private pipe 9사례: 무동작 초기화, 재tick 중 요청 중복 방지, halt 고정, 만료 후 새 frame 수용, 다른 context, duplicate/truncated/oversized JSON, wait 결과 변경, 생산용 시계 보호를 검사한다.
- 기존 18개 S/P 통합 중 분기/대기 9사례는 이제 하나의 지속 C++ 프로세스와 PendingRequests를 사용한다. BeginWait 제안은 한 번만 나오며 Rust가 결과까지 유지한다. 정상 fixture 종료는 P pause를 확인한다. 종료/응답 유실 fixture의 원래 journal 상태도 보존한다.
- Linux 전용 검사는 production binary의 잘못된 digest 거부, 실제 BOOTTIME, 같은 PID의 반복 처리, 무권한 view에서 작업 제안 없음, planner CLOSE를 확인한다. 입력 P state는 합성 복원 자료이며 실제 장비 시험이 아니다.

테스트 시계 파일을 받는 `rx-bt-engine-fixture`는 RX_BUILD_TEST_HARNESS에서만 만든다. 생산용 실행 파일에는 해당 인자가 없다. 두 제품 이미지·전체 상주 서비스·현장 timing/qualification 완료로 확대하지 않는다.
