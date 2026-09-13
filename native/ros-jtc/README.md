# ROS JTC ROS 연결 계층

2026-09-12. `rx-ros-jtc-bridge`는 ROS 지원표의 position JointTrajectoryController를 선택하여 ROS 2 action의 목표 전송·결과 조회·정확한 목표 취소를 수행한다. C++/ROS 의존성은 `rx-solutions`에 둔다. Platform core에 ROS를 추가하지 않는다.

**현재는 통신 bridge다.** 제품 image에 executable을 포함하며 phase61에서 [Rust NativeAdapter와 영속 원장](../../runtime/rx-host/ROS_JTC_ADAPTER.md)을 library 수준으로 연결했다. 제품 startup factory와 실제 Authority 제공자는 아직 연결하지 않았다. 기본 기동으로 실행되지 않으며, 현재 Host의 영속 native journal·qualification·grant/permit·handover·정상 종료 권위가 이 bridge까지 이어졌다고 주장하지 않는다. 실제 ROS 장비·장비 드라이버·controller_manager를 시작하거나 종료하는 코드도 없다.

## 1. 기본 모델과 controller 선택

빌드할 때 [기본 지원표](../../catalogs/device-support.v1.json)를 포함하고, 설정의 원본 SHA-256을 대조한다. support ID/model/controller 종류·관절 순서를 여기서 선택한다. 설정에서 관절 목록이나 plugin 종류를 임의로 덮어쓸 수 없다.

현재 기본 카탈로그는 새로 작성한 모의 선언이다. `SIM-JTC-6DOF`와 `SIM-JTC-7DOF`의 두 position JTC를 허용하며, leader·impedance와 gripper 전용 action은 거부 시험용이다. 실제 제조사 모델의 관측·검증 자료를 승계하지 않는다.

MANIPULATOR/FOLLOWER/MOBILE_BASE 역할 중 plugin이 `joint_trajectory_controller/JointTrajectoryController`이고 command interface가 position인 경우만 받는다. controller 선언의 선택은 실행 권한이 아니며, Host는 SIMULATION_FIXTURE를 실제 장비 환경으로 사용하는 것을 거부한다.

## 2. 기동 설정

```json
{
  "schema": "rx.ros-jtc-bridge.v1",
  "catalog_sha256": "<current catalog file SHA-256>",
  "support_id": "SIM-JTC-6DOF",
  "controller": "arm_controller",
  "namespace": "/cell_robot",
  "controller_manager": "/cell_robot/controller_manager",
  "domain_id": 171,
  "timeout_ms": 150,
  "capacity": 32
}
```

실제 site 값이 아닌 형식 예시다. catalog hash의 꺾쇠 값은 교체해야 한다. ROS domain은 명시하고, namespace/manager는 절대 ROS 이름이다. action 이름은 namespace + 선택한 controller + `/follow_joint_trajectory`로 생성한다. topic remapping·임의 service 이름이나 ROS global arguments를 명령마다 받지 않는다. parameter service/event publisher도 열지 않는다.

기동은 ROS clients만 만든다. hardware plugin, driver configure/activate, controller switch, init_position, torque enable/disable은 호출하지 않는다. ROS discovery/network 설정 자체가 권한 검증은 아니며, field 배포의 ROS peer/네트워크 접근 제어는 후속이다. 테스트는 명시적 Fast DDS/localhost와 network-none container에서 진행한다.

## 3. 부모와의 IPC

표준입출력의 JSON 한 줄 단위 private pipe다. 입력은 최대1MiB·depth32이며 중복 key·추가/누락 field·잘못된 UUID/counter를 거부한다. stdout은 reply JSON, stderr는 진단이다. 부모는 READY에서 bridge instance UUID와 Linux boot 기반 clock ID/ticks를 얻는다. UUID는 bridge 프로세스마다 바뀐다.

각 요청은 schema, bridge_instance, 증가하는 sequence(십진 문자열), expires_at_ns(같은 CLOCK_BOOTTIME의 절대값), command, body를 가진다. 최대 admission window는1초다. ROS 대기 시간은 설정된 timeout(최대1초)과 남은 요청 시간 중 작은 값으로 제한한다. 이는 소프트웨어 시간 경계이며 hard realtime이나 실제 모터 정지 시간을 보장하지 않는다.

| command | body | 의미 |
|---|---|---|
| inspect | 빈 object | controller 상태·정확한 claimed interfaces·service availability 관측 |
| send | operation UUID, invocation UUID, goal | catalog에 맞는 새 유한 trajectory 목표 전송 |
| result | invocation UUID | 해당 ROS goal UUID의 결과만 조회 |
| cancel | invocation UUID | 이 bridge가 알고 있는 해당 UUID 하나만 취소 요청 |

결과에는 bridge_instance, sequence, clock_id/ticks_ns, state, value, fault가 있다. `REJECTED`는 IPC 요청 거부이지 과거 native 작업이 미실행됐다는 증거가 아니다. read RPC가 응답을 주지 않으면 `RPC_UNKNOWN`이며 native 실패/완료로 바꾸지 않는다.

## 4. trajectory 표현과 preflight

goal은 joints, points, path_tolerance, goal_tolerance, goal_time_ns를 정확히 담는다. points에는 positions/velocities/accelerations/time_ns가 있다. 관절 목록·순서는 catalog 전체와 같아야 하며 partial goal을 받지 않는다. position 배열은 전체 관절 수, velocity/acceleration은 전체 또는 빈 배열이다. 값은 유한 수이고 절댓값1e6 이내이며, 이 수치 한도는 기계 joint limit가 아니다.

최대1024 point, 0보다 큰 strictly increasing time_from_start, 마지막 시간1시간 이내를 요구한다. joint별 path/goal tolerance는 이름·position/velocity/acceleration을 모두 명시하며 양수만 받는다. 이 단계에서는 ROS의 0(default)이나 -1(disabled) 허용오차를 사용하지 않는다. goal_time_tolerance는 0보다 크고60초 이하다. 모든 단위·joint limits·속도/가속도·충돌/기구/교정 검사는 실제 Profile/Envelope에서 추가해야 한다.

송신 직전 ListControllers로 선택한 controller가 active이고 type·position claimed-interface 집합이 정확히 일치하는지 확인한다. 더 많은 joint를 가진 controller에도 일부만 일치한다고 통과시키지 않는다. chained controller는 거부한다.

이 조회는 controller server의 boot identity나 같은 action server라는 암호학적 증거가 아니다. 다른 ROS client의 goal이나 실제 소재 지지 상태도 증명하지 않는다. 응답에는 `controller_generation_known=false`, `physical_readiness_proven=false`를 명시한다.

## 5. 접수·결과·취소

ROS action은 client가 정한 UUID를 goal에 사용하며, 접수와 terminal result는 별도 응답이다. result cache는 server의 설정/수명에 영향을 받는다. cancel 응답 역시 terminal canceled 상태와 구분된다. [ROS 2 action 설계](https://design.ros2.org/articles/actions.html).

bridge는 호출자의 invocation UUID를 SendGoal에 그대로 넣는다. action client가 임의로 다른 UUID를 만들어 나중에 알려주는 형태가 아니다. 송신 경계 전에 프로세스 메모리에 operation/invocation/body를 기록하고 같은 invocation/body의 반복은 원래 receipt를 반환한다. 다른 body·같은 operation의 다른 invocation을 거부한다.

이 bridge가 아는 미해결 goal이 있으면 새 goal로 preempt하지 않는다. 저장 goal 수는 설정 capacity(최대512), 본문 합계16MiB를 넘지 않는다. **이 기록은 비영속적이다.** 외부 Host가 SEND_ENTERED와 원래 UUID를 먼저 영속 저장해야 하며, bridge 재시작 뒤 새 READY를 받아 과거 작업을 다시 보내는 것이 허용되지 않는다. 영속 Host 연결의 현재 범위는 위 NativeAdapter 명세를 따른다. production controller authority와 factory는 후속이다.

SendGoal 응답 유실/시간 초과는 SEND_UNKNOWN으로 보존하고 자동 재전송하지 않는다. 이후 같은 UUID의 result를 조회할 수 있지만 admission fault latch를 자동 해제하지 않는다. 현재 명시적 latch 해제/복귀 명령은 없다. bridge를 재시작해서 차단을 우회하면 안 되며, 후속 Host의 영속 원장·복구/재검증 절차에 따라 새 시작을 결정해야 한다. 일반 결과에는 ROS goal status와 controller error_code/error_string을 함께 남긴다. status UNKNOWN과 error_code0을 성공으로 해석하지 않고, SUCCEEDED와 오류 code가 함께 온 경우도 둘 다 보존한다. 전역 Outcome 결정은 이 bridge의 역할이 아니다. ACK/result/cancel의 실제 수신 시각은 value의 captured_at_ns로 보관하며, cache 응답도 원래 시각을 유지한다. reply의 바깥 ticks_ns는 응답 전송 시각이므로 새 장비 관측 시각으로 사용하지 않는다.

cancel은 known nonzero UUID와 timestamp0만 전송한다. cancel-all/이전 시각까지 일괄 취소는 표현할 수 없다. 응답에 다른 goal UUID가 섞여도 거부한다. 동일 goal의 취소 기록은 메모리에 보존하며 반복 취소를 재송신하지 않는다. CANCEL_UNKNOWN도 보존하고 새 send를 latch한다. 취소 접수 응답의 `terminal_stop_proven`은 false이며 별도 result가 필요하다. terminal action result도 물리 정지·소재 인계 증거는 아니다.

시간 초과한 rclcpp pending request는 제거해 반복 result 조회가 미완료 future를 계속 누적하지 않도록 한다. 이미 실행 중인 장비 효과를 이 제거가 취소하는 것은 아니다.

## 6. 종료와 후속 통합

stdin EOF는 clients/context를 닫는다. goal cancel, controller stop, torque off를 자동 실행하지 않는다. 이 프로세스는 장비 드라이버/로봇 driver의 소유자가 아니므로 그 소멸자를 호출하지 않는다. 이미 실행 중인 goal은 controller에서 계속 실행될 수 있다. parent 소실·SIGTERM·강제 종료를 기계 정지나 정상 인계로 해석하지 않는다.

현재 Rust NativeAdapter는 영속 journal과 artifact/dispatch context를 연결했다. 다음에는 production controller generation·제어권 제공자, package/factory와 qualification/permit 구성을, 실제 관측/최종 오차와 handover/drop proof, 정상 종료/부모 소실 처리가 필요하다. gripper·leader·base·policy 경로와 실제 장비별 물리 검증도 남아 있다.

## 7. 검증

모의 rclpy ActionServer와 실제 C++ ROS service/action clients를 사용한다. 관절/시간/허용오차/추가 field 거부, 비활성·잘못된 claim 집합, 지정 UUID·중복/경합, result UNKNOWN/abort/모순, exact cancel·cancel response와 terminal 분리, preflight deadline, IPC 세대/sequence/중복 key, 응답 유실과 후기 사실을 검사한다. 현재 두 모의 JTC 선언의 startup 선택과 joint set을 대조하며 이 단계에서는 goal을 보내지 않는다.

이 시험은 실제 ROS driver·실장비를 구동하지 않는다. C++ bridge는 제품 S image에 포함하되 기본 process 관리 모드가 자동으로 실행하지 않는다. 과거 검증 원문은 Git 이력에 보존한다. 새 중립 fixture와 이미지의 시험은 별도 실행 결과로 확인한다.
