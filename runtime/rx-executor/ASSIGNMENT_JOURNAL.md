# 셀 서비스의 영속 run 연결 원장

이 모듈은 S가 어느 run 원장을 어떤 P 조회 문맥으로 선택했는지 기록한다. **상주 CellService·CLI/BT와 이 원장의 연결은 후속이다.** 별도로 구현된 P 배정 조회·Client와 원장 전이만으로 RunMandate, 실행 허가, 실제 완료·지지·planner 종료가 생기지 않는다.

## 저장 범위

`Identity`는 service journal UUID와 installation/store generation, principal/release, cell/definition을 고정한다. 배포가 예상 identity를 보존하고 `open_required`에 제공해야 한다. 새 프로세스나 파일 유실을 이유로 identity를 다시 발급하지 않는다. 서비스 root의 프로세스 소유권은 phase74 `ServiceOwner`가 맡으며, 호출자가 원장 사용 동안 유지해야 한다. 이 원장의 current attachment 하나는 해당 root의 직렬 실행 연결이고 플랫폼 전역의 한 셀 한 Run 제약이 아니다.

`Preparation`은 attachment UUID, run journal UUID, 기존의 정확한 `journal::Scope`, 원래 executor session/epoch와 P read basis를 고정한다. id·Scope·basis가 같은 요청은 기존 기록을 회수하고, 같은 ID의 다른 내용이나 같은 run을 다른 attachment ID로 다시 배정하는 요청은 거부한다. Closed 이력도 같은 run의 자동 재시작 근거로 재사용하지 않는다.

service DB에는 불변 header, current/count, attachment history와 run→attachment index, run 생성의 불변 `creation-entered` marker를 저장한다. marker는 정확한 원래 Preparation에 결합하며 revision 1로 유지한다. required-open은 header revision/identity, 이력 개수, index, phase별 revision, current pointer, marker와 완료 관측 구조를 대조한다. Attached/Closed의 marker 누락, 다른 예약에 속하는 marker도 거부한다. 조회와 mutation에서도 이 대조를 수행한다. 일관된 과거 DB 전체 복사에 대한 anti-rollback proof를 제공하는 것은 아니다.

service header schema는 `rx.executor-attachment-header.v2`다. creation marker가 없던 v1 원장은 필수 재열기에서 거부하며 자동 승격하지 않는다. 과거 v1 Preparing의 파일 유실을 “아직 생성하지 않은 새 원장”으로 오인하지 않기 위한 구분이다. 기존 phase와 그 record revision(Preparing 1, Attached 2, Closed 3), run index 의미는 유지한다.

run DB에는 기존 `executor/header`와 별도의 `executor/attachment-binding`을 같은 로컬 transaction에서 저장한다. 후자는 service identity, attachment와 run journal identity, 전체 원래 Preparation을 고정한다. 이미 존재하는 legacy run 원장에 이 표식을 자동 추가하지 않는다. 명시적 migration은 후속이다. 기존 `Journal::open` 구현과 요청·stop schema는 변경하지 않았다.

## 전이와 반환 경계

| 호출 | 영속 결과 | 실패·복원 |
|---|---|---|
| `initialize` / `initialize_file` | 명시적으로 비어 있는 새 service store에 header/state 저장 | 기존 파일·레코드·event history 재사용 거부. commit 응답 유실은 같은 identity로 required-open하여 확인 |
| `prepare` | Preparing와 current pointer, run index, transition event를 같은 transaction에서 저장 | 다른 현재 Preparing/Attached가 있으면 거부. 원래 전체 내용의 반복은 같은 revision/ID |
| `initialize_run` / `initialize_run_file` | 현재 Preparing의 creation-entered marker/event를 service DB에 먼저 commit하고 새 run header/binding 저장 | marker가 이미 있으면 모든 initializer 재진입 거부. 정확한 기존 파일은 required-open으로만 회수. 아직 Attached가 아니며 Worker Journal을 반환하지 않음 |
| `open_run_required` / `open_run_file_required` | 정확한 run Scope와 service/run-journal identity를 검증한 RunStore | header/binding 누락·변조·다른 root identity는 거부. Preparing/Attached/Closed의 조회·복원에 사용 가능 |
| `attach` | 검증된 RunStore를 Preparing→Attached로 commit | 성공 뒤에만 기존 요청 Journal 반환. commit 응답 유실은 Attached를 읽고 같은 run store를 다시 열어 원래 전이 회수 |
| `close_completed` | Attached→Closed, 원래 P COMPLETED 관측과 current=None 및 event를 같은 transaction에서 저장 | 동일 완료 관측 반복은 원래 기록. 다른 내용은 충돌. 늦은 옛 close 반복이 새 current를 지우지 않음 |

서비스 원장과 run 원장은 서로 다른 DB이며 하나의 transaction이 아니다. file wrapper는 **marker commit → create_new → private run-header initializer** 순서로 처리한다. generic initialize_run도 같은 marker commit을 먼저 수행하고 run 원장을 쓴다. generic 호출자가 Repository를 열 때의 파일 생성은 호출자 책임이며, 제품 파일 경계는 file wrapper를 사용한다. marker commit의 응답이 유실되면 파일 생성으로 진행하지 않는다. 미래 service는 **Attached commit 전에 remote mutation/BT 요청을 내보내지 않아야 한다.** 현재 RunStore/Journal/Repository는 신뢰된 Rust composition용 API이지, 악의적인 동일 프로세스 코드에 대한 capability sandbox가 아니다.

`close_completed`는 공유 production View 검증을 재사용하여 정확한 설치/저장 세대/cell/definition/recipe/run/원래 executor session, revision, 완료된 part와 소진된 예산을 대조한다. PAUSED/RECOVERY_REQUIRED/ABANDONED나 part 미완료를 Closed로 바꾸지 않는다. 저장된 View의 네트워크 인증·신선도 및 실제 planner retire 확인은 미래 Client/service가 먼저 수행해야 한다. 이 모듈은 raw 데이터가 P에서 실제 발급됐다는 증명을 만들지 않는다. stop intent, 원래 EMIT_ENTERED/PENDING 요청과 응답 부재는 close 후에도 변경하지 않는다. P 완료를 관측했다는 사실은 유실된 RPC 응답을 새로 만드는 근거가 아니다.

## 파일 유실과 불완전 초기화

파일 경로는 배포 root와 예약된 run UUID로 결정한다. P 응답에서 파일 경로나 실행파일을 받지 않는다. `open_file_required`와 `open_run_file_required`는 존재하는 비어 있지 않은 일반 파일만 받고 symlink·특수파일을 거부하며 SQLite 무결성을 검사한다. root는 소유된 로컬 경로여야 한다. 외부 프로세스가 소유된 디렉터리를 교체하는 공격에 대한 별도 filesystem sandbox를 제공하지 않는다.

`recover_current_file`은 Preparing이며 **creation-entered marker가 없고 파일도 없는 경우**만 `NeedsInitialization`으로 표시한다. 자동으로 생성하지 않는다. marker가 있으면 Preparing 상태에서도 파일 누락은 오류다. initializer 성공 또는 header commit 응답 유실 뒤 파일이 없어졌을 때 같은 identity로 새 원장을 생성할 수 없다. 이미 있는 빈 파일·header 없는 DB·binding 없는 DB도 오류로 남긴다. 파일이나 marker를 지우고 같은 run을 초기화하는 복구 API는 없다.

marker commit 뒤 파일 생성 전에 실패하면 marker만 남으며, 이후 파일이 없어도 초기화 재시도는 거부된다. create_new 뒤 header commit 전에 실패하면 빈 파일 또는 header 없는 SQLite 파일이 남는다. 이 경우에도 required-open과 initializer 재시도는 모두 실패한다. 원래 Preparing·marker·파일을 보존하고 별도 설치 복구 판단이 필요하다. marker transaction이 실제 rollback된 경우만 첫 생성 진입을 다시 시도할 수 있다. 첫 header commit 뒤 응답만 유실되고 파일이 남아 있으면 required-open으로 원래 identity를 회수할 수 있다. 불완전 생성·파일 유실·응답 유실을 같은 것으로 취급하지 않는다.

## 용량과 미완료 연결

이력 한도는 **service journal당 1,024개 attachment**다. 영구 운영에서는 이 한도에 도달하면 새 run 준비가 거부된다. 기존 ID 조회/동일 요청 회수는 계속 가능하다. 자동 삭제·count 초기화·새 root/identity로 우회하는 처리는 없다. retention/archival 및 기존 run의 재시작·새 session 재결합은 별도 설계·검증이 필요하다. production View와 사건은 기존 canonical/저장소 크기 한도를 그대로 적용받는다.

다음 연결은 별도로 구현된 P의 0/1/AMBIGUOUS 조회·Client 결과를 이 원장에 연결하고, service 원장 초기화/배포 identity pin 및 planner 수명주기를 기존 CLI에 적용하는 것이다. 이 원장이 구현됐다는 이유로 상주 배정 서비스나 새 run 자동 인수가 완료됐다고 표시하지 않는다.

## 작성된 반례 시험

별도 `tests/assignment_journal.rs`는 service header 및 run header의 필수 재열기, 잘못된 scope/identity, missing/empty/symlink, Preparing→Attached→Closed의 rollback과 commit 후 응답 유실, creation marker의 rollback/응답 유실/변조, 생성 완료·header 응답 유실 후 Preparing 파일 유실, marker commit 후 파일 생성 전 실패, 불완전 초기화 파일의 재사용 거부, v1 header 및 legacy run 원장의 자동 채택 거부, 잘못된 완료 관측, Closed 재배정 거부, 늦은 close의 새 current 보존, 원래 stop·미응답 요청 보존과 손상된 current pointer를 다룬다. 실제 P API·BT·장비를 호출하지 않는다. 이전 변경의 부모 검증과 이번 수정의 새 검증 결과를 구별하며, 이번 수정은 부모의 재검증을 기다린다.
