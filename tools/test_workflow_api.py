#!/usr/bin/env python3
"""Fresh-process M2 acceptance: data-driven resolution, constraints and immutable receipts."""
import argparse
import copy
import json
from pathlib import Path
import secrets
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
import uuid

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'deployment/local-skills'))
from definitions_client import Definitions,LocalAuthoring
from workflow_client import Workflows,quantity
from runtime_client import RuntimeRejected

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,required=True);parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();binary=args.binary.resolve();source=ROOT/'examples/definitions/0f-laser-simulation'
    load=lambda name:json.loads((source/name).read_text())
    checks=[]
    with tempfile.TemporaryDirectory(prefix='rx-workflow-api-') as temporary:
        root=Path(temporary);password=root/'password';password.write_text(secrets.token_urlsafe(24));password.chmod(0o600)
        with password.open() as stream:subprocess.run([str(binary),'init',str(root/'installation')],stdin=stream,capture_output=True,check=True)
        with socket.socket() as listener:listener.bind(('127.0.0.1',0));port=listener.getsockname()[1]
        url=f'http://127.0.0.1:{port}';process=None
        def start():
            nonlocal process
            process=subprocess.Popen([str(binary),'serve',str(root/'installation'),f'127.0.0.1:{port}',url],stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
            deadline=time.monotonic()+20
            while time.monotonic()<deadline:
                if process.poll() is not None:raise RuntimeError(process.stderr.read().decode())
                try:
                    with urllib.request.urlopen(url+'/api/v1/health',timeout=1) as response:
                        if response.status==200:return
                except OSError:time.sleep(.05)
            raise RuntimeError('server did not start')
        def clients():
            terminal=LocalAuthoring(url,url,'admin',password)
            return Definitions(terminal,root/'client'),Workflows(terminal,root/'client')
        try:
            start();d,w=clients()
            base=d.apply(load('cell.json'),{},str(uuid.uuid4()))
            refs=d.apply(load('m2-definitions.json'),base['references'],str(uuid.uuid4()))['references']
            model=w.apply(load('workflow.json'),refs,str(uuid.uuid4()))
            request={'workflow':model['workflow'],'contexts':{},'property_sets':[],'overrides':{},'inputs':{},'slot_index':'0'}
            def resolve(change=None):
                q=copy.deepcopy(request);q.update(change or {});return w.resolve(q,str(uuid.uuid4()))
            def prop(report,node,key):return next(s for s in report['report']['steps'] if s['node']==node)['properties'][key]['value']['data']
            def scalar(report,node,key):
                value=prop(report,node,key);assert value['kind']=='NUMBER' and value['range']['min']==value['range']['max'];return value['range']['min']
            a=resolve();b=resolve({'contexts':{'part':[refs['part.ECC_99-14']]}})
            assert a['report']['valid'] and a['report']['concrete'] and b['report']['valid']
            assert [s['node'] for s in a['report']['steps']]==['pick','load','clamp','close-door','process','open-door','unload','place']
            assert (scalar(a,'pick','grasp_width'),scalar(b,'pick','grasp_width'))==(47,77)
            assert (scalar(a,'pick','grip_force'),scalar(b,'pick','grip_force'))==(25,17.5)
            assert (scalar(a,'pick','approach_z'),scalar(b,'pick','approach_z'))==(795,825)
            assert scalar(a,'load','surface_z')==840 and scalar(a,'load','approach_z')==885
            assert scalar(b,'load','approach_z')==915
            assert prop(a,'open-door','target_state')=={'kind':'BOOLEAN','value':False}
            checks.append('A/B use the same workflow/rules and independently checked widths, forces and pose-derived heights')
            dense=resolve({'contexts':{'supply':[refs['m2.tray.dense-site']]}})
            assert not dense['report']['valid'] and any(v['location']=='nodes/pick/properties/grasp_width' for v in dense['report']['violations'])
            for override in [{'grip_force':quantity('60:N')},{'grasp_width':quantity('100:mm')},{'grip_force':quantity('0:N')},{'grasp_width':quantity('70..130:mm')}]:
                result=resolve({'overrides':{'pick':override}})
                assert not result['report']['valid'] and any(v['code']=='CONSTRAINT_VIOLATION' for v in result['report']['violations'])
            bad_unit=resolve({'overrides':{'pick':{'grip_force':quantity('40:kg')}}})
            assert not bad_unit['report']['valid'] and any(v['code']=='VALUE_INVALID' for v in bad_unit['report']['violations'])
            bounded=resolve({'overrides':{'pick':{'grip_force':quantity('20..40:N')}}})
            assert bounded['report']['valid'] and not bounded['report']['concrete'] and bounded['report']['status']=='BOUNDED_INPUT_NOT_EXECUTABLE'
            conflict=resolve({'property_sets':[refs['m2.properties.ECC_99']],'overrides':{'pick':{'clearance':quantity('30:mm')}}})
            assert not conflict['report']['valid'] and any(v['code']=='PROPERTY_SET_CONFLICT' for v in conflict['report']['violations'])
            checks.append('density, direct overrides, zero, wrong units, worst-case ranges and conflicting sets block at their nodes')

            # New object type/model and tray model/instance: only data; same workflow revision and resolver.
            package={'schema':'rx.definition-package.v1','catalog':model['workflow']['catalog'],'title':'M2 extension simulation','definitions':[]}
            def add(key,body):package['definitions'].append({'key':key,'id':str(uuid.uuid4()),'label':key+' SIMULATION','expected':None,'body':body})
            add('third-type',{'kind':'OBJECT_TYPE','parent':{'$ref':'type.part'},'fields':{}})
            part=copy.deepcopy(next(v for v in load('cell.json')['definitions'] if v['key']=='part.ECC_51-14')['body'])
            part['object_type']={'$ref':'third-type'};part['values'].update({'width':{'number':60},'height':{'number':30},'allowed_force':{'number':40}})
            add('third-part',part)
            tray=copy.deepcopy(next(v for v in load('m2-definitions.json')['definitions'] if v['key']=='m2.tray.standard')['body'])
            tray['values'].update({'rows':{'number':3},'columns':{'number':5},'row_pitch':{'number':100},'column_pitch':{'number':100}})
            add('new-tray',tray)
            add('new-site',{'kind':'RESOURCE_INSTANCE','base':{'$ref':'new-tray'},'values':{'origin':{'vector':[120,250,700]},'orientation':{'vector':[0,0,2**-.5,2**-.5]},'frame':{'text':'SIMULATION/world'}}})
            extra=d.apply(package,refs,str(uuid.uuid4()))['references']
            changed=resolve({'contexts':{'part':[extra['third-part']],'supply':[extra['new-site']]},'slot_index':'1'})
            assert changed['report']['valid'];assert scalar(changed,'pick','grasp_width')==62
            position=prop(changed,'pick','contact_position')['ranges']
            assert all(abs(v['min']-expected)<1e-8 and v['min']==v['max'] for v,expected in zip(position,[120,350,705]))
            assert abs(scalar(changed,'pick','approach_z')-745)<1e-8
            assert changed['report']['request']['workflow']==a['report']['request']['workflow']
            checks.append('a third part type and a rotated new tray/site resolve through unchanged rules with data-only additions')

            class Loss:
                def __init__(self,terminal):self.terminal=terminal;self.fingerprint=terminal.fingerprint;self.principal=terminal.principal;self.once=True
                def get(self,*a,**kw):return self.terminal.get(*a,**kw)
                def request(self,path,body):
                    result=self.terminal.request(path,body)
                    if self.once and path=='/api/v1/workflow-resolutions':self.once=False;raise OSError('injected lost committed reply')
                    return result
            lost=Workflows(Loss(w.terminal),root/'client');key=str(uuid.uuid4())
            try:lost.resolve(request,key);raise AssertionError('response loss missing')
            except OSError:pass
            recovered=lost.recover(key);assert recovered['report']['valid'];assert lost.resolve(request,key)==recovered
            assert w.report(a['reference'])==a
            process.terminate();process.wait(timeout=10);process.stderr.close();start();d,w=clients()
            assert w.report(a['reference'])==a and w.recover(key)==recovered
            reports=w.terminal.get('/api/v1/workflow-resolutions',catalog=model['workflow']['catalog'])
            assert reports['catalog']==model['workflow']['catalog']
            assert any(v['reference']==a['reference'] and v['workflow']==model['workflow'] and v['status']==a['report']['status'] for v in reports['reports'])
            assert d.points(base['references']['tray.supply'],base['references']['rule.tray-slots'])['total']=='24'
            checks.append('stored reports and original requests survive lost replies/restart; old M1 definitions and pose API still work')
            try:
                bad=copy.deepcopy(request);bad['workflow']['digest']='0'*64;resolve(bad)
                raise AssertionError('forged workflow digest accepted')
            except RuntimeRejected as e:assert e.status==400
            checks.append('forged workflow reference is refused')
        finally:
            if process is not None:
                if process.poll() is None:process.terminate();process.wait(timeout=10)
                process.stderr.close()
    result={'status':'M2_API_PRECHECK_PASSED','checks':checks,'not_performed':['M2 user acceptance','publication','preview execution','device/executor UNKNOWN recovery','physical devices']}
    args.output.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))

if __name__=='__main__':main()
