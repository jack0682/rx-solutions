"""Actual reviewed JTC package into an inert binding proposal and independent impact review."""
import json,uuid,os
def exercise_binding(browser,author,reviewer,origin,out,info,approved):
 headers={'Origin':origin,'Content-Type':'application/json','X-RX-Client':'browser-v1'}
 def get(client,path):
  r=client.get(origin+path);assert r.ok,r.text();return r.json()
 def post(client,path,body,status=200):
  r=client.post(origin+path,headers=headers,data=body);assert r.status==status,(r.status,r.text());return r.json()
 before=get(author.request,'/api/v1/overview');version=approved['version'];decision=approved['decision'];condition={'op':'EQ','fact':'ready','schema':'boolean/v1','unit':'unitless','expected':{'boolean':True}}
 command={'id':str(uuid.uuid4()),'cell':'cell/demo','review':{'id':version['review'],'revision':version['revision'],'review_digest':version['review_digest'],'decision_revision':decision['revision']},'bindings':{'robot/supply':{'action':'supply','host':'host/sim','conditions':{key:condition for key in info['catalog']['condition_ids']},'completion_postconditions':[],'handover_max_age_ns':'100000000'}},'reason':'Bind approved robot action; process/envelope/Host validation remains required'}
 body={'request_key':str(uuid.uuid4()),'command':command};plan=post(author.request,'/api/v1/device-binding-plans',body);assert plan==post(author.request,'/api/v1/device-binding-plans',body)
 assert plan['state']=='PROPOSED';assert plan['definition']['candidates']['robot/supply']['step']['intent']==info['catalog']['operations']['supply'];assert not plan['definition']['candidates']['robot/supply']['step']['predecessors']
 cells={c['id'] for c in plan['definition']['impact']['cells']};assert cells=={'cell/demo','cell/other'}
 target={'plan':plan['id'],'cell':'cell/demo','expected':plan['revision'],'plan_digest':plan['plan_digest'],'note':'Independent review of both affected cells; no configuration application'}
 post(author.request,'/api/v1/device-binding-plan/impact-review',{'request_key':str(uuid.uuid4()),'command':target},403)
 post(reviewer.request,'/api/v1/device-binding-plan/impact-review',{'request_key':str(uuid.uuid4()),'command':target},403)
 impact=browser.new_context();post(impact.request,'/api/v1/session',{'principal':'impact-reviewer','password':'browser-fixture-password'})
 body={'request_key':str(uuid.uuid4()),'command':target};reviewed=post(impact.request,'/api/v1/device-binding-plan/impact-review',body);assert reviewed==post(impact.request,'/api/v1/device-binding-plan/impact-review',body);assert reviewed['state']=='IMPACT_REVIEWED' and reviewed['revision']=='2' and reviewed['plan_digest']==plan['plan_digest']
 detail=get(impact.request,f'/api/v1/device-binding-plan?cell=cell%2Fdemo&id={plan["id"]}');assert detail['context_current'] and detail['device_approval_current'];assert not detail['activation_authorized'] and not detail['configuration_changed'] and not detail['application_supported']
 if os.environ.get('RX_DEVICE_CANDIDATES_ENABLED')=='1':
  from device_candidate_authoring import exercise_candidates
  exercise_candidates(author.request,origin,out,info,reviewed,reviewer.request,impact.request)
 after=get(author.request,'/api/v1/overview');assert [c['cell'] for c in before['cells']]==[c['cell'] for c in after['cells']];assert all(not c['runs'] for c in after['cells'])
 (out/'device-binding-plan.json').write_text(json.dumps(detail,indent=2)+'\n');(out/'device-binding-api.json').write_text(json.dumps({'status':'PASS','plan':plan['id'],'actual_s_package_and_p_api':True,'checks':['fresh approved device proof','exact signed Intent and outcome mapping','two-cell impact closure','proposer cannot review own impact','reviewer without full cell scope denied','independent all-cell impact review','same request recovers unchanged plan','no active configuration or runs changed','explicit unsupported application boundary']},indent=2)+'\n');impact.close();return plan['id']
