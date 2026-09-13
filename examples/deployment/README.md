# 솔루션 프로세스 관리 예제

`solutions-startup.json`은 관리 모드와 영속 기동/종료를 시험하는 모의 계획이다. 장비 profile ID를 검증하지만 시작하는 것은 release-owned 상태 HTTP 서비스 하나뿐이다. 장비·controller 준비나 qualification을 만들지 않는다.

상태 volume은 `/var/lib/rx-solutions`, 구성 파일은 `/config/solutions-startup.json:ro`에 제공한다. 프로그램·관련 파일이 담긴 image root는 읽기 전용으로 유지한다. [관리 모듈의 경계와 실행 방법](../../runtime/rx-supervisor/README.md)을 따른다.
