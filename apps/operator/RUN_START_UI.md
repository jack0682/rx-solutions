# 운영자 실행 시작 연결

기존 실행 준비·실행 목록에 정확한 실행 선택, 명시적인 생산 소재 시도 수량,
시작 전 조회·확인창, StartRun 접수 및 시작 시도 조회를 연결한다.
기존 화면의 구성·CSS·동일 origin API·HttpOnly cookie·직접 단말 mTLS 경계를 유지한다.
브라우저에 Host/Executor credential이나 native 제어 경로를 추가하지 않는다.

## 실제 API와 화면 의미

- `GET /api/v1/run/start-context?cell=...&run=...&purpose=PRODUCTION&budget_limit=2`:
  선택한 실행의 현재 revision·구성·예산과 읽기 시점의 차단 이유를 확인한다.
  수량은 문자열 Counter이고 양의 정수만 허용한다. 기존 예산은 고정한다.
  읽기 검사 통과는 자격 생성이나 운전 허가가 아니다.
- `POST /api/v1/runs/start`: 기존 `{request_key, command: StartRun}` 계약을 그대로 사용한다.
  확인창에서 cell/run revision·envelope·수량을 고정한다. 배경 조회가 달라지면
  재검토가 필요하며 검토한 요청을 새 revision으로 자동 변경하지 않는다.
- `GET /api/v1/run/start-attempt?cell=...&run=...&id=...`:
  저장된 PENDING/ARMING/STARTED/REJECTED와 현재 Run을 조회한다.
  Host ACK 개수로 STARTED를 추론하지 않는다. ELAPSED/CLOCK_CHANGED는 별도 시간 상태이며
  ARMING을 REJECTED로 바꾸거나 자동 재시작하지 않는다.

새 실행 준비 응답은 생성한 정확한 Run을 선택하는 데 사용한다. 시작 준비 상태와 revision은
별도 현재 조회로 확인한다. 기존 실행도 목록에서 명시적으로 선택할 수 있다.
수량 입력의 초기값은 비어 있으며, 가장 최근 실행을 자동 선택하거나 자동 시작하지 않는다.
이미 확인한 시작 시도는 동일 설치·계정에서 그 정확한 실행으로 돌아간다.

## 응답 상관과 복구

기존 pending 저장소에 `/api/v1/runs/start`와 검토한 셀 문맥을 추가한다.
송신 전에 원래 UUID key·본문·principal·installation·store generation·셀/epoch/구성
문맥을 sessionStorage에 저장한다. 미확정 요청이 있으면 다른 mutation을 막는다.
응답 유실·reload 후 기존 ‘같은 요청 확인’ 버튼은 저장된 key와 본문을 그대로 보낸다.
다른 계정·설치·복원 세대에서는 그 요청을 회수하지 않는다.

기존 StartAttempt POST receipt에는 envelope/purpose/budget이 없으므로,
cell/run/actor/검토한 revision/epoch/scope를 먼저 검증하고 같은 attempt ID의 GET에서
Run envelope/recipe/purpose/budget까지 대조한 뒤 pending을 해제한다.
GET 실패나 상관 불일치는 원래 요청을 미확정으로 보존한다. pending 해제 전 확인한
시도 ID와 원래 요청을 별도 sessionStorage 조회 표식에 보관하여 reload 뒤에도 GET으로
상태를 확인한다. 이 표식은 운전 권한이나 별도 업무 원장이 아니다.

일반 상태 polling은 GET만 사용한다. 마지막 성공 조회 후 10초 또는 조회 오류이면
이전 기록임을 표시하고 새 시작 요청을 차단한다. 확정적인 최초 거부 후에는 새 문맥을
읽고 다시 검토해야 한다. 이미 미확정인 요청의 후속 거부는 최초 미처리의 증거로 삼지 않는다.

현재 실행 상태는 최신 overview의 선택 Run 상태를 표시한다. 시작 시도 GET이 10초 이내라도
installation/store generation/runtime boot/clock 또는 현재 Run revision·상태가 일치하지
않으면 즉시 마지막 조회 기록으로 낮춘다. 따라서 운전 보류·재시작으로 갱신된 실행 상태를
이전 STARTED/EXECUTING 조회가 덮어쓰지 않는다. 후속 GET 실패도 이 판정을 되돌리지 않는다.
각 Host ACK는 해당 host_boots 값과 일치해야 하며 STARTED는 정확한 전체 Host 확인이 필요하다.
ARMING은 올바른 일부/전체 ACK가 있어도 시작 확정으로 승격하지 않는다.

## 구현 및 검증 경계

- `run-start-schema.ts`: DTO 검사, 요청/응답 상관, 확인 내용 고정, 시작 시도 조회 표식.
- `run-start.tsx`: 기존 공통 컴포넌트 스타일을 사용하는 선택·수량·검토·상태 화면.
- `run-start-schema.test.ts`: 잘못된 Counter/상태/설치/Run/attempt·구성/예산,
  응답 유실 후 원래 key/body 회수, 버전 변경 시 재검토, Host ACK와 기한 경과 반례.

사람용 소재별 production 집계 API/화면은 이번 범위에 없다.
기존 overview의 Run 상태·예산 사용량과 작업 결과·자원 인계를 각각 표시하며,
예산 소비나 일부 작업 목록을 완료 소재 수·양품 수로 해석하지 않는다.
sessionStorage는 같은 탭 reload 복구를 지원하며 브라우저 종료·새 탭·장치 교체를
포괄하는 영속 클라이언트 복구는 별도 범위다.
실장비 미등록 여부와 현재 등록 단말은 실제 조회값으로 표시한다.
화면 연결 자체는 실장비 운전 자격·실물 시험 완료를 뜻하지 않는다.

검증 실행은 전체 소스 동결 후 통합 담당자가 수행한다:
`npm run typecheck`, `npm test`, `npm run format:check`, `npm run build` 및 등록 단말 브라우저 인수.
