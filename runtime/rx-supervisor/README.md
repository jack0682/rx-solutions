# 솔루션 프로세스 관리 초안

2026-09-11. `rx-supervisor`와 `rx-solutionsd`를 추가했다. 선택한 프로그램의 기동·관측·종료를 별도 S 저장소에 기록하고, 프로세스의 생존과 RX 운전 준비를 구별한다. 이 단계의 제품 recipe는 **읽기 전용 상태 서비스 하나**이며, 실제 자사 driver/controller의 lifecycle authority 연결은 후속이다.

## 레포·제품 경계

관리 모듈은 `rx-solutions` 안에 있으며 같은 이미지에 포함된다. 자사 다섯 레포와 전이 의존성은 기존 기본 포함 집합을 유지한다. 프로그램 기동은 모든 모델의 launch를 일괄 실행하지 않는다. 선택 profile ID가 카탈로그에 존재하는지 확인하지만, 그 사실로 해당 모델의 제어 준비나 실물 검증을 선언하지 않는다.

P의 run/operation/qualification/dispatch permit를 이 저장소로 복제하지 않는다. 제어 프로세스의 실제 시작·종료는 기존 P/Host 권한과 검증된 현장 절차에 연결돼야 한다. `LifecycleAuthority`는 그 연결을 위한 포트이며 제품 기본 `SoftwareOnly`는 제어 효과가 있는 프로그램을 거부한다. 시험용 authority는 명시적 모의 backend에서만 사용한다.

## 계획과 프로그램 recipe

사이트 계획은 process ID, release program ID, 허용된 parameter, dependency, 기동/종료 timeout, restart limit/backoff를 지정한다. 실행 파일·script 경로·임의 argv·환경 변수·효과 분류를 사이트 입력으로 받지 않는다. 현재 `rx/status-http` recipe는 고정 Python interpreter와 RX 상태 script, 제한된 bind/port만 받는다.

프로그램과 효과 분류는 release-owned catalog의 책임이다. 실행 파일과 관련 파일의 SHA-256을 확인한다. 계획 digest는 원본 계획뿐 아니라 선택 프로그램의 실제 실행 정의·파일 digest·효과 분류와 선택 자사 profile에 결합한다. 같은 저장 계획에 바뀐 프로그램을 몰래 적용하지 않는다.

기동 의존성은 최대 32개 프로그램의 DAG다. 누락/중복/순환과 허용되지 않은 인자·profile을 거부한다. 기동/종료 timeout은 현재 100–30000 ms, software restart limit은 0–3으로 제한한다. 이 범위는 관리 소프트웨어의 입력 범위이며 모든 제어기의 안전한 시간 값이라는 뜻이 아니다. 제어 효과 프로그램은 자동 restart limit 0만 허용한다.

프로그램 파일의 검증과 실제 exec 사이의 무결성은 읽기 전용 release image와 신뢰하는 host OS를 전제로 한다. 이 단계에서 일반 writable 실행 경로의 원자적 fexecve나 적대적인 host 관리자에 대한 방어를 구현했다고 주장하지 않는다.

## 영속 상태와 OS 실행 경계

| 상태 | 의미 |
|---|---|
| PENDING | 아직 기동 의도를 기록하지 않음 |
| PREPARED | 고정 instance ID와 기동 의도를 기록 |
| SPAWN_ENTERED | OS 실행 경계 진입을 먼저 기록. 이것만으로 프로세스가 생성됐다고 확정하지 않음 |
| STARTING | 현재 owner가 실제 Child handle/PID를 얻음 |
| PROCESS_READY | 지정한 process probe를 확인. Control prepared나 qualification은 아님 |
| UNREADY | 기동 확인 기한 초과. 프로세스는 살아 있을 수 있음 |
| STOP_REQUESTED | 허용된 종료 의도를 기록. 실제 종료는 아직 별도 |
| EXITED | 현재 owner가 실제 OS 종료를 관측 |
| SKIPPED | 종료 요청 전에 기동되지 않았음 |
| START_FAILED | backend가 실행되지 않았음을 확인한 실패 |
| UNKNOWN | 실행/소유 연속성을 확인하지 못함. 재실행과 저장된 PID의 신호 전송 금지 |

전용 SQLite store와 단일 writer lock을 사용한다. 다른 용도의 기존 entity/event가 있는 store를 새 supervisor 저장소로 주장하지 않는다. transaction callback 안에서 프로그램을 실행하지 않는다.

PREPARED와 SPAWN_ENTERED를 먼저 commit하고 권한을 재검사한 뒤 OS backend로 진입한다. backend는 파일 검증·argument 준비 뒤 exec 직전에 authority를 다시 확인한다. 실제 기동 후 상태 저장에 실패했더라도 같은 살아 있는 owner가 보유한 Child handle로만 기록을 보완하며 다시 spawn하지 않는다.

SPAWN_ENTERED를 저장했지만 실행 진입 응답이 불명확하거나 manager가 재시작해 handle을 잃으면 UNKNOWN이다. backend의 ‘실행 안 됨’과 ‘실행 여부 불명’도 구분한다. 저장된 PID만으로 새 owner가 기존 프로세스를 채택하거나 종료하지 않는다.

## readiness·의존성·종료

일반 alive probe는 프로세스 생존만 확인한다. 현재 HTTP 상태 서비스는 생성한 instance ID를 child 환경에 넣고 응답의 같은 ID를 확인한다. 다른 프로세스가 같은 포트에서 응답해도 준비 완료로 인정하지 않는다. dependency는 process probe를 만족한 뒤 시작하며, 그 probe를 물리적 장비 조건으로 사용하지 않는다.

종료 요청은 메모리에서도 먼저 latch해 저장 실패 후 새 프로세스가 계속 시작되지 않게 한다. 의존 프로세스를 먼저 정리하고 provider를 종료한다. 종료 전후에 authority를 확인하며 제어 효과 프로그램은 허가가 없으면 유지한다. timeout만으로 제어 프로세스를 강제 종료하지 않는다.

Non-actuating 프로그램은 허용된 정상 종료 뒤 timeout이 지나면 강제 종료할 수 있다. OS backend는 자신이 소유한 Child handle을 확인하고 신호를 전송한다. 다른 owner의 PID 파일을 읽어 kill하는 기능은 없다. stdout/stderr는 instance별 파일이며 제어 프로세스의 부모 종료를 입력 EOF 기반 종료 명령으로 사용하지 않는다.

관측한 EXITED의 저장에 실패하면 동일 Child의 종료 관측을 다시 저장할 수 있다. EXITED가 확정 저장된 뒤에는 종료된 handle과 타이머를 회수해 반복된 정상 재활성화가 메모리 객체를 계속 쌓지 않게 한다. 정상 종료 의도도 없는 예상 밖 종료는 오류로 남고, non-actuating 프로그램에 한해 지정된 추가 기동 횟수와 backoff가 적용된다. 제어 프로그램 재활성화는 별도 절차가 필요하다. 기동 probe timeout 이후 살아 있는 프로세스를 자동 성공으로 바꾸지 않는다.

## 실행 방법과 현재 이미지 경로

기본 이미지 entrypoint의 `serve`/`inspect`는 기존 읽기 전용 진단 경로다. 새 관리 경로는 다음처럼 명시적으로 선택한다.

```text
/opt/rx/entrypoint.sh supervise /config/solutions-startup.json
/opt/rx/bin/rx-solutionsd inspect /config/solutions-startup.json
```

[예제 계획](../../examples/deployment/solutions-startup.json)은 `OM-05`를 참조하지만 읽기 전용 상태 HTTP 프로그램만 시작한다. controller/로봇 Host를 시작하지 않으며 control_prepared와 physical_shutdown_assessed는 false다.

state_subdirectory는 `/var/lib/rx-solutions` 아래의 제한된 상대 경로다. directory와 DB/lock symlink를 거부한다. P의 DB를 volume으로 제공하지 않는다. 명령 인자/프로그램 경로와 state 경로는 서로 다른 검증을 거친다.

정상 종료 기록은 다음 `run`에서 자동 지우지 않는다. 모든 프로세스가 확인된 terminal 상태이고 전체 계획이 non-actuating일 때만 명시적 `rx-solutionsd activate CONFIG`로 software 계획을 다시 준비할 수 있다. 강제 종료 뒤 UNKNOWN인 계획은 이 경로로도 재활성화할 수 없다.

## 검증과 아직 필요한 연결

시험은 기동 commit 전/후 장애, actual spawn 뒤 저장 실패, 불명 backend 결과, supervisor 재시작, 종료 권한 상실, 제어 프로세스의 강제 종료 거부, 저장 실패 중 stop latch, 종료 관측 재저장, dependency 순서와 제한된 software 재기동을 확인한다. 별도 실제 무동작 HTTP child로 instance-correlated readiness, 잘못된 응답과 파일 변조, owned process 종료도 확인한다.

실제 ROS/driver/BT 프로세스에 대한 release recipe·device/network 권한·P/Host lifecycle permit 검증, native 종료/지지 이관 증거, 재시작 후 process adoption·부재 증명과 전체 설치/업데이트 supervisor는 미완료다. 준비된 포트나 모의 authority를 그 검증의 대체물로 세지 않는다. 현재 상태 출력은 stdout/저장소이며 P 운영 화면과의 관리 명령·상태 연결도 후속이다. 첫 물리 셀은 NOT_COMMISSIONED다.
