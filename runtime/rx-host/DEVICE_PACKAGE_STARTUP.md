# 서명된 장비 패키지와 제품 Host 기동

작성 2026-09-12. `MELSEC_PACKAGE`를 제품 `rx-hostd`의 고정 factory에 연결했다. 패키지 검증·원장 초기화·passive open과 기존 자격 수용/Arm 경계를 구분한다. 첫 실제 레이저 셀의 commissioning은 아직 완료되지 않았다.

## 1. 제품에서 선택하는 것

제품 설정의 backend는 다음 세 형태다.

| kind | 현재 의미 |
|---|---|
| FILE_SIMULATION | 기존 파일 장비 모의 backend |
| MELSEC_PACKAGE | 서명된 DEVICE_REFERENCE 패키지의 제한 MELSEC EnsureState backend |
| VALIDATED_DRIVER | 과거 예약 형태. 일반 드라이버 레지스트리를 구현한 것으로 취급하지 않고 계속 거부 |

MELSEC_PACKAGE에는 **절대 package directory, 예상 manifest digest, 독립 policy 파일의 경로와 SHA-256**을 넣는다. 사용자 경로로 실행파일이나 공유 라이브러리를 불러오지 않는다. 선택 가능한 구현은 컴파일된 `rx.melsec.ensure-state.v1` 하나이며, 실제 모델 목록은 현재 Q03UDVCPU로 제한한다. 이것이 실제 Q03UDVCPU 현장 검증 완료를 뜻하지는 않는다.

`rx-hostd drivers`는 설정 파일 없이 현재 구현 descriptor를 JSON으로 출력한다. `inspect CONFIG`는 패키지/정책/정적 binding을 검증하고, `init CONFIG`는 새 설치를 만들고, `run CONFIG`는 기존 설치만 연다. 프로그램 내에서 임의의 plugin path를 실행하는 기능은 없다.

## 2. 패키지와 독립 정책

새 [DEVICE_REFERENCE v2](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-package/DEVICE_REFERENCE.md)는 `rx.package.v2`와 `rx.package-abi.v2`를 사용한다. 기존 DEVICE v1의 실행파일 요구를 완화하지 않았다.

MELSEC resolver는 세 개의 기본 non-executable 파일을 받는다. 새 작성 도구가 만든 형식은 서명된 `authoring/assembly.json` 네 번째 파일을 포함하며, Host가 Template/Site에서 profile을 재조립해 비교한다.

| 역할 | schema / 값 | 검사 |
|---|---|---|
| family | rx.melsec-family.v1 | mitsubishi/melsec-q, Q03UDVCPU, profile과 같은 환경 |
| profile | rx.melsec-predicate-profile.v1 | 설치·셀·주소·자원·관측·상태/완료 계약과 exact Intent |
| adapter descriptor | rx.native-driver-reference.v1 | 고정 구현명과 현재 컴파일된 source digest |

`profile`의 세부 의미와 9-word publication 전제는 [어댑터 명세](MELSEC_ADAPTER.md)를 따른다. 현재 패키지는 설치별로 고정된 Profile 하나를 포함한다. [rx-device-package](../rx-device-package/README.md)가 MELSEC Template과 Site를 분리해 작성하고 concrete 설치 패키지로 조립한다. 다른 제조사의 authoring schema는 후속이며 이 형식을 모든 장비의 최종 모델로 확정하지 않는다.

정책은 패키지 외부의 pinned 파일이다. 기존 `rx.package-verification-policy.v1`의 trusted publisher/key/kind/permission과 asset 획득 기능을 재사용한다. 정책의 base/cell hash는 binary에 포함된 현재 기준판과 일치해야 하고, target은 현재 CPU architecture의 Linux/ROS 불필요 조합이어야 한다. 현재 dependency package는 허용하지 않는다.

정확한 permission 집합은 ArtifactRead, NativeEndpoint(role=melsec-mc3e), ObservationRead(schema=rx.melsec.status-image.v1)다. 다른 permission이나 실행 가능 payload가 있으면 거부한다. content inventory는 기본 세 파일 또는 서명된 assembly를 더한 네 파일이며, acquisition은 최대 8개 파일/2MiB content로 제한한다. 정책 asset은 최대32개·개별1MiB 범위이고 실제 bytes/size/digest를 확인한다.

publication contract와 PLC program evidence의 원본 asset이 있어야 한다. 참조만 있고 bytes가 없거나 바뀌면 실패한다. **원본 bytes가 존재하고 서명이 맞는다는 사실은 그 문서의 물리적 주장이 참임을 증명하지 않는다.** 프로그램과 실제 PLC의 일치, 원자 관측·freshness·debounce·독립 보호는 별도의 실제 qualification에서 판단해야 한다. profile의 `plc_program` 참조는 이 resolver에서 program evidence artifact에 연결하며, MC로 PLC 프로그램을 읽어 hash를 대조하는 기능은 없다.

## 3. 구현 source pin의 범위

빌드 시 Host/MC driver의 source file, 해당 Cargo manifest, workspace Cargo.lock, Host build script, SDK source-lock을 정렬해 source digest를 만든다. descriptor의 값이 이 digest와 정확히 맞아야 한다. macOS에서 작성/검사한 Linux target 패키지도 같은 source inventory라면 Linux image에서 같은 digest를 사용한다.

이 값은 **source identity**다. OCI image digest, 컴파일러 재현성 증명, binary 서명이나 ABI 호환 판단을 대신하지 않는다. 운영 release의 image/설정 pin은 별도로 필요하다. 현재는 엄격한 exact match이므로 관련 source/dependency 변경 후 descriptor와 패키지 서명을 갱신해야 한다. 자동 호환 추정·native 설치 migration은 구현하지 않았다.

## 4. 초기화와 재시작

초기화 전 package/policy와 단일 셀 binding을 검사한다. profile installation ID는 Host 설치 ID와 같아야 하며, 모든 allowed Intent가 profile과 정확히 맞아야 한다. conditions와 환경도 대조한다.

새 staging 디렉토리에 Host journal과 MELSEC native journal을 만들고, `installation.json`에 두 Host journal ID 및 native journal/profile/manifest identity를 함께 기록한다. 이 단계는 PLC에 연결하거나 명령을 보내지 않는다. 파일·디렉토리를 동기화한 뒤 **no-replace rename**으로 설치를 공개한다. 동시에 누군가 만든 빈 목적 디렉토리도 덮어쓰지 않는다. 실패한 비공개 stage는 정리하며 이미 공개된 설치를 오류 복구 명목으로 삭제하지 않는다.

run은 package/policy를 다시 검증하고, 기존 설치·Host/native journal·manifest/profile identity와 소유 lock을 대조한다. native DB가 없거나 비었으면 새 원장을 만들어 채우지 않는다. passive open은 TCP 연결도 하지 않으며 실제 status read까지 연결을 지연한다.

기존 FILE_SIMULATION 설치의 native metadata는 optional이어서 계속 읽을 수 있다. MELSEC 설치에서는 해당 metadata가 필수다. 다른 패키지/manifest로 설정을 바꾸어 기존 native installation을 자동 재사용하는 경로는 없다. 업데이트·복원·controlled rebind는 후속 작업이다.

검증은 현재 프로세스의 inspect/init/run 시점에 수행한다. 실행 중 파일·키 회수를 감시하는 trust service는 아직 연결하지 않았다. 운영 중 회수에는 Platform의 권한 철회/차단과 정상 정지 절차가 필요하며, pinned 정책을 바꿨다는 이유만으로 이미 실행 중인 프로세스가 즉시 정지한다고 주장하지 않는다.

## 5. 패키지 검증과 운전 자격의 경계

PHYSICAL binding은 startup의 `qualification` ID/revision만으로 Arm할 수 없다. 현재 Host boot·journal·binding·process context에 대해 인증된 Platform이 전달한 qualification acceptance가 있어야 한다. 그 후에도 별도 Arm, 현재 grant/fence/permit와 native guard가 필요하다. 같은 규칙을 다른 물리 NativeAdapter에도 적용하며 simulation fixture의 기존 bootstrap 방식은 유지한다.

```mermaid
flowchart LR
    A[서명·원본·source pin 검사] --> B[Host와 native 원장 초기화]
    B --> C[SOFTWARE_READY_UNARMED]
    C --> D[현재 process context 수용]
    D --> E[현재 qualification 수용]
    E --> F[별도 Arm과 작업 허가]
    F --> G[최종 native guard 뒤 명령]
```

`physical_backend_available=true`는 선택한 패키지의 물리 backend 구성이 지원됨을 뜻한다. `physical_qualification_verified=false`, `activation_authorized=false`는 그대로 표시한다. 화면에서 “장비 운전 준비 완료”로 합쳐 표시하면 안 된다. startup 검사 자체가 qualification artifact의 전문 판단을 수행하는 것은 아니다.

## 6. 검증과 남은 작업

패키지 검증 시험은 잘못된 signature/content/policy/검증 시점에 회수된 key/asset/contract, 유효하게 다시 서명된 잘못된 source·controller와 설치 불일치를 거부한다. 초기화 충돌 시험은 동시에 생긴 빈 목적 디렉토리도 보존함을 확인한다.

Host library 시험에서 PHYSICAL **metadata**와 loopback PLC를 사용해 startup ID만으로 Arm 거부, 현재 context/qualification 수용 뒤 별도 Arm과 명령·완료를 확인한다. 이 자료·서명자·qualification 요청은 명시적 테스트용이다. 실제 현장에 PHYSICAL qualification을 부여하지 않는다.

제품 image 시험은 실제 `rx-hostd`에 서명된 모의 package를 입력한다. network-none·non-root·read-only root에서 init/passive run, native journal identity, mTLS h2, 중복 owner 거부, SIGTERM 후 별도 drop proof를 검사한다. Python PLC는 시험이 직접 띄운 loopback 보조 프로세스이며 패키지가 실행하는 코드가 아니다. 이 image 시험의 native write 수는 0이다.

실제 명령·결과·source/image hash는 [phase58 검증 기록](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase58_checks.json)에 둔다. 물리 장비의 publication/프로그램 확인·현장 인수, 다른 제조사 device package authoring와 작성 UI, 운영 중 trust 회수 감시, image/패키지 변경과 native 원장 migration, 전체 장애 복구·보관 정책 및 장비별 backend 연결은 남아 있다.
