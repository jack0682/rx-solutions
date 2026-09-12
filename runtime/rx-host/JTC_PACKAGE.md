# ROBOTIS JTC 장비 패키지·Host 등록

phase63. position JointTrajectoryController를 사용하는 자사 catalog 구성의 Template/Site를 DEVICE_REFERENCE v2 패키지로 작성·검증하고 Host 초기화에 연결한다. 현재 제품 factory의 JTC 실행 제공자는 미연결이다. 검사와 원장 준비는 가능하지만 `run`은 ROS client를 만들기 전에 `JTC_CONTROL_PROVIDER_NOT_CONFIGURED`로 실패한다.

## 작성 입력과 산출물

| 입력 | 내용 |
|---|---|
| Template | catalog SHA·support ID·controller, 논리 자원/조건 역할, 작업 slot·joint group·tool role, 실행/prepare timeout·Authority 관측 유효기간 |
| Site | Template digest, 설치/셀/target/site 구성, 실제 자원/조건 이름, 교정/도구 artifact, ROS namespace/manager/domain·통신 한도, slot별 Goal |
| Recipe | package 이름/version/publisher·Linux/Jazzy target |

Template은 현장 좌표·namespace·실제 자원 이름을 갖지 않는다. Site는 Template의 작업·역할 목록에 정확히 대응해야 한다. 누락/추가 slot, 중복 실제 자원/조건 alias, 잘못된 관절 순서, goal 시간보다 짧은 실행 timeout을 거부한다. Template action 순서와 calibration 목록처럼 의미 없는 순서는 정규화하지만 trajectory의 관절/point 순서는 보존한다.

현재 교정 자료는1–16개 `rx.robot-calibration.v1`, 각 tool role은 `rx.tool-definition.v1` ArtifactRef를 요구한다. 자료당1MiB, 전체32개 고유 asset 이내다. 동일 digest에 서로 다른 schema/size를 붙이는 모순은 거부한다. 파일 bytes/digest/크기 일치는 내용의 물리적 적합성이나 교정 완료를 증명하지 않는다. 첫 현장의 실제 자료가 없으면 commissioning은 완료되지 않는다.

slot별 Goal 원본으로 trajectory ArtifactRef를 계산한다. 현재 조립은 다음7개 payload를 만든다. 전부 데이터이며 실행파일·ROS launch 파일·환경 변수·임의 DLL 경로를 포함하지 않는다.

| 파일 | 역할 |
|---|---|
| family.json | catalog에서 확인한 model/support ID·환경 |
| profile.json | Host가 검사할 실제 JTC Profile |
| adapter.json | release의 JTC 구현 이름·소스 digest |
| authoring/assembly.json | Template/Site 원본 |
| operations.json | slot별 정확한 Intent·시간 제한·profile/site/tool/calibration/resource 참조 |
| outcomes.json | 같은 profile digest에 결합한 native 결과 대응표 |
| device-catalog.json | phase64 공통 작업 선언과 signed source 문서 참조; P 반입·조회용 |

manifest/signature를 더한 게시물은9파일이며 payload 합계는 현재2MiB 이내다. phase63의6-payload 패키지도 decoder에서 계속 읽지만 공통 선언 조회 자료는 없다. Profile의 개별 Goal 한도보다 패키지 전체 한도가 먼저 제한할 수 있다. 한도를 넘으면 분할이나 범위 재설계를 요청할 오류를 반환하며 일부 goal을 누락해 조립하지 않는다.

## 작성·검사 흐름

기존 `rx-device-package` 명령을 재사용한다. Template/assembly의 schema로 MELSEC와 JTC를 구별하며 미지 schema는 거부한다. 기존 MELSEC API와 `driver-identity` 출력도 유지한다.

```text
rx-device-package driver-identity jtc
rx-device-package template-digest TEMPLATE
rx-device-package assemble TEMPLATE SITE RECIPE CANDIDATE
rx-device-package request CANDIDATE KEY_ID REQUEST_FILE
# 외부 signer가 request의 원래 메시지 bytes에 서명
rx-device-package seal CANDIDATE SIGNATURE POLICY PACKAGE
rx-device-package inspect PACKAGE POLICY
```

개인키·외부 전송은 제품 CLI에 없다. 외부 서명 규칙은 [작성 도구](../rx-device-package/README.md)를 따른다. candidate 재읽기도 원본 조립과 manifest/payload를 대조하므로 중간 파일을 수정한 뒤 그대로 봉인할 수 없다.

검사기는 공통 서명·publisher·권한·내용 해시 검사 뒤 검증기가 소유한 불변 bytes를 해석한다. assembly를 재조립하여 family/profile/operations/outcomes를 다시 비교한다. 유효한 signer가 잘못된 model·timeout·결과표 또는 다른 release descriptor에 서명했어도 거부한다. 외부 calibration/tool asset 목록은 조립 원본과 정확히 같아야 한다.

JTC target은 Linux/Jazzy다. Host는 현재 CPU architecture와 base/cell/package ABI, pinned policy 및 선택한 manifest digest를 추가 검사한다. source descriptor는 Host/MC 소스, 공유 SDK lock, JTC C++ 연결 계층·catalog·자사 source lock 등을 포함한 빌드 소스 기준이다. 실제 binary·동적 library의 서명 또는 물리 controller identity와 동일하지 않으며 release 검증을 대체하지 않는다.

## Host 설정·초기화와 실행 경계

Host backend에는 `JTC_PACKAGE`와 package directory/manifest_digest/pinned policy를 지정한다. 같은 설치·셀·환경·조건 목록만 허용하며 Host의 각 allowed Intent는 operations.json의 정확한 Intent digest와 일치해야 한다. 패키지의 일부 작업만 선택할 수 있으나 timeout 등 의미를 바꾼 작업을 끼워 넣을 수 없다.

`rx-hostd drivers jtc`는 제품 executable이 기대하는 descriptor를 출력한다. `inspect`는 콘텐츠와 bindings를 검사하고 `control_provider=NOT_CONFIGURED`, activation=false, native_processes_started=0을 보여준다. `init`은 비공개 staging 디렉토리 안에 Host 원장과 native-jtc/native.sqlite3 및 원장 identity/manifest를 만든 뒤 원자 공개한다. 기존 설치를 덮어쓰지 않는다.

현재 Builtin은 JTC provider를 갖지 않는다. `run`은 제품용 Authority/lifecycle/fencing 제공자가 없다는 오류로 실패하며 READY/Arm/새 ROS client를 만들지 않는다. Site에 `safe=true`나 임의 실행 경로를 추가해 이 경계를 우회할 수 없다. 이는 provider 구현을 완료했다는 의미가 아니다.

다음에는 release 소유 provider를 연결해야 한다. 그 제공자는 실제 controller 세대·독점 제어권과 독립 보호/소재 지지, 세대별 endpoint 및 lifecycle을 확인해야 한다. 프로세스 재시작이나 bridge READY만으로 같은 controller가 유지된다고 추정하지 않는다. P의 패키지 반입/검토를 거쳐 operations/outcomes를 실제 셀 구성으로 연결하는 자동화도 후속이다.

phase64에서는 [P의 공통 작업 선언 반입·보관·조회](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/DEVICE_CATALOG.md)를 연결했다. 원본 source 상관과 현재 설치/셀/환경을 검사하지만 제조사 검증·검토 승인이나 실제 구성 적용은 수행하지 않는다. 이전 이미지의 exact source descriptor와 새 패키지를 호환한다고 추정하지 않는다.

## 검증과 제한

시험은 Template 재사용·정규화, 원본→candidate→외부 test 서명→검사, signed-but-inconsistent payload 거부, asset 변조·정확한 Intent와 Host metadata 초기화·provider 없는 run 거부를 다룬다. 컨테이너 검증은 같은 image의 실제 작성 executable을 사용하며 test signer는 호스트의 별도 시험 코드다. 정확한 수행 결과는 [phase63 기록](../../../references/implementation/phase63_checks.json)을 따른다.

이 단계는 실물 OMY/다른 자사 모델·그리퍼·leader/base/Sapiens 경로의 지원 인수가 아니다. 자료 선언을 실행 권한으로 바꾸지 않는다. [JTC 어댑터](ROBOTIS_JTC_ADAPTER.md), [결과 대응표](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/NATIVE_OUTCOMES.md), 두 이미지·필수 자사 지원 의무와 첫 물리 셀 NOT_COMMISSIONED를 유지한다.
