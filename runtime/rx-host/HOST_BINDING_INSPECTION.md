# Host 기동 설정 변경의 수동 검사

phase72의 [영속 준비·조회·취소](HOST_MAINTENANCE_PREPARATION.md)가 이 비교 다음에 연결된다. 비교와 준비 모두 실제 설치 교체나 native 운전은 수행하지 않는다.

phase71. 명령은 파일을 읽어 비교하며 Host 초기화, driver 기동, device 연결, native submit과 설치 파일 교체를 하지 않는다.

```text
rx-hostd inspect-binding-change PLAN CURRENT_CONFIG PROPOSED_CONFIG
```

PLAN은 P의 `rx.host-binding-plan.v1`이다. 두 설정은 기존 `rx.host-startup.v1`과 pinned 파일·TLS 검사를 그대로 사용한다. 검사 결과는 `rx.host-binding-inspection.v1` JSON이다. 형식·신뢰·패키지 취득 실패는 오류로 끝나며, 유효한 자료 사이의 변경 범위 불일치는 `software_matches=false`와 issues로 반환한다.

다음을 비교한다.

- 설치와 Host identity, P 계획에 있는 Host, 전체 영향 셀 cohort.
- 대상 셀의 정확한 Intent·condition 집합·definition·envelope·scope·environment.
- 다른 영향 셀의 Binding은 current와 동일해야 함.
- qualification ID/revision·purpose·platform peer는 유지됨. 파일 비교로 새 자격을 발급하지 않음.
- backend/bindings 이외의 기동 설정은 동일함. data/runtime 위치, release, TLS·publisher·network 변경은 별도 배포 계획이 필요함.
- 제안 backend는 같은 제품의 release-owned factory와 실제 서명 package decoder로 검사함.
- 장비 패키지의 manifest/signature와 정규화 catalog hash/size/schema가 P의 요구와 같음.

현재 native factory는 한 Host에 하나의 장비 패키지와 하나의 정확한 셀 binding을 요구한다. 여러 package나 추가 셀 변경은 지원된 것으로 표시하지 않는다. 이는 플랫폼 공통 artifact의 최대 범위와 구별한 backend의 현재 제한이다.

결과에는 plan digest, current/proposed installation identity, proposed bindings pin을 남긴다. `runtime_provider_available`은 해당 release의 factory 제공 여부이며 장비 준비나 자격 판정이 아니다. JTC는 소프트웨어가 맞아도 production control provider가 없으므로 false다.

`installation_changed=false`, `activation_authorized=false`, `native_processes_started=0`을 유지한다. 이 JSON은 독립적으로 서명된 검증 보고서나 durable Host 적용 receipt가 아니다. 후속 적용 시 파일·정책·현재 설치와 원장을 다시 확인해야 하며 검사 직후 파일이 계속 동일하다고 가정하지 않는다.

실제 JTC package·P 변경안·제품 CLI의 비교 통합과 qualification/storage 변경 거부를 검사했다. 별도 Host fixture 시험은 중복 작업으로 범위 확대, 누락 cohort와 다른 설치를 거부하고 data directory가 생성되지 않았음을 확인한다. 실물·production TLS 관리 작업·Host reload·복원은 미검증이다.
