# RX BehaviorTree.CPP 실행 노드

RXSequence/RXParallelAll/RXBranch/RXOperation/RXWait/RXIntervention의 실제 C++ 노드와 factory 연결이다. C++17, BehaviorTree.CPP4.8.3 commit `6e469c6ba133aaa842dac9b096b41f2d33ee2b0e`를 사용한다. [공식 소스](https://github.com/BehaviorTree/BehaviorTree.CPP/tree/6e469c6ba133aaa842dac9b096b41f2d33ee2b0e)와 MIT license는 검증 이미지의 vendor tree에 포함된다.

장비 SDK·ROS 제어·P 결과 저장 함수를 노드에 두지 않는다. 검증된 P client가 Context에 Frame을 공급하고 별도 worker에서 Request를 처리해야 한다. **실제 P 통신/상태 변환기는 아직 연결하지 않았다.**

## 실행 경계

- runtime.hpp/cpp: 실행 identity, immutable tick frame, operation/branch 연속성 검사, 요청 큐.
- nodes.cpp: 실제 BT action/control. tick은 메모리 view를 읽고 요청을 enqueue한다.
- xml.cpp: factory 이전 XML 허용 목록·resolved node/속성/자식 관계 검증.
- tests.cpp: 합성 P view의 실행/거부/복원과 Rust compiler 출력 사용.

한 tick은 한 frame을 사용한다. Frame은 run/executor session/resolved digest/visit/epoch에 결합된다. complete/current·monotonic expiry·submission 권한·eligible node를 확인하며, queue에서 꺼낼 때도 권한과 만료를 다시 검사한다. Client는 P의 현재 상태와 deadline으로 Frame을 만들어야 한다. 이 bool/ID에 임의 UI 입력을 연결하면 안 된다.

Expiry는 오래된 view 사용을 차단하는 경계이며 물리 정지 시간이나 원격 완료의 증거가 아니다. Wait도 로컬 timer로 성공을 만들지 않고 P의 decision을 받는다.

## 노드 규칙

- 반복 tick은 같은 node의 Submit 요청을 다시 만들지 않는다.
- UNKNOWN/UNRESOLVED/무결성 상충은 RUNNING으로 대기하며 일반 FAILURE로 낮추지 않는다.
- SUCCEEDED 후에도 release가 없으면 handover를 기다린다.
- Branch는 기록된 decision ID/선택을 사용한다. 기존 선택·operation ID·terminal outcome 변경과 완전한 view의 binding 유실을 거부한다.
- Parallel은 실패/불명/개입 대기에서 새 admission을 억제한다. 진행 중 sibling을 native cancel 완료로 취급하지 않는다.
- Halt는 PauseExecutor 요청을 남기고 새 요청을 중단한다. native cancel·안전 정지·자원 해제의 증거가 아니다.

잘못된 frame은 context를 정지시키고 pause를 요청한다. 정상32개 큐와 별도 pause1개를 둔다. Context의 요청 식별은 run/node/visit/kind다. P client는 이를 규범 key/activation/slot에 연결하고 송신 불명·응답 유실을 같은 요청으로 회수해야 한다. 프로세스 재시작의 영속 키/checkpoint 조정은 후속이다.

## XML 검증

한 root/한 BehaviorTree, 정확한6개 RX 노드 종류, ID·binding/condition/procedure/timeout, 자식 순서·완전한 node 목록을 대조한다. Script/pre/post condition/include/임의 builtin/추가 attribute/중복 node를 금지한다. Registry와 XML은 검증한 같은 resolved artifact에서 제공해야 한다.

## 빌드·시험

`dependencies/behaviortree_cpp.repos`의 exact commit을 `vendor/BehaviorTree.CPP`에 준비한다. Vendor는 변경하지 않는 원본 cache다.

```text
python3 tools/test_bt_executor.py --evidence-dir OUTPUT_DIRECTORY
```

도구는 commit과 source 무변경을 확인하고 executor-validation Docker target을 빌드한다. 시험은 network none/read-only/cap-drop all/no-new-privileges와 읽기 전용 fixture mount에서 수행한다. **제품 두 이미지 중 하나가 아닌 검증 target**이며 장비별 ROS stack/device 권한/현장 기동은 포함하지 않는다.

검증 범위는 실제 BT engine의 네 시나리오(순서/인계, 분기/대기/개입, 병렬 불명/권한 회수, XML/큐 한도)와 Rust가 생성한5단계 예제다. P frame·완료/clearance는 합성 fixture다. 실제 P gRPC mapping·image 상주 운영·장비 qualification을 검증했다고 표현하지 않는다.

## 다음 연결

P의 run/checkpoint/branch/visit eligibility, 정확한 protocol-to-Frame 변환, 요청 receipt·영속 key 회수, journal 재접속, process supervisor·operator UI를 연결해야 한다. Context는 P의 명시적 재시작 허가를 스스로 만들지 않는다. Native control·최신 sample/deadman은 Host가 계속 소유한다.

현재 P→S read client의 typed Frame과 [source clock/IPC 경계](FRAME_BOUNDARY.md)를 연결했다. 유한 operation의 S worker와 영속 요청 key를 연결했다. 전체 daemon·나머지 mutation과 명시적 restart/clearance는 후속이다.

지속 실행 파일은 `rx-bt-engine`이며, 초기화·STEP·HALT·CLOSE와 시계/IPC/부모 상실 규칙은 [지속 엔진 명세](PERSISTENT_ENGINE.md)에 정리했다. `rx-bt-engine-fixture`는 test-harness 빌드 전용이다.
