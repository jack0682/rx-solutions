# ROBOTIS JTC 작성 예제 — SIMULATION

catalog OM-06/arm_controller를 선택하는 작성 입력이다. calibration.bin/tool.bin은 TEST ONLY 문자열이며 실물 측정·기구 적합성 자료가 아니다. 좌표·관절 목표와 domain171은 모의 시험용이다. 패키지 서명, 운영 trust, 운전 자격은 포함하지 않는다.

```text
rx-device-package template-digest template.json
rx-device-package assemble template.json site.json recipe.json /absolute/new/candidate
```

site.json은 template의 의미 digest와 artifact bytes의 SHA-256을 참조한다. Template 변경 후에는 도구로 새 digest를 구해야 한다. 조립은 ROS/장비에 연결하지 않는다. 게시에는 외부 서명과 해당 publisher/권한/asset을 확인하는 별도 정책이 필요하다.

현재 제품 Host의 JTC 실행 제공자는 미연결이다. 패키지를 만들거나 init을 수행해도 장비를 구동할 수 없다. [JTC 패키지 구조·Host 경계](../../../runtime/rx-host/JTC_PACKAGE.md)를 따른다.
