# Host 변경 준비의 영속 기록

phase72. 현재 구현 범위는 종료 근거·변경 준비·조회·준비 취소다. 설정 파일 선택 교체, native 효과, 재기동/자격 발급과 P ReleaseManager 연동은 후속이다.

## 종료 파일 대신 원장 기록

제품 Host는 runtime owner와 기존 DB writer를 확보한 뒤, listener/native adapter를 열기 전에 STARTING을 기록한다. 새 시도 번호가 이전 STOPPED 기록의 현재성을 잃게 한다. 이후 listener bind 등이 실패해 옛 `host-status.json`이 남아 있어도 그 파일을 종료 증거로 사용하지 않는다.

RPC와 publisher를 닫고 현재 adapter의 drop 가능 상태를 다시 확인한 후, 서비스 오류가 없으면 StopSeal을 기록한다. 여기에는 기동 시도·Host boot·설치 identity·실제로 실행한 전체 Configuration digest·delivery/evidence journal·sequence, Host 운영 레코드 digest, 실제 종료 snapshot이 들어간다. Host의 미해결 operation이 남은 경우 변경 준비는 거부한다.

설치 identity는 backend/bindings 중심의 기존 의미를 유지한다. 별도의 전체 기동 설정 digest를 STARTING부터 StopSeal까지 연결하여, release/TLS/bind 같은 값을 CURRENT_CONFIG와 PROPOSED_CONFIG 양쪽에서 함께 바꾸어 비교 검사를 우회하지 못하게 한다. 독립 검토에서 이 반례를 찾아 수정했으며 실제 종료 구성과의 대조 시험을 추가했다.

Lifecycle과 maintenance 이력은 별도 이름 공간의 DB 레코드다. native evidence journal에 서비스 이벤트를 섞지 않으며 native evidence publisher의 sequence와 입력 형식을 바꾸지 않는다. 종료 snapshot의 관측은 당시 사실이며 현재 물리 지지나 새 제어권을 보증하지 않는다.

## 준비·조회·취소

```text
rx-hostd prepare-binding-change PLAN CURRENT_CONFIG PROPOSED_CONFIG REQUEST_ID
rx-hostd lookup-binding-preparation CURRENT_CONFIG REQUEST_ID
rx-hostd cancel-binding-preparation CURRENT_CONFIG REQUEST_ID
```

준비는 기존 읽기 전용 binding 검사에 성공해야 한다. 이후 서비스와 같은 runtime lock 및 기존 DB writer를 확보하고 설치 descriptor, journal identity, 가장 최근 StopSeal과 현재 운영 레코드/기록 끝을 대조한다. 누락된 DB를 초기화하지 않으며 살아 있는 서비스와 동시에 준비할 수 없다.

request 의미는 plan·current/proposed identity와 proposed configuration에 결합한다. request ID를 같은 의미로 다시 사용하면 기록된 현재 상태를 회수하고 다른 의미면 충돌이다. PREPARED가 있는 동안 다른 준비와 일반 Host 기동을 막는다. candidate 파일이 없어졌을 때에도 lookup으로 영속 상태를 회수할 수 있다.

active 준비와 request 상태·원래 준비 이력은 같은 SQLite transaction으로 기록한다. 준비 취소도 request/active 상태와 별도 취소 이력을 원자적으로 기록한다. prepare를 다시 호출해 취소한 요청을 되살리지 않는다. 취소는 아직 native/설정 적용 효과가 없는 PREPARED를 해제하는 기능이며 로봇 작업 취소나 적용 rollback이 아니다.

현재 로컬 도구의 실행 권한은 설치 파일/원장 소유자의 접근권에 따른다. P의 승인된 변경·등록 단말·ReleaseManager 위임을 인증하는 관리 프로토콜은 아직 연결하지 않았다. 따라서 이 기록을 P의 Host 구성 적용 receipt나 물리 실행 허가로 사용하지 않는다. installation_changed와 activation_authorized는 false다.

## 이전 프로그램과 설치 자료

새 설치 descriptor는 `rx.host-installation.v2`와 `maintenance_protocol=rx.host-maintenance.v1`을 쓴다. 이전 서비스의 v1 schema/strict decoder가 준비 기록을 모른 채 새 설치를 기동하지 못하도록 구별한다. 이 단계에서 과거 binary의 실제 납품 rollback 시험을 수행한 것은 아니다.

새 서비스는 v1 descriptor의 기존 일반 실행을 유지하지만 maintenance 준비는 허용하지 않는다. v1 descriptor를 자동 수정하거나 원장을 새로 만들지 않는다. 기존 설치의 명시적 migration과 rollback은 후속이며 현장 데이터로 임의 업그레이드를 실행하지 않았다.

## 검증 범위

실제 product service와 FileSimulation, SQLite 재접속을 이용해 다음을 확인했다.

- 최초 실행 전·서비스 실행 중 준비 거부, 실제 종료 후 준비 성공.
- 같은 요청 회수, 다른 의미/경쟁 요청 거부, 준비 중 기동 차단과 취소 후 실행.
- 기동 실패 뒤 남은 옛 STOPPED 파일의 재사용 거부.
- 종료 뒤 운영 DB 레코드가 바뀌면 준비 거부.
- commit 전/후 별도 테스트 프로세스를 즉시 종료하고 rollback 또는 같은 요청의 영속 회수를 확인.
- v1 설치를 자동 승격하지 않고 maintenance 준비를 거부.

프로세스 종료 주입은 test-harness에만 노출되며 제품 CLI의 외부 입력으로 설정할 수 없다. 위 결과는 native 장비 설정 변경, PLC 동작, physical shutdown 유지와 P/Host의 전체 교체·복원 인수를 증명하지 않는다.
