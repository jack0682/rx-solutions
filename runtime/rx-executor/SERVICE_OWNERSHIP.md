# Run 종료 후 실행기 소유권 회수

`RunService::run_owned()`는 기존 run loop와 stop/pause 처리를 마친 뒤 `ServiceExit<R, F>`를 반환한다. `report`는 기존 `Report`, `worker`는 사용하던 `Worker<R>`, `factory`는 사용하던 planner factory다. `Worker::into_parts()`는 그 Worker의 `Client`와 `Journal<R>`를 다시 열지 않고 돌려준다. Client의 인증 session과 공유 runtime boot·sequence·checked 시각 검증 상태가 그대로 유지된다.

기존 `RunService::run()`은 `run_owned()` 결과의 Report만 반환하는 wrapper다. 기존 run-only CLI의 출력, stop/pause 요청·관측·저장, 통신 grace와 timeout 흐름은 유지한다.

`ServiceExit::planner_cleanup`은 로컬 planner 정리 결과를 별도로 제공한다.

- `NOT_STARTED`: 이 RunService가 planner 생성을 시도하지 않았다.
- `CONFIRMED`: 생성한 모든 planner가 기존 CLOSE 응답 검증과 child 종료 확인을 완료했다.
- `UNCONFIRMED`: spawn 또는 close/serial retire가 실패했거나 취소되어 child 소유권·종료를 확인하지 못했다.

`is_confirmed()`는 NOT_STARTED와 CONFIRMED에서만 true다. 이는 다음 Run 전환의 필요조건이며 P의 실제 COMPLETED, 정확한 assignment journal 연결, 현재 session과 fresh 상태 검증을 대신하지 않는다. Host/controller의 물리 정지·자원 인계 proof도 아니다.

spawn await 이전에 미확정 상태를 남긴다. spawn 실패는 child가 아직 없었다고 단정하지 않으며, await 취소 후 로컬 handle이 없더라도 NOT_STARTED로 되돌리지 않는다. 일반 stop의 close와 직렬 소재 완료의 retire는 같은 정리 tracker를 사용한다. CLOSE 실패나 진행 중 취소는 sticky UNCONFIRMED로 남고, 후속 last_error 문자열 변경·빈 engine slot·다른 close 성공이 이를 해제하지 않는다. 취소 뒤 handle이 없는 cycle은 새 planner를 생성하지 않고 PlannerFault stop으로 수렴한다.

전용 단위시험은 반복 직렬 planner마다 필요한 cleanup, close 오류의 유지, 진행 중 정리 future 취소, spawn 미확정 상태를 다룬다. 실제 P 상태와 ownership 회수·다음 Run 전환은 상주 셀 서비스 통합 시험에서 별도로 검증한다.
