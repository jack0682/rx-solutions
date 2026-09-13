# 공정 패키지 조립·외부 서명·내용 검증

2026-09-12. `rx-process-package`는 작성된 compile input을 결정적인 Process package 후보로 묶고, 외부 detached signature를 기존 package verifier에 연결한다. 서명 키 생성·보관이나 P trust 등록·배포·실행을 수행하지 않는다.

## 단계와 명령

```text
rx-process-package assemble BUNDLE RECIPE NEW_CANDIDATE_DIRECTORY
rx-process-package request CANDIDATE KEY_ID NEW_REQUEST_FILE
rx-process-package seal CANDIDATE SIGNATURE POLICY NEW_PACKAGE_DIRECTORY
rx-process-package verify PACKAGE POLICY
rx-process-package compile PACKAGE POLICY NEW_OUTPUT_DIRECTORY
```

- assemble은 digest가 맞는 원문/바인딩을 기존 S compiler로 검사하고 `UNSIGNED_CANDIDATE`를 만든다.
- request는 key ID와 canonical manifest에 결합한 실제 signing message의 hex/digest를 기록한다. 사람이 검토하거나 외부 signer/HSM에 전달할 수 있다. 이 명령은 메시지를 외부로 보내거나 개인키를 읽지 않는다.
- seal은 외부 서명과 명시적 로컬 trust policy를 검증하고, 내용 의미를 다시 확인한 뒤 새 디렉터리에 완성한 패키지를 공개한다.
- verify/compile은 서명·현재 trust·파일·target/계약·잠긴 의존성·artifact를 검사하고 검증된 immutable bytes에서 다시 공정을 구성한다.

검증/컴파일 결과는 `CONTENT_VERIFIED_NOT_QUALIFIED`다. VerifiedPackage 또는 서명이 field qualification, native 준비, 운영자 승인, dispatch permit를 의미하지 않는다.

## 파일 구성과 결정성

| 파일 | 내용 |
|---|---|
| manifest.json | 기존 `rx.package.v1` manifest와 정확한 inventory |
| manifest.sig.json | 봉인 후에만 존재하는 Ed25519 detached signature |
| process/source.json | 공정 source |
| process/bindings.json | 정규화 검사를 거친 Host+intent 입력 |
| authoring/compile-input.json | 원래 source/binding/cell/catalog provenance |
| authoring/package-recipe.json | package/version/publisher/contract/target/dependency/asset 선택 |
| process/context-requirements.json | profile/site/calibration/tool/mode/stream 및 catalog digest의 연결 의무 |

같은 입력과 recipe는 같은 manifest digest를 만든다. 후보를 다시 취득할 때 재조립한 manifest와 모든 file bytes를 대조한다. 서명 검증 후에도 이 대응을 재확인하여, 서명은 유효하지만 패키지 내부의 원문과 바인딩이 다른 경우를 거부한다.

파생 resolved/BT 파일은 원본 패키지에 자기 package digest와 함께 넣지 않는다. 검증된 패키지에서 다시 컴파일하고 결과의 package_digest를 실제 manifest digest에 연결한다. 순환하는 자기 hash를 만들지 않는다.

## 권한과 외부 참조

요청 permission은 실제 공정에서 쓰는 OperationSubmit 및 조건/control source의 ObservationRead schema, ArtifactRead에서 도출한다. Process package에 NativeEndpoint 또는 executable file을 넣지 않는다. 쓰지 않는 추가 action binding을 조용히 승인하지 않는다.

trajectory/program/parameter-set/procedure 같은 ArtifactRef는 recipe의 assets에 같은 metadata로 선언돼야 한다. 같은 digest의 다른 metadata는 거부한다. profile/site/calibration 등 semantic digest는 단순 파일 SHA로 위장하지 않고 별도 context requirements로 남긴다. 이것들은 실제 device/profile/site authority에서 확인해야 한다.

현재 ActionBinding은 Host+intent다. 전체 Cell StepBinding의 조건·완료·절차 의미와 실제 장비 package를 조립·활성화하는 경로는 후속이다. 이 도구는 그 검증을 생략한 완성 셀 패키지를 주장하지 않는다.

## 검증 정책과 외부 signer

정책 파일은 단일 ABI의 `rx.package-verification-policy.v1` 또는 명시적인 추가 ABI 목록을 가진 `rx.package-verification-policy.v2`다. 계약/target, 명시적 key ID·publisher·공개키·허용 package kind/permissions, 확인할 asset 파일과 잠긴 dependency 경로를 지정한다. CLI 호출자가 제공하는 로컬 검증 기준이며, 이 파일을 P의 운영 trust로 자동 등록하지 않는다. v2의 `additional_package_abis`는 기본 ABI 이외의 1–8개를 중복 없이 지정한다. 장비 ABI v2와 공정 ABI v1을 한 정책으로 허용해도 signer 종류·권한과 내용 검증은 각각 유지한다.

asset 파일은 크기와 실제 SHA-256을 확인한다. dependency 디렉터리는 bounded capability 취득으로 읽고 지정 manifest digest를 확인한 뒤 순서대로 검증한다. cycle/missing dependency, revoked 또는 범위를 벗어난 key는 통과하지 않는다. 단순히 과거에 검증한 객체라는 이유로 현재 정책 검사를 건너뛰지 않는다.

정책 한도는 key128개, asset1024개/총256 MiB, dependency128개다. 이 CLI의 개별 패키지 취득은 파일32개/내용4 MiB 범위이며 더 큰 장비 package/streaming asset을 처리하는 배포 도구를 대체하지 않는다.

실제 production signing service와 운영 trust 공급/회수 절차는 아직 연결하지 않았다. 테스트는 고정된 test-only 키를 시험 코드 안에서만 사용하고 서명·공개 정책·예상 signing message만 내보낸다. 기본 제품 키나 private key 파일은 생성하지 않는다.

## 취득과 출력

공통 `rx-package::directory::acquire_directory`는 검증 전 owned bytes를 반환할 뿐이다. 이 타입/경로로 VerifiedPackage를 만들었다고 표시할 수 없다. 기존 symlink/특수파일/경로/수량·크기 제한을 유지한다. 실제 verify_directory는 취득 후 원래 서명·내용 검증을 계속 수행한다.

출력은 장비 작성 도구와 공유하는 SDK publish_files를 사용한다. 같은 parent의 임시 디렉터리에서 완성·동기화한 뒤 Linux/macOS의 no-replace rename으로 공개한다. 기존 파일/디렉터리·symlink를 덮어쓰지 않는다. Windows에서의 atomic publication backend와 host-root 공격 방어는 이 단계의 검증 범위가 아니다.

## 검증 범위

결정적 조립·외부 서명·불변 bytes 재컴파일, signer 종류/권한·key alias, 파일/inventory 변경, signed-but-inconsistent 내용, 미선언 artifact·병렬 자원 충돌, 의존성의 현재 trust, asset 내용 변경과 no-overwrite 출력을 시험한다. 실제 작성했던 공정을 최종 이미지에서 조립→서명 요청→test-only detached signature 봉인→검증→컴파일하는 증거도 별도로 남긴다.

package 검토 승인, P artifact admission·활성화, 완전한 device/site context 검증, operator UI의 검증/서명/배포 흐름과 현장 인수는 남아 있다. 첫 물리 셀은 NOT_COMMISSIONED다.

P/S의 로컬 trust policy 취득은 SDK `rx-package::policy`를 공유한다. P의 별도 보관 경계와 오프라인 도구는 [STORE.md](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-package/STORE.md)에 설명한다. P 보관 성공은 이 crate의 공정 의미 재검증이나 사용자/셀 승인으로 승격되지 않는다.

## Investigation 절차 작성과 외부 서명

```text
rx-process-package investigation-assemble INPUT_JSON NEW_CANDIDATE_DIRECTORY
rx-process-package investigation-request PROCEDURE_JSON KEY_ID NEW_REQUEST_FILE
rx-process-package investigation-seal PROCEDURE_JSON SIGNATURE_JSON PUBLIC_KEY_HEX NEW_OUTPUT_DIRECTORY
```

이 경로는 shared `investigation::Procedure`를 검증하는 독립 문서 도구다. 기존 PackageKind를 확장하거나 investigation을 Process package로 포장하지 않는다. 절차의 액션·scope·역할 등 의미 검증은 shared contract를 그대로 사용한다.

- `investigation-assemble`은 bounded strict JSON 입력을 decode/validate하고 새 디렉터리에 canonical `procedure.json` 하나만 쓴다. 결과는 서명 없는 후보이며 `signature_verified=false`, `usage_authorized=false`다.
- `investigation-request`는 assemble이 만든 정확한 canonical bytes를 읽고 key ID와 procedure에 결합한 shared contract의 signing message를 만든다. 파일은 `{schema,key,procedure,message_digest,message_hex}`이며 schema는 `rx.investigation-signing-request.v1`이다. 외부 signer에 전달할 입력만 기록하며 private key 입력·서명 생성·외부 전송은 없다.
- `investigation-seal`은 정확한 canonical procedure와 strict `SignatureEnvelope {key,signature}`를 읽는다. 64자리 lowercase hex 공개키로 실제 Ed25519 detached message를 검증한 뒤 `<procedure.sha256>.json`과 `<procedure.sha256>.sig.json` 두 파일을 새 디렉터리에 게시한다. procedure 파일은 검증한 원문 bytes 그대로다. key ID도 signing message에 결합되므로 다른 alias로 envelope를 바꾸면 실패한다.

`PUBLIC_KEY_HEX`는 **서명 수학 검증에만** 사용한다. CLI가 받은 키를 P 배포 policy의 trusted key로 등록하거나, signer 역할·현재 cell의 절차 사용·case·운전 조건을 승인하지 않는다. seal의 stdout은 `SIGNATURE_VERIFIED_NOT_AUTHORIZED`, `signature_verified=true`, `deployment_policy_verified=false`, `usage_authorized=false`와 `public_key_check=SIGNATURE_VALIDITY_ONLY`를 명시한다. P의 독립 trust 정책과 실제 사용 시 권한·조건 검사는 별도다.

입력은 기존 bounded trust/relative-file primitive로 읽으며 unknown/duplicate JSON 필드·불일치 서명·변조·비정규 procedure bytes를 거부한다. 디렉터리 출력은 기존 `publish_files`의 no-replace publication을 사용하고 서명 요청 파일도 create-new만 허용한다. 기존 파일/디렉터리·symlink를 덮어쓰지 않는다. 어느 명령도 Runtime API, planner 또는 native 동작을 호출하지 않는다.

## 검토 자료 출력

`validator-identity`, `review PACKAGE POLICY REQUEST OUT`, `review-signing-request REPORT KEY_ID OUT`을 제공한다. 실제 signed package를 compile_verified로 검사해 canonical 검토/결과 자료를 만들고 외부 서명자가 정확한 bytes에 서명하도록 한다. 서명 서비스/개인키는 포함하지 않는다. P의 독립 검사와 승인 범위는 [공정 검토](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/PROCESS_REVIEW.md)를 참조한다.
