# 실행기의 분기·대기 요청과 복원

상태: 실제 BT ResolveBranch/BeginWait 요청을 S journal·P 후보 준비·frozen CommitCheckpoint에 연결했다. 분기/대기를 포함한 전체 상주 서비스나 작업자 복구 절차의 완료 명세는 아니다.

## 준비한 상태를 확인한다

worker는 새 P snapshot의 run/session/epoch/recipe/visit와 실제 resolved node를 확인한다. 분기는 CHECKPOINT_BRANCH, 대기 시작과 결과는 CHECKPOINT_START_WAIT와 CHECKPOINT_CHECK_WAIT로 구별한다. BeginWait가 반복되어도 이미 존재하는 window를 새로 만들지 않는다.

Client는 별도 executor-plan binding hash로 후보를 요청한다. WAITING에는 후보·시각이 없어야 하며, 후보가 준비되지 않은 반복 조회를 mutation journal에 쌓지 않는다. ALREADY_APPLIED나 다른 revision은 새 snapshot으로 확인한다.

READY이면 현재 checkpoint와 후보 artifact를 각각 읽고 실제 hash/size/schema·typed payload를 확인한다. 후보는 현재 상태의 정확한 후속이어야 한다. run metadata, budget, part 목록, 기존 activation/slot, 다른 visit의 공정 상태를 변경할 수 없다. 선택한 node의 분기·window·wait 결과와 그 근거/결정 시각만 추가한다. 빈·중복 근거, 기존 decision/window ID 재사용, 조기 timeout이나 잘못된 window 시각을 거부한다.

P source clock과 준비 유효 기간, local 경과 시간을 모두 검사한다. S가 조건의 참·거짓이나 native 결과를 자체 판정하지 않는다. 실제 확정 시 현재 조건과 권한을 다시 확인하는 주체는 P다.

## 요청과 판단의 저장

| 기록 | 의미 |
|---|---|
| PREPARED | 전체 Checkpoint body·artifact reference·목표·예상 결정과 key를 저장 |
| EMIT_ENTERED/PENDING | 실제 전송 직전에 commit. 실제 적용/응답 여부는 미확정일 수 있음 |
| REPLY | 원래 revision·recipe·session·전체 Checkpoint가 맞는 P 응답을 저장 |
| CHECKPOINT_REJECTED | P가 해당 atomic commit을 적용하지 않았음을 구조화된 값으로 확인 |
| 별도 Checkpoint 관측 | 현재 P snapshot에서 확정된 branch/window/result를 확인. RPC 응답과 다름 |

네트워크 호출 전에 body와 진입을 각각 저장한다. 같은 logical target의 미확정 요청은 원래 key/body만 사용할 수 있다. 새 후보가 생겼다는 이유로 기존 미확정 요청을 교체하지 않는다.

실행기가 P에 이미 있는 결정을 확인하면 그것을 관측으로 보존하고 진행한다. 원래 PENDING을 성공 응답으로 덮어쓰지 않는다. 다른 후보가 먼저 CAS를 통과한 경우에도 실제 P 결정을 기록할 수 있다. 이미 받은 응답과 다른 결정이나, 앞서 관측한 불변 결정의 변경은 무결성 오류다. 뒤늦은 응답도 기존 P 관측과 모순되면 수락하지 않는다.

## 다시 준비할 수 있는 경우

CommitCheckpoint에 한하여 `rx-checkpoint-error-bin`의 strict ErrorDetail과 gRPC status 조합을 확인한다. REVISION_CONFLICT/REFRESH+ABORTED, EXPIRED/RECONCILE+FAILED_PRECONDITION만 확정된 거부다. 중복·잘못된 metadata, bare status, 사람용 문구, transport 오류로는 새 key를 만들지 않는다. 이 metadata는 표준 grpc-status-details-bin을 대체하지 않는다.

확정된 거부를 기존 attempt에 보존한 뒤 새 P snapshot·후보를 확인하여 다음 generation/key를 만든다. 이전 body·key·거부 이유는 삭제하지 않는다. 이 처리 범위는 [준비 binding](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/spec/executor-plan/v1/README.md)에 고정되어 있으며, 다른 RPC의 실패를 같은 의미로 일반화하지 않는다.

## 재시작

새 S boot는 원래 journal을 열고 P의 실제 checkpoint를 읽는다. 이미 확정된 branch/window/result를 각각 관측하며 미수신 응답을 만들지 않는다. checkpoint 확정 전 종료라면 P 결정을 만들어내지 않는다. P가 회수한 실행 권한을 자동 재개하지 않는다.

## 검증과 남은 범위

실제 Linux BT.CPP → 별도 S 프로세스 → P mTLS/SQLite 시험은 기존 9개와 새 9개, 총 18개 시나리오다. 새 시나리오는 참·거짓 분기, 대기 성공·반복 대기·시간 초과, 체크포인트 응답 유실, 진입 직후/원격 응답 직후 종료, 실제 P 시계의 후보 만료와 새 generation이다. 재시작 후 원래 key/응답 상태와 별도 관측·작업 수를 확인한다. journal 시험은 미확정 body 교체 금지, 확정된 만료 후 교체, 다른 후보의 P 결정 관측과 모순된 늦은 응답 거부를 확인한다.

fixture의 조건·시간과 사전 operator/part 시작은 모의 구성이다. 이 시험은 새 branch/wait 경로에서 물리 장비를 제어하지 않는다. 기존 별도 Host 통합 시험이 모의 native 결과·인계 경로를 검증한다.

상주 loop·프로세스 supervision, part coordinator, 개입/clearance·취소·명시적 restart, 장기 journal/후보 보존·index/용량, 전체 ErrorDetail 전송, UI·제품 이미지·실제 장비 검증은 남아 있다. 100ms 후보 수명과 모의 시험 시간을 현장 응답 성능 보장으로 사용하지 않는다.

분기·대기 통합 fixture는 후속 단계에서 [지속 BT 엔진과 요청 큐](../../native/executor/PERSISTENT_ENGINE.md)로 전환했다. 실제 운영 daemon을 대신하는 fixture는 아니며, 동일 프로세스의 dedup/대기 유지와 정상 종료 시 P pause를 검증한다.
