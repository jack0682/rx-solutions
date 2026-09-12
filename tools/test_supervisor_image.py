#!/usr/bin/env python3
"""Exercise only the release-owned read-only service under the solutions supervisor."""
import argparse,json,socket,subprocess,tempfile,time,urllib.request,uuid
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--image',default='rx-solutions:runtime-draft');p.add_argument('--evidence',type=Path,required=True);a=p.parse_args()
def run(*args):
 result=subprocess.run(args,capture_output=True,text=True)
 if result.returncode:raise RuntimeError(result.stdout+result.stderr)
 return result.stdout.strip()
with socket.socket() as s:s.bind(('127.0.0.1',0));port=s.getsockname()[1]
suffix=uuid.uuid4().hex[:12];volume='rx-supervisor-data-'+suffix;names=[]
with tempfile.TemporaryDirectory(prefix='rx-supervisor-image-') as tmp:
 config=Path(tmp)/'startup.json';config.write_text(json.dumps({'schema':'rx.solutions-startup.v1','state_subdirectory':'managed','plan':{'schema':'rx.solutions-process-plan.v1','id':str(uuid.uuid4()),'environment':'SIMULATION','profiles':['OM-05'],'processes':[{'id':'status','program':'rx/status-http','parameters':{'bind':'0.0.0.0','port':'8081'},'depends_on':[],'startup_timeout_ms':'10000','shutdown_timeout_ms':'5000','restart_limit':'0','restart_backoff_ms':'500'}]}}));config.chmod(0o644)
 opener=urllib.request.build_opener(urllib.request.ProxyHandler({}))
 def start(label,activate=False):
  name='rx-supervisor-'+label+'-'+suffix;names.append(name)
  command=['docker','run','-d','--name',name,'--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--tmpfs','/tmp:rw','-v',volume+':/var/lib/rx-solutions','-v',str(config)+':/config/startup.json:ro','-p',f'127.0.0.1:{port}:8081']
  if activate:command+=['--entrypoint','/opt/rx/bin/rx-solutionsd',a.image,'activate','/config/startup.json']
  else:command+=[a.image,'supervise','/config/startup.json']
  run(*command);return name
 def ready(name):
  deadline=time.monotonic()+25
  while time.monotonic()<deadline:
   state=json.loads(run('docker','inspect',name))[0]
   assert state['State']['Running'],run('docker','logs',name)
   try:
    with opener.open(f'http://127.0.0.1:{port}/health',timeout=1) as response:data=json.load(response)
    if data.get('supervisor_instance'):
     assert data['hardware_processes_started_by_entrypoint']==0 and data['physical_qualification']=='NOT_PERFORMED'
     return data,state
   except (OSError,urllib.error.URLError):pass
   time.sleep(.05)
  raise AssertionError(run('docker','logs',name))
 def state_from_volume():
  code="import sqlite3,json; c=sqlite3.connect('file:/state/managed/supervisor.db?mode=ro',uri=True); c.execute('pragma query_only=on'); print(c.execute(\"select document from entities where key='supervisor/state'\").fetchone()[0].decode())"
  return json.loads(run('docker','run','--rm','--network','none','--read-only','--user','10001:10001','-v',volume+':/state:rw','--entrypoint','/usr/bin/python3',a.image,'-c',code))['value']
 try:
  run('docker','volume','create',volume)
  run('docker','run','--rm','--network','none','--user','0','-v',volume+':/var/lib/rx-solutions','--entrypoint','/bin/sh',a.image,'-c','chown 10001:10001 /var/lib/rx-solutions')
  first=start('normal');report,container=ready(first)
  assert container['Config']['User']=='10001:10001' and container['HostConfig']['ReadonlyRootfs'] and not container['HostConfig']['Devices']
  running=state_from_volume();assert running['records']['status']['phase'] in ('STARTING','PROCESS_READY')
  run('docker','stop','--time','15',first);exit_state=json.loads(run('docker','inspect',first))[0]['State'];assert exit_state['ExitCode']==0,run('docker','logs',first)
  stopped=state_from_volume();assert stopped['stop_requested'] and stopped['records']['status']['phase']=='EXITED'
  second=start('explicit',True);second_report,_=ready(second);assert second_report['supervisor_instance']!=report['supervisor_instance']
  run('docker','kill','--signal','KILL',second)
  run('docker','start',second)
  deadline=time.monotonic()+15
  while json.loads(run('docker','inspect',second))[0]['State']['Running']:
   assert time.monotonic()<deadline;time.sleep(.05)
  restarted=json.loads(run('docker','inspect',second))[0]['State'];assert restarted['ExitCode']!=0
  unknown=state_from_volume();assert unknown['records']['status']['phase']=='UNKNOWN'
  assert unknown['records']['status']['instance']==second_report['supervisor_instance']
  bad=Path(tmp)/'changed.py';bad.write_text('raise SystemExit("tampered")\n');bad.chmod(0o644)
  denied=subprocess.run(['docker','run','--rm','--network','none','--read-only','--cap-drop','ALL','-v',str(config)+':/config/startup.json:ro','-v',str(bad)+':/opt/rx/tools/solutions_status.py:ro','--entrypoint','/opt/rx/bin/rx-solutionsd',a.image,'inspect','/config/startup.json'],capture_output=True,text=True)
  assert denied.returncode!=0 and 'program file integrity differs' in denied.stderr
  image=json.loads(run('docker','image','inspect',a.image))[0]
  result={'schema':'rx.supervisor-image-test.v1','status':'PASS','image_id':image['Id'],'architecture':image['Architecture'],'non_root':True,'read_only_root':True,'devices':[],'first_instance':report['supervisor_instance'],'second_instance':second_report['supervisor_instance'],'normal_stop':stopped,'unclean_reactivation':unknown,'tampered_program_rejected':True,'limitations':['only the release-owned non-actuating status service was launched','forced crash affected only disposable test containers with no devices','platform-controlled driver authority is not connected','process readiness is not control preparedness or physical shutdown proof']}
  a.evidence.parent.mkdir(parents=True,exist_ok=True);a.evidence.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'status':'PASS','image':image['Id']}))
 finally:
  for name in names:subprocess.run(['docker','rm','-f',name],capture_output=True)
  subprocess.run(['docker','volume','rm',volume],capture_output=True)
