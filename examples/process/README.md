# 소재 공급 공정 예제

문 열기 → 소재 배치 → 척 닫기 → 로봇 이탈 → 문 닫기의 원본과 모의 binding이다. 어떤 실제 현장의 실행 순서/인터록 검증을 완료했다는 뜻이 아니다.

`material-supply.source.json`은 공정 구조, `material-supply.bindings.json`은 명시적인 simulation 대상이다. program/parameter artifact는 placeholder다. Q03UDVCPU 주소·실제 로봇 동작·TCP·교정·센서 판단·소재 지지 정보가 들어 있지 않으며 운전에 사용하지 않는다.

`rx-process-compile`으로 컴파일할 수 있다. 이 컴파일은 파일/구조/자원 중복 검사이며 RX custom BT node 실행·장비 qualification·실물 인수는 후속이다.
