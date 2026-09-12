# Executor 시작 관계 조회

`Client::assignment_view()`는 현재 mTLS/base/cell session과 `PeerPin`에 묶인 셀을 별도 `rx.executor.assignment.v1` binding으로 조회한다. 반환형 `client::ValidatedAssignment`의 생성자는 외부에 공개하지 않는다. `data()`는 검증한 P 결과를 그대로 제공하며 `is_current()`와 `expires_at()`으로 읽기 유효기간을 확인한다.

응답의 digest·정확한 크기·schema와 엄격한 JSON/DTO를 검사한다. payload는 협상 한도와 65,536bytes 중 작은 한도를 지킨다. installation/store/runtime·caller session·현재 executor/셀/definition을 고정값과 대조하고, snapshot/production 조회와 같은 sequence 및 checked 시각의 단조 검사를 공유한다. P 시각은 같은 호스트 clock의 요청–수신 구간 안에 있어야 한다. 양수·최대100ms TTL과 요청 전부터 계산한 로컬 단조시계 deadline을 함께 검사한다. 거부되거나 만료된 응답은 클라이언트의 마지막 검증 cut을 바꾸지 않는다.

`NONE`은 후보0, `SINGLE`은 후보1, `AMBIGUOUS`는 서로 다른 후보2를 보존한다. 이전 executor session, 만료된 pending/arming attempt, PAUSED/RECOVERY_REQUIRED, 과거 configuration을 수신 후 필터링하지 않는다. 조회 결과에서 후보를 고르거나 권한을 생성하지 않는다. 연결하려는 Run의 현재 권한과 공정 상태는 기존 `Production.Inspect` 및 admission 경로에서 별도로 재검증해야 한다. 이 API 추가는 기존 RunService나 CLI의 시작 동작을 바꾸지 않는다.

전용 단위 시험은 실제 응답 수용 경로에 통제된 clock과 payload를 입력한다. 위조된 identity/cut/cardinality, 중복 후보, digest·크기·schema와 JSON 오류, TTL·clock 구간, 응답 만료와 이전 후보 보존을 다룬다. 채널은 연결을 시작하지 않는 lazy channel이며 서버나 실제 장비를 사용하지 않는다. 통합 mTLS/session/P 조회 검증은 P/S 통합 시험의 별도 범위다.
