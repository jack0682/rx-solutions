# RX 운영 화면 초안

React/TypeScript로 만든 실제 플랫폼 API 연결 화면이다. 로그인, 권한별 셀 상태, 새 실행 준비, 운전 보류, 기록/구성/내 접근 권한 조회를 제공한다. UI 안에 별도 운전 권한·결과 원장을 두지 않는다.

현재는 로컬 개발 환경이다. 실제 운전 시작, 현장 단말 인증, 공정 편집기, 복구·검증·배포·계정 관리의 전체 조작 흐름은 아직 연결하지 않았다. 계획된 기능을 동작하는 버튼처럼 표시하지 않는다.

## 소스 구성

| 파일 | 책임 |
|---|---|
| `schema.ts` | API 읽기 모델과 요청 보관 형식 검사. 알 수 없는 상태를 정상으로 표시하지 않음 |
| `api.ts` | 동일 origin HTTP, cookie, timeout·오류/미확정 결과 구별 |
| `pending.ts` | 요청 전 sessionStorage 보관, 같은 key/내용 회수, 계정·설치·store generation 결합 |
| `App.tsx` | 로그인/조회 수명, 검토한 revision 고정, 사용자 요청과 화면 연결 |
| `views.tsx` | 실행/작업 기록, 구성 읽기 화면 |
| `labels.ts` | 사용자 상태 용어 |
| `style.css` | 공통 화면/반응형/키보드 초점 표현 |

[React의 독립 앱 구성](https://react.dev/learn/build-a-react-app-from-scratch)과 [Vite proxy](https://vite.dev/config/server-options) 방식으로 구성했다. Korean font는 npm에 고정한 IBM Plex Sans KR을 로컬 asset으로 번들링한다. 외부 CDN이 필요하지 않다.

## 상태·요청 규칙

- qualification 미등록을 검증 대기로 표시한다. 네트워크 접속 성공이나 구성 존재로 운전 준비 완료를 추론하지 않는다.
- 작업의 관측·결과·무결성·자원 처분을 별도로 표시한다. 완료 확인이 자원 인계 완료를 뜻하지 않는다.
- 현재는 3초 주기 snapshot 조회다. 조회 오류 또는 마지막 확인 후 10초가 지나면 새 요청을 막고 이전 데이터임을 표시한다. 이것은 SSE 구현이 아니다.
- 확인 창을 열 때 cell revision과 요청 내용을 고정한다. 배경 조회가 바뀌어도 검토 내용을 자동 교체하지 않는다. 서버가 stale revision을 거부하면 다시 검토한다.
- 요청을 보내기 전에 원래 key와 내용을 기록한다. 응답 유실 후 reload해도 같은 요청을 확인한다. 미확정 요청이 남아 있으면 새 mutation을 막는다.
- 요청 기록은 principal/installation/store generation에 묶는다. 계정이나 복원 세대가 바뀌면 무작정 다시 보내지 않는다. 이 경우 운영자/지원자의 명시적 재조정 화면은 후속 구현이다.
- 비밀번호와 session token을 local/sessionStorage에 저장하지 않는다. 인증은 HttpOnly cookie를 쓴다. 저장하는 것은 미확정 업무 요청이다.
- sessionStorage 손상/사용 불가를 조용히 초기화하지 않는다. 조회 전용으로 제한한다. 브라우저 종료·새 탭·장치 교체까지 포괄하는 durable client recovery는 아직 후속 범위다.
- UI의 role별 비활성화는 편의 기능이다. 실제 권한·단말·자격·revision·idempotency 검사는 platform이 다시 수행한다.

## 개발·검증

```text
npm ci --ignore-scripts
npm run dev
npm run build
npm test
```

먼저 platform의 `rx-platform-local`을 127.0.0.1:8080에 실행하며 public origin을 `http://127.0.0.1:5173`로 설정한다. UI 주소는 `http://127.0.0.1:5173`다. Vite는 개발 서버이며 제품 이미지의 web ingress를 대신하지 않는다.

`tests/browser_smoke.py`는 실제 API에 로그인하고 미검증 `examples/development/cell-demo.json`을 등록한다. 실제 server commit 후 HTTP 응답을 끊어 원래 요청으로 회수되는지, 검토 중 revision 변화, 접속 상실, 모바일 overflow, logout을 확인한다. DB·응답을 성공 fixture로 교체하지 않는다.

Python 시험 의존성은 `tests/requirements-browser.txt`에 고정했다. headless Chromium을 사용한다. [Playwright Python](https://playwright.dev/python/docs/library)에 따라 독립 가상환경에 설치한다.

`tests/run_browser.py --platform-executable PATH --server-runner PATH --evidence-dir PATH`는 새 임시 설치를 만들고 webapp-testing의 `with_server.py`를 통해 두 개발 서비스를 관리한다. helper 경로는 명시적으로 전달한다. 이 helper 없이도 새 개발 설치와 두 서버를 직접 준비한 뒤 `RX_BROWSER_EVIDENCE=OUTPUT python tests/browser_smoke.py`를 실행할 수 있다. 시험용 초기 비밀번호는 `browser-fixture-password`이며 임시 설치에만 사용한다.

시험 launcher는 포트가 이미 사용 중이면 기존 프로세스를 종료하지 않고 실패한다. pytest/Playwright 결과는 UI 초안의 범위이며 제품 인수·실물 시험을 대신하지 않는다.

개입 사건 목록과 알림 확인을 실제 개발 API에 연결했다. 확인은 case/생산 제한을 해제하지 않으며, 응답 유실은 기존 sessionStorage pending key로 회수한다. 사건 생성은 현재 API/engineering 구성 경로이고, procedure catalog·생성 UI·실제 절차/복구/재시작은 후속이다. [명세](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/INTERVENTION_CASES.md)를 따른다.


## 운전 조건과 관측 근거

‘운전 조건’에서 source 값·취득 시각/age·품질·세대, Host의 등록된 사용권과 시작/유지/작업 조건을 확인한다. 표시의 유효시간이 지나면 재조회가 필요하며 PASS는 운전 허가가 아니다. [진단 모델과 검증 범위](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/OPERATOR_CONDITIONS.md)를 따른다.

`tests/run_browser.py`는 P application의 시험으로 PASS/FAIL/expired read-model fixture를 생성한다. 브라우저 시험의 이 세 경우는 응답 경계의 표시 검증이며, 실제 서비스에 native 관측을 주입하지 않는다. evidence 디렉터리는 실행마다 새 경로를 사용한다.


## 공정 설계

Engineer/Verifier는 공정 초안을 조회하고 Engineer는 새 초안·노드/흐름 편집·저장·복사·버전 비교를 수행한다. 미완성 내용은 오류와 함께 저장하고 설치된 셀은 바꾸지 않는다. 고급 JSON 입력은 적용 전 buffer로 보존한다. [초안 저장과 실행의 경계](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/PROCESS_DRAFTS.md)를 따른다.

[패키지 반입·검토 화면](PACKAGE_REVIEW_UI.md)은 실제 API와 전역 pending 복구를 연결한다. 과거 revision과 최신 승인 대상을 구별하며 소프트웨어 승인으로 운전하지 않는다.
