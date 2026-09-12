# Host 배포 입력 템플릿

`rx-hostd`는 `rx-solutions` 이미지에 포함되는 Host 실행파일이다. 이 예제는 같은 이미지의 명시적인 Host mode를 선택한다. 기본 진단 mode나 모든 자사 ROS launch를 자동으로 시작하는 설정이 아니다.

`startup.template.json`의 ID/경로/pin을 실제 검토된 값으로 채워 `config/startup.json`을 만든다. 0으로 된 hash는 유효한 설치 근거가 아니다. Binding 배열의 host/platform/cell·definition/envelope·자격·허용 intent/조건/scope는 P의 설치와 일치해야 한다. 서버 SAN은 P에서 사용하는 Host 이름과 맞아야 하고, client certificate fingerprint는 등록한 P 인증서의 DER SHA-256이다. 개인키는 owner-only 권한으로 제공한다.

현재 release의 builtin backend는 FILE_SIMULATION이다. 로봇/PLC용 VALIDATED_DRIVER 선택은 등록된 구현이 없어 거부한다. 패키지에 SDK/ROS 라이브러리가 포함돼 있다는 사실로 그 driver의 시작/종료를 검증했다고 처리하지 않는다.

먼저 같은 config/data mount와 non-root 권한으로 `host inspect /config/startup.json`, 이어 **한 번만** `host init /config/startup.json`을 수행한다. init은 장비를 열지 않고 새 Host 원장과 설치 identity를 만든다. 기존 데이터에 init을 반복하거나 유실된 원장을 자동 재생성하지 않는다. 이후 compose의 `host run`을 사용한다. 데이터 root는 UID10001이 쓸 수 있도록 설치 단계에서 준비한다. P의 authority DB를 이 volume에 공유하지 않는다.

Host는 설치 identity/원장 generation과 runtime lock을 확인하고 실제 Linux boottime clock, mTLS service를 시작한다. `/run/rx-host/host-status.json`의 instance/Host boot/phase/admission 상태로 해당 프로세스의 준비를 확인한다. `SOFTWARE_READY_UNARMED`는 장비 운전 준비나 qualification이 아니다. 원격 P Host endpoint와 publisher를 구성할 때 같은 Linux PC의 clock/네트워크·서버 이름·인증서 및 계약 조건을 맞춘다.

SIGTERM은 새 admission을 먼저 막고 어댑터의 명시적 safe-to-drop를 확인한다. 근거가 없으면 정상 종료를 보류한다. 임의 timeout 후 kill을 physical shutdown 절차로 쓰지 않는다. 템플릿의 자동 restart는 꺼져 있다. 실행 중인 제어 프로세스의 강제 종료/교체는 별도 검증된 절차가 필요하다.

현재 supervisor의 SoftwareOnly recipe는 Host control process를 자동 관리하지 않는다. 직접 Host mode와 그 원장/프로세스 수명 경계가 이번 배포 범위다. 다중 Host supervisor의 권한 연계, 실제 driver backend·장치/fieldbus 권한, update/restore 절차는 후속이다.
