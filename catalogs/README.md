# 자사 기본 지원 catalogue

`robotis-support.v1.json`은 다섯 필수 원본과22개 모델/역할 구성을 보존한다. FFW BG2 rev2/rev3/rev4는 각각 독립 ID이며, leader/follower·일반 gripper action/공유 JTC·K1 기본 제어/policy를 합치지 않는다.

`rx-solution-catalog`가 schema/누락/중복/source pin/필수 commissioning 입력을 검증한다. Host는 이 crate를 필수 의존하며 시작 시 bundled catalogue를 확인한다. **이것은 ROS 소스 전체·전이 의존성의 image build나 실제 모델 지원 완료를 뜻하지 않는다.** DynamixelSDK/DHI/open_manipulator/ai_worker/ai_sapiens의 필수 포함 목표는 그대로다.

현재 모든 행의 evidence_level은 SOURCE_OBSERVED다. 다른 단계의 PASS를 이 필드에 써서 승격할 수 없다. 실제 build·simulation·hardware/RX contract·cell qualification은 별도 검증 기록이 필요하다.

각 source는 repository/commit/relative path/file SHA-256로 연결한다. controller declaration의 순서·plugin·interface를 보존한다. GPIO는 관절 interface의 평평한 배열로 변환하지 않고 중첩 group 구조를 유지한다. declared_update_hz는 소스 설정값이며 실시간 성능 보장이 아니다.

FFW-08은 mobile-base launch와 그것이 참조하는 SG2 controller 구성을 함께 기록한다. 여기의 controller 목록은 참조 파일의 선언이며 해당 launch가 전부 활성화한다는 주장이 아니다. 독립 판매 SKU도 확정하지 않았다.

`tools/check_robotis_catalog_sources.py --source-root PATH`는 Git의 지정 commit blob을 읽어 출처22개 파일을 대조한다. 실제 ROS 파라미터를 로드하거나 controller를 켜지 않는다. runtime crate의 두 시험은 mandatory 구성이 누락/합쳐지거나 출처 revision이 바뀌면 거부되는지 확인한다.

원래 설계 입력은 workspace의 `docs/13_robotis_support_matrix.md`와 `references/own_controller_inventory_2026-09-09.json`이다. 배포 catalogue는 이 레포 안에 있으며 runtime이 다른 작업 디렉토리를 읽을 필요는 없다.
