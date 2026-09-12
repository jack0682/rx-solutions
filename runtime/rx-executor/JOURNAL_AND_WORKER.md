# 영속 요청과 유한 작업 worker

상태: 실제 C++ BT request를 받아 S journal→P ResolveActivation/SubmitOperation에 연결했다. PauseExecutor와 RequestHandover 요청도 영속 경계로 연결했다. [분기·대기 worker와 checkpoint 요청/복원](DECISIONS_AND_RECOVERY.md)도 연결했다. 전체 운영 daemon, part coordinator, 개입 worker는 후속이다.

## 상태 소유

S의 journal은 ‘어떤 요청을 어떤 key/body로 보냈는가’를 기록한다. P의 run/operation/permit 판단이나 Host native 결과를 대신 소유하지 않는다. P DB에 접근하지 않고 S 전용 SQLite 저장소·단일 writer ownership을 사용한다.

Journal header는 installation/store generation, principal/release, cell/definition, run/resolved digest에 고정한다. 다른 저장 세대·run·recipe로 같은 journal을 조용히 재사용하지 않는다. logical action은 visit/node/stage로 구분한다. ResolveActivation, SubmitOperation, ReconcileOperation은 같은 작업 mapping을 사용하고, PauseRun은 session/epoch control identity를 추가한다.

각 attempt는 key, generation, 당시 context(session/epoch), 전체 typed body와 digest, P read basis를 보존한다. current pointer는 최신 attempt를 찾는 색인이며 이전 attempt는 지우지 않는다. Request body는 intent normalization과 ID/관계/CAS를 검증하며 임의 text를 실행하지 않는다.

## 영속 경계

| 상태 | 의미 | 재접속 처리 |
|---|---|---|
| PREPARED | key/body를 저장했지만 아직 네트워크 진입을 기록하지 않음 | 같은 요청을 회수. 새 body로 덮어쓰지 않음 |
| EMIT_ENTERED + PENDING | 네트워크 호출 전에 진입 기록을 commit. 실제 전송/응답 여부는 미확정일 수 있음 | 같은 context면 원래 key/body만 사용. 임의 새 key 금지 |
| EMIT_ENTERED + REPLY | 엄격히 검증한 P 응답을 영속 기록 | 응답을 바꾸거나 다시 emit하지 않음 |
| REVISION_REJECTED | 해당 Resolve/Submit/Pause RPC의 ABORTED 응답으로 transaction 거부 확인 | 새 snapshot 후 같은 의미의 새 CAS/body·새 key·generation 허용 |
| ATTENTION | 입력/권한/응답/무결성 등을 조정해야 함 | 자동 새 요청으로 바꾸지 않음 |

worker는 prepare commit → enter commit → RPC → reply commit 순서다. local write 실패 시 다음 단계로 가지 않는다. Entered commit 후 죽으면 실제 호출 전이었더라도 임의로 Prepared로 돌리지 않는다. P 응답 후 local reply 저장이 실패하면 원래 key와 미확정 상태를 유지한다.

현재 rebase를 허용하는 machine signal은 Resolve/Submit/Pause atomic RPC의 gRPC ABORTED다. CAS가 없는 ReconcileOperation은 ABORTED로 새 key를 만들 수 없다. reason/detail 문자열을 해석해 key를 바꾸지 않는다. 사실을 기록한 뒤 업무 CAS 오류를 반환할 수 있는 다른 API에 이 규칙을 일반화하지 않는다. KEY_CONFLICT나 미지원/권한/잘못된 응답은 같은 방식으로 자동 재작성하지 않는다.

## 관측과 응답은 다르다

현재 P snapshot에 activation/operation이 있으면 worker는 그 ID와 intent/slot 연결을 확인해 재사용한다. 이것은 `ObservedTarget`과 P read basis에 별도로 기록한다. 없던 RPC reply를 만들거나 원래 EMIT_ENTERED/PENDING을 성공 응답으로 덮어쓰지 않는다.

동일 mapping의 반복 조회는 journal에 매 tick 기록하지 않는다. 새 P runtime에서 재확인하거나 mapping 내용이 바뀔 때만 갱신하며, operation/activation identity 자체의 변경은 무결성 오류다. Work의 native 성공·실패·자원 인계는 계속 P의 현재 상태에서 읽는다.

## worker 처리

1. P의 검증된 current snapshot을 새로 읽는다.
2. C++ request의 context/run/session/recipe/visit/epoch와 node kind/argument/timeout을 실제 resolved process에 대조한다. C++가 준 argument를 native intent로 사용하지 않는다.
3. 이미 매핑된 작업이 있으면 관측을 기록하고 기존 ID를 반환한다.
4. 현재 admission과 frontier가 허용하면 activation request를 journal에 준비·진입한 후 P에 보낸다.
5. T3 뒤 새 snapshot을 읽어 run revision·현재 권한·mapping을 다시 확인한다.
6. 검증된 binding에서 T1 body를 만들고 같은 journal 경계로 제출한다. Root clock 만료나 context 변경은 새 전송을 차단한다.

P에서 인정된다고 응답받았던 mapping이 완전한 새 snapshot에서 사라지면 저장소/복원 조정 대상으로 남긴다. 그 응답만 보고 새 ID를 만들지 않는다. Context가 달라진 미응답 요청도 자동 재발행하지 않는다. 적격한 새 P 상태가 생겼다는 사실을 과거 요청의 결과 확인과 혼동하지 않는다.

SubmitOperation은 유한 작업 처리에, RequestHandover는 영속 조회·해제 관측에 연결했다. ResolveBranch/BeginWait는 후보 검증·CommitCheckpoint·P 결정 관측으로 연결했다. RequestIntervention worker는 아직 Unsupported다. PauseExecutor는 별도 session/epoch control identity의 journal key로 현재 P PauseRun에 연결한다. 상위 runner는 이를 성공 완료로 소비하면 안 된다. 이 메서드들과 실제 restart/clearance 연결은 전체 목표에 남아 있다.

## 시험

- prepare/enter/reply의 transaction 직전 실패·commit 후 응답 유실, 저장소 재개방과 같은 key/body 보존.
- 미확정 request의 body 교체 거부, ABORTED 확인 뒤에만 새 CAS/key 허용, 이전 attempt 보존.
- P 관측이 RPC 응답을 위조하지 않음, unchanged mapping dedup, 다른 store generation/identity 거부.
- 실제 C++ BT request producer → 별도 S worker → P mTLS/SQLite 처리.
- 정상 처리, Submit 네트워크 진입 직후 강제 종료, P Submit 응답 직후 local reply commit 전 강제 종료.
- P Resolve/Submit 응답을 각각 commit 후 유실시킨 뒤 기존 mapping을 회수하고 작업이 하나만 남는지 확인.
- 새 boot의 S recovery process가 원래 key/네트워크 미확정 상태를 보존하고, 무허가 상태에서 새 작업을 만들지 않음.

C++ request producer와 process hooks는 validation/test-harness 전용이며 Docker image digest를 고정하고 pull/network/device access를 허용하지 않는다. Operator 시작·part coordination·qualification/Host 준비는 명시적 simulation composition이다. worker 통합 시험은 P admission/Pause와 인계 요청·해제 관측을 검증한다. 인계 시험의 Host 근거는 합성 fixture이며 별도 Host 시험이 P→H/native/query/handover를 검증한다. 아직 전체 상주 BT daemon을 납품한 상태가 아니다.

Pause의 허가·fence·결과 한계는 [P PauseRun 명세](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/EXECUTOR_PAUSE.md)를 따른다. 기존 operation logical key는 optional control identity가 absent이므로 이전 canonical key를 유지한다.

## 인계 요청과 해제 관측

실제 BT가 유효한 성공 결과 뒤 RequestHandover를 내면, worker는 ReconcileOperation body/key를 저장한 후 P에 조회를 요청한다. RPC 응답은 ReconciliationAccepted이며 실제 RELEASED와 다르다. 이미 접수된 조회는 반복 BT 요청마다 재발행하지 않는다.

새 P snapshot이 RELEASED를 확인하면 별도 ObservedTarget::Released를 저장한다. 조회 응답이 유실된 경우 원래 PENDING을 보존한다. 재시작의 recover도 같은 해제 관측을 기록하며 요청 성공 응답을 만들어내지 않는다. 실제 BT 요청·정상 응답·응답 유실·재시작을 포함한 기존 9개에 분기/대기 9개를 추가하여 wire fixture는 총 18시나리오다.

P 계획의 ATTENTION 상세를 현재 Frame/UI에 전달하는 경로와 작업자 재조회는 후속이다. 상세 처리와 제한은 [P 조회·인계 명세](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/RECONCILIATION.md)를 따른다.

Checkpoint 요청의 CHECKPOINT_REJECTED는 일반 REVISION_REJECTED와 구별한다. 구조화된 atomic 거부를 확인한 경우에만 현재 P 상태를 다시 읽고 새 후보/key를 만든다. 세부 절차는 [분기·대기와 복원](DECISIONS_AND_RECOVERY.md)을 따른다.
