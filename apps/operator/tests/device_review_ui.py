"""Actual device review UI with S-generated signed reports and P authority checks."""
import json,os,subprocess,uuid
from pathlib import Path
from playwright.sync_api import expect
def exercise(browser,context,page,origin,fixture,out,info,intake):
 headers={'Origin':origin,'Content-Type':'application/json','X-RX-Client':'browser-v1'}
 title='모의 로봇 작업 장비';panel=page.get_by_role('region',name='장비 소프트웨어 검토')
 expect(panel).to_be_visible();panel.get_by_role('button',name='장비 검토 요청 만들기',exact=True).click();expect(panel.get_by_role('button',name='장비 검증 요청 내려받기',exact=True)).to_be_visible()
 with page.expect_download() as download:panel.get_by_role('button',name='장비 검증 요청 내려받기',exact=True).click()
 request_path=out/'device-review-request.json';download.value.save_as(str(request_path));request=json.loads(request_path.read_text());assert request['intake']==intake;review=request['id']
 report_dir=fixture/'exchange/device-ui-report';r=subprocess.run([os.environ['RX_DEVICE_REVIEW_TOOL'],'review',info['package'],info['policy'],str(request_path),str(report_dir)],capture_output=True,text=True);assert r.returncode==0,r.stderr;r=json.loads(r.stdout);assert r['software_checks_passed']
 env=dict(os.environ,RX_DEVICE_REVIEW_REPORT=str(report_dir/'verification.json'));repo=Path(os.environ['RX_DEVICE_REVIEW_SOLUTIONS'])
 subprocess.run([str(repo/'tools/cargo'),'test','-p','rx-device-package','--test','jtc_authoring','sign_device_report','--locked','--offline','--','--ignored','--exact'],cwd=repo,env=env,check=True,capture_output=True)
 panel.locator('summary').filter(has_text='장비 검증 보고서 등록').click();panel.get_by_label('장비 보고서 상대 경로',exact=True).fill('device-ui-report');panel.get_by_label('장비 보고서 식별자',exact=True).fill(r['report_digest']);panel.get_by_role('button',name='장비 보고서 등록 요청',exact=True).click()
 expect(panel.get_by_text('장비 패키지 소프트웨어 검사 통과',exact=True)).to_be_visible();expect(panel.get_by_role('button',name='장비 소프트웨어 승인',exact=True)).to_be_disabled();page.screenshot(path=str(out/'device-author-cannot-approve.png'),full_page=True)
 def login(user):
  page.get_by_label('계정',exact=True).fill(user);page.get_by_label('비밀번호',exact=True).fill('browser-fixture-password');page.get_by_role('button',name='로그인',exact=True).click();expect(page.get_by_role('button',name='로그아웃',exact=True)).to_be_visible();page.get_by_role('button',name='패키지 검토',exact=True).click();page.locator('.package-item').filter(has_text=title).click();page.locator('.device-review-list button').first.click()
 page.get_by_role('button',name='로그아웃',exact=True).click();login('reviewer');expect(panel.get_by_role('heading',name='장비 검증 보고서 r1',exact=True)).to_be_visible()
 panel.get_by_label('장비 검토 의견',exact=True).fill('원본·서명·검사 범위를 확인했습니다. 소프트웨어 검토만 승인합니다.');check=panel.get_by_role('checkbox',name='장비 원본·서명·검사 범위와 이 보고서 버전을 확인했습니다.');check.check()
 def new_report(expected):
  response=context.request.post(origin+'/api/v1/device-review/reports',headers=headers,data={'request_key':str(uuid.uuid4()),'command':{'review':review,'cell':'cell/demo','expected':str(expected),'directory':'device-ui-report','report_digest':r['report_digest']}});assert response.ok,response.text()
 new_report(1);expect(panel.get_by_role('heading',name='장비 검증 보고서 r2',exact=True)).to_be_visible();expect(check).not_to_be_checked();expect(panel.get_by_role('button',name='장비 소프트웨어 승인',exact=True)).to_be_disabled()
 panel.get_by_label('확인할 장비 보고서 버전',exact=True).fill('1');panel.get_by_role('button',name='과거 장비 버전 보기',exact=True).click();expect(panel.get_by_text('과거 장비 검토 · 읽기 전용',exact=True)).to_be_visible();expect(panel.get_by_role('button',name='장비 소프트웨어 승인',exact=True)).to_be_disabled();panel.get_by_role('button',name='최신 장비 검토 보기',exact=True).click();expect(panel.get_by_role('heading',name='장비 검증 보고서 r2',exact=True)).to_be_visible()
 check.check();panel.get_by_role('button',name='장비 소프트웨어 승인',exact=True).click();expect(panel.locator('dialog[open]')).to_be_visible();new_report(2);expect(panel.locator('dialog[open]')).not_to_be_visible();expect(check).not_to_be_checked();expect(panel.get_by_role('heading',name='장비 검증 보고서 r3',exact=True)).to_be_visible()
 detail=context.request.get(origin+f'/api/v1/device-review?cell=cell%2Fdemo&id={review}').json();assert detail['decision'] is None
 check.check();panel.get_by_role('button',name='장비 소프트웨어 승인',exact=True).click();expect(panel.locator('dialog[open]')).to_be_visible()
 def changed_policy(route):
  response=route.fetch();value=response.json();value['device_review_authority_digest']='f'*64;route.fulfill(status=200,content_type='application/json',body=json.dumps(value))
 page.route('**/api/v1/package-intake-context?*',changed_policy);expect(panel.locator('dialog[open]')).not_to_be_visible(timeout=10000);expect(check).not_to_be_checked();expect(panel.get_by_role('button',name='장비 소프트웨어 승인',exact=True)).to_be_disabled()
 page.unroute('**/api/v1/package-intake-context?*',changed_policy);expect(panel.get_by_text('셀 구성 또는 검증 정책이 변경되어 이 자료로 승인할 수 없습니다.',exact=True)).not_to_be_visible(timeout=10000)
 check.check();panel.get_by_role('button',name='장비 소프트웨어 승인',exact=True).click();expect(panel.locator('dialog[open]')).to_be_visible();page.screenshot(path=str(out/'device-approval-target.png'))
 lost=[]
 def lose(route):
  lost.append(route.request.post_data_json);response=route.fetch();assert response.ok,response.text();route.abort('failed')
 page.route('**/api/v1/device-review/decisions',lose,times=1);panel.get_by_role('button',name='이 장비 버전 승인 기록',exact=True).click();expect(page.get_by_role('button',name='같은 요청 확인',exact=True)).to_be_enabled();assert context.request.get(origin+f'/api/v1/device-review?cell=cell%2Fdemo&id={review}').json()['decision']['revision']=='1'
 page.reload();page.wait_for_load_state('networkidle');received=[]
 def retry(route):received.append(route.request.post_data_json);route.continue_()
 page.route('**/api/v1/device-review/decisions',retry,times=1);page.get_by_role('button',name='같은 요청 확인',exact=True).click();expect(page.get_by_text('요청 결과를 확인해야 합니다',exact=True)).not_to_be_visible();assert received==lost
 page.get_by_role('button',name='패키지 검토',exact=True).click();page.locator('.package-item').filter(has_text=title).click();page.locator('.device-review-list button').first.click();expect(panel.get_by_text('이 장비 검토 버전에 소프트웨어 승인 기록이 있습니다.',exact=True)).to_be_visible();page.screenshot(path=str(out/'device-approved-desktop.png'),full_page=True)
 page.set_viewport_size({'width':390,'height':844});page.screenshot(path=str(out/'device-approved-mobile.png'),full_page=True);assert page.evaluate('document.documentElement.scrollWidth <= innerWidth'),'device review mobile overflow'
 current=context.request.get(origin+f'/api/v1/device-review?cell=cell%2Fdemo&id={review}').json();assert current['approval_matches_current_review'] and not current['activation_authorized'];assert current['decision']['scope']=='DEVICE_PACKAGE_SOFTWARE'
 summaries=context.request.get(origin+f'/api/v1/device-reviews?cell=cell%2Fdemo&intake={intake}').json();assert len(summaries['reviews'])==1 and 'report' not in summaries['reviews'][0]
 (out/'device-review-ui.json').write_text(json.dumps({'status':'PASS','review':review,'actual_s_report':True,'actual_p_api':True,'checks':['UI request/report workflow','author cannot approve','new report clears acknowledgement','historical report read-only','new report closes open confirmation','changed authority context clears confirmation (injected response)','concrete scope/version confirmation','lost decision response plus identical retry after reload','compact list','software scope and no activation','mobile overflow']},indent=2)+'\n')
 page.set_viewport_size({'width':1440,'height':1050});page.get_by_role('button',name='로그아웃',exact=True).click();login('admin');expect(panel.get_by_role('button',name='장비 소프트웨어 승인',exact=True)).to_be_disabled()
