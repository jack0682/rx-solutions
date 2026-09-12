# 자사 필수 의존성

robotis.repos는 현재 확인한 자사 원본 5개 레포의 정확한 commit이다. 2026-09-10 로컬 HEAD·원격 URL·작업 트리 무변경을 재확인했다.

이 파일은 전체 전이 의존성이 닫힌 제품 lock이 아니다. controller·interfaces·센서·ONNX·ROS 및 OS 패키지의 실제 의존성 해석과 CPU/GPU별 빌드 검증을 추가해야 한다. 다섯 원본을 선택 의존으로 낮추거나, source 파일 존재를 모델 지원 완료로 표시하지 않는다.

image 생성은 이 목록을 기본 입력으로 사용한다. 실제 장치 준비 전에는 source launch·controller 활성화·정책 출력을 자동 실행하지 않는다.

자사/전이 8개 소스와 42개 ROS package의 실제 build, policy/ONNX 및 설치 검사는 [native image 명세](NATIVE_IMAGE.md)에 기록한다. native-stack.lock.json과 APT inventory의 적용 범위·남은 공급망/실물 검증을 구별한다.
