# 자사 필수 스택과 솔루션 이미지

2026-09-11. CPU linux/arm64 솔루션 runtime draft를 빌드했다. 다섯 자사 원본을 선택 설치 모듈로 낮추지 않았으며, 필요한 공통 interfaces/hand 소스까지 포함해 42개 ROS 패키지를 컴파일했다. 빌드·설치·기동 확인과 실제 로봇 지원 qualification은 구별한다.

## 원본과 의존성

`native-stack.lock.json`은 다음을 고정한다.

- 사용자 지정 5개: DynamixelSDK, dynamixel_hardware_interface, open_manipulator, ai_worker, ai_sapiens.
- 전이 Git 소스 3개: dynamixel_interfaces, robotis_interfaces, robotis_hand.
- 각 Git commit·ROS Jazzy base image digest·rosdistro metadata commit.
- ONNX Runtime 1.23.2의 architecture별 배포 URL/SHA-256.
- Sapiens의 ONNX 4개와 설정/동작 자료를 포함한 11개 자산의 실제 bytes/hash.
- 이번 arm64 빌드의 APT 패키지/버전/architecture inventory. 최종 runtime은 이를 비교하고 달라지면 거부한다.

자사 5개 원본의 로컬 HEAD와 작업 트리 무변경을 확인했다. `prepare_robotis_sources.py`는 원본 디렉터리를 편집하지 않고 Git commit archive에서 복사한다. 필요한 전이 저장소만 RX `.cache/git`에 받는다. submodule/LFS pointer가 해소되지 않았거나 필수 자사 원본이 누락되면 거부한다.

기본 파일 검색에서는 보이지 않았던 Sapiens 자산을 고정 Git archive에서 확인했다. 정책이 없다는 초기 판단을 정정했고 실제 아카이브의 11개 파일을 pin했다. 폴더 이름만으로 자산 존재를 통과시키지 않는다.

`native_source_inventory.py`는 경로·파일 내용·실행 모드·symlink 대상에 대한 tree hash를 검증한다. 자체 source-lock만 믿지 않고 지원 catalog의 Git commit과 각 참조 파일 SHA도 실제 빌드 소스와 대조한다.

## 빌드 흐름

```text
python3 tools/prepare_robotis_sources.py --local-root /absolute/path/to/ROBOTIS_repositories
python3 tools/native_source_inventory.py .cache/robotis

docker build --build-context robotis=.cache/robotis \
  --build-context btcpp=vendor/BehaviorTree.CPP \
  -f docker/Solutions.Dockerfile --target runtime \
  -t rx-solutions:runtime-draft .
```

`--local-root`를 생략하면 lock의 저장소를 다운로드한다. BehaviorTree.CPP는 기존 고정 commit의 vendor source를 별도 build context로 전달한다. SDK는 platform의 현재 export와 일치해야 한다.

1. source에서 package.xml을 별도 manifest로 만들고 ROS dependency 설치를 소스 compile 단계와 분리한다.
2. rosdep 정의/ROS base를 pin하고 필수 의존성을 설치한다. 선택 결과는 APT inventory로 남긴다.
3. ONNX Runtime archive hash를 검사하고 설치한다.
4. 네트워크 없는 compile 단계에서 42개 package 전체를 sequential build한다.
5. 설치 package marker, 69개 ELF의 동적 라이브러리 의존, 17개 catalog controller/hardware plugin 선언, policy 자산, ONNX CPU session load를 검사한다.
6. Rust 실행기/공정 compiler, 고정 BT.CPP 기반의 production BT engine, 빌드한 React 운영 UI를 합친다.
7. 실행 파일/UI/catalog의 별도 hash inventory를 만들고 비루트 runtime stage로 마무리한다.

첫 GUI compile은 메모리 부족으로 실패했다. GUI를 제외하지 않고 MAKEFLAGS·CMAKE_BUILD_PARALLEL_LEVEL·Qt autogen 병렬 수를 1로 제한했다. 자사 원본의 제어 코드를 수정하거나 기능을 잘라 통과시키지 않았다. 일반 package의 미사용 CMake 인자 경고는 별도 기록이며 정상 compile 결과와 구별한다.

ONNX shared library의 runtime 검색 경로 누락은 `/etc/ld.so.conf.d/rx-onnxruntime.conf`와 ldconfig로 해결했다. 단순 compile 성공으로 동적 링크 검사를 생략하지 않았다.

## 설치 검사 범위

- ROS package.xml이 말하는 source/compile 의존과 CI repos의 common interfaces/hand 의존을 포함했다.
- 지원표 22개 profile이 사용하는 controller/hardware class 이름이 설치된 pluginlib 선언에 존재하는지 확인했다. 실제 hardware plugin 인스턴스를 만들지 않는다.
- MoveIt의 다른 plugin XML에는 템플릿 속성이 있다. 그것을 일반 XML 파서로 일괄 판정한 오류를 RX catalog class의 누락으로 처리하지 않았다. MoveIt의 모든 실행 경로를 시험한 것으로 표시하지 않는다.
- 정책 4개는 실제 ONNX Runtime CPU Session으로 열어 입력/출력 존재를 확인했다. 이 검사는 ROS 노드나 명령 publisher를 실행하지 않는다. 제로 입력 제어 명령을 장비로 내보내지 않는다.
- 정책이 열렸다는 사실은 실제 관측 배열·교정·모드·속도·1000 Hz 실행 요구를 검증한 것이 아니다.

ONNX 배포판은 [공식 v1.23.2 release](https://github.com/microsoft/onnxruntime/releases/tag/v1.23.2)의 asset URL과 공개 SHA-256을 확인했다. ROS documentation 웹 조회가 차단된 부분은 완료 근거로 사용하지 않고 실제 pinned rosdistro/rosdep 처리와 빌드 산출물을 남겼다.

## 기본 실행

기본 entrypoint는 ROS/overlay 환경을 설정하고 **읽기 전용 software diagnostics**만 실행한다. upstream Docker의 s6 agent·bringup 자동 실행 설정을 이 경로에 연결하지 않는다.

- USER 10001:10001.
- `/health`, `/api/v1/solution/support`에서 package/model inventory 상태 제공.
- control POST는 CONTROL_NOT_EXPOSED로 거부. 사용자 비밀번호를 수집하거나 service identity로 P에 전달하지 않는다.
- controller_manager, driver, policy node, BT engine, executor service, ROS router를 자동으로 기동하지 않음.
- 운영 UI bundle은 `/opt/rx/operator`에 포함하지만 현재 diagnostic server에서 production operator API를 대신하지 않음.
- 시작 시 native ELF·policy·Rust/BT 실행 파일·catalog/UI hash를 다시 검사.
- software ready는 SOFTWARE_READY_UNCOMMISSIONED이며 physical qualification은 NOT_PERFORMED.

`tools/test_solutions_image.py`는 실제 이미지에 device/GPU를 매핑하지 않고 read-only root·cap-drop ALL·no-new-privileges로 실행한다. package42/profile22/asset11과 프로세스 목록, control 거부, SIGTERM exit를 확인하고 자기 test container만 정리한다.

## 버전/운영 한계

이번 APT lock은 설치 결과와의 **불일치 거부**를 제공한다. 과거 .deb를 영구 보관하는 mirror나 완전한 offline rebuild 공급망은 아직 만들지 않았다. 공개 저장소에서 버전이 사라지면 build를 중단해야 하며 최신 버전으로 조용히 대체하지 않는다. arm64 inventory만 확정했으므로 다른 architecture의 최종 target은 별도 inventory·시험이 필요하다.

현재 runtime draft에는 진단/개발을 위한 일부 build 도구·GUI/시뮬레이션 의존성이 남아 있다. production 최소 패키지 축소, GPU/센서 variants, ZED/실제 카메라와 제어 보드 조합, 실제 realtime 성능은 후속이다. 플랫폼 이미지와 구별되는 솔루션 이미지가 생겼다는 사실을 전체 출하 검증으로 표시하지 않는다.

자동 Host 연결/프로세스 supervision, model별 준비·driver gate·관측·P permit 연결, operator TLS/delegation, 실제 현장 qualification은 계속 필요하다. 이미지에서 ros2 launch를 수동 실행할 수 있다는 것과 RX가 그 실행을 허가했다는 것은 다르다.


추가 Sapiens validation target은 동일 소스로 7개 gtest 실행 파일의 46개 시험을 통과했다. CTest 13개 그룹은 실패 없이 끝났지만 cppcheck 2.13.0은 upstream ament의 알려진 성능 문제 정책에 따라 87개 항목을 skip했다. 이를 static-analysis 완료로 세지 않는다. XML lint가 외부 schema URL을 요구하던 문제는 공식 REP 저장소의 고정 XSD와 로컬 XML catalog로 해결했고, test 네트워크 차단은 유지했다.
