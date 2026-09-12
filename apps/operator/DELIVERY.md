# 두 이미지의 운영 앱 제공

phase73. 운영 앱은 S에서 production build하고 P의 직접 단말 HTTPS에서 제공한다. 개발용 Vite proxy/5173 포트는 제품 제공 경로에 사용하지 않는다.

`npm run build`는 `dist/`와 `operator-bundle.json`을 만든다. manifest에는 API schema와 index/assets의 정렬된 path·정확한 byte 수·SHA-256을 넣는다. root/leaf symlink, 예약 이름·경로 별칭, 미지원 파일·빈 파일, 개별8MiB/총64MiB/1024파일을 거부한다. `npm run test:bundle`은 생성기의 재현성 및 반례를 확인한다.

S 이미지의 `/opt/rx/operator/`에는 이 완성 bundle이 들어간다. 설치는 선택한 S 이미지 digest에서 해당 bundle을 꺼내 불변 배포 디렉터리로 게시하고 P의 `operator_ui` 설정과 read-only mount에 연결한다. 제품 실행 중 S가 P의 UI volume을 덮어쓰는 방식은 쓰지 않는다. 제3 이미지는 추가하지 않는다.

P는 manifest pin과 전체 파일 목록/hash/크기를 먼저 검증하고 보유한 바이트만 제공한다. [P 제공 경계](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-api/OPERATOR_UI.md)를 따른다. 페이지를 받는 mTLS 신원과 로그인/API의 현재 단말·사람·역할 검사는 구별한다. 인증서 헤더 proxy나 static 경로의 업무 권한 부여는 없다.

운영 앱의 graph 들여쓰기는 고정 CSS class를 사용한다. Vite `assetsInlineLimit=0`으로 작은 글꼴도 별도 파일로 내보내며, UI의 self-only script/style/font CSP를 유지한다. 실제 이미지 시험에서 발견한 data-font CSP 위반을 정책 완화로 우회하지 않았다. 외부 CDN은 필요하지 않다.

`rx-platform/tools/test_operator_delivery.py`는 새 임시 설치와 실제 P/S 이미지, 등록 단말 인증서, production bundle을 사용하는 브라우저 인수 시험이다. 한글 font·desktop/mobile, 없는 API/asset 경로, 인증서 없는 접근/미등록 인증서 로그인 거부, CreateRun commit 응답 유실과 동일 요청 회수를 검사한다. Python TLS는 서버 CA를 검증한다. 브라우저의 임시 self-signed CA 처리와 실제 운영 인증서 설치는 구별해 기록한다.

현재 이 경로가 제공하는 조작은 기존 CreateRun 및 구성·검토 화면이다. StartRun의 ARMING/STARTED 표시, 상시 실행기 배정과 이상 후 재개는 아직 후속이다. S 이미지는 이 시험에서 불변 UI 원본으로 사용했으며 Host/controller를 함께 기동한 전체 셀 인수로 확대하지 않는다.
