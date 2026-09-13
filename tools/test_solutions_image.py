#!/usr/bin/env python3
"""Check a dormant, non-root solutions image without devices, native commands or platform access."""
import argparse,json,socket,subprocess,tempfile,time,urllib.request,uuid
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--image',default='rx-solutions:runtime-draft');p.add_argument('--evidence',type=Path,required=True);a=p.parse_args()
def run(*args):return subprocess.run(args,check=True,capture_output=True,text=True).stdout.strip()
with socket.socket() as s:s.bind(('127.0.0.1',0));port=s.getsockname()[1]
name='rx-solutions-smoke-'+uuid.uuid4().hex[:12]
try:
    run('docker','run','-d','--name',name,'--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--tmpfs','/tmp:rw','--tmpfs','/var/lib/rx-solutions:rw,uid=10001,gid=10001,mode=700','-p',f'127.0.0.1:{port}:8081',a.image)
    opener=urllib.request.build_opener(urllib.request.ProxyHandler({}));deadline=time.monotonic()+20;status=None
    while time.monotonic()<deadline:
        inspected=json.loads(run('docker','inspect',name))[0];assert inspected['State']['Running'],run('docker','logs',name)
        try:
            with opener.open(f'http://127.0.0.1:{port}/health',timeout=2) as response:status=json.load(response)
            break
        except (OSError,urllib.error.URLError):time.sleep(.1)
    assert status and status['phase']=='SOFTWARE_READY_UNCOMMISSIONED',status
    assert status['native_packages']>0 and status['support_profiles']==4 and status['simulation_profiles']==4 and status['external_device_repositories']==0
    assert status['hardware_processes_started_by_entrypoint']==0 and status['physical_qualification']=='NOT_PERFORMED'
    assert inspected['Config']['User']=='10001:10001';assert inspected['HostConfig']['ReadonlyRootfs'];assert inspected['HostConfig']['CapDrop']==['ALL'];assert not inspected['HostConfig']['Privileged'];assert not inspected['HostConfig']['Devices']
    process_list=run('docker','top',name,'-eo','pid,comm,args')
    assert not any(word in process_list for word in ['ros2_control_node','controller_manager','rx-bt-engine','rx-executor-service','sim2real_node']),process_list
    request=urllib.request.Request(f'http://127.0.0.1:{port}/api/v1/runs/start',data=b'{}',method='POST',headers={'Content-Type':'application/json'})
    try:opener.open(request,timeout=2);raise AssertionError('control endpoint exposed')
    except urllib.error.HTTPError as error:assert error.code==503 and json.load(error)['code']=='CONTROL_NOT_EXPOSED'
    run('docker','stop','--time','10',name);after=json.loads(run('docker','inspect',name))[0];assert after['State']['ExitCode']==0
    with tempfile.TemporaryDirectory(prefix='rx-catalog-tamper-') as temporary:
        replacement=Path(temporary)/'catalog.json';replacement.write_bytes(b'{}')
        replacement.chmod(0o644)
        denied=subprocess.run(['docker','run','--rm','--network','none','--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','-v',str(replacement)+':/opt/rx/catalogs/device-support.v1.json:ro',a.image,'inspect'],capture_output=True,text=True)
        assert denied.returncode!=0 and 'runtime file integrity differs' in denied.stderr
    image=json.loads(run('docker','image','inspect',a.image))[0]
    result={'schema':'rx.solutions-image-smoke.v1','status':'PASS','image_id':image['Id'],'os':image['Os'],'architecture':image['Architecture'],'user':inspected['Config']['User'],'read_only_root':True,'cap_drop':['ALL'],'devices':[],'startup':status,'process_list':process_list,'sigterm_exit':after['State']['ExitCode'],'tampered_catalog_rejected':True,'limitations':['read-only software diagnostics only','no automatic Host/driver/BT launch','no physical qualification or performance guarantee','operator API delegation not connected']}
    a.evidence.parent.mkdir(parents=True,exist_ok=True);a.evidence.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'status':'PASS','image':image['Id']}))
finally:subprocess.run(['docker','rm','-f',name],capture_output=True)
