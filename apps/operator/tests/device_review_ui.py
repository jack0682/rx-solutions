"""Actual device review UI with S-generated signed reports and P authority checks."""
import json,os,subprocess,uuid
from pathlib import Path
from playwright.sync_api import expect
def exercise(browser,context,page,origin,fixture,out,info,intake):
 headers={'Origin':origin,'Content-Type':'application/json','X-RX-Client':'browser-v1'}
 title='Simulated robot operation device';panel=page.get_by_role('region',name='Device software review')
 expect(panel).to_be_visible();panel.get_by_role('button',name='Create device review request',exact=True).click();expect(panel.get_by_role('button',name='Download device verification request',exact=True)).to_be_visible()
 with page.expect_download() as download:panel.get_by_role('button',name='Download device verification request',exact=True).click()
 request_path=out/'device-review-request.json';download.value.save_as(str(request_path));request=json.loads(request_path.read_text());assert request['intake']==intake;review=request['id']
 report_dir=fixture/'exchange/device-ui-report';r=subprocess.run([os.environ['RX_DEVICE_REVIEW_TOOL'],'review',info['package'],info['policy'],str(request_path),str(report_dir)],capture_output=True,text=True);assert r.returncode==0,r.stderr;r=json.loads(r.stdout);assert r['software_checks_passed']
 env=dict(os.environ,RX_DEVICE_REVIEW_REPORT=str(report_dir/'verification.json'));repo=Path(os.environ['RX_DEVICE_REVIEW_SOLUTIONS'])
 subprocess.run([str(repo/'tools/cargo'),'test','-p','rx-device-package','--test','jtc_authoring','sign_device_report','--locked','--offline','--','--ignored','--exact'],cwd=repo,env=env,check=True,capture_output=True)
 panel.locator('summary').filter(has_text='Register device verification report').click();panel.get_by_label('Device report relative path',exact=True).fill('device-ui-report');panel.get_by_label('Device report identifier',exact=True).fill(r['report_digest']);panel.get_by_role('button',name='Request device report registration',exact=True).click()
 expect(panel.get_by_text('Device package software checks passed',exact=True)).to_be_visible();expect(panel.get_by_role('button',name='Approve device software',exact=True)).to_be_disabled();page.screenshot(path=str(out/'device-author-cannot-approve.png'),full_page=True)
 def login(user):
  page.get_by_label('Account',exact=True).fill(user);page.get_by_label('Password',exact=True).fill('browser-fixture-password');page.get_by_role('button',name='Sign in',exact=True).click();expect(page.get_by_role('button',name='Sign out',exact=True)).to_be_visible();page.get_by_role('button',name='Package review',exact=True).click();page.locator('.package-item').filter(has_text=title).click();page.locator('.device-review-list button').first.click()
 page.get_by_role('button',name='Sign out',exact=True).click();login('reviewer');expect(panel.get_by_role('heading',name='Device verification report r1',exact=True)).to_be_visible()
 panel.get_by_label('Device review note',exact=True).fill('I reviewed the source, signatures, and check scope. I approve only the software review.');check=panel.get_by_role('checkbox',name='I have reviewed the device source, signatures, check scope, and this report version.');check.check()
 def new_report(expected):
  response=context.request.post(origin+'/api/v1/device-review/reports',headers=headers,data={'request_key':str(uuid.uuid4()),'command':{'review':review,'cell':'cell/demo','expected':str(expected),'directory':'device-ui-report','report_digest':r['report_digest']}});assert response.ok,response.text()
 new_report(1);expect(panel.get_by_role('heading',name='Device verification report r2',exact=True)).to_be_visible();expect(check).not_to_be_checked();expect(panel.get_by_role('button',name='Approve device software',exact=True)).to_be_disabled()
 panel.get_by_label('Device report version to view',exact=True).fill('1');panel.get_by_role('button',name='View historical device version',exact=True).click();expect(panel.get_by_text('Historical device review · read only',exact=True)).to_be_visible();expect(panel.get_by_role('button',name='Approve device software',exact=True)).to_be_disabled();panel.get_by_role('button',name='View latest device review',exact=True).click();expect(panel.get_by_role('heading',name='Device verification report r2',exact=True)).to_be_visible()
 check.check();panel.get_by_role('button',name='Approve device software',exact=True).click();expect(panel.locator('dialog[open]')).to_be_visible();new_report(2);expect(panel.locator('dialog[open]')).not_to_be_visible();expect(check).not_to_be_checked();expect(panel.get_by_role('heading',name='Device verification report r3',exact=True)).to_be_visible()
 detail=context.request.get(origin+f'/api/v1/device-review?cell=cell%2Fdemo&id={review}').json();assert detail['decision'] is None
 check.check();panel.get_by_role('button',name='Approve device software',exact=True).click();expect(panel.locator('dialog[open]')).to_be_visible()
 def changed_policy(route):
  response=route.fetch();value=response.json();value['device_review_authority_digest']='f'*64;route.fulfill(status=200,content_type='application/json',body=json.dumps(value))
 page.route('**/api/v1/package-intake-context?*',changed_policy);expect(panel.locator('dialog[open]')).not_to_be_visible(timeout=10000);expect(check).not_to_be_checked();expect(panel.get_by_role('button',name='Approve device software',exact=True)).to_be_disabled()
 page.unroute('**/api/v1/package-intake-context?*',changed_policy);expect(panel.get_by_text('The cell configuration or verification policy has changed. This evidence cannot be approved.',exact=True)).not_to_be_visible(timeout=10000)
 check.check();panel.get_by_role('button',name='Approve device software',exact=True).click();expect(panel.locator('dialog[open]')).to_be_visible();page.screenshot(path=str(out/'device-approval-target.png'))
 lost=[]
 def lose(route):
  lost.append(route.request.post_data_json);response=route.fetch();assert response.ok,response.text();route.abort('failed')
 page.route('**/api/v1/device-review/decisions',lose,times=1);panel.get_by_role('button',name='Record approval for this device version',exact=True).click();expect(page.get_by_role('button',name='Check original request',exact=True)).to_be_enabled();assert context.request.get(origin+f'/api/v1/device-review?cell=cell%2Fdemo&id={review}').json()['decision']['revision']=='1'
 page.reload();page.wait_for_load_state('networkidle');received=[]
 def retry(route):received.append(route.request.post_data_json);route.continue_()
 page.route('**/api/v1/device-review/decisions',retry,times=1);page.get_by_role('button',name='Check original request',exact=True).click();expect(page.get_by_text('The request outcome needs verification',exact=True)).not_to_be_visible();assert received==lost
 page.get_by_role('button',name='Package review',exact=True).click();page.locator('.package-item').filter(has_text=title).click();page.locator('.device-review-list button').first.click();expect(panel.get_by_text('Software approval is recorded for this device review version.',exact=True)).to_be_visible();page.screenshot(path=str(out/'device-approved-desktop.png'),full_page=True)
 page.set_viewport_size({'width':390,'height':844});page.screenshot(path=str(out/'device-approved-mobile.png'),full_page=True)
 if not page.evaluate('document.documentElement.scrollWidth <= innerWidth'):
  overflow=page.evaluate("Array.from(document.querySelectorAll('body *')).map(e=>({tag:e.tagName,classes:e.className,text:e.textContent?.slice(0,120),right:e.getBoundingClientRect().right,width:e.getBoundingClientRect().width})).filter(e=>e.right>innerWidth+0.5)")
  (out/'device-mobile-overflow.json').write_text(json.dumps(overflow,indent=2))
  raise AssertionError('device review mobile overflow')
 current=context.request.get(origin+f'/api/v1/device-review?cell=cell%2Fdemo&id={review}').json();assert current['approval_matches_current_review'] and not current['activation_authorized'];assert current['decision']['scope']=='DEVICE_PACKAGE_SOFTWARE'
 summaries=context.request.get(origin+f'/api/v1/device-reviews?cell=cell%2Fdemo&intake={intake}').json();assert len(summaries['reviews'])==1 and 'report' not in summaries['reviews'][0]
 (out/'device-review-ui.json').write_text(json.dumps({'status':'PASS','review':review,'actual_s_report':True,'actual_p_api':True,'checks':['UI request/report workflow','author cannot approve','new report clears acknowledgement','historical report read-only','new report closes open confirmation','changed authority context clears confirmation (injected response)','concrete scope/version confirmation','lost decision response plus identical retry after reload','compact list','software scope and no activation','mobile overflow']},indent=2)+'\n')
 page.set_viewport_size({'width':1440,'height':1050});page.get_by_role('button',name='Sign out',exact=True).click();login('admin');expect(panel.get_by_role('button',name='Approve device software',exact=True)).to_be_disabled()
