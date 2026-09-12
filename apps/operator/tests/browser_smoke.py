"""Real local API + browser checks; start a fresh local installation before running.

No device driver or simulated completion producer is started. The cell fixture is unqualified.
"""
import copy
import time
import json
import os
import uuid
from pathlib import Path
from playwright.sync_api import sync_playwright, expect
from browser_authoring import check_authoring
from browser_bindings import check_bindings

ORIGIN = "http://127.0.0.1:5173"
ROOT = Path(__file__).resolve().parents[3]
OUTPUT = Path(os.environ["RX_BROWSER_EVIDENCE"])
OUTPUT.mkdir(parents=True, exist_ok=True)
HEADERS = {"Origin": ORIGIN, "X-RX-Client": "browser-v1"}

with sync_playwright() as playwright:
    browser = playwright.chromium.launch(headless=True)
    context = browser.new_context(viewport={"width": 1440, "height": 1100})
    page = context.new_page()
    errors = []
    page.on("pageerror", lambda error: errors.append(str(error)))
    page.goto(ORIGIN)
    page.wait_for_load_state("networkidle")
    expect(page.get_by_role("heading", name="운영 공간에 로그인")).to_be_visible()
    page.screenshot(path=str(OUTPUT / "operator-login.png"), full_page=True)
    page.get_by_label("계정", exact=True).fill("admin")
    page.get_by_label("비밀번호", exact=True).fill("browser-fixture-password")
    page.get_by_role("button", name="로그인", exact=True).click()
    expect(page.get_by_role("heading", name="아직 등록된 셀이 없습니다")).to_be_visible()

    fixture = json.loads((ROOT / "examples/development/cell-demo.json").read_text())
    response = context.request.post(f"{ORIGIN}/api/v1/cells", headers=HEADERS,
        data={"request_key": str(uuid.uuid4()), "command": fixture})
    assert response.ok, response.text()
    page.get_by_role("button", name="새로고침 ↻").click()
    expect(page.get_by_role("tab", name="cell/demo")).to_be_visible()
    expect(page.get_by_text("검증 대기", exact=True)).to_be_visible()
    expect(page.locator(".facts").get_by_text("셋업", exact=True)).to_be_visible()
    expect(page.locator(".facts").get_by_text("미등록", exact=True)).to_be_visible()
    expect(page.get_by_text("실장비 시작 비활성", exact=True)).to_be_visible()
    expect(page.get_by_role("button", name="새 실행 준비")).to_be_enabled()
    page.screenshot(path=str(OUTPUT / "operator-overview.png"), full_page=True)

    page.get_by_role("button",name="운전 조건",exact=True).click()
    expect(page.get_by_role("heading",name="운전 조건과 관측 근거",exact=True)).to_be_visible()
    expect(page.get_by_text("아직 관측하지 못함",exact=True)).to_be_visible()
    expect(page.locator('[data-condition-group="START"] .badge')).to_have_text("미확정")
    expect(page.get_by_text("진단 미연결",exact=True)).to_be_visible()
    actual_unqualified = context.request.get(f"{ORIGIN}/api/v1/overview").json()
    assert actual_unqualified["cells"][0]["diagnostics"]["sources"][0]["observation"] is None
    page.screenshot(path=str(OUTPUT/"operator-conditions-missing.png"),full_page=True)

    # Presentation contract scenarios use actual P-generated, unqualified snapshots.
    # No native source or qualification is injected into the running service.
    fixtures=Path(os.environ["RX_DIAGNOSTICS_FIXTURES"])
    displayed={"view":json.loads((fixtures/"pass.json").read_text()),"delay":0.0}
    def diagnostic_response(route):
        if displayed["delay"]: time.sleep(displayed["delay"])
        route.fulfill(status=200,content_type="application/json",body=json.dumps(displayed["view"]))
    page.route("**/api/v1/overview",diagnostic_response)
    page.get_by_role("button",name="새로고침 ↻").click()
    expect(page.locator('[data-condition-group="START"] .badge')).to_have_text("충족")
    expect(page.locator('.condition-context .pill')).to_have_text("미등록")
    expect(page.get_by_text("관측 사용 가능",exact=True)).to_be_visible()
    page.screenshot(path=str(OUTPUT/"operator-conditions-pass.png"),full_page=True)

    displayed["view"]=json.loads((fixtures/"fail.json").read_text())
    page.get_by_role("button",name="새로고침 ↻").click()
    expect(page.locator('[data-condition-group="START"] .badge')).to_have_text("미충족")
    expect(page.get_by_text("관측 사용 가능",exact=True)).to_be_visible()
    page.screenshot(path=str(OUTPUT/"operator-conditions-fail.png"),full_page=True)

    displayed["view"]=json.loads((fixtures/"expired.json").read_text())
    page.get_by_role("button",name="새로고침 ↻").click()
    expect(page.locator('[data-condition-group="START"] .badge')).to_have_text("미확정")
    expect(page.get_by_text("관측 유효시간 초과",exact=True)).to_be_visible()
    page.screenshot(path=str(OUTPUT/"operator-conditions-expired.png"),full_page=True)

    health_fixtures=Path(os.environ["RX_SERVICE_HEALTH_FIXTURES"])
    displayed["view"]=json.loads((health_fixtures/"fresh.json").read_text())
    page.get_by_role("button",name="새로고침 ↻").click()
    expect(page.get_by_text("상태 보고 수신",exact=True)).to_be_visible()
    expect(page.get_by_text("관측 응답 수신",exact=True)).to_be_visible()
    expect(page.get_by_text("전달 루프 확인",exact=True)).to_be_visible()
    page.screenshot(path=str(OUTPUT/"operator-services-fresh.png"),full_page=True)
    displayed["view"]=json.loads((health_fixtures/"stale.json").read_text())
    page.get_by_role("button",name="새로고침 ↻").click()
    expect(page.get_by_text("최근 상태 확인 필요",exact=True)).to_be_visible()
    expect(page.get_by_text("관측 응답 수신",exact=True)).not_to_be_visible()
    displayed["view"]=json.loads((health_fixtures/"heartbeat.json").read_text())
    page.get_by_role("button",name="새로고침 ↻").click()
    expect(page.get_by_text("상태 보고 수신",exact=True)).to_be_visible()
    expect(page.get_by_text("최근 관측 응답 확인 필요",exact=True)).to_be_visible()
    expect(page.get_by_text("최근 루프 동작 확인 필요",exact=True)).to_be_visible()
    page.screenshot(path=str(OUTPUT/"operator-services-old-activity.png"),full_page=True)
    page.locator('.runtime-services').screenshot(path=str(OUTPUT/"operator-service-detail.png"))
    page.set_viewport_size({"width":390,"height":844})
    page.locator('.runtime-services').screenshot(path=str(OUTPUT/"operator-service-detail-mobile.png"))
    assert page.evaluate("document.documentElement.scrollWidth <= innerWidth"),"runtime service mobile overflow"
    page.set_viewport_size({"width":1440,"height":1100})
    displayed["view"]=json.loads((health_fixtures/"replaced.json").read_text())
    page.get_by_role("button",name="새로고침 ↻").click()
    expect(page.get_by_text("첫 상태 보고 대기",exact=True)).to_be_visible()
    expect(page.get_by_text("관측 응답 수신",exact=True)).not_to_be_visible()

    displayed["view"]=copy.deepcopy(json.loads((fixtures/"pass.json").read_text()))
    displayed["view"]["cells"][0]["diagnostics"]["display_valid_for_ns"]="50000000"
    displayed["delay"]=0.12
    page.get_by_role("button",name="새로고침 ↻").click()
    expect(page.locator('[data-condition-group="START"] .badge')).to_have_text("재조회 필요")
    expect(page.get_by_text("관측 사용 가능",exact=True)).not_to_be_visible()
    page.screenshot(path=str(OUTPUT/"operator-conditions-delayed.png"),full_page=True)
    page.set_viewport_size({"width":390,"height":844})
    page.screenshot(path=str(OUTPUT/"operator-conditions-mobile.png"),full_page=True)
    assert page.evaluate("document.documentElement.scrollWidth <= innerWidth"),"conditions mobile overflow"
    page.set_viewport_size({"width":1440,"height":1100})
    page.unroute("**/api/v1/overview",diagnostic_response)
    page.get_by_role("button",name="새로고침 ↻").click()
    expect(page.get_by_role("tab",name="cell/demo",exact=True)).to_be_visible()
    page.get_by_role("button",name="운영",exact=True).click()

    sent = []
    def lose_committed_response(route):
        sent.append(route.request.post_data_json)
        response = route.fetch()
        assert response.ok, response.text()
        route.abort("failed")
    page.route("**/api/v1/runs", lose_committed_response, times=1)
    page.get_by_role("button", name="새 실행 준비").click()
    page.get_by_role("button", name="실행 기록 만들기").click()
    expect(page.get_by_text("요청 결과를 확인해야 합니다", exact=True)).to_be_visible()
    expect(page.get_by_role("button", name="새 실행 준비")).to_be_disabled()
    stored = context.request.get(f"{ORIGIN}/api/v1/overview").json()
    assert len(stored["cells"][0]["runs"]) == 1
    run_id = stored["cells"][0]["runs"][0]["value"]["id"]
    page.screenshot(path=str(OUTPUT / "operator-pending.png"), full_page=True)

    page.reload()
    page.wait_for_load_state("networkidle")
    expect(page.get_by_text("요청 결과를 확인해야 합니다", exact=True)).to_be_visible()
    recovered = []
    def inspect_recovery(route):
        recovered.append(route.request.post_data_json)
        route.continue_()
    page.route("**/api/v1/runs", inspect_recovery, times=1)
    page.get_by_role("button", name="같은 요청 확인").click()
    expect(page.get_by_text("요청 결과를 확인해야 합니다", exact=True)).not_to_be_visible()
    assert sent == recovered, "recovery must keep the exact key and reviewed content"
    stored = context.request.get(f"{ORIGIN}/api/v1/overview").json()
    assert len(stored["cells"][0]["runs"]) == 1
    assert stored["cells"][0]["runs"][0]["value"]["id"] == run_id

    # A changed background snapshot must never silently replace the content the operator reviewed.
    page.get_by_role("button", name="새 실행 준비").click()
    response = context.request.post(f"{ORIGIN}/api/v1/cells/hold", headers=HEADERS,
        data={"request_key":str(uuid.uuid4()), "command":{"cell":"cell/demo"}})
    assert response.ok
    page.get_by_role("button", name="실행 기록 만들기").click()
    expect(page.get_by_text("구성이 변경되었습니다.", exact=False)).to_be_visible()
    assert len(context.request.get(f"{ORIGIN}/api/v1/overview").json()["cells"][0]["runs"]) == 1

    page.get_by_role("button", name="새로고침 ↻").click()
    expect(page.get_by_text("작업자 운전 보류", exact=True)).to_be_visible()
    page.route("**/api/v1/overview", lambda route: route.abort("failed"))
    page.get_by_role("button", name="새로고침 ↻").click()
    expect(page.get_by_text("최신 상태 확인 필요", exact=True)).to_be_visible()
    expect(page.get_by_role("button", name="새 실행 준비")).to_be_disabled()
    page.unroute("**/api/v1/overview")
    page.get_by_role("button", name="새로고침 ↻").click()
    expect(page.get_by_text("조회 연결됨", exact=True)).to_be_visible()

    opened=context.request.post(f"{ORIGIN}/api/v1/cases/open",headers=HEADERS,data={"request_key":str(uuid.uuid4()),"command":{"cell":"cell/demo","kind":"FAULT_RECOVERY","scopes":[],"procedure":{"sha256":"95"*32,"schema_id":"rx.test.procedure.v1","size_bytes":"1"},"lead":"admin","operation_ids":[],"material_ids":[]}})
    assert opened.ok,opened.text()
    case_id=opened.json()["case"]["id"]
    before_case=context.request.get(f"{ORIGIN}/api/v1/overview").json()["cells"][0]["cell"]["value"]
    page.get_by_role("button",name="개입 사건",exact=True).click()
    expect(page.get_by_role("heading",name="조치·격리 확인 대기",exact=True)).to_be_visible()
    expect(page.get_by_text("알림 확인과 작업 허가는 별도입니다",exact=True)).to_be_visible()
    case_sent=[]
    def lose_case_response(route):
        case_sent.append(route.request.post_data_json)
        response=route.fetch();assert response.ok,response.text();route.abort("failed")
    page.route("**/api/v1/cases/acknowledge",lose_case_response,times=1)
    page.get_by_role("button",name="알림 확인 기록",exact=True).click()
    expect(page.get_by_text("요청 결과를 확인해야 합니다",exact=True)).to_be_visible()
    record=context.request.get(f"{ORIGIN}/api/v1/case?cell=cell%2Fdemo&case={case_id}").json()
    assert record["snapshot"]["case"]["state"]=="CONTAINMENT_PENDING" and len(record["acknowledgments"])==1
    page.reload();page.wait_for_load_state("networkidle")
    case_recovered=[]
    def replay_case(route):case_recovered.append(route.request.post_data_json);route.continue_()
    page.route("**/api/v1/cases/acknowledge",replay_case,times=1)
    page.get_by_role("button",name="같은 요청 확인").click()
    expect(page.get_by_text("요청 결과를 확인해야 합니다",exact=True)).not_to_be_visible()
    assert case_sent==case_recovered
    after_case=context.request.get(f"{ORIGIN}/api/v1/overview").json()["cells"][0]["cell"]["value"]
    assert before_case["epoch"]==after_case["epoch"] and before_case["blocks"]==after_case["blocks"]
    page.get_by_role("button",name="개입 사건",exact=True).click()
    expect(page.get_by_role("heading",name="조치·격리 확인 대기",exact=True)).to_be_visible()
    expect(page.get_by_text("1건",exact=True)).to_be_visible()
    page.screenshot(path=str(OUTPUT/"operator-cases.png"),full_page=True)

    check_authoring(page,context,ORIGIN,HEADERS,OUTPUT)
    check_bindings(page,context,ORIGIN,HEADERS,OUTPUT)
    page.get_by_role("button",name="개입 사건",exact=True).click()
    page.set_viewport_size({"width":390,"height":844})
    page.screenshot(path=str(OUTPUT / "operator-mobile.png"), full_page=True)
    assert page.evaluate("document.documentElement.scrollWidth <= innerWidth"), "mobile horizontal overflow"
    page.get_by_role("button",name="구성",exact=True).click()
    expect(page.get_by_role("heading",name="현재 등록된 구성")).to_be_visible()
    assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
    page.get_by_role("button",name="로그아웃",exact=True).click()
    expect(page.get_by_role("heading", name="운영 공간에 로그인")).to_be_visible()
    assert context.request.get(f"{ORIGIN}/api/v1/overview").status == 401
    assert not errors, errors
    result = {"schema":"rx.browser-check.v1","status":"PASS","browser":browser.version,
        "checks":["real login and logout","unqualified cell display", "real process draft create/edit/history/conflict/recovery", "real binding selection/recovery/staleness and matched export", "real missing-observation diagnostics", "P-generated pass/fail/expired read-model rendering", "response delay shortens display validity", "conditions mobile layout", "missing and stale runtime telemetry", "heartbeats do not refresh old worker activity", "new service owner waits for its own report", "recorded RX mode and commissioning labels","commit then HTTP response loss",
                  "same request recovery after reload; exactly one run","reviewed revision stays pinned",
                  "connection loss disables new requests","case acknowledgment preserves containment and same key after reload","mobile overflow","no page JavaScript errors"],
        "limitations":["development loopback service","unqualified synthetic cell","no device process or physical validation","condition and runtime-health presentation uses intercepted P-generated snapshots, not native data in the running service","not complete UI acceptance"]}
    (OUTPUT / "browser-result.json").write_text(json.dumps(result,ensure_ascii=False,indent=2)+"\n")
    print(json.dumps(result,ensure_ascii=False))
    context.close()
    browser.close()
