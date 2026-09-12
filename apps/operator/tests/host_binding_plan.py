"""P device change proposal -> passive product Host configuration inspection."""
import copy
import hashlib
import json
import os
import subprocess
import uuid
from pathlib import Path


def exercise_host_plan(author, verifier, origin, headers, out, info, change):
    def post(client, route, command):
        body={'request_key':str(uuid.uuid4()),'command':command}
        result=client.post(origin+route,headers=headers,data=body)
        assert result.ok,(route,result.status,result.text())
        assert result.json()==client.post(origin+route,headers=headers,data=body).json()
        return result.json()
    def target(v):
        return {'change':v['id'],'cell':v['cell'],'expected':v['revision'],'plan_digest':v['plan_digest']}
    plan=change['host_binding_plan'];assert plan and plan['hosts']['host/sim']['device_packages']
    url=origin+'/api/v1/process-change?cell=cell%2Fdemo&id='+change['id']
    detail=author.get(url).json()
    assert {'kind':'HOST_BINDING_CHANGE_REQUIRED'} in detail['blockers']
    before=detail['before'];after=detail['after']
    resolved=json.loads((out/'candidate-compiled/resolved.json').read_text())
    assert after['steps'][0]['id']==resolved['root']['id']
    assert len(after['steps'])==1
    assert after['steps'][0]['intent']==info['catalog']['operations']['supply']
    assert plan['after_configuration']==change['after'] and plan['before_configuration']==change['before']
    reviewed=post(verifier,'/api/v1/process-change/impact-review',{'target':target(change),'note':'Independent device/source impact review; Host application still required'})
    staged=post(author,'/api/v1/process-change/stage',target(reviewed));assert staged['state']=='STAGED'
    # No terminal or Host installation proof is supplied: neither preparation nor metadata dispatch may proceed.
    for route,command in [('/api/v1/process-change/prepare',{'target':target(staged),'refresh':False}),('/api/v1/process-change/configure-hosts',target(staged)),('/api/v1/process-change/apply',target(staged))]:
        response=author.post(origin+route,headers=headers,data={'request_key':str(uuid.uuid4()),'command':command})
        assert not response.ok,(route,response.text())
    assert author.get(url).json()['change']['preparation'] is None
    native=Path(info['policy']).parent
    current=json.loads((native/'host-startup.json').read_text())
    binding=json.loads((native/'bindings.json').read_text())[0]
    binding.update(host='host/sim',cell=plan['cell'],definition=before['definition'],envelope=before['envelope'],
                   allowed_intents=[s['intent'] for s in before['steps'] if s['host']=='host/sim'],
                   scope_ids=before['scopes'],condition_ids=sorted({k for s in before['steps'] for k in s['condition_ids']}))
    def write(path,value):
        data=json.dumps(value,separators=(',',':'),sort_keys=True).encode();path.write_bytes(data)
        return {'path':str(path),'sha256':hashlib.sha256(data).hexdigest()}
    current.update(installation=plan['installation'],host='host/sim',backend={'kind':'FILE_SIMULATION'},
                   data_directory=str(native/'host-data'),runtime_directory=str(native/'host-runtime'))
    (native/'host-runtime').mkdir(exist_ok=True)
    for field in current['tls'].values():
        file=native/Path(field['path']).name;field.update(path=str(file),sha256=hashlib.sha256(file.read_bytes()).hexdigest())
        if file.suffix=='.key':file.chmod(0o600)
    current['bindings']=write(native/'current-bindings.json',[binding])
    proposed=copy.deepcopy(current);next_binding=copy.deepcopy(binding);h=plan['hosts']['host/sim']
    assert not h['other_affected_cells']
    next_binding.update(definition=plan['definition'],envelope=plan['envelope'],allowed_intents=h['required_intents'],
                        scope_ids=plan['scopes'],condition_ids=h['required_conditions'])
    proposed['bindings']=write(native/'proposed-bindings.json',[next_binding])
    proposed['backend']={'kind':'JTC_PACKAGE','directory':info['package'],
                         'manifest_digest':h['device_packages'][0]['manifest'],
                         'policy':{'path':info['policy'],'sha256':hashlib.sha256(Path(info['policy']).read_bytes()).hexdigest()}}
    request=out/'host-binding-plan.json';write(request,plan)
    current_file=native/'current-startup.json';proposed_file=native/'proposed-startup.json'
    write(current_file,current);write(proposed_file,proposed)
    binary=Path(os.environ['RX_DEVICE_REVIEW_SOLUTIONS'])/'target/debug/rx-hostd'
    def inspect():
        r=subprocess.run([str(binary),'inspect-binding-change',str(request),str(current_file),str(proposed_file)],capture_output=True,text=True)
        assert r.returncode==0,r.stderr
        return json.loads(r.stdout)
    inspected=inspect();assert inspected['software_matches'],inspected
    assert not inspected['runtime_provider_available'] and not inspected['activation_authorized'] and not inspected['installation_changed']
    assert inspected['native_processes_started']=='0'
    assert not Path(current['data_directory']).exists()
    # A matching operation does not authorize an unrelated identity/qualification or deployment change.
    next_binding['qualification_revision']='2';proposed['bindings']=write(native/'proposed-bindings.json',[next_binding]);write(proposed_file,proposed)
    assert not inspect()['software_matches']
    next_binding['qualification_revision']=binding['qualification_revision'];proposed['bindings']=write(native/'proposed-bindings.json',[next_binding])
    proposed['data_directory']=str(native/'other-state');write(proposed_file,proposed)
    assert not inspect()['software_matches']
    (out/'host-binding-inspection.json').write_text(json.dumps(inspected,indent=2)+'\n')
    (out/'host-binding-integration.json').write_text(json.dumps({'status':'PASS','change':change['id'],
        'checks':['actual approved device process produces exact target steps and signed package requirements',
                  'independent impact review and fresh package revalidation at stage','no native preparation/dispatch/apply',
                  'actual product Host CLI accepts exact JTC candidate software',
                  'qualification and storage changes rejected','JTC control provider remains unavailable',
                  'no Host installation or native process created']},indent=2)+'\n')
