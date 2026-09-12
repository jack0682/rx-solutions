# Host 증거 전송

`publication::Publisher`는 Host가 이미 저장한 evidence journal을 P로 보낸다. 장비 호출·grant/permit·production 재전송 API를 갖지 않는다. P→H의 native 전달과 H→P의 결과 재전송을 구별한다.

## 저장과 전송

1. 같은 Host gate 안에서 journal ID, 마지막 event seq, 해당 destination의 영속 ack cursor와 다음 연속 batch를 읽는다. entity 전체 snapshot을 만들지 않는 `Repository::journal_head`를 사용한다.
2. TLS와 peer/base/cell 협상을 완료한다. 알 수 없는 required feature, 다른 설치·정의·peer/boot 응답은 거부한다. 협상한 record/byte 한도와 고정128개/1 MiB 상한을 모두 지킨다.
3. 연결 첫 probe는 현재 prefix를 확인한다. 이후 저장된 cursor 다음부터 연속 전송한다. 순번이나 evidence ID를 다시 만들지 않는다.
4. P ack의 installation/store generation/view/journal/through와 범위를 대조한 뒤 cursor를 H DB에 저장한다. 원격 ack만 받고 로컬 저장에 실패하면 같은 증거를 다시 보내며 P가 중복을 제거한다.
5. 같은 generation의 ack 후퇴나 로컬 tail보다 큰 through, 부분 batch ack, journal 누락/손상은 차단한다. 새 P store generation은 별도 cursor key로 관리한다. 복원 세대 자체를 원격 값에 맞춰 자동 변경하지 않는다.

P와 H의 cursor는 별개다. H에서는 delivery journal seq와 evidence journal seq도 서로 다르다. 오래된 동일 방향 ack는 저장된 cursor를 되돌리지 않는다. 현재는 ack 뒤에도 원본 evidence를 삭제하지 않으며 retention/압축은 미구현이다.

## 실행 수명

`tick`은 하나의 제한된 publication 작업이다. `run`은 이를 반복하며, 정상 유휴 확인100ms와 일시 오류100ms→최대5초 backoff를 적용한다. 증거가 없어도 마지막으로 성공한 빈 원격 Publish probe에서1초가 지나면 현재 session으로 빈 batch를 보내 세션과 수락된 prefix를 확인한다. 이 주기는 별도 monotonic 시각으로 계산하며 로컬 Idle tick으로 미루지 않는다. probe 실패도 성공 시각을 갱신하지 않으므로 재시도가 로컬 Idle 성공으로 바뀌어 기존 backoff가 초기화되지 않는다. 전송할 증거가 있으면 기존 batch 처리를 우선하고, 연결 직후에는 기존대로 빈 prefix probe를 먼저 수행한다.

Publish가 `Unauthenticated`를 반환하면 증거 경로의 연결을 버리고 같은 pinned endpoint·destination·installation/store generation·release와 같은 Host boot/evidence journal로 Session.Open, 모든 Cell.Open, prefix probe를 다시 수행한다. timeout/Unavailable은 현재 연결과 기존 요청·cursor 의미를 유지한 채 backoff한다. 원격 응답에 맞춰 pin이나 journal을 교체하지 않는다. 이 재접속은 증거 통신 회복이며 operating HostRegistration rebind, qualification·Arm·grant·새 Run 시작이나 native 권한 복구가 아니다.

협상 불일치·무결성/연속성 상실 등은 Blocked 상태로 종료한다. timeout·ack 유실은 이미 저장한 같은 evidence의 재전송으로 처리한다. 제어용 native 명령의 재실행은 하지 않는다. shutdown/watch sender 종료로 loop를 닫으며, 완료가 미확정인 장비 작업을 완료 처리하지 않는다.

StatusView의 Idle은 전송할 증거가 없다는 뜻이다. 실제 장비 정상·현재 네트워크 건강·작업 완료를 뜻하지 않는다. native gate가 장기 block되면 journal 읽기도 기다릴 수 있으므로 현지 보호 경로를 이 publisher에 의존시키지 않는다.

성공한 빈 원격 probe도 검증된 ACK cursor를 `Published`로 알린다. 이후 로컬 tick은 다시 Idle을 표시할 수 있다. 이 원격 probe는 daemon의 software guarded heartbeat와 별개이며, 어느 쪽도 P admission freshness나 물리 안전 증명이 아니다. 1초는 정상 유휴 상태의 probe 주기이며 통신 timeout·backoff·Host gate 대기까지 포함한 복구시간 보장은 아니다.

현재 P는 빈 Publish에도 cursor revision과 감사 사건을 기록한다. Native evidence sequence는 증가하지 않지만, 1초 간격이면 유휴 Host당 하루 약86,400회 감사 기록이 가능하다. 장기 운영의 보존·용량·부하 검증 및 probe 전달 방식 최적화는 미완료이며, 이번 재연결 검증을 그 검증의 대체물로 사용하지 않는다.

현재 P의 `platform_cursor`는 별도 control journal의 cut과 `site-cell-control-v1` 이름을 사용한다. H cursor key에도 view 식별을 포함하여 예전 audit/base 위치를 재사용하지 않는다. publisher의 영속 재전송 위치는 **producer journal+through_seq**이며, platform cursor로 공개 사건을 재생하는 consumer는 아직 없다. 공개 Journal/Snapshot 연결 전에 P의 전체 wire mapping과 snapshot/구독 정책을 완성해야 한다.

## 모의 시험

`rx-host-sim-server`의 `test_seed_evidence`와 `publisher` 설정은 test-harness feature에만 존재한다. 새 test journal을 seed하고 loopback P에만 전송한다. seed는 native 효과가 실행됐다는 증명이 아니다. 기존 journal에 seed를 덧붙이는 것은 거부한다.

- Host unit: 잘못된 destination/journal/range·ack 후퇴 거부, H restart 후 cursor 보존, 새 store generation의 별도 cursor.
- Publisher unit: 100ms 로컬 tick과1초 원격 probe 분리, 실패 후 probe 계속 필요, 실제 loopback Publish에서 빈 batch/session/prefix 유지, Unauthenticated 연결 폐기 및 Unavailable/잘못된 ACK의 기존 cursor 보존. 이 시험은 이미 협상된 연결의 전송 경로를 격리하며 mTLS 재협상은 별도 실제 P–H 프로세스 시험에서 검증한다.
- 별도 P–H 프로세스 시험:131개 source slot, 첫 ack 유실, 같은 ID 재전송, metadata 보존, TLS 등록 검사, H restart/session 무효화.
- `PUBLISH` 자체의 device effect는0개이며, native gate 시험은 별도 범위다.
