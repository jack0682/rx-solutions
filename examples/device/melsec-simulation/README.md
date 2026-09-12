# 오프라인 MELSEC 패키지 작성 예제

이 입력은 **SIMULATION 전용 예제**다. 첫 레이저 장비의 실제 IP·주소·프로그램·안전 조건이 아니다. 원본 두 파일에도 TEST ONLY를 명시했다. production key나 trust policy는 포함하지 않는다.

`template.json`은 논리 controller 역할과 close-request slot을 정의한다. `site.json`은 loopback endpoint와 예제 M/D 주소를 연결한다. `recipe.json`은 Linux arm64 예제 package metadata다. 실제 생성은 해당 release의 도구 또는 image에서 수행한다.

```sh
rx-device-package template-digest template.json
rx-device-package assemble template.json site.json recipe.json candidate
rx-device-package request candidate YOUR_KEY_ID signing-request.json
```

위 명령은 파일만 생성한다. KEY_ID는 외부 signer와 검증 정책이 실제로 사용하는 식별자로 바꾼다. Template을 수정했다면 template-digest 결과로 site.json의 template_digest도 갱신한다. `site_config`는 실제 구성 권위가 검증할 semantic digest이며 예제값을 현장 근거로 쓰지 않는다.

외부 서명을 받은 다음 별도의 정책으로 seal/verify한다. 외부 signer나 운영 trust를 자동 생성하지 않는다. [전체 작성 절차](../../../runtime/rx-device-package/README.md)를 따른다.
