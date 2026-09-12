# Protocol-guarded Host·Executor recipe

`ProtocolGuardedService`는 release catalog가 고정한 `rx-hostd run`과 `rx-executor-service cell run`의 software bootstrap/협력 종료 분류다. site plan에서 effect·executable·argv·환경변수·상태 경로를 지정하지 않는다. 기존 NonActuating/SoftwareOnly와 RequiresPlatformAuthority 의미는 유지한다. GuardedServices는 새 분류의 협력 요청만 허용하고 physical lifecycle permission을 만들지 않는다.

startup configuration의 선택적 `services`는 이름별 map이다. 각 항목은 `configuration:{path,sha256}`와 공통 status crate의 typed `scope`다. Host scope는 HOST/installation/host/installation_identity, Executor scope는 EXECUTOR/installation/cell/service_journal(UUID)/configuration_digest다. 이름은64자 이내 alphanumeric/underscore/hyphen의 flat label이며, release가 `rx/service/<name>` program ID와 실제 binary/init/run/effect를 생성한다. 이름이나 scope로 임의 executable을 입력할 수 없다. 이는 배포의 데이터 pin 입력이며 UI/P operation 입력이 아니다. 기존 services 없는 startup v1은 그대로 지원한다.

builder는 실제 pinned config bytes에서 Host 설치 identity와 Executor normalized configuration digest를 대조하고, Host Binding과 Executor cell/definition, 서로 분리된 data/runtime root를 확인한다. initializer/daemon executable은 runtime inventory의 고정 `/opt/rx/bin/rx-hostd`, `/opt/rx/bin/rx-executor-service`뿐이다. 두 프로그램의 arguments map은 비어 있고 site에서 추가 인자를 끼울 수 없다. 각 daemon의 원래 strict loader, TLS 검증, backend/원장/현재 권한 검사는 그대로 수행한다. composition의 projection 검사는 이를 대체하지 않는다.

기존 Plan의 최대32 process 범위에서 복수 Host·Executor·셀을 지원한다. 각 선택 서비스는 process 하나에만 연결하며 같은 installation/principal의 별도 process가 서로의 session을 철회하는 구성을 거부한다. Host는 여러 셀의 Binding을 제공할 수 있다. 각 Executor는 같은 installation/cell을 제공하는 모든 선택 Host의 definition과 일치하고 그 Host process ID들에 직접 의존해야 한다. 해당 managed Host가 없는 Executor 구성은 이 composition에서 미지원이다. 선택 status-http가 있으면 각 Host가 status process들에 의존하여 종료 중 진단이 먼저 사라지지 않게 한다. 이는 P의 전역 한 셀 한 Run 제약이 아니다.

## 명시 초기화와 기동

`rx-solutionsd init CONFIG`는 선택한 서비스의 metadata initializer 목록을 위상 순서로 실행한다. 명령은 역할별 고정 Host init 또는 Executor cell init뿐이다. supervisor state 하위 `initialization.db`에 resolved plan/initializer 목록·순서의 digest와 각 단계의 program/role/instance·Pending/Entered/Completed/Failed를 보존한다. Entered를 commit한 뒤 OS 실행 경계에 들어간다. 이미 Completed인 동일 단계는 회수하며, Entered/Failed인 단계는 자동 재실행하지 않는다. initializer exit0을 실제 관측한 경우에만 Completed를 기록한다. 30초 또는 종료 요청이면 자신이 가진 child leader에 TERM을 한 번 요청하고 실제 종료까지 handle을 유지한다. 강제 kill이나 timeout을 성공 결과로 쓰지 않는다.

`run`은 해당 초기화 원장이 존재하고 정확한 digest·목록·역할·순서의 모든 단계가 Completed인지 확인한다. 일부 완료만으로 서비스들을 시작하지 않는다. 초기화 파일/기록 누락을 새 init으로 대체하지 않는다. 이후 기존 Supervisor의 Prepared→SpawnEntered→owned Child 경계를 사용한다. Host/E의 자체 required-open은 실제 데이터 유실·손상을 다시 거부한다. child init 성공 후 supervisor commit 응답이 불명인 경우 원래 Completed 기록이면 회수할 수 있지만, Entered만 남은 경우 현재는 별도 설치 조회/복구가 필요하다. stop seal/installation reader를 통해 그 상태를 조정하는 API는 후속이며, 새 id나 init 재실행으로 우회하지 않는다.

초기화 store와 supervisor store는 daemon의 data root와 겹칠 수 없다. `/run/rx-solutions`는 실제 writable runtime 디렉터리로 제공해야 한다. 현재 이미지의 `supervise CONFIG`는 run 경로이며 init은 `/opt/rx/bin/rx-solutionsd init CONFIG`를 명시 호출한다. 기존 software-only `inspect/run/activate`와 구성은 유지한다. `activate`는 기존 non-actuating 계획에만 적용되고 guarded 재기동을 자동 승인하지 않는다.

## 현재 인스턴스의 관측

공통 `rx-service-status`의 guarded status를 사용한다. release base path `/run/rx-solutions/<service-name>.json`은 launch마다 `<service-name>.<instance UUID>.json`으로 파생된다. manager는 기존 RX_PROCESS_INSTANCE_ID와 새 RX_PROCESS_STATUS_PATH를 함께 제공한다. 예전 파일을 삭제하거나 foreign owner 상태를 덮어쓰지 않는다.

reader는 scope/instance/owned PID·파일 종류·크기·Linux clock·최대2초 source age를 검사한다. backend는 동일 instance의 sequence/time 역행 또는 같은 sequence의 변경된 payload digest를 거부한다. 원래 observation 시각과 digest를 유지한다. Starting에서 AliveOnly나 다른 HTTP 응답은 guarded readiness로 인정하지 않는다. Ready 이후에도 관측하며 protocol readiness 상실은 해당 composition의 협력 종료 latch로 연결한다. Ready는 software protocol 상태이며 RunMandate/물리 제어 준비가 아니다.

상주 셀 실행기는 내부 RunService의 완료·planner 정리 중에도 프로토콜을 제공한다. 내부 작업의 Stopping/Stopped를 daemon 전체의 Stopping으로 전달하지 않는다. 명시적 셀 서비스 종료 요청·전체 Stopped와 Attention은 별도로 반영하며, 정상 완료 뒤 같은 실행기가 Idle로 복귀해 다음 작업을 기다린다.

## 종료와 근거

protocol-guarded child는 automatic restart0이고 force kill을 backend에서도 거부한다. 일반 software recipe의 기존 process-group 신호 정책은 유지하지만, Host/Executor는 **positive leader PID에만 TERM**을 받는다. 따라서 Host 하위 bridge/controller 및 Executor planner를 supervisor가 먼저 종료하지 않는다. adapter safe-to-drop와 실제 planner cleanup은 각 daemon이 기존 경로로 확인한다.

종료 의존성은 Executor → Host → 선택적 status 순서다. 협력 종료 요청을 허용하는 것과 safe-to-drop/종료 완료가 확인됐다는 것은 다르다. Host가 지원/잔류 명령 근거를 얻지 못하면 기존 owner를 유지하고 supervisor도 계속 기다린다. 이미 safe-to-drop여야만 TERM을 요청하게 만들지 않는다.

TERM 전달이 실패하면 같은 owned child를 유지한 채 협력 요청을 재시도한다. 실제 전달 성공 뒤에만 전송 완료와 대기 시작을 기록한다. metadata initializer도 같은 원칙을 적용하며 timeout이나 실패한 signal command를 종료 성공으로 바꾸지 않는다.

OS exit는 Exited이고, 새 `guarded_exit`는 별도다. 현재 instance의 Stopped metadata와 owned child exit0가 함께 있어야 Confirmed다. missing/stale/다른 instance·Ready/Attention 또는 exit1/signal 종료는 Unconfirmed다. 원래 observation 시각/digest/sequence와 reconciliation_required를 보존하며, 로컬 종료 기록 commit 실패 후에는 같은 owner가 이미 관측한 종료 사실을 재사용한다. metadata를 새 시각으로 갱신하지 않는다. guarded daemon의 예상 밖 종료는 남은 구성의 협력 종료를 latch하고 자동 재시작하지 않는다.

`all_exited`는 OS 사실이다. `guarded_shutdown_confirmed`가 별도로 true여야 daemon CLI가 전체 종료를 성공으로 반환한다. 확인된 종료라도 남은 evidence/attachment는 `reconciliation_required`로 유지한다. 두 bool이나 자식 exit를 물리 qualification·작업 결과·native 지지 proof로 새로 해석하지 않는다. 실제 Host proof 검사/봉인은 Host 내부에서 그대로 수행한다. container runtime의 외부 SIGKILL/전원 상실을 이 절차가 방지한다고 주장하지 않는다.

최초 이미지 인수는 FILE_SIMULATION Host와 기존 P–S 흐름이다. 같은 구조의 전용 catalog 시험은2 Host/2 Executor/2 cell 목록, 각각의 dependency/definition, 중복 principal 거부를 다룬다. initializer 시험도4개 named step의 완료/재조회와 partial/Entered replay 금지를 다룬다. 실물 ROBOTIS JTC provider/DHI lifecycle, guarded 계획의 명시 재활성화, 초기화 Entered 상태의 관리형 복구, 로그/status 이력 보존·rotation은 후속이다. 추가 시험 소스는 alive-only bypass, restart/임의 인자 거부, instance 경로 분리, Ready 상실, exit+최종 report 조합, timeout 뒤 force 금지와 실제 child group에 TERM이 전달되지 않는 반례를 다룬다. 빌드·시험 결과는 부모 검증 기록을 따른다.
