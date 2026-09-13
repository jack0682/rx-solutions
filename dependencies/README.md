# 기본 의존성과 선택 장비 소스

기본 솔루션 이미지는 ROS Jazzy, 일반 JointTrajectoryController 통신, BehaviorTree.CPP 및 RX 실행파일과 운영 앱으로 구성한다. 특정 제조사 SDK·bringup·정책모델은 기본 의존성이 아니다.

`native-stack.lock.json`은 ROS base image digest와 의존성 범위를 기록한다. `native.repos`의 외부 장비 저장소 목록은 현재 비어 있다. 장비를 추가할 때 저장소별 불변 Git commit과 필요한 자산의 실제 SHA-256을 기록하고 별도 qualification을 수행한다. 저장소에 포함됐다는 사실은 운전 권한이 아니다.

선택 소스의 materialization은 `tools/prepare_native_sources.py`와 `tools/native_source_inventory.py`로 검사할 수 있다. 현재 기본 이미지는 이 외부 소스 context를 요구하지 않는다. 구체 빌드·진단 경계와 재현성 한계는 [Native image](NATIVE_IMAGE.md)를 따른다.
