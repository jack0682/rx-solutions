"""Authoring checks use real P API mutations; no compilation/activation or device calls are made."""
import json
import uuid
from playwright.sync_api import expect

def check_authoring(page,context,origin,headers,output):
    cell='cell/demo'
    before=context.request.get(f'{origin}/api/v1/overview').json()['cells'][0]['cell']
    page.get_by_role('button',name='Workflow design',exact=True).click()
    expect(page.get_by_role('heading',name='Design a workflow draft',exact=True)).to_be_visible()
    page.get_by_role('button',name='New draft',exact=True).click()
    page.get_by_label('Draft title',exact=True).fill('Laser material supply draft')
    page.get_by_role('button',name='Save draft and validate structure',exact=True).click()
    expect(page.locator('.draft-validation .badge')).to_have_text('Structure needs verification')
    drafts=context.request.get(f'{origin}/api/v1/process-drafts?cell=cell%2Fdemo').json()['drafts']
    assert len(drafts)==1 and drafts[0]['revision']=='1' and not drafts[0]['structurally_valid']
    draft_id=drafts[0]['id']
    page.get_by_role('button',name='Add node',exact=True).click()
    page.get_by_label('Operation binding name',exact=True).fill('load-material')
    page.get_by_role('button',name='Save draft and validate structure',exact=True).click()
    expect(page.locator('.draft-validation .badge')).to_have_text('Structure verified')
    detail_url=f'{origin}/api/v1/process-draft?cell=cell%2Fdemo&id={draft_id}'
    second=context.request.get(detail_url).json();assert second['version']['revision']=='2'
    assert second['version']['validation']['required_bindings']==['load-material']
    first=context.request.get(detail_url+'&revision=1').json();assert first['document']['flows'][0]['nodes'][0]['body']['children']==[]
    page.get_by_label('Draft title',exact=True).fill('Draft to recover after response loss')
    sent=[]
    def lose_response(route):
        sent.append(route.request.post_data_json);response=route.fetch();assert response.ok,response.text();route.abort('failed')
    page.route('**/api/v1/process-drafts',lose_response,times=1)
    page.get_by_role('button',name='Save draft and validate structure',exact=True).click()
    expect(page.get_by_text('The request outcome needs verification',exact=True)).to_be_visible()
    expect(page.get_by_label('Draft title',exact=True)).to_be_disabled()
    assert context.request.get(detail_url).json()['version']['revision']=='3'
    page.once('dialog',lambda dialog:dialog.accept())
    page.reload();page.wait_for_load_state('networkidle')
    recovered=[]
    def recovery(route):recovered.append(route.request.post_data_json);route.continue_()
    page.route('**/api/v1/process-drafts',recovery,times=1)
    page.get_by_role('button',name='Check original request',exact=True).click()
    expect(page.get_by_text('The request outcome needs verification',exact=True)).not_to_be_visible()
    assert sent==recovered
    page.get_by_role('button',name='Workflow design',exact=True).click()
    page.get_by_role('button',name='Draft to recover after response loss',exact=False).click()
    expect(page.get_by_label('Draft title',exact=True)).to_have_value('Draft to recover after response loss')
    page.get_by_label('Draft title',exact=True).fill('Preserve my edits')
    current=context.request.get(detail_url).json()
    response=context.request.post(f'{origin}/api/v1/process-drafts',headers=headers,data={'request_key':str(uuid.uuid4()),'command':{'id':draft_id,'cell':cell,'expected':'3','title':'Draft changed on the server','document':current['document']}})
    assert response.ok,response.text()
    page.get_by_role('button',name='Save draft and validate structure',exact=True).click()
    expect(page.get_by_text('configuration has changed.',exact=False)).to_be_visible()
    expect(page.get_by_label('Draft title',exact=True)).to_have_value('Preserve my edits')
    assert context.request.get(detail_url).json()['version']['revision']=='4'
    page.get_by_role('button',name='Compare server version',exact=True).click()
    expect(page.get_by_text('Draft changed on the server',exact=True)).to_be_visible()
    page.get_by_label('Saved version to compare',exact=True).fill('1')
    page.get_by_role('button',name='Load this version',exact=True).click()
    expect(page.locator('.draft-comparison').get_by_text('server r1',exact=False)).to_be_visible()
    page.screenshot(path=str(output/'authoring-conflict.png'),full_page=True)
    page.get_by_role('button',name='Operations',exact=True).click()
    page.get_by_role('button',name='Workflow design',exact=True).click()
    expect(page.get_by_label('Draft title',exact=True)).to_have_value('Preserve my edits')
    page.get_by_role('button',name='Discard changes',exact=True).click()
    page.get_by_role('button',name='Draft changed on the server',exact=False).click()
    expect(page.get_by_label('Draft title',exact=True)).to_have_value('Draft changed on the server')
    page.locator('summary').filter(has_text='Advanced source editing and import').click()
    page.get_by_role('button',name='Open source editor',exact=True).click()
    source=context.request.get(detail_url).json()['document'];source['process']='process/laser-material-supply'
    text=json.dumps(source,ensure_ascii=False,indent=2)
    page.get_by_label('Workflow source JSON',exact=True).fill(text)
    expect(page.get_by_role('button',name='Save draft and validate structure',exact=True)).to_be_disabled()
    page.get_by_role('button',name='Operations',exact=True).click()
    page.get_by_role('button',name='Workflow design',exact=True).click()
    page.locator('summary').filter(has_text='Advanced source editing and import').click()
    expect(page.get_by_label('Workflow source JSON',exact=True)).to_have_value(text)
    page.get_by_role('button',name='Apply source',exact=True).click()
    page.get_by_label('Draft title',exact=True).fill('Laser material supply design')
    page.get_by_role('button',name='Save draft and validate structure',exact=True).click()
    expect(page.locator('.draft-validation .badge')).to_have_text('Structure verified')
    page.get_by_role('button',name='Open condition editor',exact=True).click()
    conditions={'ready':{'op':'EQ','fact':'sensor/ready','schema':'boolean/v1','unit':'unitless','expected':{'boolean':True}}}
    page.get_by_label('Condition definitions JSON',exact=True).fill(json.dumps(conditions))
    page.get_by_role('button',name='Apply conditions',exact=True).click()
    page.get_by_role('button',name='Save draft and validate structure',exact=True).click()
    expect(page.locator('.draft-validation .badge')).to_have_text('Structure verified')
    final=context.request.get(detail_url).json()
    assert final['version']['revision']=='6' and final['document']['conditions']==conditions
    assert final['document']['process']=='process/laser-material-supply'
    assert context.request.get(f'{origin}/api/v1/overview').json()['cells'][0]['cell']==before
    with page.expect_download() as download:
        page.get_by_role('button',name='Export source',exact=True).click()
    download.value.save_as(str(output/'authoring-source.json'))
    assert json.loads((output/'authoring-source.json').read_text())==final['document']
    page.screenshot(path=str(output/'authoring-editor.png'),full_page=True)
    page.set_viewport_size({'width':390,'height':844})
    page.screenshot(path=str(output/'authoring-editor-mobile.png'),full_page=True)
    assert page.evaluate('document.documentElement.scrollWidth <= innerWidth'),'authoring horizontal overflow'
    page.set_viewport_size({'width':1440,'height':1100})
    page.get_by_role('button',name='Copy draft',exact=True).click()
    page.get_by_label('Draft title',exact=True).fill('Material supply copy')
    page.get_by_role('button',name='Save draft and validate structure',exact=True).click()
    expect(page.locator('.draft-validation .badge')).to_have_text('Structure verified')
    copies=context.request.get(f'{origin}/api/v1/process-drafts?cell=cell%2Fdemo').json()['drafts']
    assert len(copies)==2 and any(d['title']=='Material supply copy' and d['id']!=draft_id and d['revision']=='1' for d in copies)
    assert context.request.get(detail_url).json()['version']['revision']=='6'
    (output/'authoring-result.json').write_text(json.dumps({'status':'PASS','draft':draft_id,'final_revision':'6','checks':['incomplete draft save','visual operation insertion','same-key recovery after lost commit response and reload','concurrent update preserves local edits','server comparison','buffers survive tab navigation','JSON and condition edits applied atomically','export exactly matches the saved source','history preserved','historical revision comparison','copy gets a separate draft identity','installed cell unchanged','mobile overflow']},ensure_ascii=False,indent=2)+'\n')
