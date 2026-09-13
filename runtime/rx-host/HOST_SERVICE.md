# 납품용 Host 실행파일과 프로세스 수명

상태: phase58 구현 초안. `rx-hostd drivers` 및 `rx-hostd inspect|init|run CONFIG`와 동일 solutions 이미지의 `host` entrypoint를 제공한다. 테스트 전용 clock/응답 유실/seed 옵션은 제품 config에 없다. builtin backend는 FILE_SIMULATION과 서명된 MELSEC_PACKAGE다. [장비 패키지 기동 명세](DEVICE_PACKAGE_STARTUP.md)를 따르며, 미등록 일반 driver는 기동 전에 거부한다.

## 시작·저장소 소유

startup JSON은 installation/release/Host, bind 주소, 서로 분리된 data/runtime 경로, pinned Binding 파일·TLS 자료, 허용 P certificate fingerprint, backend 및 선택 publisher를 담는다. 임의 executable/argv나 라이브러리 경로를 받지 않는다. policy/file pin·schema/크기·peer/환경 및 TLS 자료를 검증한다.

init은 어댑터를 열지 않고 새로운 SQLite Host 원장과 installation descriptor를 만든다. 임시 설치 디렉토리에서 완성한 뒤 공개하며, 기존 설치에 반복하지 않는다. descriptor는 Host/installation/Binding/backend identity와 delivery/evidence journal identity를 묶는다.

run은 descriptor와 기존 DB/Host metadata/cell generation을 먼저 대조한다. 유실된 DB/metadata를 새 것으로 만들지 않는다. runtime lock과 기존 SQLite/device owner lock을 확인한다. 저장된 PID를 채택하는 기능이 아니다. status 파일이 다른 Host/installation에 속하거나 symlink인 경우에도 새 owner로 덮어쓰지 않는다.

제품 clock은 Linux kernel boot UUID와 CLOCK_BOOTTIME이다. CLI에서 시간 값을 설정할 수 없다. 다른 OS의 run은 아직 지원하지 않고 inspect는 입력 검사를 수행한다. `run_with`의 typed factory/clock은 별도 구현·시험을 위한 library composition port이며 site JSON에 clock override를 노출하지 않는다.

## 어댑터 선택

release-owned AdapterFactory가 metadata를 검증하고 `open_passive`로 어댑터를 만든다. 이 함수는 native 움직임/토크/모드 변경을 수행하지 않아야 하며, 최초 admission 이전에 연결을 닫는 것도 물리 제어 효과가 없어야 한다. 실제 driver 초기화가 동작을 유발한다면 그 부분은 별도의 허가된 lifecycle operation으로 옮겨야 한다.

현재 Builtin은 FileDevice와 검증된 DEVICE_REFERENCE 패키지의 Melsec 어댑터를 등록한다. VALIDATED_DRIVER profile/digest는 명시적으로 미지원 오류다. 외부 장비 SDK/ROS/model은 별도 검증된 구성으로 추가하며 개별 driver의 validation 없이 일괄 launch하거나 constructor/destructor의 물리 효과를 허용하지 않는다. 물리 운전에는 현재 process context/qualification 수용과 별도 Arm이 필요하다. 일반 driver factory와 전체 lifecycle authority는 후속이다.

## 준비와 실행

기동은 새로운 Host boot와 비활성 Arm으로 시작한다. mTLS/base/cell 협상, P grant/fence/qualification/start/permit와 기존 native guard를 그대로 거친다. 상태의 SOFTWARE_READY_UNARMED는 RPC 프로세스 준비를 뜻한다. 재시작만으로 qualification/Arm/native command가 복원되지 않는다.

Host data는 delivery facts와 evidence journal이며 P의 결과/자원/운전 authority 원장을 복제하지 않는다. publisher가 구성되면 기존 순서·ack cursor를 사용하고, transient transport 오류는 제한된 backoff로 재시도한다. publisher의 확정 실패나 RPC owner 실패는 admission 종료 절차를 시작한다.

## 정상 종료와 어댑터 보존

종료 요청은 atomic admission latch를 먼저 내린다. DB lock이나 진행 중 native 호출을 기다리지 않는다. 새 grant/renewal/Arm/prepare/authorize/configuration/qualification 수용을 거부한다. 증거/receipt 조회와 제한을 늘리는 fence·reconciliation은 물리 효과가 없는 경계에서 계속 가능하다. native 진입 직전에도 latch를 검사한다.

native `shutdown_snapshot`은 기존 handover_snapshot과 별개다. 현재 support_stable이 true라는 것만으로 destructor/연결 종료가 안전하다고 추론하지 않는다. shutdown proof는 전체 resource 범위, 현재 device session/시각/불확실성, 잔류 명령 없음과 **safe_to_drop**를 명시해야 한다. 기본 port는 미지원이며 exit를 허용하지 않는다.

정상 종료는 같은 native owner를 유지하며 근거를 재확인한다. native/gate가 응답하지 않아도 새로운 검사 스레드를 계속 만들지 않고 기존 검사 handle을 기다린다. timeout만으로 제어기를 drop/kill하지 않는다. transport/publisher drain 뒤에도 최종 drop 근거를 다시 확인한다. 예기치 않은 library owner 상실은 admission을 닫고 독립 protection port에 알린다. 강제 프로세스/runtime 종료·정전은 이 정상 종료 절차로 막을 수 없으며 검증된 외부 보호가 필요하다.

준비/진입 불명 작업은 원장에서 지우거나 성공/미실행으로 변환하지 않는다. FILE_SIMULATION은 잔류 비동기 native queue와 물리 지지가 없으므로 독립 safe-to-drop가 확인되면 이력을 보존한 채 종료할 수 있다. configured publisher의 drain 시간이 끝나도 미전송 근거는 남긴다. publisher가 없으면 보관된 근거를 발행 완료로 간주하지 않는다.

## 상태·배포

`host-status.json`은 installation/Host/instance/Host boot/endpoint/clock, admission, publisher 상태와 stop snapshot을 담는다. STOP_WAITING_FOR_ADAPTER·STOP_STATE_UNAVAILABLE·STOP_DRAINING_EVIDENCE·STOPPED·STOPPED_WITH_RECONCILIATION_REQUIRED를 구별한다. ready와 stop 출력은 물리 qualification을 생성하지 않는다. status 갱신 실패도 새 admission을 막는다.

`rx-hostd`를 image runtime의 `/opt/rx/bin`으로 복사하고 runtime inventory에 포함한다. Host mode는 ROS setup script 실행 전에 직접 binary로 진입한다. 기본 diagnostics mode는 기존대로 유지한다. [배포 템플릿](../../examples/deployment/host/README.md)은 기존 solutions 이미지·분리된 volume·비루트/read-only·최소 권한과 수동 기동을 설명한다.

supervisor의 control lifecycle authority/검증된 driver recipe는 아직 연결하지 않았다. 단순 NonActuating recipe로 분류해 모든 driver를 자동 시작하거나 재시작시키지 않는다.

## 검증 범위

[phase55 기록](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase55_checks.json)에 실제 결과를 보관한다. generic composition은 초기화의 no-adapter-open, 현재 프로세스 소유, 정상 종료, 유실 journal 거부, 미지원 backend·변조/unknown config 거부, drop permission 전 native owner 보존을 검사한다. gate 시험은 stop latch와 미확정 작업 보존, stable handover와 drop 허가 구별을 다룬다.

제품 binary/image 시험은 Linux clock·mTLS 준비·중복 owner 거부·SIGTERM/restart와 native effect0을 확인한다. 제품 Host와 P의 실제 작업 통합은 rx-hostd/Linux kernel clock으로 수행한다. 자격 발급·활성화·별도 시작·native 모의 동작1개·근거/인계·Run 완료를 확인하고, 종료 때 보관된 evidence를 발행 완료로 가정하지 않는 상태를 확인한다. 응답 유실은 테스트 클라이언트가 실제 RPC 응답을 받은 뒤 P writer로 전달하기 전에 주입하며 제품 server fault 기능을 쓰지 않는다. 제품 config에는 오류 주입/수동 clock 옵션이 없다. simulation 시험은 실제 로봇/PLC·하중/정지·지지 안전성 검증이 아니다. 첫 물리 셀은 NOT_COMMISSIONED다.


MELSEC 상태 보장 어댑터 library와 native journal/모의 Host 시험을 추가했다. [구현 범위·publication 전제](MELSEC_ADAPTER.md)를 따른다. phase58에서 signed-package factory·native identity의 원자 초기화와 물리 binding의 현재 qualification 필수 검사를 연결했다. 현장 qualification을 완료한 것으로 표시하지 않는다.
# Host binding 변경 검사 추가

phase71의 읽기 전용 `inspect-binding-change`는 [현재·제안 설정 비교](HOST_BINDING_INSPECTION.md)를 따른다. 설정 일치 결과는 Host 설치/실행 권한이 아니며 기존 `init`·`run`의 설치 identity 검사를 우회하지 않는다.
