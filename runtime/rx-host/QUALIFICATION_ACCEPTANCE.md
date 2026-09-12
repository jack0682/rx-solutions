# Host 자격 수용과 별도 시작 절차

상태: phase53 구현 초안. 선택 `rx.host.qualification.v1`의 Inspect/Accept/Lookup과 shared data model, S의 원자 수용/최종 gate, P transport client를 연결했다. 기존 base/cell 규범은 변경하지 않았다. P의 [영속 발급·전역 활성화](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/QUALIFICATION_ACTIVATION.md)를 연결했다.

## 수용하는 것

인증된 P는 검토된 qualification ID/revision을 현재 Host의 공정 문맥에 연결하도록 요청한다. Request는 다음을 고정한다.

- Host ID, 예상 Host boot/delivery journal, 현재 static Binding fingerprint.
- change/application digest, review ID/version/digest, decision revision, policy digest.
- 전체 Host 관리 셀별 configuration digest, process context의 원래 request/receipt sequence.
- definition/envelope/environment, 새 qualification ID/revision, dependency hash와 limitations ArtifactRef.
- 허용 intent digest/purpose의 명시적 부분집합.
- 현재 cell epoch/scope map, 정확한 fence request와 필요한 block ID.

현재 Binding을 벗어난 정의/환경/intent/purpose를 수용하지 않는다. 수용 intent가 빈 집합이면 허용할 operation도 없다. Host의 모든 관리 셀을 같은 요청에 포함하고 같은 base/cell session에서 협상해야 한다. 일부 셀을 생략하거나 다른 셀의 문맥/fence로 대신하지 않는다.

Host는 P의 검토 문서를 독립적으로 다시 심사하지 않는다. P가 권위 있는 발급자라는 인증 경계를 사용하며, 실제 문서·서명·사람 검토와 전역 적용 판단은 P의 책임이다. 현재 raw client는 이 책임을 구현한 발급자가 아니다.

## 전제와 저장 경계

Accept는 현재 caller/Binding/boot/journal·전체 cohort를 검사한다. 각 셀은 Arm되지 않았고, 현재 epoch/scope·block과 해당 fence receipt가 일치해야 한다. 원래 process-context의 configuration/request/sequence/change/binding도 대조한다.

Host 전체에 PREPARED/SEND_ENTERED/NATIVE_ACCEPTED work가 남아 있으면 NOT_ACCEPTED다. adapter의 실제 handover_snapshot에서 no_pending_commands/control_available/support_stable과 age+uncertainty100ms 이내를 확인한다. 미지원·불명·오래된 관측을 성공으로 만들지 않는다. transaction 직전에 age를 한 번 더 검사한다.

다음을 한 transaction으로 기록한다.

| 기록 | 의미 |
|---|---|
| accepted-qualification | 셀별 자격·좁힌 허용 범위·원래 공정 문맥·Host/device session·epoch |
| qualification-request | 원래 Request/digest·sequence·ACCEPTED 또는 NOT_ACCEPTED·이유·관측 |
| qualification-slot | `(P peer, review ID, review revision)`당 한 요청 ID |
| qualification-identity/maximum | 자격 ID/revision의 구성·범위·검토 의미 불변 및 revision 후퇴 방지 |
| qualification-history | sequence에 따른 불변 receipt 이력 |

qualification ID/revision을 다른 구성·제약·검토 근거로 재사용하지 않는다. 다른 수용으로 교체하려면 더 높은 cell epoch가 필요하다. 같은 epoch에서 허용하는 반복은 원래 request ID/body의 receipt 회수다. NOT_ACCEPTED는 기존 수용을 덮어쓰지 않는다.

수용은 block을 지우거나 Arm/grant/permit를 만들지 않으며 native submit/lookup을 호출하지 않는다. receipt의 quiescence는 수용 시점의 관측이고 지속 물리 안정성 보증이 아니다.

## 오류·재시도·재시작

같은 ID/같은 정규화 본문은 원래 receipt를 돌려준다. 같은 ID의 다른 본문, 같은 review/version의 다른 ID는 충돌한다. 새 검토 결과가 필요하면 새 review/version/epoch의 명시적 절차를 거친다.

RPC 오류는 NOT_ACCEPTED가 아니다. 보내는 쪽은 전송 전에 전체 Request를 영속 기록하고, 응답 유실에 같은 ID를 Lookup해야 한다. 빈 Lookup 역시 미수용의 확정 증거로 해석하지 않는다. P의 영속 qualification task는 원래 요청 조회와 현재 권한을 확인하는 제한된 재전송을 연결한다.

조회는 원래 receipt와 현재 snapshot/accepted identities를 함께 반환한다. `receipt_matches_current_host`는 Host boot/journal/Binding/process context/epoch와 자격 기록의 일치 여부다. 현장 장비가 계속 안정적이라는 뜻은 아니며 `activation_authorized=false`다. client는 이 파생값이 실제 payload와 일치하는지 검사한다.

Host 재시작은 receipt/자격 이력을 보존하지만 새로운 boot에서 과거 자격을 현재 권한으로 쓰지 않는다. process context 또는 cell/scope epoch가 바뀌어도 옛 수용은 부적합하다. cached Accept/Arm 재전송으로 Arm 상태를 복원하지 않는다.

## 이후 Arm과 native gate

process-context가 있는 셀은 현재 유효한 accepted-qualification이 없으면 startup Binding의 옛 자격으로 fallback하지 않는다. 자격 수용 후에도 별도 Arm이 필요하며, 그 Arm이 지정된 block을 처리한 뒤에만 operation gate가 진행할 수 있다.

최종 native 진입은 기존 guard에 더해 다음을 검사한다.

- Permit의 qualification ID/revision이 현재 수용한 자격과 일치.
- 요청 intent/purpose가 static Binding과 수용한 부분집합 양쪽에 포함.
- Host boot/journal/Binding/process context/epoch/scope가 수용 기록과 일치.
- Native guard의 device session이 수용 시 quiescence의 device session과 일치.
- 기존 armed 상태, 빈 block, 현재 grant/fence/만료, permit 및 현지 조건 검사를 모두 통과.

같은 Host 프로세스 안에서 장치 session이 바뀌어도 새 동작을 거부한다. 준비된 operation의 native 진입 전에도 기존 재검사를 유지한다. 이미 시작한 작업의 미확정 결과를 새 자격으로 성공/미실행 처리하거나 재제출하지 않는다. 독립 현지 보호 포트는 계속 별개다.

## RPC·구현 파일

[wire binding](../../sdk/spec/host-qualification/v1/README.md)의 exact hash와 1,000,000-byte canonical payload/1 MiB gRPC 한도를 적용한다. 기존 mTLS/base/cell session과 엄격 decoder를 사용한다. envelope의 reference hash/size/schema, 요청 ID/cell과 전체 cohort 협상을 검사한다. test-harness의 응답 유실/commit hook은 product RPC로 선택할 수 없다.

- `gate/qualification.rs`: 수용/이력/현재 자격 검사.
- `gate/scopes.rs`, `gate/validation.rs`: 별도 Arm과 최종 native gate 연결.
- `rpc/qualification.rs`: 선택 service.
- P `rx-host-client::qualification`: Inspect/Accept/Lookup과 payload/current-claim 검사.

static Binding은 바꾸지 않고 수용된 자격을 별도 원장으로 보관한다. Binding 목록은 startup 범위다. 현재 자격의 조회는 이 service를 사용해야 한다.

## 검증·남은 일

[phase53 기록](../../../references/implementation/phase53_checks.json)에 범위를 구분한다. Host 시험은 수용만으로 동작하지 않음, 명시적 Arm/새 자격의 모의 동작, 옛 자격·purpose·빈 intent 범위 거부, key/slot/자격 의미 충돌, 전체 cohort 원자성, 남은 native work, 미지원/오래된 quiescence와 device session 변경을 다룬다. commit 직후 SIGKILL은 receipt 보존과 재시작 후 Arm 거부를 확인한다.

P/S mTLS 통합은 실제 P 검토 승인 view에서 **테스트 harness가 구성한** Request를 전송 전에 파일에 저장하고, Host commit 응답 유실·같은 ID 회수/재전송·충돌 거부를 검사한다. phase53에서는 이 범위였으며, 후속 phase54는 실제 P 발급/전역 활성화와 별도 모의 작업 완료까지 확장했다. 이 통합에서는 native effect가0개다. 별도 Host 단위 시험의 명시적 모의 동작은 독립 effect log로 검사한다.

P의 발급/영속 Host task·부분/불명 조회·전역 활성화와 별도 사용자 시작은 [phase54 경로](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/QUALIFICATION_ACTIVATION.md)로 연결했다. 전체 취소/철회·운영 복구와 전용 UI는 후속이다. 취소/철회·업그레이드/복원·장치별 실제 성능/보호 검증과 physical commissioning도 남아 있다. 첫 물리 셀은 NOT_COMMISSIONED다.
