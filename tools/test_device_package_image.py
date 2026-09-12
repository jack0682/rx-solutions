#!/usr/bin/env python3
"""Exercise the shipped offline device authoring tool; use a test-only external signer."""
import argparse, json, os, subprocess, tempfile, uuid
from pathlib import Path
parser=argparse.ArgumentParser()
parser.add_argument('--image',default='rx-solutions:runtime-draft')
parser.add_argument('--evidence',type=Path,required=True)
parser.add_argument('--kind',choices=['melsec','jtc'],default='melsec')
args=parser.parse_args();root=Path(__file__).resolve().parents[1]
test_module='jtc_authoring' if args.kind=='jtc' else 'authoring'
def run(*cmd,**kwargs):return subprocess.run(cmd,check=True,capture_output=True,text=True,**kwargs).stdout.strip()
suffix=uuid.uuid4().hex[:12];volume='rx-device-authoring-'+suffix;setup='rx-device-authoring-setup-'+suffix
with tempfile.TemporaryDirectory(prefix='rx-device-authoring-') as temporary:
    temp=Path(temporary);fixture=temp/'fixture';env=os.environ.copy();env['RX_DEVICE_TOOL_FIXTURE']=str(fixture)
    run(str(root/'tools/cargo'),'test','-p','rx-device-package','--test',test_module,'export_image_fixture','--locked','--','--ignored','--exact',cwd=root,env=env)
    try:
        run('docker','volume','create',volume)
        run('docker','create','--name',setup,'--user','0','--network','none','-v',volume+':/data','--entrypoint','/bin/sh',args.image,'-c','mkdir -p /data/runtime; chown -R 10001:10001 /data')
        run('docker','cp',str(fixture),setup+':/data/fixture');run('docker','start','--attach',setup)
        assert json.loads(run('docker','inspect',setup))[0]['State']['ExitCode']==0
        def tool(*params):
            return json.loads(run('docker','run','--rm','--network','none','--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--tmpfs','/tmp:rw','-v',volume+':/data','--entrypoint','/opt/rx/bin/rx-device-package',args.image,*params))
        template=tool('template-digest','/data/fixture/template.json')
        candidate=tool('assemble','/data/fixture/template.json','/data/fixture/site.json','/data/fixture/recipe.json','/data/candidate')
        assert candidate['status']=='UNSIGNED_CANDIDATE' and not candidate['activation_authorized']
        tool('request','/data/candidate','test/key','/data/request.json')
        run('docker','cp',setup+':/data/request.json',str(temp/'request.json'))
        env['RX_DEVICE_TOOL_REQUEST']=str(temp/'request.json');env['RX_DEVICE_TOOL_SIGNATURE']=str(temp/'signature.json')
        run(str(root/'tools/cargo'),'test','-p','rx-device-package','--test',test_module,'sign_image_request','--locked','--','--ignored','--exact',cwd=root,env=env)
        run('docker','cp',str(temp/'signature.json'),setup+':/data/signature.json')
        sealed=tool('seal','/data/candidate','/data/signature.json','/data/fixture/policy.json','/data/package')
        inspected=tool('inspect','/data/package','/data/fixture/policy.json')
        assert sealed['status']=='CONTENT_VERIFIED_NOT_QUALIFIED' and not inspected['activation_authorized']
        assert inspected['manifest_digest']==candidate['manifest_digest']
        if args.kind=='jtc':
            assert inspected['profile']['environment']=='SIMULATION'
            assert inspected['control_provider']=='NOT_CONFIGURED'
            assert inspected['operations']['supply']['profile_digest']==inspected['profile_digest']
            assert inspected['outcomes']['profile_digest']==inspected['profile_digest']
        else:
            assert inspected['profile']['transport']['environment']=='SIMULATION'
            assert inspected['profile']['predicates'][0]['command_m']==200
        # The current product resolver is used inside inspect, including signed provenance reconstruction.
        package=temp/'package';run('docker','cp',setup+':/data/package',str(package))
        assert (package/'authoring/assembly.json').is_file() and not (package/'candidate-recipe.json').exists()
        host_test=None
        if args.kind=='jtc':
            startup=json.loads((fixture/'host-startup.json').read_text())
            startup['backend']['manifest_digest']=candidate['manifest_digest']
            updated=temp/'host-startup.json';updated.write_text(json.dumps(startup))
            run('docker','cp',str(updated),setup+':/data/fixture/host-startup.json')
            host_command=['docker','run','--rm','--network','none','--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--tmpfs','/tmp:rw','-v',volume+':/data','--entrypoint','/opt/rx/bin/rx-hostd',args.image]
            inspection=json.loads(run(*host_command,'inspect','/data/fixture/host-startup.json'))
            assert inspection['control_provider']=='NOT_CONFIGURED' and not inspection['activation_authorized']
            run(*host_command,'init','/data/fixture/host-startup.json')
            denied=subprocess.run([*host_command,'run','/data/fixture/host-startup.json'],capture_output=True,text=True,timeout=20)
            assert denied.returncode!=0 and 'JTC_CONTROL_PROVIDER_NOT_CONFIGURED' in denied.stderr,denied.stderr
            run('docker','cp',setup+':/data/host-data/installation.json',str(temp/'installation.json'))
            installation=json.loads((temp/'installation.json').read_text())
            assert installation['native']['kind']=='JTC' and installation['native']['manifest_digest']==candidate['manifest_digest']
            host_test={'inspection':inspection,'native_metadata_kind':'JTC','run_refused_without_provider':True,'run_exit':denied.returncode}
        image=json.loads(run('docker','image','inspect',args.image))[0]
        assert image['Config']['User']=='10001:10001'
        result={'schema':'rx.device-package-image-test.v1','status':'PASS','image_id':image['Id'],'architecture':image['Architecture'],
                'binary':'/opt/rx/bin/rx-device-package','network':'none','read_only_root':True,'non_root':True,
                'device_kind':args.kind,'template':template,'candidate':candidate,'sealed':sealed,'inspection':inspected,
                'signed_authoring_source_present':True,'host_package_startup':host_test,'signer':'external host test harness with embedded test-only key',
                'device_connections':0,'limitations':['SIMULATION inputs only; no hardware/Host operation or physical qualification','Production signer, trust installation and deployment are not performed']}
        args.evidence.parent.mkdir(parents=True,exist_ok=True);args.evidence.write_text(json.dumps(result,indent=2)+'\n')
        print(json.dumps({'status':'PASS','image_id':image['Id'],'manifest_digest':candidate['manifest_digest']}))
    finally:
        subprocess.run(['docker','rm','-f',setup],capture_output=True)
        subprocess.run(['docker','volume','rm','-f',volume],capture_output=True)
