# RX Solutions

RX의 장비 Host·ROS/native 연동, 선언형 공정과 BT 실행기, 현장 패키지, 구성·운영 웹 앱을 소유한다. 플랫폼의 권위 원장·허가 규칙을 우회하지 않는다.

자사 필수 지원: DynamixelSDK, dynamixel_hardware_interface, open_manipulator, ai_worker, ai_sapiens와 각 구성에 필요한 전이 의존성. 기본 포함과 실제 모델/현장 qualification은 별도로 검증한다.

**구현 초안 v0.1을 2026-09-13에 마감했다.** 검증된 phase80 런타임을 인계하며 실제 장비 운전·모든 모델 지원 완료는 아니다. [초안 인계·핵심 미결](https://github.com/jack0682/rx_docs/blob/codex/initial-draft/docs/implementation/draft_handoff.md)을 먼저 확인한다. 미검증 조사 절차 도구는 `codex/investigation-wip`에 별도 보존했다. 전체 설계는 [rx_docs](https://github.com/jack0682/rx_docs), 현재 목표와 상태는 [구현 기록](https://github.com/jack0682/rx_docs/tree/codex/initial-draft/docs/implementation)을 따른다. ROS 비의존 코어·권위 원장·API는 [rx-platform](https://github.com/jack0682/rx-platform)에 둔다.

[운영 앱](apps/operator/README.md)은 platform 로컬 API에 연결한다. [개발용 셀 자료](examples/development/README.md)는 미검증 모의 구성으로만 사용한다. 운영 앱의 요청 회수 기록은 실행 권한이나 장비 결과 원장을 대신하지 않는다.

자사 필수 스택의 실제 CPU arm64 이미지 구성과 기동 경계는 [Native image](dependencies/NATIVE_IMAGE.md)를 따른다.
