#!/usr/bin/env python3
"""Exercise the actual resident daemon after manager SIGKILL in one PID namespace.

A separate PID-1 test observer keeps the namespace alive and reaps fixture
orphans. It never supplies process identity or ownership to the product API.
No image is published and no physical device is attached.
"""
import argparse
import hashlib
import json
import subprocess
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OBSERVER = r'''
import json,os,signal,subprocess,time,urllib.request
from pathlib import Path
E=Path('/evidence')
D='/test/rx-solutionsd'
C='/evidence/current.json'
N='/evidence/next.json'
processes=[]
replacement=None
def start(label,*args):
 out=(E/(label+'.stdout')).open('w');err=(E/(label+'.stderr')).open('w')
 p=subprocess.Popen([D,*args],stdout=out,stderr=err);processes.append(p);return p
def rows(label):
 return [json.loads(l) for l in (E/(label+'.stdout')).read_text().splitlines() if l.startswith('{')]
def call(label,*args,success=True):
 p=start(label,*args);rc=p.wait(timeout=20)
 if success: assert rc==0,(label,rc,(E/(label+'.stderr')).read_text())
 else: assert rc!=0,(label,rc)
 return rows(label)
def wait_for(check):
 end=time.monotonic()+25
 while time.monotonic()<end:
  value=check()
  if value:return value
  time.sleep(.05)
 raise AssertionError('observation deadline')
def ready(label):
 def check():
  rr=rows(label)
  for r in reversed(rr):
   if r.get('schema')=='rx.resident-registration-observation.v1':
    e=r['registrations']['status']['executions']
    for x in e:
     if x['last_observed']['state']=='RUNNING' and x['last_observed'].get('process_identity'):return x
 return wait_for(check)
def kernel(pid):
 return {'pid':pid,'stat':Path('/proc/%s/stat'%pid).read_text(),
   'boot':Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
   'pid_namespace':os.readlink('/proc/self/ns/pid'),'time_namespace':os.readlink('/proc/self/ns/time')}
try:
 first=start('original','run',C);old=ready('original');pid=old['last_observed']['pid']
 first_kernel=kernel(pid)
 # A fixture handle opened before manager loss is only for cleanup by this
 # observer. The fresh RX manager never receives it or adopts the saved PID.
 fd=os.pidfd_open(pid)
 first.kill();assert first.wait(timeout=10)==-9
 assert Path('/proc/%s/stat'%pid).exists()
 live=call('live-investigation','investigate',C)[0]['finding']
 assert live['outcome']['state']=='MATCHING_PROCESS_PRESENT',live
 call('live-resume-denied','resume',C,N,success=False)
 assert 'investigation-blocks-resume' in (E/'live-resume-denied.stderr').read_text()
 assert not Path('/var/lib/rx-solutions/managed/runs').exists()
 unknown=call('unknown-run','run',N,success=False)
 assert 'unknown-run-id-no-store-created' in (E/'unknown-run.stderr').read_text()
 assert not Path('/var/lib/rx-solutions/managed/runs').exists()
 # Observed original child still alive. Close it using the pre-loss fixture fd,
 # then PID1 reaps it. This is not a product recovery action or outcome proof.
 signal.pidfd_send_signal(fd,signal.SIGKILL);os.waitpid(pid,0);os.close(fd)
 absent=call('absent-investigation','investigate',C)[0]['finding']
 assert absent['outcome']=={'state':'ORIGINAL_NOT_RUNNING','reason':'scoped-kernel-pid-absent'},absent
 reuse=None
 if os.environ.get('RX_PID_REUSE_FIXTURE')=='1':
  import ctypes
  class CloneArgs(ctypes.Structure):
   _fields_=[(n,ctypes.c_uint64) for n in ['flags','pidfd','child_tid','parent_tid','exit_signal','stack','stack_size','tls','set_tid','set_tid_size','cgroup']]
  libc=ctypes.CDLL(None,use_errno=True);libc.syscall.restype=ctypes.c_long
  desired=(ctypes.c_int*1)(pid)
  ca=CloneArgs(exit_signal=signal.SIGCHLD,set_tid=ctypes.addressof(desired),set_tid_size=1)
  time.sleep(.05)
  replacement=libc.syscall(ctypes.c_long(435),ctypes.byref(ca),ctypes.sizeof(ca))
  if replacement==0:os.execv('/bin/sleep',['/bin/sleep','60'])
  assert replacement==pid,(replacement,ctypes.get_errno())
  reuse=call('reused-pid-investigation','investigate',C)[0]['finding']
  assert reuse['outcome']=={'state':'ORIGINAL_NOT_RUNNING','reason':'scoped-pid-reused-different-starttime'},reuse
  assert Path('/proc/%s/stat'%replacement).exists()
 resumed=start('resumed','resume',C,N);new=ready('resumed')
 assert new['binding']['run']!=old['binding']['run']
 assert new['binding']['instance']!=old['binding']['instance']
 assert new['binding']['registration']==old['binding']['registration']
 if replacement is not None:
  assert Path('/proc/%s/stat'%replacement).exists(),'product must not signal unrelated PID replacement'
 rec=[r for r in rows('resumed') if r.get('schema')=='rx.resident-reconciliation.v1'][0]
 view=rec['registrations']['status'];history=next(e for e in view['executions'] if e['binding']['instance']==old['binding']['instance'])
 assert history['last_observed']['state']=='UNKNOWN'
 assert view['recovery']['dispositions'][0]['past_outcome']=='UNRESOLVED'
 # Stop through the new owner's normal signal path. Original history persists.
 resumed.terminate();assert resumed.wait(timeout=20)==0
 replay=call('consumed-denied','resume',C,N,success=False)
 assert 'consumed' in (E/'consumed-denied.stderr').read_text()
 reopened=call('new-run-reopen','run',N)
 assert any(r.get('schema')=='rx.resident-reconciliation.v1' for r in reopened)
 result={'result':'PASS','observer_pid':os.getpid(),'manager_pid':first.pid,
  'original':old,'external_kernel_observation':first_kernel,'live':live,'absent':absent,
  'pid_reuse':reuse,'resumed':new,'recovery':view['recovery'],'old_unknown':history,
  'state_files':sorted(str(p.relative_to('/var/lib/rx-solutions/managed')) for p in Path('/var/lib/rx-solutions/managed').rglob('*.db')),
  'scope':'same-namespace direct non-actuating child; no ownership adoption; original outcome unresolved'}
 assert result['observer_pid']!=result['manager_pid']
 assert len(result['state_files'])==3,result['state_files']
 (E/'scene.json').write_text(json.dumps(result,indent=2)+'\n')
 print(json.dumps({'result':'PASS','scene':'actual-manager-loss-explicit-resume'}))
finally:
 if replacement is not None and replacement>0:
  os.kill(replacement,signal.SIGKILL);os.waitpid(replacement,0)
 for p in processes:
  if p.poll() is None:p.kill();p.wait()
'''

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--image', required=True)
    p.add_argument('--builder', required=True)
    p.add_argument('--evidence', type=Path, required=True)
    p.add_argument('--pid-reuse-fixture', action='store_true', help='Counterexample only: root + CAP_CHECKPOINT_RESTORE + seccomp unconfined in private PID namespace')
    a = p.parse_args()
    e = a.evidence.resolve(); e.mkdir(parents=True, exist_ok=False)
    commands = []
    def run(label, argv, check=True):
        r = subprocess.run(argv, capture_output=True, text=True)
        (e/(label+'.stdout')).write_text(r.stdout); (e/(label+'.stderr')).write_text(r.stderr)
        commands.append({'label':label,'argv':argv,'exit_code':r.returncode})
        (e/'commands.json').write_text(json.dumps(commands,indent=2)+'\n')
        if check and r.returncode: raise RuntimeError(f'{label}: {r.stderr}')
        return r
    runtime=json.loads(run('runtime',['docker','image','inspect',a.image]).stdout)[0]['Id']
    builder=json.loads(run('builder',['docker','image','inspect',a.builder]).stdout)[0]['Id']
    run('build',['docker','run','--rm','-v',f'{ROOT}:/source:ro','-v',f'{e}:/evidence','-w','/source',
        '-e','CARGO_HOME=/evidence/cargo-home','--entrypoint','cargo',builder,'build','--locked','-p','rx-supervisor',
        '--bin','rx-solutionsd','--target-dir','/evidence/target'])
    binary=e/'target/debug/rx-solutionsd'
    config={'schema':'rx.solutions-startup.v1','state_subdirectory':'managed','plan':{
        'schema':'rx.solutions-process-plan.v1','id':str(uuid.uuid4()),'environment':'SIMULATION','profiles':['SIM-JTC-6DOF'],
        'processes':[{'id':'status','program':'rx/status-http','parameters':{'bind':'127.0.0.1','port':'8081'},
            'depends_on':[],'startup_timeout_ms':'10000','shutdown_timeout_ms':'5000','restart_limit':'0','restart_backoff_ms':'500'}]}}
    (e/'current.json').write_text(json.dumps(config,indent=2)+'\n')
    config['plan']['id']=str(uuid.uuid4());(e/'next.json').write_text(json.dumps(config,indent=2)+'\n')
    (e/'observer.py').write_text(OBSERVER)
    data=e/'data';data.mkdir(mode=0o777);data.chmod(0o777);e.chmod(0o777)
    common=['docker','run','--rm','--network','none','--read-only','--cap-drop','ALL','--security-opt','no-new-privileges',
        '--tmpfs','/tmp:rw','-v',f'{data}:/var/lib/rx-solutions','-v',f'{e}:/evidence','-v',f'{binary}:/test/rx-solutionsd:ro']
    if a.pid_reuse_fixture:
        common += ['--user','0:0','--cap-add','CHECKPOINT_RESTORE','--security-opt','seccomp=unconfined','-e','RX_PID_REUSE_FIXTURE=1']
    run('scene',common+['--entrypoint','/usr/bin/python3',runtime,'-I','-B','/evidence/observer.py'])
    # A different real namespace must not turn the old PID into positive absence.
    foreign=run('foreign-namespace',common+['--entrypoint','/test/rx-solutionsd',runtime,'investigate','/evidence/current.json'])
    finding=json.loads(foreign.stdout)['finding'];assert finding['outcome']=={'state':'UNVERIFIABLE','reason':'saved-namespace-not-observable-in-current-scope'},finding
    result=json.loads((e/'scene.json').read_text())
    result.update(privileged_counterexample_fixture=a.pid_reuse_fixture,runtime_image=runtime,builder_image=builder,daemon_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),foreign_namespace=finding)
    (e/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'result':'PASS','evidence':str(e)}))

if __name__=='__main__':main()
