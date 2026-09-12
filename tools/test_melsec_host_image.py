#!/usr/bin/env python3
"""Product Host with signed DEVICE_REFERENCE package and an explicitly test-only local PLC."""
import argparse, json, os, subprocess, tempfile, time, uuid
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('--image', default='rx-solutions:runtime-draft')
parser.add_argument('--evidence', type=Path, required=True)
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]

def run(*command, **kwargs):
    return subprocess.run(command, check=True, capture_output=True, text=True, **kwargs).stdout.strip()

SIMULATOR = r'''
import json,socket,struct
from pathlib import Path
state={'reads':0,'writes':0,'sequence':1,'test_only':True}
def record(): Path('/data/plc-test-state.json').write_text(json.dumps(state))
def exact(c,n):
    data=b''
    while len(data)<n:
        block=c.recv(n-len(data))
        if not block: raise EOFError()
        data+=block
    return data
record()
with socket.socket() as server:
    server.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1)
    server.bind(('127.0.0.1',5010));server.listen()
    Path('/data/plc-ready').write_text('TEST ONLY')
    while True:
        c,_=server.accept()
        with c:
            while True:
                try: header=exact(c,9)
                except EOFError: break
                assert header[:7]==bytes([0x50,0,0,255,255,3,0])
                body=exact(c,int.from_bytes(header[7:9],'little'))
                command=int.from_bytes(body[2:4],'little')
                address=int.from_bytes(body[6:9],'little')
                if command==0x0401:
                    assert address==350 and body[9:12]==bytes([0xa8,9,0])
                    state['reads']+=1;state['sequence']+=1
                    payload=struct.pack('<QQH',1,state['sequence'],0b111111)
                else:
                    state['writes']+=1;record()
                    raise AssertionError('No native write is permitted in this startup/shutdown test')
                record()
                c.sendall(bytes([0xd0,0,0,255,255,3,0])+struct.pack('<HH',len(payload)+2,0)+payload)
'''

suffix=uuid.uuid4().hex[:12]
config='rx-melsec-config-'+suffix; data='rx-melsec-data-'+suffix
setup='rx-melsec-setup-'+suffix; service='rx-melsec-host-'+suffix
with tempfile.TemporaryDirectory(prefix='rx-melsec-image-') as temp:
    fixture=Path(temp)/'fixture';env=os.environ.copy();env['RX_MELSEC_IMAGE_FIXTURE']=str(fixture)
    run(str(root/'tools/cargo'),'test','-p','rx-host','--test','melsec_service','export_melsec_image_fixture','--locked','--','--ignored','--exact',cwd=root,env=env)
    (fixture/'plc_sim.py').write_text(SIMULATOR)
    try:
        for volume in [config,data]:run('docker','volume','create',volume)
        run('docker','create','--name',setup,'--user','0','--network','none','-v',config+':/config','-v',data+':/data','--entrypoint','/bin/sh',args.image,'-c',
            'mkdir -p /data/runtime; chmod -R u+rwX,go-rwx /config; /opt/rx/bin/rx-hostd inspect /config/startup.json && /opt/rx/bin/rx-hostd init /config/startup.json && chown -R 10001:10001 /config /data')
        run('docker','cp',str(fixture)+'/.',setup+':/config');initial=run('docker','start','--attach',setup)
        assert json.loads(run('docker','inspect',setup))[0]['State']['ExitCode']==0
        # Explicit harness process; it is never selected by package content or product factory.
        run('docker','run','-d','--name',service,'--network','none','--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--tmpfs','/tmp:rw',
            '-v',config+':/config:ro','-v',data+':/data','--entrypoint','/bin/sh',args.image,'-c',
            'python3 /config/plc_sim.py & while [ ! -e /data/plc-ready ]; do sleep 0.02; done; exec /opt/rx/bin/rx-hostd run /config/startup.json')
        deadline=time.monotonic()+20
        while True:
            inspected=json.loads(run('docker','inspect',service))[0]
            assert inspected['State']['Running'],run('docker','logs',service)
            query=subprocess.run(['docker','exec',service,'cat','/data/runtime/host-status.json'],capture_output=True,text=True)
            if query.returncode==0:
                ready=json.loads(query.stdout)
                if ready['phase']=='SOFTWARE_READY_UNARMED':break
            assert time.monotonic()<deadline
            time.sleep(.1)
        assert inspected['HostConfig']['NetworkMode']=='none' and not inspected['HostConfig']['Devices']
        assert inspected['Config']['User']=='10001:10001' and inspected['HostConfig']['ReadonlyRootfs']
        assert not ready['qualification_or_arm_restored']
        tls="import ssl,socket;c=ssl.create_default_context(cafile='/config/ca.pem');c.load_cert_chain('/config/client.pem','/config/client.key');c.set_alpn_protocols(['h2']);s=c.wrap_socket(socket.create_connection(('127.0.0.1',7444)),server_hostname='localhost');assert s.selected_alpn_protocol()=='h2';s.close()"
        run('docker','exec',service,'python3','-c',tls)
        descriptor=json.loads(run('docker','exec',service,'cat','/data/host/installation.json'))
        assert descriptor['native']['kind']=='MELSEC'
        duplicate=subprocess.run(['docker','run','--rm','--network','none','--read-only','--cap-drop','ALL','-v',config+':/config:ro','-v',data+':/data',args.image,'host','run','/config/startup.json'],capture_output=True,text=True)
        assert duplicate.returncode!=0
        # SIGTERM only. Timeout is a failed test; cleanup force affects disposable simulation only.
        run('docker','kill','--signal','TERM',service)
        deadline=time.monotonic()+20
        while json.loads(run('docker','inspect',service))[0]['State']['Running']:
            assert time.monotonic()<deadline,'Host did not prove safe native drop'
            time.sleep(.1)
        assert json.loads(run('docker','inspect',service))[0]['State']['ExitCode']==0
        for file in ['runtime/host-status.json','plc-test-state.json']:
            run('docker','cp',service+':/data/'+file,str(Path(temp)/Path(file).name))
        stopped=json.loads((Path(temp)/'host-status.json').read_text());plc=json.loads((Path(temp)/'plc-test-state.json').read_text())
        assert stopped['phase']=='STOPPED' and stopped['stop']['safe_to_drop']
        assert not stopped['stop']['physical_shutdown_assessed']
        assert plc['reads']>=2 and plc['writes']==0
        image=json.loads(run('docker','image','inspect',args.image))[0]
        result={'schema':'rx.melsec-host-image-test.v1','status':'PASS','image_id':image['Id'],'architecture':image['Architecture'],
                'non_root':True,'read_only_root':True,'network':'none','devices':[],
                'signed_package_verified':True,'release_source_pin_checked':True,'native_journal_initialized':True,
                'ready':ready,'stop':stopped,'plc':plc,'tls_client_auth_h2':True,'duplicate_owner_rejected':True,
                'limitations':['Test-only key and assets; SIMULATION profile and loopback PLC','No physical qualification or process operation was performed','Python PLC is an explicit test harness process, not a product-deployed backend']}
        args.evidence.parent.mkdir(parents=True,exist_ok=True);args.evidence.write_text(json.dumps(result,indent=2)+'\n')
        print(json.dumps({'status':'PASS','image_id':image['Id'],'native_writes':plc['writes']}))
    finally:
        for name in [service,setup]:subprocess.run(['docker','rm','-f',name],capture_output=True)
        for volume in [config,data]:subprocess.run(['docker','volume','rm','-f',volume],capture_output=True)
