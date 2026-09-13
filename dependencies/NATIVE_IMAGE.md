# 제조사 중립 솔루션 이미지

2026-09-14 구성. ROS Jazzy, position JTC bridge, BT engine, Rust Host·Executor·Supervisor·패키지 도구, 운영 UI를 포함한다. 기존 제조사별 이미지의 빌드·설치 시험 결과는 Git 이력에 보존하며 새 이미지의 성공 근거로 사용하지 않는다. 물리 장비 qualification은 `NOT_PERFORMED`다.

## 소스와 빌드

ROS·Rust·Node base image는 Dockerfile의 digest로 고정한다. BehaviorTree.CPP는 기존 고정 commit을 `dependencies/behaviortree_cpp.repos`에 맞춰 준비한다. 기본 장비 카탈로그는 실제 모델을 대체해 이름을 바꾼 자료가 아니라 새로 작성한 `SIMULATION_FIXTURE`다. 외부 장비 저장소와 정책 자산 목록은 비어 있다.

```sh
vcs import vendor < dependencies/behaviortree_cpp.repos
python3 tools/check_device_catalog_sources.py
docker build --build-context btcpp=vendor/BehaviorTree.CPP \
  -f docker/Solutions.Dockerfile --target runtime \
  -t rx-solutions:runtime-draft .
python3 tools/test_solutions_image.py --image rx-solutions:runtime-draft --evidence /tmp/rx-solutions-image.json
```

SDK는 platform의 현재 export와 일치해야 한다. ROS JTC와 BT의 C++ compile 단계는 네트워크를 차단한다. 설치 검사는 필수 RX 실행파일의 존재·ELF 형식·동적 링크를 확인하며, controller나 하드웨어 plugin을 실행하지 않는다.

APT 패키지의 실제 버전·architecture는 이미지별 `native-packages.tsv`에 기록하고 runtime inventory에 포함한다. **APT 저장소 snapshot이나 architecture별 승인 inventory는 아직 고정하지 않았다.** 기본 이미지 digest만으로 완전한 재현 빌드가 보장되지 않는다. 이전 제조사 구성의 APT lock을 새 구성에 재사용하지 않는다.

선택 외부 소스가 필요하면 `native-stack.lock.json`의 repositories에 HTTPS URL과 40자리 commit을 기록하고 `tools/prepare_native_sources.py`를 사용한다. 이 도구는 원본을 수정하지 않고 Git archive를 별도 경로에 준비하며 submodule·미해결 LFS·자산 해시 불일치를 거부한다. 선택 소스의 설치·실행 경로는 별도 변경과 검증이 필요하다.

## 기본 실행과 검증 경계

기본 entrypoint는 ROS 환경을 설정한 뒤 읽기 전용 software diagnostics만 실행한다. 사용자 `10001:10001`로 실행하며 상태는 `SOFTWARE_READY_UNCOMMISSIONED`다. `/health`와 `/api/v1/solution/support`는 설치 수와 모의 선언 수를 제공하고 control POST는 `CONTROL_NOT_EXPOSED`로 거부한다.

기동 때 실행파일·카탈로그·UI·진단 코드·설치 목록의 SHA-256을 확인한다. 기본 기동은 controller manager, 장비 driver, BT engine, Host·Executor나 ROS router를 시작하지 않는다. UI bundle은 포함하지만 진단 서버가 production 운영 API를 대신하지 않는다.

이미지 smoke test는 read-only root, cap-drop ALL, no-new-privileges, device 미연결 상태에서 진단·control 거부·SIGTERM 종료와 카탈로그 변조 거부를 검사한다. 실제 장비·현장 인계·실시간 성능·다중 영역 운영·production Authority 제공자는 별도 미결이다. `supervise`와 `host`의 명시적 실행은 각 서비스의 계약을 따른다.
