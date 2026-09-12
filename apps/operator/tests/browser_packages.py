import json,os,subprocess,uuid
from pathlib import Path
from playwright.sync_api import sync_playwright,expect
out=Path(os.environ['RX_PACKAGE_EVIDENCE']);fixture=Path(os.environ['RX_BROWSER_PACKAGE_FIXTURE']);info=json.loads((fixture/'fixture.json').read_text());platform=Path(os.environ['RX_PLATFORM_REPO']);tool=os.environ['RX_PROCESS_PACKAGE_BIN'];origin='http://127.0.0.1:5173';headers={'Origin':origin,'Content-Type':'application/json','X-RX-Client':'browser-v1'}

def sign(report):
 env=dict(os.environ,RX_BROWSER_REVIEW_REPORT=str(report));subprocess.run([str(platform/'tools/cargo'),'test','-p','rx-api','--test','http','sign_operator_review_fixture','--locked','--','--ignored','--exact'],cwd=platform,env=env,check=True,capture_output=True)
def login(page,user):
 page.get_by_label('계정',exact=True).fill(user);page.get_by_label('비밀번호',exact=True).fill('browser-fixture-password');page.get_by_role('button',name='로그인',exact=True).click();expect(page.get_by_role('button',name='로그아웃',exact=True)).to_be_visible();page.get_by_role('button',name='패키지 검토',exact=True).click();expect(page.get_by_role('heading',name='패키지를 검토하고 기록합니다')).to_be_visible()
with sync_playwright() as p:
 browser=p.chromium.launch(headless=True);context=browser.new_context(viewport={'width':1440,'height':1100});page=context.new_page();errors=[];page.on('pageerror',lambda e:errors.append(str(e)))
 page.goto(origin);page.wait_for_load_state('networkidle');page.screenshot(path=str(out/'initial.png'),full_page=True)
 # Reuse the actual app's accessible labels; adapt only after inspecting rendered DOM if needed.
 (out/'initial-dom.html').write_text(page.content())
 login(page,'admin')
 page.locator('summary').filter(has_text='서명 패키지 반입').click();page.get_by_label('반입 제목',exact=True).fill('레이저 소재 공급 검토');page.get_by_label('반입 폴더의 상대 경로',exact=True).fill('package');page.get_by_label('패키지 내용 식별자',exact=True).fill(info['object']['manifest']);page.get_by_label('패키지 서명 식별자',exact=True).fill(info['object']['signature']);page.get_by_role('button',name='패키지 반입 요청',exact=True).click()
 expect(page.locator('.package-item').filter(has_text='레이저 소재 공급 검토')).to_be_visible();expect(page.get_by_role('heading',name='레이저 소재 공급 검토',exact=True)).to_be_visible()
 page.locator('summary').filter(has_text='새 검토 요청 만들기').click();page.get_by_label('load 작업 연결',exact=True).select_option(index=1);page.get_by_role('button',name='검토 요청 생성',exact=True).click();expect(page.get_by_role('button',name='검증 요청 내보내기')).to_be_visible()
 with page.expect_download() as download:page.get_by_role('button',name='검증 요청 내보내기').click()
 request_path=out/'review-request.json';download.value.save_as(str(request_path));request=json.loads(request_path.read_text());review=request['id'];intake=request['intake']
 report_dir=Path(info['exchange'])/'report';result=subprocess.run([tool,'review',str(Path(info['exchange'])/'package'),info['policy'],str(request_path),str(report_dir)],check=True,capture_output=True,text=True);assert json.loads(result.stdout)['compiler_checks_passed'];sign(report_dir/'verification.json');report_digest=json.loads(result.stdout)['report_digest']
 page.locator('summary').filter(has_text='서명된 검증 자료 등록').click();page.get_by_label('검증 자료 상대 경로',exact=True).fill('report');page.get_by_label('검증 보고서 식별자',exact=True).fill(report_digest);page.get_by_role('button',name='검증 자료 등록 요청').click();expect(page.get_by_text('소프트웨어 검토 자료 준비됨',exact=True)).to_be_visible();expect(page.get_by_role('button',name='소프트웨어 검토 승인',exact=True)).to_be_disabled()
 page.screenshot(path=str(out/'author-cannot-approve.png'),full_page=True)
 page.get_by_role('button',name='로그아웃',exact=True).click();login(page,'reviewer');page.locator('.package-item').filter(has_text='레이저 소재 공급 검토').click();page.locator('.review-list button').first.click();expect(page.get_by_text('소프트웨어 검토 자료 준비됨',exact=True)).to_be_visible()
 page.get_by_label('검토 의견',exact=True).fill('원문과 컴파일 결과, 셀 작업을 확인했습니다.');page.get_by_role('checkbox',name='원문·컴파일 결과·셀 연결과 이 검토 버전을 확인했습니다.').check()
 # A new report version under the same signed report invalidates the displayed acknowledgement.
 response=context.request.post(origin+'/api/v1/process-review/reports',headers=headers,data={'request_key':str(uuid.uuid4()),'command':{'review':review,'cell':'cell/demo','expected':'1','directory':'report','report_digest':report_digest}});assert response.ok,response.text()
 expect(page.get_by_role('heading',name='검증 자료 r2',exact=True)).to_be_visible();expect(page.get_by_role('checkbox')).not_to_be_checked();expect(page.get_by_role('button',name='소프트웨어 검토 승인',exact=True)).to_be_disabled()
 page.get_by_label('확인할 검증 버전',exact=True).fill('1');page.get_by_role('button',name='과거 버전 보기',exact=True).click();expect(page.get_by_text('과거 검토 · 읽기 전용',exact=True)).to_be_visible();expect(page.get_by_role('button',name='소프트웨어 검토 승인',exact=True)).to_be_disabled();page.get_by_role('button',name='최신 검토 보기',exact=True).click();expect(page.get_by_role('heading',name='검증 자료 r2',exact=True)).to_be_visible()
 page.get_by_role('checkbox').check();page.get_by_role('button',name='소프트웨어 검토 승인',exact=True).click();expect(page.locator('dialog[open]')).to_be_visible()
 response=context.request.post(origin+'/api/v1/process-review/reports',headers=headers,data={'request_key':str(uuid.uuid4()),'command':{'review':review,'cell':'cell/demo','expected':'2','directory':'report','report_digest':report_digest}});assert response.ok,response.text()
 expect(page.locator('dialog[open]')).not_to_be_visible();expect(page.get_by_role('heading',name='검증 자료 r3',exact=True)).to_be_visible();expect(page.get_by_role('checkbox')).not_to_be_checked()
 page.get_by_role('checkbox').check();page.get_by_role('button',name='소프트웨어 검토 승인',exact=True).click();expect(page.locator('dialog[open]')).to_be_visible();page.screenshot(path=str(out/'approval-target.png'))
 lost=[]
 def lose(route):
  lost.append(route.request.post_data_json);response=route.fetch();assert response.ok,response.text();route.abort('failed')
 page.route('**/api/v1/process-review/decisions',lose,times=1);page.get_by_role('button',name='이 버전 승인 기록',exact=True).click();expect(page.get_by_text('요청 결과를 확인해야 합니다',exact=True)).to_be_visible();expect(page.get_by_role('button',name='같은 요청 확인',exact=True)).to_be_enabled();assert context.request.get(origin+f'/api/v1/process-review?cell=cell%2Fdemo&id={review}').json()['decision']['revision']=='1'
 page.reload();page.wait_for_load_state('networkidle');received=[]
 def retry(route):received.append(route.request.post_data_json);route.continue_()
 page.route('**/api/v1/process-review/decisions',retry,times=1);page.get_by_role('button',name='같은 요청 확인',exact=True).click();expect(page.get_by_text('요청 결과를 확인해야 합니다',exact=True)).not_to_be_visible();assert received==lost
 page.get_by_role('button',name='패키지 검토',exact=True).click();page.locator('.package-item').filter(has_text='레이저 소재 공급 검토').click();page.locator('.review-list button').first.click();expect(page.get_by_text('이 버전에 소프트웨어 승인 기록이 있습니다',exact=True)).to_be_visible();page.screenshot(path=str(out/'approved-software-review.png'),full_page=True)
 page.set_viewport_size({'width':390,'height':844});page.screenshot(path=str(out/'review-mobile.png'),full_page=True);assert page.evaluate('document.documentElement.scrollWidth <= innerWidth'),'mobile overflow'
 view=context.request.get(origin+f'/api/v1/process-review?cell=cell%2Fdemo&id={review}').json();assert not view['activation_authorized'] and view['approval_matches_current_review'];assert view['decision']['revision']=='1';assert not context.request.get(origin+'/api/v1/overview').json()['cells'][0]['runs'];assert not errors,errors
 (out/'result.json').write_text(json.dumps({'status':'PASS','review':review,'intake':intake,'real_api':True,'real_s_compiler':True,'test_only_signer':True,'checks':['intake','step binding selection','review request download','signed report submission','self approval denied','new report clears acknowledgement','history is read only','open approval dialog invalidated by new version','explicit target confirmation','lost decision response and same-key recovery after reload','no run or activation','mobile overflow','no JavaScript errors']},indent=2)+'\n');context.close();browser.close()
