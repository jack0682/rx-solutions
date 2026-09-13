"""Actual API binding choices, pending-key recovery and paired compilation export."""
import json
import uuid
from playwright.sync_api import expect

def check_bindings(page,context,origin,headers,output):
    cell='cell/demo';before=context.request.get(f'{origin}/api/v1/overview').json()['cells'][0]['cell']
    page.get_by_role('button',name='Workflow design',exact=True).click()
    page.get_by_role('button',name='Material supply copy',exact=False).click()
    expect(page.get_by_label('Draft title',exact=True)).to_have_value('Material supply copy')
    drafts=context.request.get(f'{origin}/api/v1/process-drafts?cell=cell%2Fdemo').json()['drafts'];draft=next(d for d in drafts if d['title']=='Material supply copy')
    draft_id=draft['id'];detail_url=f'{origin}/api/v1/process-draft?cell=cell%2Fdemo&id={draft_id}';bindings_url=f'{origin}/api/v1/process-draft-bindings?cell=cell%2Fdemo&id={draft_id}'
    page.get_by_label('load-material Device operation',exact=True).select_option('step/place')
    expect(page.get_by_role('button',name='Save bindings',exact=True)).to_be_enabled()
    page.get_by_role('button',name='Operations',exact=True).click();page.get_by_role('button',name='Workflow design',exact=True).click()
    expect(page.get_by_label('load-material Device operation',exact=True)).to_have_value('step/place')
    sent=[]
    def lose(route):sent.append(route.request.post_data_json);response=route.fetch();assert response.ok,response.text();route.abort('failed')
    page.route('**/api/v1/process-draft-bindings',lose,times=1)
    page.get_by_role('button',name='Save bindings',exact=True).click()
    expect(page.get_by_text('The request outcome needs verification',exact=True)).to_be_visible()
    assert context.request.get(bindings_url).json()['binding']['revision']=='1'
    page.once('dialog',lambda dialog:dialog.accept());page.reload();page.wait_for_load_state('networkidle')
    recovered=[]
    def recover(route):recovered.append(route.request.post_data_json);route.continue_()
    page.route('**/api/v1/process-draft-bindings',recover,times=1)
    page.get_by_role('button',name='Check original request',exact=True).click();expect(page.get_by_text('The request outcome needs verification',exact=True)).not_to_be_visible();assert sent==recovered
    page.get_by_role('button',name='Workflow design',exact=True).click();page.get_by_role('button',name='Material supply copy',exact=False).click()
    expect(page.locator('.draft-bindings .badge')).to_have_text('All bindings selected')
    with page.expect_download() as download:page.get_by_role('button',name='Export compiler input',exact=True).click()
    download.value.save_as(str(output/'draft-compile-input.json'))
    bundle=json.loads((output/'draft-compile-input.json').read_text());assert bundle['draft']==draft_id and bundle['source_revision']=='1' and bundle['binding_revision']=='1'
    assert bundle['bindings']['load-material']['host']=='host/sim'
    page.screenshot(path=str(output/'authoring-bindings.png'),full_page=True)
    page.locator('.draft-bindings').screenshot(path=str(output/'binding-detail.png'))
    page.set_viewport_size({'width':390,'height':844});page.locator('.draft-bindings').screenshot(path=str(output/'binding-detail-mobile.png'));assert page.evaluate('document.documentElement.scrollWidth <= innerWidth');page.set_viewport_size({'width':1440,'height':1100})
    # A source change keeps the old binding record but prevents paired export until reviewed.
    source=context.request.get(detail_url).json();source['document']['process']='process/binding-recheck'
    response=context.request.post(f'{origin}/api/v1/process-drafts',headers=headers,data={'request_key':str(uuid.uuid4()),'command':{'id':draft_id,'cell':cell,'expected':'1','title':'Updated material supply','document':source['document']}});assert response.ok,response.text()
    page.get_by_role('button',name='Refresh device operations',exact=True).click()
    expect(page.locator('.draft-bindings .badge')).to_have_text('Configuration review required')
    expect(page.get_by_role('button',name='Export compiler input',exact=True)).to_be_disabled()
    stale=context.request.get(bindings_url).json();assert stale['stale']==['SOURCE_CHANGED'] and stale['binding']['source_revision']=='1'
    assert context.request.get(f'{origin}/api/v1/overview').json()['cells'][0]['cell']==before
    page.get_by_role('button',name='Operations',exact=True).click();page.get_by_role('button',name='Workflow design',exact=True).click()
    page.get_by_role('button',name='Updated material supply',exact=False).click()
    page.get_by_role('button',name='Review against current baseline',exact=True).click();page.get_by_role('button',name='Save bindings',exact=True).click()
    expect(page.locator('.draft-bindings .badge')).to_have_text('All bindings selected')
    final=context.request.get(bindings_url).json();assert final['binding']['revision']=='2' and final['binding']['source_revision']=='2' and not final['stale']
    (output/'binding-result.json').write_text(json.dumps({'status':'PASS','draft':draft_id,'checks':['registered step selection','pending selections survive navigation','same-key recovery after lost binding reply','matched source/bindings export','source change keeps old binding and blocks export','explicit review creates a new binding revision','installed cell unchanged','mobile layout']},indent=2)+'\n')
