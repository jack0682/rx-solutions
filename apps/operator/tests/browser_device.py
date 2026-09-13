import json,os
from pathlib import Path
from playwright.sync_api import sync_playwright,expect
fixture=Path(os.environ['RX_DEVICE_BROWSER_FIXTURE']);out=Path(os.environ['RX_DEVICE_BROWSER_EVIDENCE']);info=json.loads((fixture/'device-info.json').read_text());origin='http://127.0.0.1:5173'
with sync_playwright() as p:
 browser=p.chromium.launch(headless=True);context=browser.new_context(viewport={'width':1440,'height':1050});page=context.new_page();errors=[];page.on('pageerror',lambda e:errors.append(str(e)))
 page.goto(origin);page.wait_for_load_state('networkidle');(out/'initial-dom.html').write_text(page.content())
 page.get_by_label('계정',exact=True).fill('admin');page.get_by_label('비밀번호',exact=True).fill('browser-fixture-password');page.get_by_role('button',name='로그인',exact=True).click();expect(page.get_by_role('button',name='로그아웃',exact=True)).to_be_visible()
 before=context.request.get(origin+'/api/v1/overview').json();before_context=context.request.get(origin+'/api/v1/package-intake-context?cell=cell%2Fdemo').json()
 page.get_by_role('button',name='패키지 검토',exact=True).click();page.locator('summary').filter(has_text='서명 패키지 반입').click()
 page.get_by_label('반입 제목',exact=True).fill('모의 로봇 작업 장비');page.get_by_label('반입 폴더의 상대 경로',exact=True).fill('jtc');page.get_by_label('패키지 내용 식별자',exact=True).fill(info['object']['manifest']);page.get_by_label('패키지 서명 식별자',exact=True).fill(info['object']['signature'])
 lost=[]
 def lose(route):
  lost.append(route.request.post_data_json);response=route.fetch();assert response.ok,response.text();route.abort('failed')
 page.route('**/api/v1/package-intakes',lose,times=1);page.get_by_role('button',name='패키지 반입 요청',exact=True).click();expect(page.get_by_role('button',name='같은 요청 확인',exact=True)).to_be_enabled();page.get_by_role('button',name='같은 요청 확인',exact=True).click()
 expect(page.get_by_role('heading',name='모의 로봇 작업 장비',exact=True)).to_be_visible();panel=page.get_by_role('region',name='장비 작업 선언');expect(panel).to_be_visible();expect(panel.get_by_text('supply',exact=True)).to_be_visible();expect(panel.get_by_text('장비 검증 필요',exact=True)).to_be_visible();expect(page.get_by_role('button',name='검토 요청 생성',exact=True)).not_to_be_visible()
 with page.expect_download() as download:panel.get_by_role('button',name='장비 선언 자료 내려받기').click()
 file=out/'device-declaration.json';download.value.save_as(str(file));detail=json.loads(file.read_text());assert detail['catalog']==info['catalog'];assert detail['object']==info['object'];assert not detail['activation_authorized'] and detail['manufacturer_validation_required']
 receipt=lost[0]['command']['id'];assert detail['intake']==receipt
 denied=context.request.get(origin+f'/api/v1/package-intake/device-catalog?cell=cell%2Fother&id={receipt}');assert denied.status==403,denied.text()
 if os.environ.get('RX_DEVICE_REVIEW_ENABLED')=='1':
  from device_review_api import exercise
  exercise(browser,context,origin,fixture,out,info,receipt)
 if os.environ.get('RX_DEVICE_REVIEW_UI_ENABLED')=='1':
  from device_review_ui import exercise
  exercise(browser,context,page,origin,fixture,out,info,receipt)
 page.screenshot(path=str(out/'device-desktop.png'),full_page=True)
 page.set_viewport_size({'width':390,'height':844});page.screenshot(path=str(out/'device-mobile.png'),full_page=True);assert page.evaluate('document.documentElement.scrollWidth <= innerWidth'),'mobile overflow'
 def stale(route):
  response=route.fetch();value=response.json();value['review_context_current']=False;route.fulfill(status=200,content_type='application/json',body=json.dumps(value))
 page.route('**/api/v1/package-intake/device-catalog?*',stale);page.get_by_role('button',name='자료 새로고침',exact=True).click();expect(panel.get_by_text('현재 셀·반입 정책과 다시 대조해야 하는 보관 자료입니다.',exact=True)).to_be_visible();page.unroute('**/api/v1/package-intake/device-catalog?*',stale)
 after=context.request.get(origin+'/api/v1/overview').json();assert [c['cell'] for c in before['cells']]==[c['cell'] for c in after['cells']];assert context.request.get(origin+'/api/v1/package-intake-context?cell=cell%2Fdemo').json()['configuration_digest']==before_context['configuration_digest'];assert all(not c['runs'] and c['cell']['value']['qualification'] is None for c in after['cells']);assert not errors,errors
 (out/'result.json').write_text(json.dumps({'status':'PASS','real_s_jtc_authoring':True,'real_platform_store_and_api':True,'intake':receipt,'checks':['DEVICE_REFERENCE receipt and catalog display','lost intake reply and same-request recovery','download equals signed package catalog','wrong cell denied','no process review action on device package','stale context displayed (injected response)','unchanged active configuration and no runs/qualification','desktop/mobile overflow','no JavaScript errors']},indent=2)+'\n');context.close();browser.close()
