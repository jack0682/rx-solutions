# 장비 소프트웨어 검토 화면

phase66. 기존 장비 선언 화면 아래에 검증 요청·보고서·독립 검토 결정을 연결한다. API 규범은 [장비 검토](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/DEVICE_REVIEW.md)를 따른다. 실제 셀 구성 적용·운전 자격·로봇 동작은 이 화면에서 생성하지 않는다.

## 사용자 흐름

1. 반입한 장비 패키지를 선택하여 선언된 작업·조건·원본을 확인한다.
2. 현재 구성 문맥과 장비 검증 서명자 설정이 있으면 검토 요청을 만든다.
3. 요청을 내려받아 외부 검증 도구로 보고서를 생성·서명한다.
4. 보고서 상대 경로와 식별자로 등록한다. 화면은 세 소프트웨어 검사와 issue를 표시한다.
5. 패키지 제출자와 다른 Verifier가 현재 보고서와 원문/서명/범위를 확인한다.
6. 검토 의견·확인 체크 후 구체적인 버전/식별자를 표시하는 창에서 승인 또는 반려를 기록한다.

보고서와 검토 자료를 내려받을 수 있으며, 과거 보고서 버전은 읽기 전용이다. scope는 DEVICE_PACKAGE_SOFTWARE로 고정하고 실물 검증·운전 자격이 별도임을 표시한다. 실패/미수행 검사가 있는 자료, 제출자의 자기 승인, 오래된 보고서 또는 현재 문맥이 맞지 않는 자료는 승인 조작을 활성화하지 않는다. 반려는 backend 정책대로 현재 Verifier와 최신 대상을 요구하며 positive approval의 원본 proof를 대신 만들지 않는다.

## 응답 상관과 확인 해제

목록·상세는 선택한 cell/intake/review와 package manifest/signature/catalog 참조를 대조한다. 상세의 Job/Report/Decision, 상태·scope·현재 버전 표기가 서로 맞지 않으면 표시 자료를 현재 승인 근거로 받아들이지 않는다.

자료의 정규화 stamp에는 보고서·서명·결정·현재 구성/registration/장비 authority와 계정 문맥이 들어간다. 내용이 같은 polling은 확인 체크를 유지한다. 새 보고서나 문맥 변화, 조회 오류/만료, 비활성 화면과 조작 불가 상태는 체크와 열린 확인창을 해제한다. 조회는 abort/generation으로 늦은 응답을 버린다. 최종 버튼에서도 동일 stamp인지 검사하며 서버는 별도로 현재 원본과 권한을 다시 검증한다.

확인창은 선택한 report revision/review digest/expected decision revision/choice/note를 고정한다. 창을 연 뒤 변경된 입력을 조용히 보내지 않는다. scope는 패키지 소프트웨어 검토이며 physical qualification으로 표기하지 않는다.

## 응답 유실

세 장비 mutation 경로를 기존 sessionStorage pending 체계에 연결한다. 보내기 전에 동일 request key·command·계정·installation/store generation을 저장하고, 결과 불명 또는 응답 상관 오류가 있으면 유지한다. 새로고침 뒤에도 같은 요청 확인으로 원래 결과를 회수한다. 다른 계정/설치/저장 세대에서 자동 재전송하지 않는다.

응답의 review/cell/report revision/digest·choice/note와 증가한 revision을 정확히 대조한다. 64-bit revision은 문자열/BigInt로 처리한다. 과거 승인 receipt를 회수한 사실과 현재 조회의 승인 적용 가능성은 구별한다. 자료 선택이나 조회만으로 새로운 결정을 보내지 않는다.

## 목록 크기

P 목록은 한 번에50개의 Summary만 반환한다. 요청자/생성 시각, 최신 보고서 revision/digest·승인 후보 상태, 결정 요약과 현재성만 담는다. 보고서/서명/긴 의견은 별도 상세 endpoint에서 읽는다. 전체 보고서를 목록 행마다 복사하던 초기 형태를 대체했다.

더 보기를 통해 가져온 요약은 일반 갱신 때 유지하고 중복 ID를 합친다. 조회 결과의 범위를 확인한다. backend 전체 Job scan과 장기간 보관/최대 부하 검증은 여전히 후속이다. 이번 변경은 응답 payload 구조를 제한한 것이며 전체 저장소 성능 인수가 아니다.

## 검증

실제 S JTC CLI·외부 test signer·P API를 사용해 화면에서 요청 생성, 보고서 등록, 작성자 승인 비활성, 독립 승인, 새 버전/과거 버전, 열린 창 무효화와 응답 유실 후 동일 요청 회수를 확인한다. authority 문맥 변화는 별도의 응답 주입 UI 반례로 구분한다. 실제 backend authority 파일 변경 거부는 phase65 API 검증을 유지한다.

화면과 실제 결과는 [phase66 기록](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase66_checks.json)에 연결한다. 장비 소프트웨어 승인 이후의 구성 변경·qualification과 실제 장비 검증은 다음 작업이다.
