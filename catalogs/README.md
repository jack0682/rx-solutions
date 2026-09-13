# 장비 지원 카탈로그

`device-support.v1.json`은 제조사와 무관한 장비 선언 형식이다. 외부 장비를 연결할 때는 저장소 URL·불변 Git commit·참조 파일 SHA-256과 별도 현장 commissioning 근거를 제공한다. 특정 회사나 모델 수를 공통 코어의 필수 목록으로 강제하지 않는다.

현재 기본 카탈로그에는 `fixtures/controllers.v1.json`에서 새로 작성한 **모의 선언 4개**만 있다. 6축·7축 position JTC는 허용 경로, leader와 impedance는 지원 경계의 거부 시험용이다. gripper 전용 action도 JTC로 허용하지 않는다. `SIMULATION_FIXTURE`는 실물 모델·제조사 소스 관측·장비 검증 완료를 뜻하지 않는다.

`rx-solution-catalog`는 중복, Git revision, fixture 출처, controller 선언, 필수 commissioning 입력을 검증한다. bundled fixture 원본의 실제 해시와 각 controller 선언도 비교한다. `SOURCE_OBSERVED`와 `SIMULATION_FIXTURE`는 서로 다른 근거이며, 이름만 바꾸어 전환할 수 없다. Host는 모의 fixture를 물리 환경 profile로 사용하는 것을 거부한다.

```sh
python3 tools/check_device_catalog_sources.py
./tools/cargo test -p rx-solution-catalog
```

외부 카탈로그를 검증할 때 `--catalog PATH --source-root PATH`를 사용한다. 명령은 source identity를 확인하며 ROS controller를 실행하지 않는다. native 권한, calibration, 기동 영향, 완료 근거, 인계 조건은 실제 장비를 추가할 때 별도로 검증해야 한다. 과거 지원표와 시험 원문은 Git 이력에 보존하며, 그 결과를 이 새 카탈로그의 검증 결과로 승계하지 않는다.
