# 상주 셀 실행기 CLI

기존 `rx-executor-service CONFIG`는 기존 per-Run 설정·수동 visit/직렬 생산 동작을 유지한다. 새 명령은 같은 제품 executable의 명시적인 `cell init CONFIG`와 `cell run CONFIG`다. Linux BOOTTIME 제품 경계를 유지하며, 다른 OS에서 대체 시계로 운전하지 않는다.

```text
rx-executor-service /absolute/legacy-run.json
rx-executor-service cell init /absolute/cell.json
rx-executor-service cell run /absolute/cell.json
```

## 새 설정

새 schema는 `rx.executor-cell-service.v1`이다. CONFIG는 65,536bytes 이하의 절대 경로 일반 JSON 파일이며 duplicate/unknown field를 거부한다.

| 필드 | 내용 |
|---|---|
| `service_root` | 이 서비스가 소유할 절대 디렉터리. run/visit별로 바꾸지 않는다. |
| `expected_service` | 기존 AssignmentJournal `Identity`: 고정 journal UUID와 installation/store generation, principal/release, cell/definition Scope. 초기화 전에 배포가 한 번 선정해 보존한다. |
| `platform` | `uri`, `server_name`, `ca`, `certificate`, `key`. URI는 HTTPS이고 각 파일은 `{path, sha256}` pin이다. |
| `engine` | `{path, sha256}`로 지정한 release 소유 고정 BT executable. |
| `options` | 선택적 `poll_ms`, `communication_grace_ms`, `stop_timeout_ms`. 새 셀 모드는 SerialProduction으로 고정하며 임의 coordination을 받지 않는다. |

고정 `run/visit`, `peer_boot/clock_id`, 임의 argv/launch·shell을 새 설정에서 받지 않는다. 현재 P 시작 관계와 원장의 run을 CellService가 확인하고, visit은 기존 SerialProduction이 P part 상태에서 회수한다. PeerPin의 고정 부분은 expected_service.scope에서 만들고, peer_boot는 프로세스마다 새 UUID, clock_id는 실제 LinuxBoottime에서 가져온다.

설정과 파일 경로는 배포의 로컬 입력이다. UI/P 요청이 이 CONFIG나 파일 경로·engine을 전달하는 API는 없다. 인증서/CA/private key는 각각 최대1MiB, engine은 기존 engine 검증과 같은 최대128MiB이며 symlink·특수파일·pin 불일치를 거부한다. private key는 owner-only 권한, engine은 실행 권한을 요구한다. private key bytes는 출력하지 않는다. EngineProcess도 실제 planner 기동 때 기존 hash 검사를 다시 수행한다.

## init과 설정 pin

`init`은 ServiceOwner로 root 소유권을 얻고 실제 root 경로를 정규화한다. `.rx-executor-service.lock` 이외 파일이 있으면 기존 또는 부분 설치로 보고 거부한다. root는 owner-only 디렉터리로 만들고 TLS/engine 파일 pin을 검증한다. 이 단계는 P에 연결하거나 engine을 실행하지 않는다.

먼저 `cell-installation.json`을 create_new로 만들고 fsync한다. 이 파일은 schema, normalized configuration digest, 실제 root 경로와 expected_service Identity를 보존한다. digest는 기본 옵션을 채운 CellConfig 전체를 `RX-EXECUTOR-CELL-CONFIG-v1` domain으로 계산한다. 이어 `assignment.sqlite3`를 기존 AssignmentJournal::initialize_file로 만들고 디렉터리를 fsync한다. 성공 출력은 고정한 installation descriptor다.

두 파일을 하나의 transaction이라고 주장하지 않는다. manifest 생성 뒤 DB 초기화가 실패하거나 프로세스가 사라지면 부분 설치를 보존한다. 다음 init은 덮어쓰지 않으며 run도 누락·손상된 DB를 자동 생성하지 않는다. header commit 응답만 유실되어 두 파일이 정상이라면 run의 required-open으로 원래 identity를 확인할 수 있다. key rotation, engine/release·endpoint·옵션 변경, root 이동은 configuration pin을 바꾸므로 조용히 채택하지 않는다. 해당 설치 변경/복구 절차는 별도 범위다.

## run과 종료

기동 순서는 **root owner → manifest/config/root/Identity 대조 → AssignmentJournal 필수 재열기 → 현재 run 파일 필수 검증 → TLS/engine pins → 실제 clock·새 peer boot → Session.Open → CellService**다. Preparing에 creation-entered가 있는데 파일이 없어졌거나 Attached의 파일이 누락된 경우 P peer를 등록하기 전에 거부한다. marker 없는 최초 Preparing은 예약으로만 넘기며 이후 CellService의 현재 P 권한 검사를 생략하지 않는다.

CellService가 같은 Client/session을 유지하면서 Idle에서 시작 관계를 조회한다. 연결된 run의 종료 뒤 Client와 planner factory를 반환받아 다음 run을 기다리며, 새 run마다 Session.Open을 반복하지 않는다. AMBIGUOUS 후보, 과거 session/공정, pause·복구·기존 stop intent는 자동 Run 선택/재시작으로 바꾸지 않는다.

SIGINT/SIGTERM은 watch shutdown으로 연결한다. Idle 관측 중에는 가짜 run/stop record 없이 Stopped로 종료한다. 실제 active RunService가 있으면 기존 durable stop 경계와 planner 정리를 따른다. Preparing·미결 상태를 단순 idle 성공으로 감추지 않으며, Attention은 비정상 종료다. 최초 접속이 끝나기 전 signal은 startup-interrupted 오류로 반환한다. CLI가 종료됐다는 사실은 Host/native의 물리 정지·지지 인계를 증명하지 않는다.

상태는 현재 CellService의 `Idle/Arming/Running/Attention/Stopped`, session, optional run/attachment, 이 프로세스의 completed_runs, active RunService 상태와 detail을 JSON으로 출력한다. watch는 최신 상태 하나를 유지하고 변경된 상태만 전달하며, 마지막 Report도 출력한다. CLI 성공 종료는 CellService의 최종 Stopped이고, Attention 또는 기동 오류는 실패다.

기본 poll은50ms(허용10–1,000ms), 통신 grace는5초(100–60,000ms), stop timeout은10초(100–60,000ms)다. CellService의 일시적 조회 오류 backoff는 현재 poll에서 두 배씩 최대1,000ms까지 늘고 정상 조회 후 초기화된다. Client의 connect/RPC 제한은 각각2초, engine CLOSE 제한은 기존2초다. 이 수치는 소프트웨어 처리 한도이며 물리 정지 기한이 아니다. 인증/session/clock 변경을 단순 retry로 자동 채택하지 않는다.

## 배포와 시험 범위

서비스 root는 영속 writable volume, CONFIG·인증 자료와 release engine은 root 밖의 관리된 read-only 입력으로 제공한다. 다른 서비스는 다른 root를 사용한다. UI의 StartRun은 P의 typed 요청이고 S 프로세스·설정 파일 생성 요청이 아니다. 현재 supervisor release catalog가 이 CLI를 등록했다고 가정하지 않는다. 후속 등록은 고정 program/executable/config argument와 `RequiresPlatformAuthority`, restart_limit=0 정책을 사용해야 하며 일반 site executable/path argument를 열지 않는다.

`tests/cell_cli.rs`는 Linux+test-harness에서 실제 제품 entrypoint의 init와 pre-connect 반례를 다룬다. 정상/기본옵션 정규화, 중복/alias, 부분 설치, 누락·손상 header, Preparing/Attached 파일 유실, config/asset/root 변경, 금지 field/시간 한도와 파일 정책을 확인한다. test-harness의 before-connect probe는 실제 TLS/서버/BT 전에 기록하고 중단한다. manifest 직후 init 실패 주입도 test-harness 전용이다. 기본 제품 빌드에는 두 테스트 경로가 없다. 이 시험과 부모가 수행할 실제 P–S·signal 인수 범위를 구별한다.
