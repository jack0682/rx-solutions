# 실행 서비스와 중단 의도 보존

`rx-executor-service`는 Linux에서 지정한 run/visit의 P Client, S journal, 지속 BT 프로세스와 pending queue를 실행한다. P가 현재 서비스 session에 run을 허가하고 해당 소재 시도가 존재할 때까지 기다린다. 소프트웨어 시작이 operator admission·소재 예산 소비·실장비 기동을 대신하지 않는다.

## 현재 실행 범위

서비스는 한 run에 고정된다. ManualVisit은 지정 visit만 처리하고, 기본 SerialProduction은 P part/예산을 따라 visit을 순서대로 전환한다. 현재 P 상태를 읽어 같은 context의 BT에 전달하고, 한 번에 하나의 일반 worker 요청을 처리한다. 통신 공백 동안 새 BT frame을 만들지 않고, 설정한 grace를 넘거나 planner/worker가 실패하면 중단을 시작한다. context 변경을 새 실행 권한으로 자동 채택하지 않는다.

BT graph 완료 자체는 part/run 완료가 아니다. SerialProduction은 [소재 조정자](PRODUCTION_COORDINATOR.md)를 통해 P 완료 확인과 다음 소재 admission을 진행한다. ManualVisit은 GraphComplete에서 별도 조정자를 기다린다. 명시적 재시작 절차는 후속이다. Graph failure는 새 일반 요청을 중단하고 P pause를 요청한다.

## 중단은 run 수명주기 기록이다

중단 의도는 `executor-stop/current`의 `rx.executor-stop.v1`에 보존한다. 이 기록은 BT의 node/visit 요청을 가장하지 않는다. run, 최초 요청 ID·session·선택적 실행 context·시각, 원인, 단계, pause 시도와 P 관측을 가진다. 아직 part나 BT root가 없는 시작 전 상태도 중단할 수 있다.

| 단계 | 의미 |
|---|---|
| PENDING | 중단 의도를 저장했지만 P의 제한 상태를 아직 확인하지 못함 |
| PAUSE_OBSERVED | 인증된 P의 PAUSED/RECOVERY_REQUIRED/COMPLETED/ABANDONED를 확인 |
| SUPERSEDED | P가 다른 executor session에 run을 결합했음을 확인. 이전 의도로 새 owner를 pause하지 않음 |
| ATTENTION | 권한/세대/응답 등의 조정이 필요함. 완료로 표현하지 않음 |

최초 의도는 바뀌지 않으며 새 서비스 시작이 이를 지우지 않는다. Pending 중에 추가된 pause attempt는 key/session/expected revision을 유지한다. PREPARED를 저장하고 ENTERED를 저장한 다음 P에 보낸다. 응답 유실은 ENTERED로 남으며, 다음 P 조회가 제한 상태를 확인해도 없던 RPC 응답을 만들지 않는다.

현재 atomic PauseRun의 확인된 revision 거부에서만 새 attempt/key를 만든다. 이전 attempt/history는 보존한다. 일반 transport 오류로는 body나 key를 바꾸지 않는다. 최대 64개의 시도 기록으로 제한하며 새 시도 한도와 기존 미확정 시도 회수를 구별한다.

## 종료·재시작

중단 의도를 먼저 저장하고 일반 계획을 동결한 뒤, planner 자식만 닫는다. P와의 통신은 pause 확인까지 유지한다. Host/DHI/토크 제어 프로세스를 이 서비스의 종료와 묶지 않는다. P pause 관측은 물리 정지·소재 지지 해제·자원 인계 확인이 아니다.

서비스는 제한된 시간 안에 P의 제한 상태를 확인하려고 한다. P에 도달하지 못하면 PENDING을 남기고 attention 결과로 종료한다. 프로그램 재시작 시 그 의도를 먼저 읽어 처리하며 BT를 새로 띄우지 않는다. 이미 완료된 중단 기록도 자동으로 지우지 않는다. 같은 run의 재운전에는 후속 명시적 재시작/수명주기 재결합이 필요하다. 정상 직렬 소재 전환은 P 완료 확인으로 기존 planner만 retire하며 run-pause를 발생시키지 않는다.

로컬 저장이 실패해도 planner 동결과 P pause를 시도한다. 이때 key/body는 해당 프로세스 메모리에서 유지하고 결과의 durability_fault를 표시한다. P가 제한된 상태를 확인했더라도 로컬 기록 장애를 성공으로 감추지 않는다. 이 경로를 durable stop intent 보존으로 주장하지 않는다.

## 실행 파일

Linux CLI는 JSON 설정 파일 한 개를 받는다. 설정은 P URI/server name, CA/client certificate/key 경로, PeerPin, run/visit, S journal 경로, immutable BT binary 경로/SHA-256, poll/grace/stop 옵션이다. endpoint는 TLS이고 clock_id는 실제 로컬 Linux boot clock과 일치해야 한다. peer_boot는 설정 값 대신 프로세스마다 한 번 새로 생성한다. SIGINT/SIGTERM을 shutdown 요청으로 연결하며, 지원하지 않는 OS에서 다른 시계로 대신 실행하지 않는다.

CLI는 구성 해석 직후, P 연결/Session.Open 전에 설정된 journal의 부모 디렉터리에 `.rx-executor-service.lock` 소유권을 확보한다. 범위는 **설정된 service/journal root당 한 프로세스**이며, 서로 다른 root의 정상 서비스는 독립적으로 실행할 수 있다. 셀 전체의 Run 수를 제한하는 정책이 아니다. 같은 root의 서로 다른 run journal도 기존 P peer를 교체하는 중복 기동으로 취급하여 연결 전에 거부한다. 디렉터리 alias는 실제 root로 정규화하고 기존 잠금 경로의 symlink·특수파일을 거부한다.

소유권은 연결·원장 열기·서비스 운전·중단 확인을 포함한 함수 전체에서 유지한다. 정상 반환·오류 반환·unwind에서는 소유자의 Drop이 명시적으로 unlock한 뒤 파일 handle을 닫는다. 따라서 동시 자식 기동의 fork→exec 사이에 잠시 상속된 descriptor가 남아도 정상 반납을 지연시키지 않는다. 강제 종료에서는 커널의 handle 정리에 따른 해제에 의존한다. 잠금 파일은 다른 프로세스가 같은 inode를 기다릴 수 있으므로 삭제하지 않는다. 이 잠금은 기존 run Scope 검사, 요청/stop 원장과 새 boot의 권한 철회를 대신하지 않는다. `test-harness` 빌드의 접속 직전 probe는 독립 CLI 프로세스의 접속 단계 진입 횟수만 기록하고 즉시 실패한다. 기본 제품 빌드에는 probe 환경변수 처리나 해당 경로가 포함되지 않는다.

상태는 변경될 때만 JSON으로 출력한다. 중단 결과에 PENDING/ATTENTION 또는 durability_fault가 있으면 CLI는 비정상 종료한다. 연결/인증 전의 구성·기동 실패는 실행 허가가 아니다. journal 부모 디렉터리·계정/certificate·P 서비스·readonly release를 제공하는 배포 도구는 아직 별도 구현 범위다.

기본 coordination은 SERIAL_PRODUCTION이다. MANUAL_VISIT은 개별 visit·setup용이다. 기본 poll은 50ms, 통신 grace는 5초, P pause 확인 window는 10초다. 조정 가능한 범위는 코드에서 검증한다. planner CLOSE에는 별도 최대 2초를 사용한다. 이 수치는 서비스 처리 정책이며 장비 정지 기한이나 현장 성능 보장이 아니다. 모델별 지지/정지 요구는 Host와 현장 인수 명세에서 별도로 충족해야 한다.

## 검증

실제 서비스 loop·별도 S 프로세스·지속 C++ 엔진·P mTLS/SQLite에 다음 6개 시나리오를 추가했다: 정상 중단, 실행 배정 전 중단, pause 응답 유실, 의도 저장 직후 프로세스 종료, 로컬 journal 장애, P pause 미응답. 기존 18개와 합쳐 총 24개다.

새 boot 복원에서 BT 기동 0회, 원래 intent/attempt 보존, 유실된 응답의 부재, P 미응답 때 Pending 유지, 저장 장애 중 P pause 시도와 장애 표시를 확인한다. journal 시험은 rollback/commit 응답 유실, key/history 불변성과 중단 기록의 자동 재개 금지를 확인한다. 서비스 시험의 조건·clock·초기 operator/part 시작은 simulation이다. 실제 CLI signal-to-P 전체 경로·제품 supervisor·물리 장비 종료 인수까지 검증했다고 확대하지 않는다.

직렬 part coordinator는 연결했다. 남은 것은 병렬 소재·실물 genealogy, 같은 run의 명시적 재시작/rebind, intervention/clearance/cancel과 continuous control, 전체 P/Host/S supervisor·두 제품 이미지·자사 스택·설치복원/인수 및 전체 UI다.
