# 장비 템플릿·현장 설정·서명 패키지 작성

2026-09-12. `rx-device-package`는 재사용할 장비 의미와 현장 연결 값을 분리하고, 제품 Host가 읽는 불변 DEVICE_REFERENCE 패키지로 조립한다. 현재 MELSEC Q03UDVCPU의 제한 EnsureState와 ROS position JTC 패키지를 작성한다. 아래는 MELSEC 입력 설명이며, JTC의 Template/Site·operations/outcomes·Jazzy target과 실행 제공자 미연결 경계는 [JTC 패키지 명세](../rx-host/JTC_PACKAGE.md)를 따른다. 다른 로봇·장비까지 작성할 수 있는 범용 schema로 완료 표시하지 않는다.

이 도구는 파일만 읽고 쓴다. PLC/로봇에 연결하거나 개인키를 읽지 않으며, 서명 메시지를 외부로 보내거나 trust·qualification·운전 허가를 설치하지 않는다.

phase65에서는 `validator-identity`, `review PACKAGE POLICY REQUEST OUT`, `review-signing-request REPORT KEY_ID OUT_FILE`로 장비 패키지 소프트웨어 보고서를 생성한다. 실제 decoder의 원본 재조립/선언 검사를 수행하고 원래 P 요청과 package/catalog를 결합한다. 서명은 별도이며 physical validation은 NOT_PERFORMED다. 정책 fingerprint는 원래 반입 정책을 대조하고, 파일 취득에는 추가적인8 payload/2MiB 한도를 적용한다. [검증·서명·독립 승인 API](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/DEVICE_REVIEW.md)를 따른다.

## 작성자가 나눠 관리할 입력

| 입력 | 담는 것 | 담지 않는 것 |
|---|---|---|
| Template | 모델, version, 논리 자원 역할·command slot, 조건/관측 이름과 bit 의미, publication 계약, 완료/settle 규칙 | 현장 IP, PLC M/D 주소, 설치·셀 ID |
| Site | 선택한 template digest, 설치·셀/target/site configuration·calibration, 역할→실제 자원, slot→M 주소, status D 주소, 접속 설정·PLC program evidence | 공통 완료 의미의 임의 덮어쓰기 |
| Recipe | package 이름/version/publisher와 Linux target | 개인키, trust 등록, dispatch 권한 |

Template의 `resource_roles`와 predicate의 `command_slot`이 Site에 정확히 한 번씩 연결되어야 한다. 누락·추가·중복 실제 자원·겹치는 command 주소를 거부한다. 범용 문자열 치환이나 임의 JSON patch로 profile을 만드는 방식이 아니다.

Template digest는 정규화한 구조의 semantic digest다. 원본 파일의 단순 SHA-256과 다르므로 아래 명령으로 구한다. publication/program artifact의 `sha256`은 실제 bytes의 SHA-256이다. Site의 `site_config`와 calibration digest는 별도 권위가 확인할 참조이며 이 도구가 실제 현장 구성/교정을 입증하지 않는다.

같은 Template을 유지하면서 Site의 endpoint·주소·실제 자원 이름을 바꿀 수 있다. 새 Template 의미를 사용하려면 Site도 해당 새 digest를 명시해야 한다. 자료 구조에서 순서가 의미 없는 resource role, predicate, calibration, target 집합은 정규화하여 순서만 바뀐 경우 서명 메시지가 달라지지 않는다.

## 명령 순서

```text
rx-device-package driver-identity
rx-device-package template-digest TEMPLATE
rx-device-package assemble TEMPLATE SITE RECIPE NEW_CANDIDATE_DIRECTORY
rx-device-package request CANDIDATE KEY_ID NEW_REQUEST_FILE
rx-device-package seal CANDIDATE SIGNATURE POLICY NEW_PACKAGE_DIRECTORY
rx-device-package verify PACKAGE POLICY
rx-device-package inspect PACKAGE POLICY
```

1. `driver-identity`로 현재 도구/Host release가 가리키는 고정 구현을 확인한다. 대상 image에 포함된 도구로 패키지를 작성하면 source identity가 같은 release를 사용한다.
2. Template을 작성하고 `template-digest` 결과를 Site의 `template_digest`에 넣는다. 결과 `TEMPLATE_STRUCTURE_VALID`는 구조 검사 통과이며 물리 qualification이 아니다.
3. Site와 Recipe를 작성하여 `assemble`한다. 주소·환경·bit/settle·자원·시간 한도를 기존 Host Profile validator로 검사하고 `UNSIGNED_CANDIDATE`를 만든다.
4. `request`는 key ID와 canonical manifest에 결합한 signing message의 hex와 digest를 새 파일에 저장한다. 실제 외부 signer/HSM이 검토 후 이 메시지에 서명해야 한다.
5. 외부에서 받은 `SignatureEnvelope`와 독립 로컬 정책으로 `seal`한다. 서명·publisher/kind/permission·계약/target·원본 asset을 확인하고 `CONTENT_VERIFIED_NOT_QUALIFIED` 패키지를 새 디렉토리에 공개한다.
6. `verify`는 현재 policy와 bytes를 다시 검사한다. `inspect`는 같은 검증 후 해석된 profile과 digest·설치/셀/환경을 보여준다.

외부 signer는 요청의 `message_hex`를 hex 디코딩한 **원래 메시지 bytes 전체**에 Ed25519로 서명한다. hex 문자열 자체나 `message_digest` 값에 서명하는 것이 아니다. `message_digest`는 전달 중 메시지 일치를 확인할 SHA-256이다. signing message는 key ID와 canonical manifest에 결합되므로 반환 key ID도 요청과 같아야 한다.

반환 파일은 다음 두 필드의 `SignatureEnvelope` JSON이다. `signature`는 Ed25519 원시 서명 64바이트를 **소문자 hex 128글자**로 표현한다. 아래 대괄호 부분은 형식 설명이며 실제 유효한 서명이 아니다.

```json
{"key":"publisher/key-id","signature":"<128 lowercase hexadecimal characters>"}
```

개인키·base64·DER·추가 schema 필드를 넣지 않는다. 공개키와 publisher/kind/permission은 별도 Policy의 같은 key ID에 등록되어 있어야 한다. 생성한 후보와 정확한 요청 내용을 검토하는 책임은 signer에게 있으며, 이 CLI가 그 검토를 대신하지 않는다.

결과 디렉토리나 서명 요청 파일이 이미 있으면 덮어쓰지 않는다. 디렉토리 출력은 공정 패키지와 공유하는 `rx-package::directory::publish_files`의 동기화·no-replace 공개를 사용한다. `candidate`처럼 현재 디렉토리의 단순 상대 출력 이름도 지원한다. Linux/macOS 이외의 원자 공개 backend는 아직 지원하지 않는다.

## 서명되는 파일과 검증

| 파일 | 역할 |
|---|---|
| manifest.json | package v2 identity, ABI, target, permission·asset와 file inventory |
| manifest.sig.json | 봉인 후의 외부 Ed25519 signature |
| family.json | 모델·환경 |
| profile.json | Template과 Site에서 생성한 기존 Host Profile |
| adapter.json | 제품에 고정된 구현명과 source digest |
| authoring/assembly.json | 정규화한 Template과 Site 원문; 서명 inventory에 포함 |

후보 디렉토리에만 `candidate-recipe.json`이 추가된다. 후보를 다시 읽을 때 이 Recipe와 서명 대상 assembly에서 모든 파일·manifest를 재조립해 실제 bytes와 비교한다. 봉인 결과에는 후보 Recipe를 넣지 않으며 package 이름/version/publisher/target은 서명된 manifest 자체에 남는다.

Host resolver는 4파일 형식의 `authoring/assembly.json`을 다시 해석하여 family/profile 및 asset metadata와 비교한다. **서명은 유효하더라도 원문과 profile이 서로 다르면 거부한다.** 원문을 단순 참고 주석으로 보관하는 방식이 아니다. 기존 3파일 DEVICE_REFERENCE 형식도 계속 해석하지만, 그 형식에는 이런 재조립 원문이 없다는 차이가 있다.

generated profile digest는 아직 설치·주소를 포함한 concrete profile의 digest다. Template digest와 서로 바꾸어 사용하지 않는다. 다른 현장에 같은 Template을 적용하면 Template digest는 같고 concrete profile/package digest는 달라진다. 기존 Host/Intent 프로토콜의 digest 의미를 이번 도구에 맞춰 바꾸지 않았다.

## 정책·원본·보장 범위

정책은 기존 `rx.package-verification-policy.v1`이며 package 외부의 명시적 입력이다. key128개, asset32개·개별1MiB 범위, package dependency 없음, Linux/ROS 불필요 target과 현재 base/cell hash·package ABI v2를 요구한다. 실제 asset bytes/크기/hash를 확인한다. authoring은 해당 policy target에 맞는 package를 검사하며 실제 Host startup은 실행 중인 CPU architecture와도 대조한다.

publication 계약과 PLC program evidence의 존재/서명을 검사하는 것과 그 물리적 참을 입증하는 것은 다르다. 원자 status image·sensor freshness·PLC debounce·독립 보호 및 실제 프로그램 일치는 여전히 현장 qualification 대상이다. test-only 입력·키를 production trust로 등록하지 않는다.

도구에는 기본 운전 key, 자동 서명, 자동 trust 갱신, 자동 배포, Host 초기화·Arm·명령 기능이 없다. 작성/검사 성공에서 운전 허가가 파생되지 않으며 출력의 `activation_authorized`는 false다. 완성한 패키지를 제품에 연결하는 절차는 [Host package startup](../rx-host/DEVICE_PACKAGE_STARTUP.md)을 따른다.

## 검증과 후속

시험은 한 Template의 두 Site 재사용, 정확한 role/slot 범위, 순서 정규화, 후보 재취득·현재 key/asset/permission, 서명된 원문/profile 불일치, 실제 CLI의 조립→서명 요청→외부 test signer→봉인→검사와 no-overwrite를 다룬다. 이미지 시험은 실제 `/opt/rx/bin/rx-device-package`를 network-none/non-root/read-only root로 실행한다. 키는 테스트 harness에만 있고 제품 CLI는 signature만 받는다.

실행 결과·source/image hash는 [phase59 검증 기록](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase59_checks.json)에 둔다. 편집 UI, 일반 제조사 template registry, 실제 장비·모드별 authoring, P의 장비 검토·배포/변경·복구 흐름과 현장 인수는 남아 있다. 첫 물리 셀은 NOT_COMMISSIONED다.
