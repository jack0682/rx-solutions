# RX Solutions

RX는 2026-09-13부터 개인 프로젝트로서 이기종 로봇·시설의 협업, door-to-door 전체 작업과 구역·마을·도시 규모의 확장을 목표로 한다. 이 저장소는 장비·시설 연결, 작업 흐름, 운영 앱과 적용 패키지를 맡는다. [현재 프로젝트 목표](https://github.com/jack0682/rx_docs/blob/main/docs/01_product_definition.md)와 [범위](https://github.com/jack0682/rx_docs/blob/main/docs/03_product_scope.md)는 같은 workspace의 문서 저장소에서 관리한다.

RX의 장비 Host·ROS/native 연동, 선언형 공정과 BT 실행기, 현장 패키지, 구성·운영 웹 앱을 소유한다. 플랫폼의 권위 원장·허가 규칙을 우회하지 않는다.

제조사 중립 장비 계약을 사용한다. 기본 카탈로그는 소프트웨어 시험용 모의 JTC 선언이며, 외부 장비와 설비는 source pin·권한·완료·인계 조건을 가진 패키지로 추가한다. 현재 모의 선언을 실제 장비 지원 완료로 간주하지 않는다.

**구현 초안 v0.1을 2026-09-13에 마감했다.** 검증된 phase80 런타임을 인계하며 실제 장비 운전·모든 모델 지원 완료는 아니다. [초안 인계·핵심 미결](https://github.com/jack0682/rx_docs/blob/codex/initial-draft/docs/implementation/draft_handoff.md)은 기존 산업·모의 셀 초안의 상태 기록이다. 미검증 조사 절차 도구는 `codex/investigation-wip`에 별도 보존했다. 전체 설계는 [rx_docs](https://github.com/jack0682/rx_docs), 기존 초안의 구현 범위와 상태는 [구현 기록](https://github.com/jack0682/rx_docs/tree/codex/initial-draft/docs/implementation)을 따른다. 새 목표의 분산·도시 규모 검증은 후속 대상이다. ROS 비의존 코어·권위 원장·API는 [rx-platform](https://github.com/jack0682/rx-platform)에 둔다.

[운영 앱](apps/operator/README.md)은 platform 로컬 API에 연결한다. [개발용 셀 자료](examples/development/README.md)는 미검증 모의 구성으로만 사용한다. 운영 앱의 요청 회수 기록은 실행 권한이나 장비 결과 원장을 대신하지 않는다.

제조사 중립 솔루션 이미지 구성과 기동 경계는 [Native image](dependencies/NATIVE_IMAGE.md)를 따른다.

## License

RX Solutions is licensed under [Apache License 2.0](LICENSE). Third-party components retain their own licenses and notices.
