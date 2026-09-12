#!/usr/bin/env python3
"""Launch the shipped rx-hostd binary with isolated simulation data and no device permissions."""
import argparse,json,os,socket,ssl,subprocess,tempfile,time,uuid
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--image',default='rx-solutions:runtime-draft');p.add_argument('--evidence',type=Path,required=True);a=p.parse_args();root=Path(__file__).resolve().parents[1]
def run(*args,**kw):return subprocess.run(args,check=True,capture_output=True,text=True,**kw).stdout.strip()
suffix=uuid.uuid4().hex[:12];configuration='rx-host-config-'+suffix;data='rx-host-data-'+suffix;setup='rx-host-setup-'+suffix;service='rx-host-service-'+suffix
with tempfile.TemporaryDirectory(prefix='rx-host-image-') as temp:
    fixture=Path(temp)/'fixture';env=os.environ.copy();env['RX_HOST_IMAGE_FIXTURE']=str(fixture)
    run(str(root/'tools/cargo'),'test','-p','rx-host','--test','service','export_host_image_fixture','--locked','--','--ignored','--exact',cwd=root,env=env)
    with socket.socket() as s:s.bind(('127.0.0.1',0));port=s.getsockname()[1]
    try:
        for volume in [configuration,data]:run('docker','volume','create',volume)
        run('docker','create','--name',setup,'--user','0','--network','none','-v',configuration+':/config','-v',data+':/data','--entrypoint','/bin/sh',a.image,'-c','chmod -R u+rwX,go-rwx /config; mkdir -p /data/runtime; /opt/rx/bin/rx-hostd init /config/startup.json && chown -R 10001:10001 /data /config')
        run('docker','cp',str(fixture)+'/.',setup+':/config');run('docker','start','--attach',setup);assert json.loads(run('docker','inspect',setup))[0]['State']['ExitCode']==0
        run('docker','run','-d','--name',service,'--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--tmpfs','/tmp:rw','-v',configuration+':/config:ro','-v',data+':/data','-p',f'127.0.0.1:{port}:7444',a.image,'host','run','/config/startup.json')
        deadline=time.monotonic()+20
        while True:
            state=json.loads(run('docker','inspect',service))[0];assert state['State']['Running'],run('docker','logs',service)
            out=subprocess.run(['docker','exec',service,'cat','/data/runtime/host-status.json'],capture_output=True,text=True)
            if out.returncode==0:
                ready=json.loads(out.stdout)
                if ready['phase']=='SOFTWARE_READY_UNARMED':break
            assert time.monotonic()<deadline;time.sleep(.1)
        assert ready['admission_open'] and not ready['qualification_or_arm_restored'];assert ready['clock_id'].startswith('linux-boottime/')
        assert state['Config']['User']=='10001:10001' and state['HostConfig']['ReadonlyRootfs'];assert state['HostConfig']['CapDrop']==['ALL'] and not state['HostConfig']['Devices']
        ctx=ssl.create_default_context(cafile=str(fixture/'ca.pem'));ctx.load_cert_chain(str(fixture/'client.pem'),str(fixture/'client.key'));ctx.set_alpn_protocols(['h2'])
        with socket.create_connection(('127.0.0.1',port),timeout=3) as raw:
            with ctx.wrap_socket(raw,server_hostname='localhost') as tls:assert tls.selected_alpn_protocol()=='h2'
        duplicate=subprocess.run(['docker','run','--rm','--network','none','--read-only','--cap-drop','ALL','-v',configuration+':/config:ro','-v',data+':/data',a.image,'host','run','/config/startup.json'],capture_output=True,text=True);assert duplicate.returncode!=0
        process=run('docker','top',service,'-eo','pid,comm,args');assert 'rx-hostd' in process and 'rx-host-sim-server' not in process
        run('docker','stop','--time','10',service);stopped_state=json.loads(run('docker','inspect',service))[0]['State'];assert stopped_state['ExitCode']==0
        run('docker','cp',service+':/data/runtime/host-status.json',str(Path(temp)/'stopped.json'));stopped=json.loads((Path(temp)/'stopped.json').read_text());assert stopped['phase']=='STOPPED' and stopped['stop']['safe_to_drop'];assert not stopped['stop']['physical_shutdown_assessed']
        run('docker','start',service);deadline=time.monotonic()+20
        while True:
            second=json.loads(run('docker','exec',service,'cat','/data/runtime/host-status.json'))
            if second['phase']=='SOFTWARE_READY_UNARMED' and second['host_boot']!=ready['host_boot']:break
            assert time.monotonic()<deadline;time.sleep(.1)
        assert not second['qualification_or_arm_restored'];run('docker','stop','--time','10',service)
        clean=run('docker','run','--rm','--network','none','-v',data+':/data:ro','--entrypoint','/bin/sh',a.image,'-c','test ! -e /data/host/device/effects.jsonl && echo NO_NATIVE_EFFECTS');assert clean=='NO_NATIVE_EFFECTS'
        image=json.loads(run('docker','image','inspect',a.image))[0]
        result={'schema':'rx.host-image-test.v1','status':'PASS','image_id':image['Id'],'architecture':image['Architecture'],'user':'10001:10001','read_only_root':True,'devices':[],'binary':'/opt/rx/bin/rx-hostd','ready':ready,'stop':stopped,'restarted':second,'tls_h2_client_auth':True,'duplicate_owner_rejected':True,'native_effects':0,'processes':process,'limitations':['FILE_SIMULATION backend; MELSEC package backend is verified in its separate test','TLS handshake is separate from full P-Host protocol test','no automatic driver/controller activation','signal stop uses explicit safe-to-drop proof; forced kill is not a physical safety mechanism']}
        a.evidence.parent.mkdir(parents=True,exist_ok=True);a.evidence.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'status':'PASS','image':image['Id'],'binary':'rx-hostd'}))
    finally:
        for name in [service,setup]:subprocess.run(['docker','rm','-f',name],capture_output=True)
        for name in [configuration,data]:subprocess.run(['docker','volume','rm','-f',name],capture_output=True)
