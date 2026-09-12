# 로컬 서비스 상태 파일

`rx-service-status`는 supervisor가 소유한 Host/Executor 프로세스의 초기화·종료 상태를 읽는 공통 S 라이브러리다. `READY`는 소프트웨어 초기화 완료이며 P admission이나 실장비 qualification·물리 정지·자원 인계 proof가 아니다. `STOPPED`도 소유한 child의 실제 종료 확인과 함께 해석해야 한다. `reconciliation_required`를 다른 권한으로 변환하지 않는다.

`GuardedStatusBinding`은 release가 지정한 절대 status 파일 경로와 정확한 역할/범위를 갖는다. Host 범위는 installation UUID, host Name, installation identity Digest다. Executor 범위는 installation UUID, cell Name, assignment service journal UUID, configuration Digest다. 파일 envelope `rx.protocol-guarded-status.v1`에는 경로를 뺀 `GuardedScope`, launch instance UUID, PID, 양수 sequence, source observed_at, `GuardedState`가 들어간다. 상태는 kind로 구분한 STARTING/READY/STOPPING/STOPPED/ATTENTION 객체다.

`Reporter::from_environment(scope)`는 `RX_PROCESS_STATUS_PATH`가 없으면 None으로 기존 INSTANCE_ID-only 사용을 유지한다. STATUS_PATH를 지정하면 유효한 `RX_PROCESS_INSTANCE_ID`가 필수다. 잘못된 guarded 설정은 오류이며 legacy 동작으로 조용히 전환하지 않는다. 이 경로는 실제 Linux BOOTTIME과 현재 PID를 사용한다. 테스트·라이브러리의 명시적 `Reporter::with_clock`와 `Reader::with_clock` 외에 환경변수나 파일에 의한 가짜 clock fallback은 없다.

`publish(&mut self, state)`는 sequence를 증가시키며 source 시각을 기록한다. 한 경로의 writer는 지속 lock으로 배타 소유한다. 기존 파일은 동일 role/scope/instance/PID일 때만 이어 쓸 수 있고, 갱신 사이의 삭제나 내용 교체도 거부한다. 새 regular 임시 파일을 0600으로 생성해 제한된 bytes를 쓰고 fsync한 뒤 같은 디렉터리의 rename과 directory fsync로 교체한다. 파일 크기는 최대65,536bytes다. writer는 heartbeat를 예약하지 않으며 Host/Executor가 250ms 이하 간격의 publish를 소유한다.

파일 부모는 미리 생성된 실제 디렉터리여야 한다. 상대 경로·부모 symlink·leaf symlink·특수 파일은 거부한다. reader는 NOFOLLOW/NONBLOCK으로 열고 bounded read·엄격한 JSON/schema·role/scope/instance/PID·양수 sequence를 검사한다. 실제 BOOTTIME과 다른 clock, 미래 source 시각, 2초를 초과한 나이는 거부한다. 부모가 사라지거나 dangling symlink이면 None이 아닌 오류다. 실제 leaf 부재만 None이다.

읽기는 source observed_at·sequence·payload digest를 그대로 반환하며 반복 조회 시각을 새 관측으로 만들지 않는다. supervisor는 같은 instance의 연속 관측 간 sequence와 digest 회귀/동일 sequence 내용 변경을 별도로 검사한다. supervisor가 launch UUID마다 고유 status 경로를 만들므로 이전 instance의 파일을 삭제하거나 덮어써서 새 owner로 채택하지 않는다.

단위시험 소스는 정확한 역할/소유자/구성, stale/future/clock mismatch, malformed·zero sequence, writer 경쟁·내용 교체, symlink·특수 파일, 원자 교체의 기존 열린 reader 보존, clock 회귀와 sequence overflow, legacy 환경 호환을 다룬다. 실제 Host/Executor heartbeat와 supervisor readiness·stop 판정은 별도 통합 시험 범위다.
