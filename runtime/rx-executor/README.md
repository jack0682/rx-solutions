# RX 실행기 client와 Frame 경계

현재 구현은 P의 인증된 복원/현재 상태 읽기, C++ BT Frame, [영속 요청과 유한 작업 worker](JOURNAL_AND_WORKER.md)다. [분기·대기와 checkpoint commit·관측 복원](DECISIONS_AND_RECOVERY.md)도 연결했다. 전체 daemon/part coordinator와 개입·재시작 조정은 후속이다.

- Client.connect는 deployment가 지정한 TLS endpoint/CA·서비스 credential, 설치/저장 세대/release/shared clock/cell definition을 사용한다. 브라우저 session을 쓰지 않는다. 실제 프로세스 시작마다 새로운 peer_boot를 발급하고, 같은 프로세스의 transport reconnect에서만 같은 boot를 유지해야 한다.
- Client.restore는 frozen GetRun과 run-scoped artifact 조회를 결합해 원래 activation/slot/intent를 회수한다. 역사적인 EXECUTING을 현재 권한으로 쓰지 않는다.
- Client.snapshot은 선택적 executor-read binding hash, canonical payload hash/size/schema, P 신원/순서/시계, 실제 resolved process와 shared validation을 확인한다. P의 내부 DB나 rx-application을 import하지 않는다.
- ValidatedSnapshot은 client만 만들 수 있고 내부 데이터는 읽기용이다. Frame은 명시적으로 선택한 Context identity와 대조한다. 새로운 epoch/session/digest를 자동 채택하지 않는다.
- Clock은 신뢰된 동일 호스트 adapter다. LinuxBoottime은 kernel boot ID와 CLOCK_BOOTTIME을 사용한다. 요청 전후 시계 범위와 P absolute expiry, local request-send deadline을 함께 검사한다.
- Frame은 RPC request 제안용 data다. native permit가 아니며 P/H gate를 대체하지 않는다. C++에 SourceDeadline을 함께 전달해 suspend/clock 변화 후에도 오래된 Frame을 쓰지 않게 한다.

`rx-executor-read-fixture`는 test-harness 전용이다. 명시적인 loopback simulation clock에서만 동작하고 native/작업 제출을 하지 않는다. 새 peer 접속은 P의 이전 세션/권한을 철회할 수 있다. fixture 결과에는 checkpoint/snapshot/resolved/Frame/XML이 있고 실제 P와 TLS로 연결해 만든 자료다.

현재 S client의 복원 자료는 shared DTO이며 실행 허가 writer가 아니다. payload bytes는 서명이 아니며 신뢰는 고정 endpoint·현재 인증/셀 권한·구조 검사에서 온다. 일반 URL fetch나 임의 파일 접근을 artifact API로 제공하지 않는다.

C++ decoder/Context와 SourceDeadline 의무는 [native executor](../../native/executor/FRAME_BOUNDARY.md)를 따른다. Linux 검사와 mock clock 시험을 물리 로봇 성능/현장 qualification으로 확대하지 않는다.

[지속 BT 엔진과 Rust private-pipe/요청 큐](../../native/executor/PERSISTENT_ENGINE.md)를 연결했다. 준비된 tree는 한 프로세스에서 반복 처리하고, 대기 요청을 결과까지 유지한다. 전체 daemon/supervision·part coordinator·durable shutdown intent는 후속이다. P gRPC 채널은 connect/RPC에 각각 2초 제한을 두며 transport timeout을 미적용 증거로 사용하지 않는다.

[Run/visit 실행 서비스와 중단 의도 보존](SERVICE_LIFECYCLE.md)을 연결했다. Linux CLI, 배정 대기·지속 처리·통신 grace, 별도 stop journal과 restart 시 재개 차단을 제공한다. 전체 배포 supervisor·part coordinator·같은 run의 명시적 restart/rebind는 후속이다.

[직렬 소재 조정](PRODUCTION_COORDINATOR.md)을 서비스의 기본 모드로 연결했다. 소재 admission/완료는 P에서 검증하고, 응답 유실에서도 기존 ID와 budget 소비를 보존한다. ManualVisit은 별도 설정으로 유지한다.
