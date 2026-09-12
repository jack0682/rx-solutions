# ROBOTIS JTC NativeAdapter — 영속 작업·제어권·bridge 소유

2026-09-12. `ros_jtc::Jtc`가 기존 Host의 NativeAdapter에 연결된다. 실제 Rust Host → private pipe → C++ ROS bridge → 모의 ActionServer 경로를 구현했다. **제품 startup factory 등록과 실제 controller authority 제공자는 아직 미완료**다. 이 library를 첫 현장 운전 적합성으로 승격하지 않는다.

## 1. 구성과 책임

| 구성 | 책임 |
|---|---|
| Profile / TrajectoryAsset | 지원표에 맞는 model/controller·관절 순서, 원본 trajectory bytes의 hash/size, site/calibration/resource와 Intent binding |
| Process | release-owned executable pin·개인 디렉토리 복사·config·bounded stdin/stdout·bridge instance/clock/sequence 검사·자식 회수 |
| Jtc native journal | 송신 전 operation/invocation/intent/artifact/controller-session 기록, ACK/result·capture·분쟁 보존, 재실행 금지 |
| Authority | 현재 controller 세대·독점 제어권·외부 goal·조건·지지·client 종료 허가의 독립 근거 |
| Host | 현재 qualification/epoch/grant/permit·최종 장치 세대/만료 검사·delivery/evidence와 전역 조정 |

Authority는 release-owned Rust trait다. site JSON에 `safe=true`를 적는 경로가 아니다. 기본 `UnavailableAuthority`는 snapshot을 제공하지 않아 동작을 거부한다. 이 단계의 Authority 구현은 모의 controller를 소유한 테스트 fixture다. 실제 제공자는 세대 재사용 방지와 관측 신선도, 보호 요청 이후의 상태 철회를 구현해야 한다.

## 2. 프로파일과 목표 원본

현재는 FiniteAction/Trajectory만 받는다. Intent의 target/profile/site/calibration/resource, trajectory ArtifactRef/joint_group/tool과 완료·취소 규칙을 정확히 맞춘다. 목표의 canonical bytes가 선언된 SHA-256/size와 일치해야 하며, artifact schema는 `rx.ros-jtc.goal.v1`이다. 바뀐 trajectory를 같은 참조로 보내지 않는다.

관절·point·finite value·time/tolerance 검사는 [ROS bridge 명세](../../native/ros-jtc/README.md)와 맞춘다. 목표당512KiB, 프로파일당16개 trajectory를 제한한다. 이들은 입력/자원 한도이며 실제 robot geometry·joint limit·collision·calibration 검증이 아니다. 실제 Profile/Envelope qualification이 그 적합성을 별도로 확인해야 한다.

## 3. bridge 프로세스와 통신

실행파일은 operation이나 site JSON이 아닌 release composition이 제공한다. 원본 regular file을 최대16MiB 범위로 취득해 hash를 확인하고, owner 전용 임시 디렉토리에 복사해 실행한다. 원본 파일이 이후 바뀌어도 이미 선택한 실행 bytes가 바뀌지 않는다. config도 해당 디렉토리에 쓴다. shell을 거치지 않고 고정 인자 한 개로 실행한다.

환경 변수는 ROS/library 경로와 discovery/log 관련 허용 목록만 받으며 기본 환경은 비운다. 이 환경 역시 trusted release 입력이다. library 경로의 실제 공급망·ROS peer 보안 정책을 검사하는 production resolver는 후속이다.

Unix nonblocking pipe와 poll로 송수신을 제한한다. 최대1MiB reply/command, 정확한 schema·bridge instance·sequence·현재 boot clock을 확인한다. 일부만 받은 줄·oversize·다른 instance·I/O timeout은 pipe를 faulted로 만들며 자동 재기동/재송신하지 않는다. stdout과 stdin은 이 child 전용이다.

복사한 executable을 실행하므로 runtime 디렉토리의 filesystem에 execute 권한이 있어야 한다. Linux 시험은 `--tmpfs /tmp:rw,exec`를 사용한다. 초기 noexec tmpfs 시험은 실행 거부로 실패했으며 원래 파일을 우회 실행하는 fallback을 추가하지 않았다. 실제 배포에서는 private runtime mount 정책을 명시해야 한다.

## 4. controller 세대와 허가 유효기간

ROS bridge instance와 controller session은 다르다. bridge가 재시작돼도 같은 controller가 살아 있을 수 있고, bridge가 유지돼도 controller가 바뀔 수 있다. NativeCapture의 device_session에는 Authority가 확인한 controller session을 사용한다. bridge의 `controller_generation_known=false`를 controller 식별 증거로 바꾸지 않는다.

guard는 독립 Authority snapshot과 현재 ROS controller/type/claim/service 상태를 확인한다. 그 앞뒤 controller session이 같아야 한다. ready/조건과 관측 age도 확인한다.

Host는 마지막 guard의 device_session과 **permit expiry/guard expiry 중 이른 값**을 NativeDispatch로 넘긴다. Jtc는 원장 commit 후에도 그 session/expiry를 다시 확인하고, C++ send 요청의 expires_at_ns까지 전달한다. 중간 조회·저장이 오래 걸리거나 세대가 바뀌면 goal을 보내지 않는다. 물리 adapter가 이 context를 처리하지 않으면 기본 구현은 거부한다. 기존 Melsec도 같은 context를 최종 ready 검사에 연결했다.

현재 device session 검사는 Rust의 마지막 Authority snapshot에서 끝난다. C++/표준 ROS SendGoal은 controller session을 검증하는 필드를 받지 않는다. 그 확인과 실제 ROS 수신 사이의 controller 교체까지 이 코드만으로 막았다고 주장하지 않는다. production Authority/관리자가 세대를 재사용하지 않는 endpoint·lifecycle/fencing 등의 방식으로 이 간격을 닫고 검증해야 한다. 그 전에는 실장비 지원 완료가 아니다.

이는 dispatch admission 경계다. 이미 controller가 받은 trajectory를 만료 시각에 물리적으로 멈춘다는 뜻은 아니며, 실제 정지/유지 조건은 별도다.

## 5. native journal과 재시작

새 디렉토리에만 native journal을 초기화하며 기존 identity/profile을 고정한다. open은 DB 유실·빈 파일·다른 journal/profile·개수/index와 잘못된 capture를 거부한다. Host journal과 native journal은 별개이며 장비와 공통 transaction을 만들지 않는다.

송신 전에 operation/invocation, 전체 Intent와 digest, profile/goal digest, controller session, bridge instance, 진입 시각을 원자 기록한다. 한 native pending slot을 두고 다른 operation ID로 우회하지 못하게 한다. 동일 ID의 다른 body는 충돌이며 동일 요청의 반복 submit은 원래 기록만 반환한다.

ACK는 접수 사실로 보관한다. 접수만으로 Host evidence에 성공을 기록하지 않는다. action 서버가 거부한 경우는 별도의 `rx.ros-jtc.goal-rejected.v1` native fact다. 완료 결과는 succeeded/canceled/aborted schema와 controller code를 구분한다. ROS success와 nonzero controller error의 모순은 raw reply·분쟁을 보존하고 capture를 만들지 않는다. 그 뒤 깨끗한 결과가 왔다고 기존 분쟁을 자동 덮어쓰지 않는다.

reply 유실이나 재시작 후에는 original invocation의 result만 조회한다. Authority가 현재 controller session을 원래 기록과 같다고 확인할 때만 새 result를 연결한다. 세대가 다르면 None/미확정 상태를 유지한다. 이미 영속 capture가 있으면 원래 session·수신 시각으로 회수한다. 새로운 send를 만들어 결과를 추정하지 않는다.

현재 native journal은10,000개 entry 한도가 있고 보관/정리·수동 분쟁 복구 API는 없다. 미해결·분쟁 작업을 새 ID나 재시작으로 우회하는 방법은 제공하지 않는다.

## 6. 인계와 정상 종료

action 완료는 지지·종료 허가가 아니다. handover는 pending 없음과 독립 control/support 근거를 반환한다. 정상 종료도 native pending이 없어야 하며 no-external-goals·support-stable·client-drop-allowed가 필요하다. 미해결 또는 분쟁 entry가 남아 있으면 child close를 시작하지 않고 owner를 유지한다. 별도 복구·책임 인계 경로는 아직 없다.

Host admission을 닫은 뒤 `prepare_shutdown`을 호출한다. 조건이 충족되면 bridge stdin EOF를 시작하고 실제 child 종료를 반복 확인한다. child가 아직 살아 있으면 Busy로 owner를 유지한다. timeout만으로 kill하지 않는다. `shutdown_snapshot.safe_to_drop`는 child가 종료되고 최신 Authority 조건이 계속 맞을 때만 true다.

뜻하지 않은 adapter 소멸에서는 독립 보호 callback을 호출한다. Process는 pipe를 닫고 child와 실행 디렉토리를 reaper에 넘기며, goal cancel/driver stop/torque off를 자동 보내지 않는다. 보호 callback이나 child 종료 자체를 물리 정지로 해석하지 않는다. 부모 전체 강제 종료/전원 상실의 기계 거동은 별도 검증 대상이다.

## 7. 검증 범위와 다음 연결

시험은 executable pin/원본 변경·pipe timeout/oversize/instance mismatch, 원장 유실·capture 손상, 미확정 재시작·새 ID 우회 거부, controller 세대/제어권·지지/child exit, 모순 결과 보존을 다룬다. 별도 자식 프로세스를 entry/send/capture 후 종료하고 추가 native effect가 없는지도 확인한다.

Linux 통합은 실제 SystemClock, 복사해 실행한 C++ bridge, 실제 ROS action/service 메시지와 모의 controller를 사용한다. Host prepare/authorize→원래 UUID 하나의 goal→reconcile/evidence→인계/종료를 확인한다. 이때 Authority는 명시적 fixture이며 실제 로봇에 대한 proof가 아니다.

실제 실행 결과와 source/image hash는 [phase61 검증 기록](../../../references/implementation/phase61_checks.json)을 따른다. 생산용 Authority·ROS controller lifecycle/identity, 장비 package authoring와 factory 등록, native cancel의 영속 Host 조정·복구, 실제 ROBOTIS 장비의 calibration/지지·인수는 남아 있다. 첫 물리 셀은 NOT_COMMISSIONED다.

## 8. phase62 · 플랫폼 결과 해석 연결

`Profile::outcome_table()`은 profile digest와 완료 규칙에 결합한 공유 데이터 표를 반환한다. 플랫폼의 `CompletionRule::NativeOutcomes`에 이 표와 성공 후조건을 넣으면 P의 기존 증거 transaction에서 성공·실패·취소를 판정할 수 있다. 실제 표 생성과 native capture의 해석을 모의 adapter로 확인했다.

성공 schema/code0, canceled schema/알려진 code, aborted schema/알려진 code를 구별한다. goal rejection은 FAILED이며 native 송신 전 NOT_EXECUTED로 바꾸지 않는다. 모르는 조합은 원본만 보존하고 결론을 만들지 않는다. 현재 이 반환값을 서명된 장비 패키지·P 구성으로 자동 전달하는 resolver는 후속이다. 생성 함수 호출만으로 설치/승인/운전 허가가 생기지 않는다.

정확한 대응표·호환성·반례는 [코어 결과 명세](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/NATIVE_OUTCOMES.md), 실제 시험 범위는 [phase62 검증 기록](../../../references/implementation/phase62_checks.json)을 따른다.

phase63에서 [Template/Site 패키지 작성·검증·Host 등록](JTC_PACKAGE.md)을 연결했다. 서명된 assembly를 재계산하여 profile/operations/outcomes를 대조하며 JTC_PACKAGE 검사와 원장 초기화를 지원한다. 현재 제품 factory에는 실행용 Authority/lifecycle 제공자가 없어 run은 ROS client를 만들기 전에 거부한다. 위의 phase61/62 후속 항목 중 패키지 작성·metadata 등록을 진전시킨 것이며, 실제 제어권·물리 검증 및 P 구성 반입 자동화까지 완료한 것은 아니다.
