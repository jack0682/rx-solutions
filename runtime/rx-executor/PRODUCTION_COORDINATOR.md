# 직렬 소재 조정과 완료 확인

SerialProduction 모드는 P가 허가한 run의 소재를 순서대로 처리한다. 첫 소재와 다음 소재의 admission, 해당 visit의 BT 실행, P 완료 확인과 다음 context 준비를 연결했다. ManualVisit 모드는 기존처럼 지정된 visit만 처리한다. CLI Options의 기본값은 SerialProduction이며 setup/operation-count 작업은 ManualVisit을 사용한다.

## 상태의 소유자

P Production.Inspect는 첫 part 이전과 마지막 완료 이후에도 읽을 수 있는 일관된 상태를 제공한다. run/checkpoint, 전체 part 목록과 각 revision/disposition, budget, caller/session·cell epoch·scope, 제한된 시각 범위를 같은 control cut에 묶는다. S는 bytes/hash/size/schema, 원래 checkpoint artifact와 실제 cell/recipe, part ID/ordinal·예산 소비를 검증한다.

P의 part 목록은 순서와 ID가 보존된다. 같은 revision에서 값이 바뀌거나, 같은 logical 소재의 ID가 바뀌면 수용하지 않는다. material_id를 임의로 생성하지 않는다. 현재 실제 소재 식별·genealogy는 별도 미완료 범위다.

## 한 소재의 흐름

1. 미완료 IN_PROGRESS 소재가 있으면 그 ID/ordinal을 회수한다. 다른 상태의 미정리 소재가 있으면 자동으로 새 소재를 만들지 않는다.
2. 다음 소재가 필요하고 현재 P admission과 남은 budget이 허용하면 frozen Cell.BeginPartAttempt를 사용한다.
3. S는 예상 ordinal과 전체 body/key를 journal에 저장하고 ENTERED를 저장한 뒤 전송한다. P가 실제 part를 생성하고 budget을 한 번 소비한다.
4. 해당 visit의 BT·worker가 기존 실행 계약으로 작업한다. C++ SUCCESS만으로 part를 완료 기록하지 않는다.
5. S가 P의 공정 frontier 완료를 다시 확인한 뒤 Production.CompletePart를 요청한다. P는 현재 권한, run/part CAS, 선택된 공정의 실제 결과와 인계 근거를 다시 확인한다.
6. 새 P 조회가 그 part의 CONFIRMED_COMPLETED를 확인하면 그 정확한 context를 retire한다. 이후에 다음 소재를 준비한다.

P의 `complete_part_transition`은 기존 내부 completion과 새 API가 공유한다. part disposition·run revision/checkpoint·mandate exhaustion·사건·원래 응답이 같은 commit이다. 모든 소재가 완료되고 budget remaining이 0이면 run을 COMPLETED로 바꾼다. 완료나 재요청이 budget을 환불하지 않는다.

## 응답 유실과 같은 요청

BEGIN_PART/COMPLETE_PART는 node 실행 요청과 구분한 logical stage를 사용한다. 같은 미확정 body는 원래 key만 사용할 수 있으며, 확정된 atomic revision 거부 후에만 같은 의미의 새 CAS/key를 허용한다.

P 조회에서 소재가 이미 보이면 기존 ID를 관측으로 기록한다. 누락된 RPC 응답을 만들어내지 않는다. 완료 응답도 같은 방식으로 회수한다. 늦은 응답의 다른 part ID, 같은 revision의 다른 상태, 변경된 body의 같은 key를 거부한다. 이미 완료된 part의 반복 완료 요청은 새 revision/사건/예산 변경을 만들지 않는다.

## context retire와 중단은 다르다

P의 인증된 최신 조회로만 CompletedVisit 값을 만들 수 있다. EngineProcess.retire는 이 값의 run/session/recipe/visit/epoch를 현재 planner와 비교한다. 일치할 때만 CLOSE가 만든 run-pause 제안을 전달하지 않고 해당 planner를 정리한다. part 완료 확인 없이 이 경로로 planner를 교체하지 않는다.

새 part는 새 BT context를 사용하지만 run의 현재 session/epoch/mandate를 유지한다. 마지막 context도 P 완료 확인 뒤 retire한다. 전체 run이 P에서 COMPLETED면 service stop reason은 COMPLETED이며 P에 별도 pause mutation을 보낼 필요 없이 완료 관측을 보존한다. 사용자 중단/오류/새 session·epoch는 기존 stop lifecycle을 따른다. 이전 실행 세대를 다음 part의 권한으로 바꾸지 않는다.

## 통신과 지원 범위

추가 API는 [별도 production binding](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/spec/production/v1/README.md)으로 고정했다. base/cell과 기존 executor-read/prepare binding은 유지한다. BeginPartAttempt의 cell revision은 CellCall.expected_cell_revision, complete의 run/part revision은 body에 둔다.

연속 두 소재 통합은 정상, 첫 Begin 응답 유실, 첫 Complete 응답 유실의 3사례다. 실제 S 서비스·지속 BT·P mTLS/SQLite가 part admission/완료/다음 context를 처리한다. fixture는 Host 완료·세 인계 근거를 합성해 입력한다. S가 인계 조회를 요청하기 전에는 해제하지 않는다. P budget 소비2회·part2개·작업2개, planner2회, 중간 pause 없음, 유실된 응답의 PENDING 보존과 재시작 추가 planner0회를 확인한다. 실제 모의 Host/native 전달은 별도 기존 통합 시험으로 구분한다.

이 변경은 직렬 소재 조정이다. 병렬 소재 pipeline, 실제 material genealogy, 새로운 session의 명시적 재시작/rebind, setup 완료 조정, intervention/clearance/cancel/control-session, 큰 run의 paging/index/용량, 전체 supervisor·제품 이미지·실제 장비 인수는 계속 남아 있다.
