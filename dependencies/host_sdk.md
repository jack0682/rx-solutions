# Host SDK binding

Host의 journal·gate는 Rust로 구현한다. C++는 ROS·SDK·장비별 native 연동 경계에 유지한다. 기존 두 레포와 두 제품 이미지 경계는 변경하지 않는다.

이 선택은 이미 검증한 정규화·타입·저장 구현을 재사용하고 Host의 native 전달 원자를 일관되게 만들기 위한 구현 결정이다. 플랫폼의 업무 판단을 Host로 복제하지 않는다.

platform의 tools/export_host_sdk.py는 rx-domain / rx-ports / rx-storage / rx-protocol과 규범·IDL을 content hash로 내보낸다. rx-application과 P Runtime state writer는 포함하지 않는다. solutions는 다른 레포의 작업 경로 없이 이 SDK를 빌드한다.

Host 빌드는 SDK 전체 파일 inventory와 각 digest를 검사한다. 기존 파일 변경뿐 아니라 새로운 자동 실행 build.rs 같은 미등록 소스도 거부한다. 이것은 코드 번들의 무결성 검사이며 서명자 신뢰·현장 qualification의 대체가 아니다.
