import json,os
from pathlib import Path
from playwright.sync_api import sync_playwright,expect
fixture=Path(os.environ['RX_DEVICE_BROWSER_FIXTURE']);out=Path(os.environ['RX_DEVICE_BROWSER_EVIDENCE']);info=json.loads((fixture/'device-info.json').read_text());origin='http://127.0.0.1:5173'
with sync_playwright() as p:
 browser=p.chromium.launch(headless=True);context=browser.new_context(viewport={'width':1440,'height':1050});page=context.new_page();errors=[];page.on('pageerror',lambda e:errors.append(str(e)))
 page.goto(origin);page.wait_for_load_state('networkidle');(out/'initial-dom.html').write_text(page.content())
 page.get_by_label('Account',exact=True).fill('admin');page.get_by_label('Password',exact=True).fill('browser-fixture-password');page.get_by_role('button',name='Sign in',exact=True).click();expect(page.get_by_role('button',name='Sign out',exact=True)).to_be_visible()
 before=context.request.get(origin+'/api/v1/overview').json();before_context=context.request.get(origin+'/api/v1/package-intake-context?cell=cell%2Fdemo').json()
 page.get_by_role('button',name='Package review',exact=True).click();page.locator('summary').filter(has_text='Import signed package').click()
 page.get_by_label('Intake title',exact=True).fill('Simulated robot operation device');page.get_by_label('Relative path in intake directory',exact=True).fill('jtc');page.get_by_label('Package content identifier',exact=True).fill(info['object']['manifest']);page.get_by_label('Package signature identifier',exact=True).fill(info['object']['signature'])
 lost=[]
 def lose(route):
  lost.append(route.request.post_data_json);response=route.fetch();assert response.ok,response.text();route.abort('failed')
 page.route('**/api/v1/package-intakes',lose,times=1);page.get_by_role('button',name='Request package intake',exact=True).click();expect(page.get_by_role('button',name='Check original request',exact=True)).to_be_enabled();page.get_by_role('button',name='Check original request',exact=True).click()
 expect(page.get_by_role('heading',name='Simulated robot operation device',exact=True)).to_be_visible();panel=page.get_by_role('region',name='Device operation declarations');expect(panel).to_be_visible();expect(panel.get_by_text('supply',exact=True)).to_be_visible();expect(panel.get_by_text('Device verification required',exact=True)).to_be_visible();expect(page.get_by_role('button',name='Create review request',exact=True)).not_to_be_visible()
 with page.expect_download() as download:panel.get_by_role('button',name='Download device declarations').click()
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
 page.route('**/api/v1/package-intake/device-catalog?*',stale);page.get_by_role('button',name='Refresh evidence',exact=True).click();expect(panel.get_by_text('This archived evidence must be checked again against the current cell and intake policy.',exact=True)).to_be_visible();page.unroute('**/api/v1/package-intake/device-catalog?*',stale)
 after=context.request.get(origin+'/api/v1/overview').json();assert [c['cell'] for c in before['cells']]==[c['cell'] for c in after['cells']];assert context.request.get(origin+'/api/v1/package-intake-context?cell=cell%2Fdemo').json()['configuration_digest']==before_context['configuration_digest'];assert all(not c['runs'] and c['cell']['value']['qualification'] is None for c in after['cells']);assert not errors,errors
 (out/'result.json').write_text(json.dumps({'status':'PASS','real_s_jtc_authoring':True,'real_platform_store_and_api':True,'intake':receipt,'checks':['DEVICE_REFERENCE receipt and catalog display','lost intake reply and same-request recovery','download equals signed package catalog','wrong cell denied','no process review action on device package','stale context displayed (injected response)','unchanged active configuration and no runs/qualification','desktop/mobile overflow','no JavaScript errors']},indent=2)+'\n');context.close();browser.close()
