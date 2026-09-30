#!/usr/bin/env python3
"""Installed Builtin Python backend lifecycle, not registered P admission or physical qualification."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import uuid
from test_python_environment import wheel

ROOT=Path(__file__).resolve().parents[1]

def run(*args, **kw):
    return subprocess.run(args,check=True,capture_output=True,text=True,**kw).stdout.strip()

PREPARE=r'''
import hashlib,json,pathlib,subprocess
p=pathlib.Path('/config');cfg=json.loads((p/'startup.json').read_text());bindings=json.loads((p/'bindings.json').read_text())
skill=p/'skill';skill.mkdir();(skill/'skill.json').write_text('{"name":"sdk-test","version":"1.0.0"}')
(skill/'skill.py').write_text('from pathlib import Path\nPath("/data/skill-imported").write_text("imported")\nfrom rx_fixture_sdk import translate\ndef main(inputs): return {"value":translate(inputs["value"])}\n')
subprocess.run(['python3','/opt/rx/client/rx','skill','prepare-environment',str(skill),'--wheel',str(next(p.glob('*.whl'))),'--output','/skills/prepared'],check=True)
env=pathlib.Path('/skills/prepared');manifest=(env/'environment.json').read_bytes();record=json.loads(manifest)
def encoded(v):return json.dumps(v,sort_keys=True,separators=(',',':')).encode()
def sha(v):return hashlib.sha256(v).hexdigest()
def ref(raw,schema):return {'sha256':sha(raw),'schema_id':schema,'size_bytes':str(len(raw))}
inputs={'value':5};intent=bindings[0]['allowed_intents'][0]
intent['kind']='FINITE_ACTION';intent['completion_rule']='rx.python.returned.v1'
intent['body']={'program':{'program':ref(manifest,'rx.python-environment.v1'),'parameter_set':ref(encoded(inputs),'rx.python-input.v1')}}
registration={'schema':'rx.python-skill-registration.v1','installation':cfg['installation'],'host':cfg['host'],
 'cell':bindings[0]['cell'],'environment':str(env),'environment_digest':record['environment_digest'],'input':inputs,'intent':intent}
raw=encoded(registration);(p/'python-registration.json').write_bytes(raw)
cfg['backend']={'kind':'PYTHON_SKILL_SIMULATION','registration':{'path':'/config/python-registration.json','sha256':sha(raw)}}
raw=encoded(bindings);(p/'bindings.json').write_bytes(raw);cfg['bindings']['sha256']=sha(raw)
(p/'startup.json').write_bytes(encoded(cfg))
negative=json.loads(json.dumps(bindings));negative[0]['environment']='PHYSICAL';raw=encoded(negative);(p/'physical-bindings.json').write_bytes(raw)
negative_cfg=json.loads(json.dumps(cfg));negative_cfg['bindings']={'path':'/config/physical-bindings.json','sha256':sha(raw)}
(p/'physical-startup.json').write_bytes(encoded(negative_cfg))
wrong=json.loads(json.dumps(registration));wrong['input']={'value':6};raw=encoded(wrong);(p/'changed-registration.json').write_bytes(raw)
wrong_cfg=json.loads(json.dumps(cfg));wrong_cfg['backend']['registration']={'path':'/config/changed-registration.json','sha256':sha(raw)}
(p/'changed-startup.json').write_bytes(encoded(wrong_cfg))
'''

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--image',required=True);p.add_argument('--evidence',type=Path,required=True);args=p.parse_args()
    args.evidence.mkdir(parents=True,exist_ok=False)
    token='rx-python-service-'+uuid.uuid4().hex[:10];volumes=[token+'-'+v for v in ('config','data','skills')]
    setup=token+'-setup';service=token+'-service'
    with tempfile.TemporaryDirectory(prefix='rx-python-service-') as temp:
        fixture=Path(temp)/'fixture';environment=dict(os.environ,RX_HOST_IMAGE_FIXTURE=str(fixture))
        output=run(str(ROOT/'tools/cargo'),'test','-p','rx-host','--test','service','export_host_image_fixture','--locked','--','--ignored','--exact',cwd=ROOT,env=environment)
        assert '1 passed; 0 failed' in output
        sdk=b'from pathlib import Path\nPath("/data/sdk-imported").write_text("imported")\ndef translate(value): return value+7\n'
        wheel(fixture,{'rx_fixture_sdk/__init__.py':sdk})
        try:
            for v in volumes:run('docker','volume','create','--label','rx.acceptance='+token,v)
            run('docker','create','--name',setup,'--network','none','--user','0',
                '-v',volumes[0]+':/config','-v',volumes[1]+':/data','-v',volumes[2]+':/skills',
                '--entrypoint','python3',args.image,'-c',PREPARE)
            inventory=Path(temp)/'image-source.sha256'
            run('docker','cp',setup+':/opt/rx/bin/source.sha256',str(inventory))
            for line in inventory.read_text().splitlines():
                expected,path=line.split('  ',1);relative=path.removeprefix('/source/')
                assert relative!=path and hashlib.sha256((ROOT/relative).read_bytes()).hexdigest()==expected,relative
            run('docker','cp',str(fixture)+'/.',setup+':/config')
            run('docker','start','--attach',setup)
            assert json.loads(run('docker','inspect',setup))[0]['State']['ExitCode']==0
            mounts=['-v',volumes[0]+':/config','-v',volumes[1]+':/data','-v',volumes[2]+':/skills']
            run('docker','run','--rm','--network','none','--user','0',*mounts,'--entrypoint','/bin/sh',args.image,'-c',
                'mkdir -p /data/runtime; chown -R 10001:10001 /config /data /skills; chmod -R u+rwX,go-rwx /config /data /skills')
            common=['docker','run','--rm','--network','none','--read-only','--cap-drop','ALL',
                '-v',volumes[0]+':/config:ro','-v',volumes[1]+':/data','-v',volumes[2]+':/skills:ro',
                '--entrypoint','/opt/rx/bin/rx-hostd',args.image]
            denied=subprocess.run(common+['inspect','/config/physical-startup.json'],capture_output=True,text=True)
            assert denied.returncode!=0
            changed=subprocess.run(common+['inspect','/config/changed-startup.json'],capture_output=True,text=True)
            assert changed.returncode!=0
            inspected=json.loads(run(*common,'inspect','/config/startup.json'));assert not inspected['activation_authorized']
            run(*common,'init','/config/startup.json')
            run('docker','run','-d','--name',service,'--network','none','--read-only','--cap-drop','ALL',
                '--security-opt','no-new-privileges','--tmpfs','/tmp:rw',
                '-v',volumes[0]+':/config:ro','-v',volumes[1]+':/data','-v',volumes[2]+':/skills:ro',
                '--entrypoint','/opt/rx/bin/rx-hostd',args.image,'run','/config/startup.json')
            deadline=time.monotonic()+20
            while True:
                state=json.loads(run('docker','inspect',service))[0];assert state['State']['Running'],run('docker','logs',service)
                read=subprocess.run(['docker','exec',service,'cat','/data/runtime/host-status.json'],capture_output=True,text=True)
                if read.returncode==0:
                    ready=json.loads(read.stdout)
                    if ready['phase']=='SOFTWARE_READY_UNARMED':break
                assert time.monotonic()<deadline;time.sleep(.1)
            assert not ready['qualification_or_arm_restored']
            run('docker','exec',service,'/bin/sh','-c','test ! -e /data/skill-imported && test ! -e /data/sdk-imported')
            run('docker','stop','--time','10',service)
            state=json.loads(run('docker','inspect',service))[0];assert state['State']['ExitCode']==0
            stopped_path=Path(temp)/'stopped.json';run('docker','cp',service+':/data/runtime/host-status.json',str(stopped_path))
            stopped=json.loads(stopped_path.read_text());assert stopped['phase']=='STOPPED' and stopped['stop']['safe_to_drop']
            image=json.loads(run('docker','image','inspect',args.image))[0]
            result={'schema':'rx.python-host-service-test.v1','status':'PASS','image':image['Id'],'inspection':inspected,
                'ready':ready,'stopped':stopped,'physical_binding_denied':True,'changed_input_denied':True,'image_source_inventory_matched':True,'sdk_imports_before_admission':0,
                'scope':'Installed product Builtin Python backend init/run/stop; no P registration or Python operation dispatch in this scene'}
            (args.evidence/'result.json').write_text(json.dumps(result,indent=2)+'\n')
            print(json.dumps({'status':'PASS','image':image['Id']}))
        finally:
            for name in [service,setup]:subprocess.run(['docker','rm','-f',name],capture_output=True)
            for v in volumes:subprocess.run(['docker','volume','rm',v],capture_output=True)

if __name__=='__main__':main()
