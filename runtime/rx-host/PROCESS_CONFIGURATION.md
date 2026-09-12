# Host 공정 문맥 확인·영속 receipt

2026-09-12. 새 optional `rx.host.configuration.v1`는 Host에 공정 구성 문맥을 기록하는 계약이다. **PLC/로봇/driver 설정이나 프로그램을 적용했다는 receipt가 아니다.** 기존 Host Binding이 제공하는 의도·조건·definition/envelope/environment 범위 안에서 P가 지정한 구성 digest를 수용하고, 운전 자격이 없는 상태로 유지한다.

## 범위와 상태

- `APPLIED_UNQUALIFIED`: 해당 Host의 공정 문맥 record들이 한 transaction으로 기록됐다. 전체 P ChangeRecord가 적용됐다는 뜻이 아니다.
- `NOT_APPLIED`: 동일한 요청에 대해 미지원 binding 변경, 남은 Host work 또는 확인할 수 없는 quiescence 등의 거부를 영속 기록했다. 일부 셀을 먼저 적용하지 않는다.
- RPC/storage 오류: 위 둘 중 하나라고 추정하지 않는다. 보낸 ID/body를 유지하고 Lookup으로 확인한다. Lookup에 receipt가 없다는 사실도 이전 요청이 실행 경계에 도달하지 않았다는 일반 증명이 아니다.

효과는 `INSTALLED`, `ALREADY_PRESENT`, `NONE`으로 구별한다. 새 요청이 이미 같은 target context를 기대하고 확인한 경우 ALREADY_PRESENT를 기록할 수 있다. 같은 요청의 단순 재전송은 원래 sequence/receipt를 반환한다.

## 요청의 결합

request ID와 change/preparation slot, plan digest, Host ID·예상 boot/journal, 현재 Binding fingerprint, 각 관리 셀의 fence/epoch/scope와 before/after configuration digest를 결합한다. 모든 Host 관리 셀을 포함해야 하며 부분 cohort는 거부한다. RPC로 쓸 때 해당 셀들이 모두 현재 base session에서 cell 계약을 협상했어야 한다.

Binding fingerprint는 현재 승인 Binding의 의도 정규화/정렬과 scope/condition 집합을 포함한다. Host가 관리하는 실제 승인 범위에 필요한 intent/condition이 포함되지 않거나 definition/envelope/environment가 다르면 수용하지 않는다. 새 low-level binding/driver를 이 API로 설치할 수 없다.

`expected_context`는 Inspect로 읽은 현재 Host 공정 문맥 digest 또는 부재(null)다. 이전 문맥을 몰랐던 Host가 P의 before_configuration을 자체 검증한 것으로 취급하지 않는다. 최초 before hash는 인증된 P의 계획 참조이고, 실제 Host의 비교 조건은 expected_context와 승인된 Binding/fence다. 이후 문맥이 있으면 기대한 digest가 정확히 맞아야 한다.

같은 request ID의 다른 의미, 같은 change/preparation slot을 다른 ID로 바꾼 재적용, 잘못된 boot/journal/fence는 거부한다. 현재 Platform peer/session 인증은 cached receipt 반환 전에도 확인한다.

## 적용 전 확인

1. Host의 명령 gate lock을 소유하고 현재 caller를 검사한다.
2. 현재 관리 셀 전체와 Binding fingerprint, scope 구조를 검사한다.
3. 각 셀의 현재 blocked 상태와 정확한 fence receipt의 boot/journal/epoch/scope/block 집합을 확인한다. 이미 Arm된 셀에서는 적용하지 않는다.
4. Host 전체에 PREPARED/SEND_ENTERED/NATIVE_ACCEPTED work가 남으면 미적용을 기록한다. 기존 work나 outcome을 성공으로 바꾸거나 삭제하지 않는다. 준비된 작업의 적절한 void/조정은 기존 계약으로 수행해야 한다.
5. adapter의 read-only handover_snapshot으로 Host 전체 resource 집합의 no-pending/control-available/support-stable 상태를 읽는다. 기본 구현은 확인 불가이므로 통과하지 못한다. 관측 age+uncertainty가100ms 이내여야 한다.
6. context·slot·receipt·delivery sequence/history를 같은 SQLite transaction으로 기록한다. 기록 실패 시 전체가 rollback된다. 이 과정에서 native submit/lookup이나 driver configure를 호출하지 않는다.

receipt의 `recorded_at`은 transaction 직전 검사 시각이다. 물리 상태가 이후에도 계속 유지된다는 보장은 아니다. P는 적용 직전 자신이 요구하는 현재성·영향 scope·작업/지지 처분을 다시 확인해야 한다. 서명이 있는 물리 안전 검증이나 장비 정지 증명으로 확대하지 않는다.

## 적용 뒤 gate와 재시작

process-context record가 있고 현재 [자격 수용](QUALIFICATION_ACCEPTANCE.md)이 없으면 Arm과 native 진입 검사를 거부한다. 프로세스 재시작이나 새 grant, 기존 Arm request 재전송으로 운전 자격을 복원하지 않는다. Host 자격 수용을 연결했으며 P의 전역 자격 활성화는 후속이다.

Lookup은 원래 receipt의 boot/journal/sequence를 보존하고 현재 Host snapshot을 함께 반환한다. `context_matches_current_host`는 현재 boot/binding/epoch/scope와 마지막 적용 context의 request/sequence/digest가 같은지를 비교한 metadata 판정이다. 새 물리 quiescence 관측이나 실행 권한이 아니다. 과거 boot의 receipt는 이 값이 false다. `activation_authorized`는 항상 false다.

commit 뒤 응답을 잃거나 프로세스가 종료되어도 같은 요청으로 receipt를 조회한다. Host 재시작 뒤 현재 문맥 확인이 필요하면 현재 Inspect/fence/expected_context와 새 change 준비 slot을 사용해야 하며, 이전 receipt를 현재 ack로 이름만 바꿔 제출하지 않는다.

## 전송

동일 mTLS 서버에 Inspect/Apply/Lookup을 추가했다. frozen base Session/CellCall을 재사용하고 신규 binding hash와 JSON artifact hash/size를 검사한다. 모든 payload는 strict JSON/Protobuf 검사와1,000,000-byte 한도를 적용한다. 모든 counter는 기존 정확한 정수 표현을 사용한다.

P의 HostClient는 현재 Host identity, 응답 hash/schema/size, request digest 및 metadata currency 판정을 검증한다. 직접 API 사용자는 **전송 전에 request를 영속 보관해야 한다.** 이 transport helper 자체가 P 업무 원장의 승인·보내기 경계·혼합 구성 조정을 대신하지 않는다.

## 검증과 남은 연결

시험은 동일 요청 재전송, 다른 intent/slot 거부, 현재 caller, 전체 cohort/부분 거부, 없는/오래된 quiescence, 남은 준비 work, commit rollback, commit 직후 SIGKILL/재시작/Arm 거부를 포함한다. 별도 S simulation server와 P client의 mTLS 시험은 commit 뒤 정상 응답 유실을 주입하고 원래 receipt 회수와 재시작 후 역사 판정을 확인한다. 테스트 hook과 응답 유실 주입은 외부 요청으로 설정되지 않으며 제품 이미지의 test-harness 기능은 꺼져 있다.

P의 공정 문맥 coordinator/receipt 집계와 실제 configuration 선택 교체·재검증 검토는 연결했다. 전역 qualification 활성화, 취소/복원은 아직 연결하지 않았다. 실제 장비를 제어하거나 NativeAdapter의 물리 지원을 검증하지 않았다.
