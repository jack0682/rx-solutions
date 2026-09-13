# 실행기 미완료 원장의 오프라인 점검

`rx-executor-service cell recovery-inspect CONFIG`는 기존 실행기 서비스의 원장을 읽어 결정적인 JSON을 출력한다. 실제 P 상태 조회, 현재 세션 등록, 원래 정리 요청 완료, attachment 인수, 장비 또는 planner 실행을 하지 않는다. 기존 `cell init`·`cell run`·legacy CONFIG 호출과 분리된 명령이다.

## 입력과 원본 보존

배포 설정·설치 표식·서비스 scope와 기존 소유 파일을 검증한다. 소유 중인 서비스나 SQLite writer와 경쟁하지 않는다. 존재하지 않는 root/owner/journal/header를 생성하거나 schema를 이전하지 않는다. symlink·특수파일·빈 필수 DB·다른 설치/구성·변조된 기록·한도 초과는 실패다.

원 DB를 일반 저장소 초기화 함수로 열지 않는다. 원본 writer lock을 확보한 뒤 DB/WAL/SHM을 크기가 제한된 임시 사본으로 읽고 SQLite 검증은 사본에서 수행한다. WAL의 commit을 무시하는 immutable-main-only 읽기를 사용하지 않는다. source 파일의 inode·시각·내용을 다시 검사하고 완료 시까지 기존 lock을 유지한다. 원본에 repair/commit·새 stop/요청·새 event를 쓰지 않는다.

assignment header·current pointer·Preparing/Attached/Closed와 event history, run creation marker와 실제 run header의 연결을 확인한다. Preparing이고 creation 진입 전 실제 Run 파일이 없으면 그 상태를 그대로 표시한다. creation 진입 이후 필수 파일이 없으면 새 파일로 메우지 않는다. 원 request의 key/body/context/send/resolution과 원 stop 상태·revision·digest를 별도로 출력한다.

## 출력 의미

schema는 `rx.executor-recovery-inspection.v1`이다. service scope와 configuration/assignment digest, 원 attachment/Run journal/stop/attempt/관측 기록 및 미완료 집계, inspection digest를 갖는다. 새 시각이나 PID를 넣지 않으므로 같은 원본의 반복 점검은 같은 출력이다.

`execution_authorized`, `network_accessed`, `session_opened`, `planner_started`, `current_p_state_observed`, `original_records_modified`는 모두 false다. PENDING stop이 보인다는 것은 원 요청이 남았다는 뜻이다. 새 reader의 미실행 상태를 과거 planner의 종료 증거로 사용하지 않는다. inspection 성공을 원 서비스의 정상 종료로 소급하지 않는다.

전체 검증 후 stdout에 JSON 하나를 출력한다. 실패하면 성공 JSON을 출력하지 않고 비정상 종료한다. 이 점검 출력은 현재 장비 상태·물리 지지·참여자/격리·새 생산 허가의 증거가 아니다.

## 검증과 후속

로컬 원장 반례와 Linux 실제 CLI 반례, 실제 제품 E의 ATTENTION/PENDING/exit1 뒤 network-none에서 두 번 읽는 인수를 구분한다. 인수는 원 E data의 모든 파일 내용·mtime·mode·size, 같은 stop/attachment·결정적 출력, P/H 권한과 native 호출 불변을 대조한다. 시험 실행 결과와 정확한 소스·이미지는 문서 저장소의 단계별 evidence에서 관리한다.

새 current E 세션을 등록하는 후속 collect는 아직 제공하지 않는다. 그 등록은 P의 epoch/block을 바꾸는 별도 권한 변경이며 Host 복구 승인과의 순서/CAS가 필요하다. 원래 미완료 요청의 결과 회수, UNRESOLVED 조사 처분, 자원 인계, 명시 RestartRun은 각각 별도 구현 경계다.

WAL은 SQLite로 읽기 전에 구조·salt·누적 checksum과 완결된 commit 끝을 검사한다. 정상 SQLite가 허용하는 재사용 old-salt tail이나 미완료/미commit tail도 이 점검에서는 `RECOVERY_WAL_TAIL_UNPROVEN`으로 거부하며 손상이라고 단정하지 않는다. 임시 SHM은 원본 digest를 보존하되 SQLite가 읽을 사본에서는 버리고, 검증된 WAL로 인덱스를 다시 만든다. 원 SHM은 변경하지 않는다. WAL 없음·0바이트·정상 header-only는 현재 frame이 없다는 뜻이며 과거 committed head를 증명하지 않는다. WAL 전체 삭제·유효한 과거 prefix까지의 절단·DB/WAL 동시 rollback 탐지는 별도 신뢰 가능한 이력 기준이 필요하다. [SQLite 공식 형식](https://www.sqlite.org/fileformat2.html#walformat), [WAL 인덱스·복구 설명](https://www.sqlite.org/walformat.html)을 참고한다.
