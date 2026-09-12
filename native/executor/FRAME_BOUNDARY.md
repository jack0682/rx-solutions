# P-derived Frame 입력과 source clock

`rx.bt-frame.v1`은 Rust executor client와 C++ BT 사이의 로컬 typed IPC 표현이다. frozen Protobuf를 임의 JSON으로 바꿔 호출하는 API가 아니다. Counter는 canonical decimal string이며 optional identity/decision 필드는 이 schema의 null 표현을 쓴다.

Rust가 P 데이터를 인증/검증하고 같은 Context identity의 Packet을 생성한다. C++ `decode_frame`은 전체 key 집합·타입·UUID/Name/digest/uint64·closed enum·중복·배열/크기·시간 범위를 확인한다. Context는 그 결과도 node definition/identity/history/현재성을 대조한다. 임의 Script/BT builtin 허용 규칙은 변경하지 않았다.

Frame.source에는 동일 호스트 clock ID, P checked_at_ns와 valid_until_ns가 필수다. C++ Context에는 신뢰된 FrameClock을 연결한다. Linux 구현은 `/proc/sys/kernel/random/boot_id`와 CLOCK_BOOTTIME을 사용한다. publication, tick, queue handoff에서 source clock을 확인하며 clock 실패·불일치·만료는 새로운 요청을 차단한다. local steady deadline만으로 suspend를 숨기지 않는다.

frame이 잠시 만료되면 아직 handoff되지 않은 요청은 제한된 queue에 남고, 같은 Context의 새 유효 frame/eligibility를 확인한 후 전달된다. 이미 넘긴 요청을 repeated tick으로 다시 만들지 않는다. producer/worker는 받은 요청의 key를 native 제출 전에 영속 기록해야 하며 유한 operation의 S worker/journal은 연결했고 전체 운영 daemon과 나머지 요청 종류는 후속이다.

기존 C++ synthetic in-process Frame 시험은 source_deadline을 생략할 수 있다. 실제 IPC decoder는 source 필드를 반드시 요구하고, source-bound frame은 FrameClock 없이 Context에 게시할 수 없다. 이 차이를 production 입력 우회로 사용해서는 안 된다.

추가 검증은 source expiry 후 queue 차단/새 frame 재개, clock identity 변화, 숫자/overflow/duplicate/unknown 입력과 실제 Linux boottime 조회를 포함한다. 실제 P→별도 S read client가 만든 재시작 Frame도 BT.CPP에 넣어 기존 작업과 무허가 상태를 보존함을 확인했다.

해당 통합 시험은 명시적인 frozen simulation clock과 모의 cell이다. Frame 파일 수송의 실시간 성능이나 실제 장비/센서/보호 기능의 검증이 아니다. 새 Frame 유효성은 실제 운전 허가·native 완료·자원 인계·공정 품질을 대신하지 않는다.

`rx-bt-request-fixture`는 RX_BUILD_TEST_HARNESS를 명시적으로 켠 검증 빌드에서만 생성한다. simulation clock으로 한 Frame을 tick하고 typed request JSON을 출력한다. S worker의 실제 RPC 통합 시험에 사용하며 production launcher가 아니다.
