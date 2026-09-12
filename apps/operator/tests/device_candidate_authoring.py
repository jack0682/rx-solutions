"""Actual approved plan -> P draft binding -> v2 bundle -> S compiler/package candidate."""
import json,os,subprocess,uuid
from pathlib import Path
def exercise_candidates(client,origin,out,info,plan,limited=None,verifier=None):
 headers={'Origin':origin,'Content-Type':'application/json','X-RX-Client':'browser-v1'}
 def post(path,body):
  r=client.post(origin+path,headers=headers,data=body);assert r.ok,(r.status,r.text());return r.json()
 assert not plan['definition']['issues'];ref={'id':plan['id'],'revision':plan['revision'],'plan_digest':plan['plan_digest']}
 source={'schema':'rx.process-source.v1','process':'test/proposed-tending','entry':'main','conditions':{},'flows':[{'id':'main','root':'supply-step','nodes':[{'id':'supply-step','body':{'kind':'OPERATION','binding':'load'}}]}]}
 draft=post('/api/v1/process-drafts',{'request_key':str(uuid.uuid4()),'command':{'id':str(uuid.uuid4()),'cell':'cell/demo','expected':None,'title':'Proposed robot process','document':source}})
 catalog=post('/api/v1/process-draft/binding-options',{'cell':'cell/demo','device_plans':[ref]});candidate=next(v for v in catalog['candidates'] if v['step']=='robot/supply');assert candidate['device_plan']==ref
 command={'device_plans':[ref],'draft':draft['version']['id'],'cell':'cell/demo','source_revision':draft['version']['revision'],'expected':None,'catalog_digest':catalog['catalog_digest'],'selections':{'load':'robot/supply'}}
 request={'request_key':str(uuid.uuid4()),'command':command};saved=post('/api/v1/process-draft-bindings',request);assert saved==post('/api/v1/process-draft-bindings',request)
 r=client.get(origin+f'/api/v1/process-draft-compile-input?cell=cell%2Fdemo&id={command["draft"]}&source_revision={command["source_revision"]}&binding_revision={saved["revision"]}');assert r.ok,r.text();bundle=r.json();assert bundle['schema']=='rx.process-compile-input.v2';assert bundle['device_sources']['load']['plan']==ref
 assert bundle['bindings']['load']['intent']==info['catalog']['operations']['supply'];file=out/'candidate-compile-input.json';file.write_text(json.dumps(bundle))
 repo=Path(os.environ['RX_DEVICE_REVIEW_SOLUTIONS']);compiled=out/'candidate-compiled';result=subprocess.run([str(repo/'target/debug/rx-process-compile'),'--bundle',str(file),str(compiled)],capture_output=True,text=True);assert result.returncode==0,result.stderr
 resolved=json.loads((compiled/'resolved.json').read_text());report=json.loads((compiled/'compile-report.json').read_text());assert resolved['bindings']['load']['intent']==bundle['bindings']['load']['intent'];assert report['authoring_input']['device_sources']==bundle['device_sources'];assert report['status']=='COMPILED_NOT_QUALIFIED'
 manifest=json.loads((Path(info['package'])/'manifest.json').read_text());contracts=dict(manifest['contracts'],package_abi='rx.package-abi.v1')
 recipe={'schema':'rx.process-package-recipe.v1','package':'test/proposed-process','version':'1.0.0','publisher':'test/publisher','contracts':contracts,'targets':manifest['targets'],'dependencies':[],'assets':[bundle['bindings']['load']['intent']['body']['trajectory']['trajectory']]}
 recipe_file=out/'candidate-recipe.json';recipe_file.write_text(json.dumps(recipe));candidate_dir=out/'candidate-package';r=subprocess.run([os.environ['RX_PROCESS_PACKAGE_BIN'],'assemble',str(file),str(recipe_file),str(candidate_dir)],capture_output=True,text=True);assert r.returncode==0,r.stderr
 archived=json.loads((candidate_dir/'authoring/compile-input.json').read_text());assert archived['device_sources']==bundle['device_sources'];assert not (candidate_dir/'manifest.sig.json').exists()
 if os.environ.get('RX_MIXED_POLICY_ENABLED')=='1':
  key_request=out/'process-signing-request.json';signature=out/'process-signature.json'
  r=subprocess.run([os.environ['RX_PROCESS_PACKAGE_BIN'],'request',str(candidate_dir),'test/process-key',str(key_request)],capture_output=True,text=True);assert r.returncode==0,r.stderr
  env=dict(os.environ,RX_MIXED_PROCESS_REQUEST=str(key_request),RX_MIXED_PROCESS_SIGNATURE=str(signature))
  subprocess.run([str(repo/'tools/cargo'),'test','-p','rx-process-package','--test','package','sign_mixed_process_request','--locked','--offline','--','--ignored','--exact'],cwd=repo,env=env,check=True,capture_output=True)
  published=Path(info['package']).parent/'proposed-process'
  r=subprocess.run([os.environ['RX_PROCESS_PACKAGE_BIN'],'seal',str(candidate_dir),str(signature),info['policy'],str(published)],capture_output=True,text=True);assert r.returncode==0,r.stderr
  before=client.get(origin+'/api/v1/package-intake-context?cell=cell%2Fdemo').json()
  import hashlib
  obj={'manifest':hashlib.sha256((published/'manifest.json').read_bytes()).hexdigest(),'signature':hashlib.sha256((published/'manifest.sig.json').read_bytes()).hexdigest()}
  receipt=post('/api/v1/package-intakes',{'request_key':str(uuid.uuid4()),'command':{'id':str(uuid.uuid4()),'cell':'cell/demo','title':'Proposed process under one mixed policy','relative_path':'proposed-process','object':obj,'configuration_digest':before['configuration_digest'],'policy_generation':before['registration']['generation']}})
  after=client.get(origin+'/api/v1/package-intake-context?cell=cell%2Fdemo').json();assert before['registration']==after['registration']
  detail=client.get(origin+f'/api/v1/device-binding-plan?cell=cell%2Fdemo&id={plan["id"]}').json();assert detail['device_approval_current'] and detail['context_current'] and not detail['configuration_changed']
  packages=client.get(origin+'/api/v1/package-intakes?cell=cell%2Fdemo').json();kinds={v['receipt']['manifest']['entry']['kind'] for v in packages['packages']};assert {'PROCESS','DEVICE_REFERENCE'}<=kinds
  (out/'mixed-package-policy.json').write_text(json.dumps({'status':'PASS','process_intake':receipt['id'],'registration_unchanged':True,'device_approval_still_current':True,'one_store_and_policy':True,'kinds':sorted(kinds),'checks':['actual signed JTC and v2-provenance process packages','explicit ABI allow-list and separate signer kinds','same registration/fingerprint across both intakes','device plan approval/context retained','no active configuration change']},indent=2)+'\n')
  if os.environ.get('RX_DEVICE_PROCESS_REVIEW_ENABLED')=='1':
   from device_process_review import exercise_process_review
   exercise_process_review(client,limited,verifier,origin,out,info,plan,receipt,after,published)
 (out/'candidate-authoring.json').write_text(json.dumps({'status':'PASS','draft':command['draft'],'plan':plan['id'],'actual_p_draft_api':True,'actual_s_compiler':True,'checks':['clean current impact-reviewed plan','exact device binding selection','idempotent draft save','v2 bundle retains plan/step/action provenance','actual S resolved process and BT generation','unsigned process package preserves full input','no qualification or execution']},indent=2)+'\n')
