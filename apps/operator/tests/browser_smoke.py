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
    expect(page.get_by_role("heading", name="Sign in to the operations workspace")).to_be_visible()
    page.screenshot(path=str(OUTPUT / "operator-login.png"), full_page=True)
    page.get_by_label("Account", exact=True).fill("admin")
    page.get_by_label("Password", exact=True).fill("browser-fixture-password")
    page.get_by_role("button", name="Sign in", exact=True).click()
    expect(page.get_by_role("heading", name="No cells have been registered yet")).to_be_visible()

    fixture = json.loads((ROOT / "examples/development/cell-demo.json").read_text())
    response = context.request.post(f"{ORIGIN}/api/v1/cells", headers=HEADERS,
        data={"request_key": str(uuid.uuid4()), "command": fixture})
    assert response.ok, response.text()
    page.get_by_role("button", name="Refresh ↻").click()
    expect(page.get_by_role("tab", name="cell/demo")).to_be_visible()
    expect(page.get_by_text("Awaiting verification", exact=True)).to_be_visible()
    expect(page.locator(".facts").get_by_text("Setup", exact=True)).to_be_visible()
    expect(page.locator(".facts").get_by_text("Not registered", exact=True)).to_be_visible()
    expect(page.get_by_text("No registered terminal authentication", exact=True)).to_be_visible()
    expect(page.get_by_role("button", name="Prepare new run")).to_be_enabled()
    expect(page.get_by_role("button", name="Review start details", exact=True)).to_have_count(0)
    assert page.locator("html").get_attribute("lang") == "en"
    page.screenshot(path=str(OUTPUT / "operator-overview.png"), full_page=True)

    page.get_by_role("button",name="Operating conditions",exact=True).click()
    expect(page.get_by_role("heading",name="Operating conditions and observation evidence",exact=True)).to_be_visible()
    expect(page.get_by_text("Not observed yet",exact=True)).to_be_visible()
    expect(page.locator('[data-condition-group="START"] .badge')).to_have_text("Undetermined")
    expect(page.get_by_text("Diagnostics unavailable",exact=True)).to_be_visible()
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
    page.get_by_role("button",name="Refresh ↻").click()
    expect(page.locator('[data-condition-group="START"] .badge')).to_have_text("Met")
    expect(page.locator('.condition-context .pill')).to_have_text("Not registered")
    expect(page.get_by_text("Observation usable",exact=True)).to_be_visible()
    page.screenshot(path=str(OUTPUT/"operator-conditions-pass.png"),full_page=True)

    displayed["view"]=json.loads((fixtures/"fail.json").read_text())
    page.get_by_role("button",name="Refresh ↻").click()
    expect(page.locator('[data-condition-group="START"] .badge')).to_have_text("Not met")
    expect(page.get_by_text("Observation usable",exact=True)).to_be_visible()
    page.screenshot(path=str(OUTPUT/"operator-conditions-fail.png"),full_page=True)

    displayed["view"]=json.loads((fixtures/"expired.json").read_text())
    page.get_by_role("button",name="Refresh ↻").click()
    expect(page.locator('[data-condition-group="START"] .badge')).to_have_text("Undetermined")
    expect(page.get_by_text("Observation validity period exceeded",exact=True)).to_be_visible()
    page.screenshot(path=str(OUTPUT/"operator-conditions-expired.png"),full_page=True)

    health_fixtures=Path(os.environ["RX_SERVICE_HEALTH_FIXTURES"])
    displayed["view"]=json.loads((health_fixtures/"fresh.json").read_text())
    page.get_by_role("button",name="Refresh ↻").click()
    expect(page.get_by_text("Status report received",exact=True)).to_be_visible()
    expect(page.get_by_text("Observation response received",exact=True)).to_be_visible()
    expect(page.get_by_text("Dispatch loop verified",exact=True)).to_be_visible()
    page.screenshot(path=str(OUTPUT/"operator-services-fresh.png"),full_page=True)
    displayed["view"]=json.loads((health_fixtures/"stale.json").read_text())
    page.get_by_role("button",name="Refresh ↻").click()
    expect(page.get_by_text("Recent status needs verification",exact=True)).to_be_visible()
    expect(page.get_by_text("Observation response received",exact=True)).not_to_be_visible()
    displayed["view"]=json.loads((health_fixtures/"heartbeat.json").read_text())
    page.get_by_role("button",name="Refresh ↻").click()
    expect(page.get_by_text("Status report received",exact=True)).to_be_visible()
    expect(page.get_by_text("Recent observation response needs verification",exact=True)).to_be_visible()
    expect(page.get_by_text("Recent loop activity needs verification",exact=True)).to_be_visible()
    page.screenshot(path=str(OUTPUT/"operator-services-old-activity.png"),full_page=True)
    page.locator('.runtime-services').screenshot(path=str(OUTPUT/"operator-service-detail.png"))
    page.set_viewport_size({"width":390,"height":844})
    page.locator('.runtime-services').screenshot(path=str(OUTPUT/"operator-service-detail-mobile.png"))
    assert page.evaluate("document.documentElement.scrollWidth <= innerWidth"),"runtime service mobile overflow"
    page.set_viewport_size({"width":1440,"height":1100})
    displayed["view"]=json.loads((health_fixtures/"replaced.json").read_text())
    page.get_by_role("button",name="Refresh ↻").click()
    expect(page.get_by_text("Awaiting first status report",exact=True)).to_be_visible()
    expect(page.get_by_text("Observation response received",exact=True)).not_to_be_visible()

    displayed["view"]=copy.deepcopy(json.loads((fixtures/"pass.json").read_text()))
    displayed["view"]["cells"][0]["diagnostics"]["display_valid_for_ns"]="50000000"
    displayed["delay"]=0.12
    page.get_by_role("button",name="Refresh ↻").click()
    expect(page.locator('[data-condition-group="START"] .badge')).to_have_text("Refresh required")
    expect(page.get_by_text("Observation usable",exact=True)).not_to_be_visible()
    page.screenshot(path=str(OUTPUT/"operator-conditions-delayed.png"),full_page=True)
    page.set_viewport_size({"width":390,"height":844})
    page.screenshot(path=str(OUTPUT/"operator-conditions-mobile.png"),full_page=True)
    assert page.evaluate("document.documentElement.scrollWidth <= innerWidth"),"conditions mobile overflow"
    page.set_viewport_size({"width":1440,"height":1100})
    page.unroute("**/api/v1/overview",diagnostic_response)
    page.get_by_role("button",name="Refresh ↻").click()
    expect(page.get_by_role("tab",name="cell/demo",exact=True)).to_be_visible()
    page.get_by_role("button",name="Operations",exact=True).click()

    sent = []
    def lose_committed_response(route):
        sent.append(route.request.post_data_json)
        response = route.fetch()
        assert response.ok, response.text()
        route.abort("failed")
    page.route("**/api/v1/runs", lose_committed_response, times=1)
    page.get_by_role("button", name="Prepare new run").click()
    page.get_by_role("button", name="Create run record").click()
    expect(page.get_by_text("The request outcome needs verification", exact=True)).to_be_visible()
    expect(page.get_by_role("button", name="Prepare new run")).to_be_disabled()
    stored = context.request.get(f"{ORIGIN}/api/v1/overview").json()
    assert len(stored["cells"][0]["runs"]) == 1
    run_id = stored["cells"][0]["runs"][0]["value"]["id"]
    page.screenshot(path=str(OUTPUT / "operator-pending.png"), full_page=True)

    page.reload()
    page.wait_for_load_state("networkidle")
    expect(page.get_by_text("The request outcome needs verification", exact=True)).to_be_visible()
    recovered = []
    def inspect_recovery(route):
        recovered.append(route.request.post_data_json)
        route.continue_()
    page.route("**/api/v1/runs", inspect_recovery, times=1)
    page.get_by_role("button", name="Check original request").click()
    expect(page.get_by_text("The request outcome needs verification", exact=True)).not_to_be_visible()
    assert sent == recovered, "recovery must keep the exact key and reviewed content"
    stored = context.request.get(f"{ORIGIN}/api/v1/overview").json()
    assert len(stored["cells"][0]["runs"]) == 1
    assert stored["cells"][0]["runs"][0]["value"]["id"] == run_id

    # An unqualified cell on an unregistered terminal cannot request a physical start.
    page.get_by_label("Material attempt count", exact=True).fill("2")
    expect(page.get_by_text("Start request currently blocked", exact=True)).to_be_visible()
    expect(page.get_by_role("button", name="Review start details", exact=True)).to_be_disabled()

    # A changed background snapshot must never silently replace the content the operator reviewed.
    page.get_by_role("button", name="Prepare new run").click()
    response = context.request.post(f"{ORIGIN}/api/v1/cells/hold", headers=HEADERS,
        data={"request_key":str(uuid.uuid4()), "command":{"cell":"cell/demo"}})
    assert response.ok
    page.get_by_role("button", name="Create run record").click()
    expect(page.get_by_text("configuration has changed.", exact=False)).to_be_visible()
    assert len(context.request.get(f"{ORIGIN}/api/v1/overview").json()["cells"][0]["runs"]) == 1

    page.get_by_role("button", name="Refresh ↻").click()
    expect(page.get_by_text("Operator hold", exact=True)).to_be_visible()
    page.route("**/api/v1/overview", lambda route: route.abort("failed"))
    page.get_by_role("button", name="Refresh ↻").click()
    expect(page.get_by_text("Latest state needs verification", exact=True)).to_be_visible()
    expect(page.get_by_role("button", name="Prepare new run")).to_be_disabled()
    page.unroute("**/api/v1/overview")
    page.get_by_role("button", name="Refresh ↻").click()
    expect(page.get_by_text("Read connection active", exact=True)).to_be_visible()

    opened=context.request.post(f"{ORIGIN}/api/v1/cases/open",headers=HEADERS,data={"request_key":str(uuid.uuid4()),"command":{"cell":"cell/demo","kind":"FAULT_RECOVERY","scopes":[],"procedure":{"sha256":"95"*32,"schema_id":"rx.test.procedure.v1","size_bytes":"1"},"lead":"admin","operation_ids":[],"material_ids":[]}})
    assert opened.ok,opened.text()
    case_id=opened.json()["case"]["id"]
    before_case=context.request.get(f"{ORIGIN}/api/v1/overview").json()["cells"][0]["cell"]["value"]
    page.get_by_role("button",name="Intervention cases",exact=True).click()
    expect(page.get_by_role("heading",name="Awaiting containment confirmation",exact=True)).to_be_visible()
    expect(page.get_by_text("Acknowledgment and task permission are separate",exact=True)).to_be_visible()
    case_sent=[]
    def lose_case_response(route):
        case_sent.append(route.request.post_data_json)
        response=route.fetch();assert response.ok,response.text();route.abort("failed")
    page.route("**/api/v1/cases/acknowledge",lose_case_response,times=1)
    page.get_by_role("button",name="Notification acknowledgment",exact=True).click()
    expect(page.get_by_text("The request outcome needs verification",exact=True)).to_be_visible()
    record=context.request.get(f"{ORIGIN}/api/v1/case?cell=cell%2Fdemo&case={case_id}").json()
    assert record["snapshot"]["case"]["state"]=="CONTAINMENT_PENDING" and len(record["acknowledgments"])==1
    page.reload();page.wait_for_load_state("networkidle")
    case_recovered=[]
    def replay_case(route):case_recovered.append(route.request.post_data_json);route.continue_()
    page.route("**/api/v1/cases/acknowledge",replay_case,times=1)
    page.get_by_role("button",name="Check original request").click()
    expect(page.get_by_text("The request outcome needs verification",exact=True)).not_to_be_visible()
    assert case_sent==case_recovered
    after_case=context.request.get(f"{ORIGIN}/api/v1/overview").json()["cells"][0]["cell"]["value"]
    assert before_case["epoch"]==after_case["epoch"] and before_case["blocks"]==after_case["blocks"]
    page.get_by_role("button",name="Intervention cases",exact=True).click()
    expect(page.get_by_role("heading",name="Awaiting containment confirmation",exact=True)).to_be_visible()
    expect(page.get_by_text("1 record",exact=True)).to_be_visible()
    page.screenshot(path=str(OUTPUT/"operator-cases.png"),full_page=True)

    check_authoring(page,context,ORIGIN,HEADERS,OUTPUT)
    check_bindings(page,context,ORIGIN,HEADERS,OUTPUT)
    page.get_by_role("button",name="Intervention cases",exact=True).click()
    page.set_viewport_size({"width":390,"height":844})
    page.screenshot(path=str(OUTPUT / "operator-mobile.png"), full_page=True)
    assert page.evaluate("document.documentElement.scrollWidth <= innerWidth"), "mobile horizontal overflow"
    page.get_by_role("button",name="Configuration",exact=True).click()
    expect(page.get_by_role("heading",name="Currently registered configuration")).to_be_visible()
    assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
    page.get_by_role("button",name="Sign out",exact=True).click()
    expect(page.get_by_role("heading", name="Sign in to the operations workspace")).to_be_visible()
    assert context.request.get(f"{ORIGIN}/api/v1/overview").status == 401
    assert not errors, errors
    result = {"schema":"rx.browser-check.v1","status":"PASS","browser":browser.version,
        "checks":["real login and logout","unqualified cell display", "English document language and labels", "unqualified cell and unregistered terminal block run start", "real process draft create/edit/history/conflict/recovery", "real binding selection/recovery/staleness and matched export", "real missing-observation diagnostics", "P-generated pass/fail/expired read-model rendering", "response delay shortens display validity", "conditions mobile layout", "missing and stale runtime telemetry", "heartbeats do not refresh old worker activity", "new service owner waits for its own report", "recorded RX mode and commissioning labels","commit then HTTP response loss",
                  "same request recovery after reload; exactly one run","reviewed revision stays pinned",
                  "connection loss disables new requests","case acknowledgment preserves containment and same key after reload","mobile overflow","no page JavaScript errors"],
        "limitations":["development loopback service","unqualified synthetic cell","no device process or physical validation","condition and runtime-health presentation uses intercepted P-generated snapshots, not native data in the running service","not complete UI acceptance"]}
    (OUTPUT / "browser-result.json").write_text(json.dumps(result,ensure_ascii=False,indent=2)+"\n")
    print(json.dumps(result,ensure_ascii=False))
    context.close()
    browser.close()
