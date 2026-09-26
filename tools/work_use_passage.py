#!/usr/bin/env python3
"""Exercise support-gap work commitment and the shipping daemon's default denial.

The test issuer signs in a separate OpenSSL process against an explicitly authored
fixture catalog. Shipping anchors remain absent; no image/service is published.
"""
import argparse
import hashlib
import json
import subprocess
import time
import uuid
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--image', required=True)
    p.add_argument('--builder', default='rust:1.98.1-slim-bookworm')
    p.add_argument('--evidence', type=Path, required=True)
    a = p.parse_args(); e = a.evidence.resolve(); e.mkdir(parents=True, exist_ok=False)
    commands = []
    def run(label, argv, check=True):
        r = subprocess.run(argv, capture_output=True, text=True)
        (e/(label+'.stdout')).write_text(r.stdout); (e/(label+'.stderr')).write_text(r.stderr)
        commands.append({'label':label,'argv':argv,'exit_code':r.returncode})
        (e/'commands.json').write_text(json.dumps(commands,indent=2)+'\n')
        if check and r.returncode: raise RuntimeError(f'{label}: {r.returncode}: {r.stderr}; evidence retained')
        return r
    runtime = json.loads(run('runtime',['docker','image','inspect',a.image]).stdout)[0]['Id']
    builder = json.loads(run('builder',['docker','image','inspect',a.builder]).stdout)[0]['Id']
    build = ['docker','run','--rm','-v',f'{ROOT}:/source:ro','-v',f'{e}:/evidence','-w','/source',
        '-e','CARGO_HOME=/evidence/cargo-home','--entrypoint','cargo',builder]
    run('daemon-build',build+['build','--locked','-p','rx-supervisor','--bin','rx-solutionsd','--target-dir','/evidence/target'])
    compiled = run('test-build',build+['test','--locked','-p','rx-supervisor','--test','work_use','--no-run','--target-dir','/evidence/target','--message-format=json'])
    artifacts = [json.loads(x) for x in compiled.stdout.splitlines() if x.startswith('{')]
    executables = [x['executable'] for x in artifacts if x.get('reason')=='compiler-artifact' and x.get('executable') and x['target']['name']=='work_use']
    assert len(executables)==1
    tested = run('work-tests',['docker','run','--rm','--network','none','--read-only','--cap-drop','ALL',
        '--security-opt','no-new-privileges','--tmpfs','/tmp:rw','-v',f'{e}:/evidence:ro','-e','RX_WORK_INSTALLED=1',
        '--entrypoint',executables[0],runtime,'--nocapture'])
    derived = json.loads(next(x.split('=',1)[1] for x in tested.stdout.splitlines() if x.startswith('work_derived_result=')))
    assert derived['installed_release'] is True
    assert derived['first']['support_profiles']['shortfall']=='2' and derived['second']['support_profiles']['shortfall']=='0'
    assert derived['first']['source_digest']==derived['second']['source_digest']
    assert derived['first']['current_permission']=='NONE; HISTORICAL_WORK_RESULT_ONLY'
    for marker in ['readiness-not-satisfied','changed-since-judgment','current-registration-changed','decision/expired','decision/revoked',
        'already-committed','test rollback','response lost','revocation_serialized_through_commit=true','expired_during_post_cut_io_result_retained=',
        'diagnostic_access_retained=','revocation_after_rollback_refused=true']:
        assert marker in tested.stdout, marker
    config={'schema':'rx.solutions-startup.v1','state_subdirectory':'managed','plan':{'schema':'rx.solutions-process-plan.v1',
        'id':str(uuid.uuid4()),'environment':'SIMULATION','profiles':['SIM-JTC-6DOF'],'processes':[{'id':'status','program':'rx/status-http',
        'parameters':{'bind':'127.0.0.1','port':'8081'},'depends_on':[],'startup_timeout_ms':'10000','shutdown_timeout_ms':'5000',
        'restart_limit':'0','restart_backoff_ms':'500'}]}}
    task={'operation':str(uuid.uuid4()),'selection':'status','operating_area':'example/unconnected-area',
        'required_native_packages':'1000','required_support_profiles':'6'}
    (e/'startup.json').write_text(json.dumps(config,indent=2)+'\n');(e/'task.json').write_text(json.dumps(task,indent=2)+'\n')
    data=e/'data';data.mkdir(mode=0o777);data.chmod(0o777)
    binary=e/'target/debug/rx-solutionsd';container='rx-work-'+uuid.uuid4().hex[:12]
    run('daemon-start',['docker','run','-d','--name',container,'--network','none','--read-only','--cap-drop','ALL',
        '--security-opt','no-new-privileges','--tmpfs','/tmp:rw','-v',f'{data}:/var/lib/rx-solutions','-v',f'{e}:/evidence:ro',
        '--entrypoint','/evidence/target/debug/rx-solutionsd',runtime,'run','/evidence/startup.json','/evidence/task.json'])
    try:
        deadline=time.monotonic()+25; denial=None
        while time.monotonic()<deadline:
            output=run('daemon-log',['docker','logs',container]).stdout
            for line in output.splitlines():
                if line.startswith('{'):
                    row=json.loads(line)
                    if row.get('schema')=='rx.work-use-result.v1':denial=row
            if denial:break
            time.sleep(.1)
        assert denial and denial['gate']=='DENIED' and 'author-policy-absent' in denial['reason'],denial
        external=run('outside-observer',['docker','exec',container,'/usr/bin/python3','-I','-B','-c',
            "import os,json,sqlite3,urllib.request; db=sqlite3.connect('file:/var/lib/rx-solutions/managed/registration.db?mode=ro',uri=True); print(json.dumps({'observer_pid':os.getpid(),'health':json.load(urllib.request.urlopen('http://127.0.0.1:8081/health')),'work_rows':db.execute(\"SELECT count(*) FROM entities WHERE key LIKE 'work/%'\").fetchone()[0]}))"])
        observation=json.loads(external.stdout);assert observation['work_rows']==0 and observation['observer_pid']!=1
        assert observation['health']['phase']=='SOFTWARE_READY_UNCOMMISSIONED'
        run('normal-stop',['docker','stop','--time','15',container])
        state=json.loads(run('stopped',['docker','inspect',container]).stdout)[0]['State'];assert state['ExitCode']==0,state
        run('final-log',['docker','logs',container])
        result={'result':'PASS','runtime_image':runtime,'builder_image':builder,'daemon_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),
            'derived_work':derived,'shipping_daemon_denial':denial,'outside_observation':observation,
            'scope':'actual Linux receiving gate and derived repository work; explicitly test-authored external issuer only',
            'not_established':['operating-area policy integration','physical operation','HTTP observation atomicity with SQL commit','TTL validity after logical cut during commit IO']}
        (e/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'result':'PASS','evidence':str(e)}))
    finally:
        run('cleanup-stop',['docker','stop','--time','15',container],check=False)
        run('cleanup-remove',['docker','rm',container],check=False)


if __name__=='__main__':main()
