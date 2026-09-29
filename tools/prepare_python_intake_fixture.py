#!/usr/bin/env python3
"""Prepare an actual Linux SDK environment/package for isolated P intake tests."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import uuid
from test_python_environment import wheel
ROOT=Path(__file__).resolve().parents[1]
def run(*args,**kw):return subprocess.run(args,check=True,capture_output=True,text=True,**kw).stdout.strip()
PREPARE=r'''
import hashlib,json,pathlib,platform,subprocess,sys
p=pathlib.Path('/fixture');source=p/'skill';source.mkdir()
(source/'skill.json').write_text('{"name":"fixture-sdk","version":"1.0.0","inputs":{"value":"integer"},"outputs":{"value":"integer"},"timeout_ms":2000}')
(source/'skill.py').write_text('from rx_fixture_sdk import translate\ndef main(inputs): return {"value":translate(inputs["value"])}\n')
subprocess.run(['python3','/opt/rx/client/rx','skill','prepare-environment',str(source),'--wheel',str(next(p.glob('*.whl'))),'--output','/fixture/environment'],check=True)
def enc(v):return json.dumps(v,sort_keys=True,separators=(',',':')).encode()
def ref(raw,s):return {'sha256':hashlib.sha256(raw).hexdigest(),'size_bytes':str(len(raw)),'schema_id':s}
env=(p/'environment/environment.json').read_bytes();record=json.loads(env);inputs={'value':5};raw=enc(inputs)
assets=p/'assets';assets.mkdir();(assets/hashlib.sha256(env).hexdigest()).write_bytes(env);(assets/hashlib.sha256(raw).hexdigest()).write_bytes(raw)
intent={'kind':'FINITE_ACTION','target':'device/python-sdk','profile_digest':'00'*32,'site_config_digest':'04'*32,'calibration_digests':[],
 'resource_set':['controller/sim'],'execution_timeout_ms':'2000','prepare_validity_ms':'1000','completion_rule':'rx.python.returned.v1','cancel_rule':'python/unknown',
 'body':{'program':{'program':ref(env,'rx.python-environment.v1'),'parameter_set':ref(raw,'rx.python-input.v1')}}}
registration={'schema':'rx.python-skill-registration.v1','installation':sys.argv[1],'host':'host/sim','cell':'cell/a',
 'environment':'/fixture/environment','environment_digest':record['environment_digest'],'input':inputs,'intent':intent}
(p/'registration.json').write_bytes(enc(registration))
recipe={'schema':'rx.device-package-recipe.v1','package':'test/python-sdk','version':'1.0.0','publisher':'test',
 'targets':[{'os':'LINUX','architecture':'ARM64' if platform.machine()=='aarch64' else 'AMD64','ros_distribution':None}]}
(p/'recipe.json').write_bytes(enc(recipe))
subprocess.run(['/opt/rx/bin/rx-device-package','python-assemble','/fixture/registration.json','/fixture/environment/environment.json','/fixture/recipe.json','/fixture/candidate'],check=True)
subprocess.run(['/opt/rx/bin/rx-device-package','request','/fixture/candidate','test/key','/fixture/signing.json'],check=True)
'''
POLICY=r'''
import json,pathlib,subprocess
p=pathlib.Path('/fixture');m=json.loads((p/'candidate/manifest.json').read_text());public=json.loads((p/'signature.public.json').read_text())
policy={'schema':'rx.package-verification-policy.v1','contracts':m['contracts'],'target':m['targets'][0],
 'keys':[{'id':'test/key','publisher':'test','verifying_key':public['verifying_key'],'kinds':['DEVICE'],'permissions':m['permissions']}],
 'assets':[{'reference':a,'path':'/fixture/assets/'+a['sha256']} for a in m['assets']],'dependencies':[]}
(p/'policy.json').write_text(json.dumps(policy,sort_keys=True,separators=(',',':')))
subprocess.run(['/opt/rx/bin/rx-device-package','seal','/fixture/candidate','/fixture/signature.json','/fixture/policy.json','/fixture/package'],check=True)
subprocess.run(['/opt/rx/bin/rx-device-package','inspect','/fixture/package','/fixture/policy.json'],check=True)
'''
def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--image',required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
    a.output.mkdir(parents=True,exist_ok=False);a.output=a.output.resolve()
    token='rx-python-intake-'+uuid.uuid4().hex[:10];volume=token+'-files';holder=token+'-prepare';installation=str(uuid.uuid4())
    with tempfile.TemporaryDirectory(prefix='rx-python-sign-') as temp:
        temporary=Path(temp)
        try:
            run('docker','volume','create','--label','rx.acceptance='+token,volume)
            run('docker','create','--name',holder,'--user','0','--network','none','-v',volume+':/fixture','--entrypoint','python3',a.image,'-c',PREPARE,installation)
            sdk=wheel(temporary);run('docker','cp',str(sdk),holder+':/fixture/'+sdk.name)
            run('docker','start','--attach',holder);assert json.loads(run('docker','inspect',holder))[0]['State']['ExitCode']==0
            request=temporary/'signing.json';run('docker','cp',holder+':/fixture/signing.json',str(request))
            signature=temporary/'signature.json';env=dict(os.environ,RX_PYTHON_SIGN_REQUEST=str(request),RX_PYTHON_SIGN_OUTPUT=str(signature))
            log=run(str(ROOT/'tools/cargo'),'test','-p','rx-device-package','--test','python','sign_python_fixture_message','--locked','--','--ignored','--exact',env=env,cwd=ROOT)
            assert '1 passed; 0 failed' in log
            for f in [signature,signature.with_suffix('.public.json')]:run('docker','cp',str(f),holder+':/fixture/'+f.name)
            run('docker','run','--rm','--user','0','--network','none','-v',volume+':/fixture','--entrypoint','python3',a.image,'-c',POLICY)
            run('docker','cp',holder+':/fixture/.',str(a.output))
            policy=json.loads((a.output/'policy.json').read_text())
            for asset in policy['assets']:asset['path']=str(a.output/'assets'/asset['reference']['sha256'])
            (a.output/'policy.json').write_text(json.dumps(policy,sort_keys=True,separators=(',',':')))
            (a.output/'build.json').write_text(json.dumps({'installation':installation,'image':json.loads(run('docker','image','inspect',a.image))[0]['Id'],
                'scope':'Actual Linux SDK environment and signed Python package; no runtime dispatch','private_key_mounted':False},indent=2)+'\n')
            print(json.dumps({'status':'PASS','output':str(a.output),'installation':installation}))
        finally:
            subprocess.run(['docker','rm','-f',holder],capture_output=True)
            subprocess.run(['docker','volume','rm',volume],capture_output=True)
if __name__=='__main__':main()
