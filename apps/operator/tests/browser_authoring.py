"""Authoring checks use real P API mutations; no compilation/activation or device calls are made."""
import json
import uuid
from playwright.sync_api import expect

def check_authoring(page,context,origin,headers,output):
    cell='cell/demo'
    before=context.request.get(f'{origin}/api/v1/overview').json()['cells'][0]['cell']
    page.get_by_role('button',name='공정 설계',exact=True).click()
    expect(page.get_by_role('heading',name='공정을 초안으로 설계합니다',exact=True)).to_be_visible()
    page.get_by_role('button',name='새 초안',exact=True).click()
    page.get_by_label('초안 제목',exact=True).fill('레이저 소재 공급 초안')
    page.get_by_role('button',name='초안 저장·구조 확인',exact=True).click()
    expect(page.locator('.draft-validation .badge')).to_have_text('구조 확인 필요')
    drafts=context.request.get(f'{origin}/api/v1/process-drafts?cell=cell%2Fdemo').json()['drafts']
    assert len(drafts)==1 and drafts[0]['revision']=='1' and not drafts[0]['structurally_valid']
    draft_id=drafts[0]['id']
    page.get_by_role('button',name='노드 추가',exact=True).click()
    page.get_by_label('작업 연결 이름',exact=True).fill('load-material')
    page.get_by_role('button',name='초안 저장·구조 확인',exact=True).click()
    expect(page.locator('.draft-validation .badge')).to_have_text('구조 확인됨')
    detail_url=f'{origin}/api/v1/process-draft?cell=cell%2Fdemo&id={draft_id}'
    second=context.request.get(detail_url).json();assert second['version']['revision']=='2'
    assert second['version']['validation']['required_bindings']==['load-material']
    first=context.request.get(detail_url+'&revision=1').json();assert first['document']['flows'][0]['nodes'][0]['body']['children']==[]
    page.get_by_label('초안 제목',exact=True).fill('응답 유실 후 복원할 초안')
    sent=[]
    def lose_response(route):
        sent.append(route.request.post_data_json);response=route.fetch();assert response.ok,response.text();route.abort('failed')
    page.route('**/api/v1/process-drafts',lose_response,times=1)
    page.get_by_role('button',name='초안 저장·구조 확인',exact=True).click()
    expect(page.get_by_text('요청 결과를 확인해야 합니다',exact=True)).to_be_visible()
    expect(page.get_by_label('초안 제목',exact=True)).to_be_disabled()
    assert context.request.get(detail_url).json()['version']['revision']=='3'
    page.once('dialog',lambda dialog:dialog.accept())
    page.reload();page.wait_for_load_state('networkidle')
    recovered=[]
    def recovery(route):recovered.append(route.request.post_data_json);route.continue_()
    page.route('**/api/v1/process-drafts',recovery,times=1)
    page.get_by_role('button',name='같은 요청 확인',exact=True).click()
    expect(page.get_by_text('요청 결과를 확인해야 합니다',exact=True)).not_to_be_visible()
    assert sent==recovered
    page.get_by_role('button',name='공정 설계',exact=True).click()
    page.get_by_role('button',name='응답 유실 후 복원할 초안',exact=False).click()
    expect(page.get_by_label('초안 제목',exact=True)).to_have_value('응답 유실 후 복원할 초안')
    page.get_by_label('초안 제목',exact=True).fill('내 편집을 보존해야 합니다')
    current=context.request.get(detail_url).json()
    response=context.request.post(f'{origin}/api/v1/process-drafts',headers=headers,data={'request_key':str(uuid.uuid4()),'command':{'id':draft_id,'cell':cell,'expected':'3','title':'서버에서 변경된 초안','document':current['document']}})
    assert response.ok,response.text()
    page.get_by_role('button',name='초안 저장·구조 확인',exact=True).click()
    expect(page.get_by_text('구성이 변경되었습니다.',exact=False)).to_be_visible()
    expect(page.get_by_label('초안 제목',exact=True)).to_have_value('내 편집을 보존해야 합니다')
    assert context.request.get(detail_url).json()['version']['revision']=='4'
    page.get_by_role('button',name='서버 버전 비교',exact=True).click()
    expect(page.get_by_text('서버에서 변경된 초안',exact=True)).to_be_visible()
    page.get_by_label('비교할 저장 버전',exact=True).fill('1')
    page.get_by_role('button',name='이 버전 불러오기',exact=True).click()
    expect(page.locator('.draft-comparison').get_by_text('서버 r1',exact=False)).to_be_visible()
    page.screenshot(path=str(output/'authoring-conflict.png'),full_page=True)
    page.get_by_role('button',name='운영',exact=True).click()
    page.get_by_role('button',name='공정 설계',exact=True).click()
    expect(page.get_by_label('초안 제목',exact=True)).to_have_value('내 편집을 보존해야 합니다')
    page.get_by_role('button',name='변경 버리기',exact=True).click()
    page.get_by_role('button',name='서버에서 변경된 초안',exact=False).click()
    expect(page.get_by_label('초안 제목',exact=True)).to_have_value('서버에서 변경된 초안')
    page.locator('summary').filter(has_text='고급 소스 편집·가져오기').click()
    page.get_by_role('button',name='소스 편집 열기',exact=True).click()
    source=context.request.get(detail_url).json()['document'];source['process']='process/laser-material-supply'
    text=json.dumps(source,ensure_ascii=False,indent=2)
    page.get_by_label('공정 소스 JSON',exact=True).fill(text)
    expect(page.get_by_role('button',name='초안 저장·구조 확인',exact=True)).to_be_disabled()
    page.get_by_role('button',name='운영',exact=True).click()
    page.get_by_role('button',name='공정 설계',exact=True).click()
    page.locator('summary').filter(has_text='고급 소스 편집·가져오기').click()
    expect(page.get_by_label('공정 소스 JSON',exact=True)).to_have_value(text)
    page.get_by_role('button',name='소스 적용',exact=True).click()
    page.get_by_label('초안 제목',exact=True).fill('레이저 소재 공급 설계')
    page.get_by_role('button',name='초안 저장·구조 확인',exact=True).click()
    expect(page.locator('.draft-validation .badge')).to_have_text('구조 확인됨')
    page.get_by_role('button',name='조건 편집 열기',exact=True).click()
    conditions={'ready':{'op':'EQ','fact':'sensor/ready','schema':'boolean/v1','unit':'unitless','expected':{'boolean':True}}}
    page.get_by_label('조건 정의 JSON',exact=True).fill(json.dumps(conditions))
    page.get_by_role('button',name='조건 적용',exact=True).click()
    page.get_by_role('button',name='초안 저장·구조 확인',exact=True).click()
    expect(page.locator('.draft-validation .badge')).to_have_text('구조 확인됨')
    final=context.request.get(detail_url).json()
    assert final['version']['revision']=='6' and final['document']['conditions']==conditions
    assert final['document']['process']=='process/laser-material-supply'
    assert context.request.get(f'{origin}/api/v1/overview').json()['cells'][0]['cell']==before
    with page.expect_download() as download:
        page.get_by_role('button',name='소스 내보내기',exact=True).click()
    download.value.save_as(str(output/'authoring-source.json'))
    assert json.loads((output/'authoring-source.json').read_text())==final['document']
    page.screenshot(path=str(output/'authoring-editor.png'),full_page=True)
    page.set_viewport_size({'width':390,'height':844})
    page.screenshot(path=str(output/'authoring-editor-mobile.png'),full_page=True)
    assert page.evaluate('document.documentElement.scrollWidth <= innerWidth'),'authoring horizontal overflow'
    page.set_viewport_size({'width':1440,'height':1100})
    page.get_by_role('button',name='초안 복사',exact=True).click()
    page.get_by_label('초안 제목',exact=True).fill('소재 공급 복사본')
    page.get_by_role('button',name='초안 저장·구조 확인',exact=True).click()
    expect(page.locator('.draft-validation .badge')).to_have_text('구조 확인됨')
    copies=context.request.get(f'{origin}/api/v1/process-drafts?cell=cell%2Fdemo').json()['drafts']
    assert len(copies)==2 and any(d['title']=='소재 공급 복사본' and d['id']!=draft_id and d['revision']=='1' for d in copies)
    assert context.request.get(detail_url).json()['version']['revision']=='6'
    (output/'authoring-result.json').write_text(json.dumps({'status':'PASS','draft':draft_id,'final_revision':'6','checks':['incomplete draft save','visual operation insertion','same-key recovery after lost commit response and reload','concurrent update preserves local edits','server comparison','buffers survive tab navigation','JSON and condition edits applied atomically','export exactly matches the saved source','history preserved','historical revision comparison','copy gets a separate draft identity','installed cell unchanged','mobile overflow']},ensure_ascii=False,indent=2)+'\n')
