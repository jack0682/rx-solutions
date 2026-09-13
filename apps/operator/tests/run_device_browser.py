"""Actual S JTC authoring -> P package store/API -> browser declaration preview. Test keys only."""
import argparse, hashlib, json, os, shlex, socket, subprocess, sys, tempfile
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--server-runner',type=Path,default=Path(__file__).with_name('with_servers.py'));p.add_argument('--evidence-dir',type=Path,required=True);p.add_argument('--review-api',action='store_true');p.add_argument('--review-ui',action='store_true');p.add_argument('--binding-api',action='store_true');p.add_argument('--candidate-authoring',action='store_true');p.add_argument('--mixed-package-policy',action='store_true');p.add_argument('--device-process-review',action='store_true');p.add_argument('--host-binding-plan',action='store_true');a=p.parse_args()
project=Path(__file__).resolve().parents[1];ws=project.parents[2];platform=ws/'rx-platform';solutions=ws/'rx-solutions'
for port in (8080,5173):
 with socket.socket() as sock:
  if sock.connect_ex(('127.0.0.1',port))==0:raise SystemExit(f'port {port} occupied; existing process not touched')
a.evidence_dir.mkdir(parents=True,exist_ok=False)
def run(cmd,cwd,env):return subprocess.run(cmd,cwd=cwd,env=env,check=True,capture_output=True,text=True)
def write(path,value):path.write_text(json.dumps(value,separators=(',',':'),sort_keys=True))
env=dict(os.environ,CARGO_INCREMENTAL='0')
if a.host_binding_plan:a.device_process_review=True;env['RX_HOST_BINDING_PLAN_ENABLED']='1'
if a.device_process_review:a.mixed_package_policy=True;env['RX_DEVICE_PROCESS_REVIEW_ENABLED']='1'
if a.mixed_package_policy:a.candidate_authoring=True;env['RX_MIXED_POLICY_ENABLED']='1'
if a.candidate_authoring:a.binding_api=True;env['RX_DEVICE_CANDIDATES_ENABLED']='1'
if a.binding_api:env['RX_DEVICE_BINDING_ENABLED']='1'
run([str(solutions/'tools/cargo'),'build','-p','rx-device-package','-p','rx-process-package','-p','rx-process','-p','rx-host','--locked','--offline'],solutions,env)
run([str(platform/'tools/cargo'),'build','-p','rx-api','--bin','rx-platform-local','--locked','--offline'],platform,env)
with tempfile.TemporaryDirectory(prefix='rx-device-browser-') as temp:
 temp=Path(temp);fixture=temp/'platform';native=temp/'device'
 env.update(RX_PACKAGE_BROWSER_FIXTURE=str(fixture),RX_PROCESS_PACKAGE_BIN=str(solutions/'target/debug/rx-process-package'))
 run([str(platform/'tools/cargo'),'test','-p','rx-api','--test','http','export_operator_package_fixture','--locked','--offline','--','--ignored','--exact'],platform,env)
 env['RX_DEVICE_TOOL_FIXTURE']=str(native)
 run([str(solutions/'tools/cargo'),'test','-p','rx-device-package','--test','jtc_authoring','export_image_fixture','--locked','--offline','--','--ignored','--exact'],solutions,env)
 installation=json.loads((fixture/'installation/installation.json').read_text())['installation_id']
 site=json.loads((native/'site.json').read_text());site['installation']=installation;site['cell']='cell/demo'
 if a.candidate_authoring:site['site_config']=json.loads((fixture/'fixture.json').read_text())['site_config_digest']
 write(native/'site.json',site)
 policy=json.loads((native/'policy.json').read_text())
 for asset in policy['assets']:asset['path']=str(native/Path(asset['path']).name)
 if a.mixed_package_policy:
  public_key=native/'process-key.json';env['RX_MIXED_PROCESS_KEY']=str(public_key)
  run([str(solutions/'tools/cargo'),'test','-p','rx-process-package','--test','package','export_process_authoring_key','--locked','--offline','--','--ignored','--exact'],solutions,env)
  policy['schema']='rx.package-verification-policy.v2';policy['additional_package_abis']=['rx.package-abi.v1'];policy['keys'].append(json.loads(public_key.read_text()))
  goal=site['goals']['supply'];goal_bytes=json.dumps(goal,separators=(',',':'),sort_keys=True,ensure_ascii=False).encode();goal_file=native/'trajectory.json';goal_file.write_bytes(goal_bytes)
  reference={'sha256':hashlib.sha256(goal_bytes).hexdigest(),'schema_id':'rx.ros-jtc.goal.v1','size_bytes':str(len(goal_bytes))};policy['assets'].append({'reference':reference,'path':str(goal_file)})
 write(native/'policy.json',policy)
 tool=str(solutions/'target/debug/rx-device-package');candidate=temp/'candidate';request=temp/'request.json';signature=temp/'signature.json';package=fixture/'exchange/jtc'
 run([tool,'assemble',str(native/'template.json'),str(native/'site.json'),str(native/'recipe.json'),str(candidate)],solutions,env)
 run([tool,'request',str(candidate),'test/key',str(request)],solutions,env)
 env.update(RX_DEVICE_TOOL_REQUEST=str(request),RX_DEVICE_TOOL_SIGNATURE=str(signature))
 run([str(solutions/'tools/cargo'),'test','-p','rx-device-package','--test','jtc_authoring','sign_image_request','--locked','--offline','--','--ignored','--exact'],solutions,env)
 run([tool,'seal',str(candidate),str(signature),str(native/'policy.json'),str(package)],solutions,env)
 service=json.loads((fixture/'installation/package-service.json').read_text());service['policy']={'path':str(native/'policy.json'),'sha256':hashlib.sha256((native/'policy.json').read_bytes()).hexdigest()};service.pop('review_authority',None)
 if a.review_api or a.review_ui or a.binding_api:
  authority=fixture/'installation/device-authority.json';env['RX_DEVICE_REVIEW_AUTHORITY']=str(authority)
  run([str(solutions/'tools/cargo'),'test','-p','rx-device-package','--test','jtc_authoring','export_device_review_authority','--locked','--offline','--','--ignored','--exact'],solutions,env)
  service['device_review_authority']={'path':str(authority),'sha256':hashlib.sha256(authority.read_bytes()).hexdigest()}
  env.update(RX_DEVICE_REVIEW_ENABLED='1' if a.review_api or a.binding_api else '0',RX_DEVICE_REVIEW_UI_ENABLED='1' if a.review_ui else '0',RX_DEVICE_REVIEW_SOLUTIONS=str(solutions),RX_DEVICE_REVIEW_TOOL=tool)
 if a.device_process_review:
  process_authority=fixture/'installation/device-process-authority.json';env['RX_PROCESS_DEVICE_AUTHORITY']=str(process_authority)
  run([str(solutions/'tools/cargo'),'test','-p','rx-process-package','--test','package','export_device_process_review_authority','--locked','--offline','--','--ignored','--exact'],solutions,env)
  service['review_authority']={'path':str(process_authority),'sha256':hashlib.sha256(process_authority.read_bytes()).hexdigest()}
 write(fixture/'installation/package-service.json',service)
 info={'object':{'manifest':hashlib.sha256((package/'manifest.json').read_bytes()).hexdigest(),'signature':hashlib.sha256((package/'manifest.sig.json').read_bytes()).hexdigest()},'catalog':json.loads((package/'device-catalog.json').read_text()),'package':str(package),'policy':str(native/'policy.json')}
 write(fixture/'device-info.json',info)
 helper=str(a.server_runner.resolve())
 command=[sys.executable,helper,'--server',shlex.join([str(platform/'target/debug/rx-platform-local'),'serve',str(fixture/'installation'),'127.0.0.1:8080','http://127.0.0.1:5173']),'--port','8080','--server',shlex.join(['npm','--prefix',str(project),'run','dev']),'--port','5173','--',sys.executable,str(project/'tests/browser_device.py')]
 env.update(RX_DEVICE_BROWSER_FIXTURE=str(fixture),RX_DEVICE_BROWSER_EVIDENCE=str(a.evidence_dir.resolve()))
 result=subprocess.run(command,env=env,capture_output=True,text=True)
 (a.evidence_dir/'browser-run.log').write_text(result.stdout+result.stderr);print(result.stdout+result.stderr);raise SystemExit(result.returncode)
