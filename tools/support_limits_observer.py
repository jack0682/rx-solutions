#!/usr/bin/env python3
"""Outside-process FD observation of the real resident status recipe, not ownership adoption."""
import subprocess,json,time,os,signal,fcntl,sys
from pathlib import Path
log=Path('/tmp/daemon.log');out=log.open('w');manager=subprocess.Popen([sys.argv[1],'run',sys.argv[2]],stdout=out,stderr=subprocess.STDOUT)
handles=[]
def fds(pid):
 result={}
 for p in Path(f'/proc/{pid}/fd').iterdir():
  try:result[p.name]={'target':os.readlink(p),'info':Path(f'/proc/{pid}/fdinfo/{p.name}').read_text()}
  except FileNotFoundError:pass
 return result
try:
 deadline=time.monotonic()+15;ready=None
 while time.monotonic()<deadline:
  assert manager.poll() is None,log.read_text()
  for l in log.read_text().splitlines():
   try:v=json.loads(l)
   except json.JSONDecodeError:continue
   records=v.get('state',{}).get('records',{})
   if records and all(r['phase']=='PROCESS_READY' for r in records.values()):ready=records
  if ready:break
  time.sleep(.05)
 assert ready,log.read_text()
 parent=fds(manager.pid);locks=[x['target'] for x in parent.values() if x['target'].endswith('.writer.lock')];assert len(locks)>=2,parent
 children={}
 for name,r in ready.items():
  pid=r['pid'];handles.append(os.pidfd_open(pid));children[name]={'pid':pid,'fds':fds(pid)}
  assert not any(x['target'].endswith(('.writer.lock','.db','.db-wal','.db-shm')) for x in children[name]['fds'].values()),children
 manager.kill();manager.wait();unlocked=[]
 for p in locks:
  with open(p,'a+') as f:fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB);unlocked.append(p)
 alive=[Path(f"/proc/{v['pid']}").exists() for v in children.values()];assert all(alive)
 print(json.dumps({'result':'PASS','parent_lock_fds':parent,'children_after_exec':children,'locks_reacquired_after_manager_loss':unlocked,'original_children_still_present':alive,'scope':'product daemon real spawn after exec; no surviving-child storage lock leak observed; does not exclude transient pre-exec inheritance'}))
finally:
 if manager.poll() is None:manager.terminate();manager.wait(timeout=10)
 for fd in handles:
  try:signal.pidfd_send_signal(fd,signal.SIGTERM)
  except ProcessLookupError:pass
  os.close(fd)
