# RX 공정 원본과 결정적 컴파일

S가 소유하는 ProcessSource·resolved tree·다음 작업 후보 계산이다. 장비 I/O와 P의 운전 권한/결과 원장을 포함하지 않는다. P가 검증한 결과와 checkpoint를 바탕으로 다음 요청을 제안하는 계층이다.

## 원본 표현

`ProcessSource`는 process ID, entry flow, 조건 정의와 flow별 고유 node graph다. 각 flow는 tree 형태이며 재사용은 고유한 Call node로 표현한다.

| 노드 | 의미 |
|---|---|
| Sequence | 앞 단계가 완료되고 필요한 자원 인계가 확인된 뒤 다음 단계 |
| ParallelAll | 분리된 실제 resource set을 사용하는 병렬 경로. 모두 완료되어야 성공 |
| Branch | P에 영속된 조건 결정으로 선택된 한 경로 |
| Repeat | 명시한 유한 횟수만큼 서로 다른 실행 위치로 펼침 |
| Call | 하위 flow를 호출 위치별로 별도 인스턴스화 |
| Operation | 현장 resolver가 제공한 Host/Intent binding 참조 |
| Wait | 조건과 명시적 deadline. P의 기다림 결정으로 진행 |
| Intervention | 절차 참조와 개입 대기. 허용된 P continuation이 있어야 진행 |

source/flow/node 중복, 누락·도달 불가·cycle·공유 node, recursive call, 빈 control node, 반복/깊이/확장 한도 초과를 거부한다. 조건 grammar도 크기·빈 그룹·범위 역전을 검사한다.

## 컴파일 identity

반복/호출은 인스턴스 경로를 포함한 고유 node ID와 SourceLocation으로 펼친다. 원본 node ID·call/반복 경로가 같으면 tree 정의를 직렬화하는 순서가 바뀌어도 identity가 유지된다. flow/node 목록은 source digest에서 정규화하며, Sequence child 순서는 의미 있는 실행 순서로 보존한다.

resolved digest는 원본 identity와 실제 normalized Host/Intent binding에 결합한다. 병렬 경로의 resource union이 겹치면 거부한다. 이는 이름이 다른 두 논리 기능이 같은 controller를 공유할 때도 해당 resolved resource 이름을 기준으로 한다. 현장 resolver가 alias를 제대로 통합했는지는 별도 검증 대상이다.

`compile_package`는 VerifiedPackage의 immutable Process entry만 읽고, 사용한 binding의 OperationSubmit 요청이 package permission에 선언되어 있는지 확인한다. 서명이 맞아도 잘못된 process source는 통과하지 못한다. 이 permission 검사는 site/Host의 native grant를 부여하지 않는다.

## Frontier 계산

입력 ProgressView는 **인증·검증한 P의 완전한 run view**여야 한다. 로컬 UI/BT가 임의로 만든 evidence ID나 bool을 이 입력으로 신뢰하면 안 된다. 현재는 pure planning API이며 그 P wire/checkpoint adapter는 후속이다.

- UNKNOWN/DISPUTED/UNRESOLVED는 BLOCKED로 유지한다. FAILURE로 바꾸어 자동 fallback/retry하지 않는다.
- SUCCEEDED만으로 다음 sequence를 시작하지 않고 필요한 resource release를 기다린다.
- branch decision이 없으면 decision 요청 후보만 만든다. 현재 센서값을 읽었다는 이유로 지역 메모리에서 분기를 확정하지 않는다.
- 선택하지 않은 branch 이력, 앞 단계가 빠진 뒤 단계 이력, 다른 resolved digest, partial view, 잘못된 node/operation 상관관계는 거부한다.
- parallel의 실패/불명에서는 새 admission 후보를 억제하고 이미 진행한 작업은 남긴다. native cancel 성공을 추론하지 않는다.
- wait timeout은 명시적인 P 결정이며 물리 완료가 아니다. intervention clearance도 실제 P authorization과 결합해야 한다.

Frontier의 COMPLETED는 이 계획 view의 구조상 완료다. part의 CONFIRMED_COMPLETED/양품/운전 재개를 직접 기록하지 않는다.

## BT XML과 현재 제한

BT.CPP format4 XML을 생성한다. [공식 XML 형식](https://behaviortree.dev/docs/tutorial-basics/tutorial_07_multiple_xml/)을 사용하되 RXSequence/RXParallelAll/RXBranch/RXOperation/RXWait/RXIntervention **전용 노드 등록이 필요하다**. 임의 Script/include, generic RetryUntilSuccessful, UNKNOWN을 FAILURE로 낮추는 변환은 출력하지 않는다.

여섯 RX C++ 노드의 실제 BT.CPP factory 등록과 합성 P view 실행 시험은 [native executor](../../native/executor/README.md)에 구현했다. 실제 P client·영속 요청/분기/checkpoint 복원은 아직 연결하지 않았다. XML과 단위 실행만으로 제품 운전 경로가 완성됐다고 주장하지 않는다.

공통 model/frontier는 P 소유의 rx-process-contract SDK를 사용한다. P는 graph 설정에서 branch/wait/checkpoint와 activation/Submit eligibility를 실제 transaction으로 검증한다. 외부 RPC/checkpoint artifact·C++ client와 전체 intervention/restart 연결은 아직 미완료다. 기존 process=None finite 설정에 graph를 임의로 평평하게 넣어 운전하지 않는다.

## CLI와 예제

```text
rx-process-compile SOURCE.json BINDINGS.json NEW_OUTPUT_DIRECTORY
```

새 디렉토리에 resolved.json, process.bt.xml, compile-report.json을 만든다. 기존 산출물은 덮어쓰지 않는다. 결과는 COMPILED_NOT_QUALIFIED이며 device/process를 실행하지 않는다.

`examples/process`는 소재 공급의 다섯 단계를 표현한 **미검증 예제**다. 실제 로봇/PLC 신호·program·교정·그리퍼·지그 binding이 아니다. placeholder artifact는 실제 배포 자료로 대체하고 검증해야 한다.
