"""Actual device report API integration; human reviewer identities and signer are test-only."""
import json,os,subprocess,uuid
from pathlib import Path
def exercise(browser,context,origin,fixture,out,info,intake):
 headers={'Origin':origin,'Content-Type':'application/json','X-RX-Client':'browser-v1'}
 def get(client,path):
  r=client.get(origin+path);assert r.ok,r.text();return r.json()
 def post(client,path,body,expected=200):
  r=client.post(origin+path,headers=headers,data=body);assert r.status==expected,(r.status,r.text());return r.json()
 ctx=get(context.request,'/api/v1/package-intake-context?cell=cell%2Fdemo');assert ctx['device_review_authority_digest']
 job=post(context.request,'/api/v1/device-reviews',{'request_key':str(uuid.uuid4()),'command':{'id':str(uuid.uuid4()),'intake':intake,'cell':'cell/demo','configuration_digest':ctx['configuration_digest'],'policy_generation':ctx['registration']['generation']}})
 request=fixture/'device-request.json';request.write_text(json.dumps(job['request']));report_dir=fixture/'exchange/device-report'
 result=subprocess.run([os.environ['RX_DEVICE_REVIEW_TOOL'],'review',info['package'],info['policy'],str(request),str(report_dir)],capture_output=True,text=True);assert result.returncode==0,result.stderr;result=json.loads(result.stdout);assert result['software_checks_passed'] and result['physical_validation']=='NOT_PERFORMED'
 env=dict(os.environ,RX_DEVICE_REVIEW_REPORT=str(report_dir/'verification.json'));repo=Path(os.environ['RX_DEVICE_REVIEW_SOLUTIONS'])
 subprocess.run([str(repo/'tools/cargo'),'test','-p','rx-device-package','--test','jtc_authoring','sign_device_report','--locked','--offline','--','--ignored','--exact'],cwd=repo,env=env,check=True,capture_output=True)
 review=job['request']['id'];report_input={'review':review,'cell':'cell/demo','expected':None,'directory':'device-report','report_digest':result['report_digest']}
 version=post(context.request,'/api/v1/device-review/reports',{'request_key':str(uuid.uuid4()),'command':report_input});assert version['ready_for_software_approval']
 command={'review':review,'cell':'cell/demo','report_revision':version['revision'],'review_digest':version['review_digest'],'expected':None,'choice':'APPROVE','note':'Independent test review of software source consistency only'}
 post(context.request,'/api/v1/device-review/decisions',{'request_key':str(uuid.uuid4()),'command':command},403)
 reviewer=browser.new_context();post(reviewer.request,'/api/v1/session',{'principal':'reviewer','password':'browser-fixture-password'})
 query=f'/api/v1/device-review?cell=cell%2Fdemo&id={review}';read=get(reviewer.request,query);assert read['context_current'] and not read['activation_authorized']
 request_body={'request_key':str(uuid.uuid4()),'command':command};decision=post(reviewer.request,'/api/v1/device-review/decisions',request_body);assert decision['scope']=='DEVICE_PACKAGE_SOFTWARE';assert post(reviewer.request,'/api/v1/device-review/decisions',request_body)==decision
 approved=get(reviewer.request,query);assert approved['approval_matches_current_review'] and not approved['activation_authorized']
 plan_id=None
 if os.environ.get('RX_DEVICE_BINDING_ENABLED')=='1':
  from device_binding_api import exercise_binding
  plan_id=exercise_binding(browser,context,reviewer,origin,out,info,approved)
 report_input['expected']=version['revision'];latest=post(context.request,'/api/v1/device-review/reports',{'request_key':str(uuid.uuid4()),'command':report_input});assert latest['revision']=='2';assert not get(reviewer.request,query)['approval_matches_current_review']
 if plan_id:assert not get(context.request,f'/api/v1/device-binding-plan?cell=cell%2Fdemo&id={plan_id}')['device_approval_current']
 stale=dict(command,expected=decision['revision']);post(reviewer.request,'/api/v1/device-review/decisions',{'request_key':str(uuid.uuid4()),'command':stale},409)
 historical=get(reviewer.request,query+'&revision=1');assert not historical['is_latest'] and historical['version']['report_digest']==version['report_digest']
 authority=fixture/'installation/device-authority.json';saved=authority.read_bytes();value=json.loads(saved);value['keys'][0]['validators']=[];authority.write_text(json.dumps(value))
 current=dict(command,expected=decision['revision'],report_revision=latest['revision'],review_digest=latest['review_digest']);post(reviewer.request,'/api/v1/device-review/decisions',{'request_key':str(uuid.uuid4()),'command':current},409);authority.write_bytes(saved)
 assert get(reviewer.request,query)['decision']['revision']=='1'
 denied=reviewer.request.get(origin+f'/api/v1/device-review?cell=cell%2Fother&id={review}');assert denied.status==403
 (out/'device-review-api.json').write_text(json.dumps({'status':'PASS','review':review,'scope':'DEVICE_PACKAGE_SOFTWARE','actual_s_decoder_report':True,'actual_platform_api':True,'checks':['separate device authority root','signed report registration','author approval denied','independent current-version approval','idempotent decision','new report invalidates old approval','stale target denied','history remains read-only','authority file change blocks fresh approval','wrong cell denied','no qualification/activation']},indent=2)+'\n');reviewer.close()
