#!/usr/bin/env python3
"""Freeze copies of a completed resident scene and test an exact prior released reader.

No original database is changed. No devices/network are attached. This verifies
source custody only, not P acceptance, process recovery or execution assignment.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import uuid

p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--legacy-image',required=True)
p.add_argument('--source-evidence',required=True,type=Path)
p.add_argument('--cli',required=True,type=Path)
p.add_argument('--evidence',required=True,type=Path)
a=p.parse_args();source=a.source_evidence.resolve();out=a.evidence.resolve();cli=a.cli.resolve()
out.mkdir(parents=True,exist_ok=False)
original=json.loads((source/'result.json').read_text())
assert original['result']=='PASS_REPORTER_UNAVAILABLE_LOCAL_STOP'
image=json.loads(subprocess.check_output(['docker','image','inspect',a.legacy_image],text=True))[0]
assert image['Id']==original['runtime_image']
assert cli.is_file()
assert not any(f.is_symlink() for f in (source/'data').rglob('*'))
def fingerprint(folder):
 return {str(p.relative_to(folder)):hashlib.sha256(p.read_bytes()).hexdigest() for p in folder.rglob('*') if p.is_file()}
source_before=fingerprint(source/'data')
commands=[]
def run(label,args,expected=0):
 r=subprocess.run(list(map(str,args)),capture_output=True,text=True,timeout=40)
 (out/(label+'.stdout')).write_text(r.stdout);(out/(label+'.stderr')).write_text(r.stderr)
 commands.append({'label':label,'argv':list(map(str,args)),'exit_code':r.returncode})
 (out/'commands.json').write_text(json.dumps(commands,indent=2)+'\n')
 if expected is not None:assert r.returncode==expected,(label,r.returncode,r.stderr)
 return r

def legacy(label,data):
 name='rx-transfer-'+uuid.uuid4().hex[:12]
 try:
  return run(label,['docker','run','--name',name,'--network','none','--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--tmpfs','/tmp:rw',
       '-v',str(data)+':/var/lib/rx-solutions','-v',str(source/'startup.json')+':/config/startup.json:ro',
       '--entrypoint','/opt/rx/bin/rx-solutionsd',image['Id'],'run','/config/startup.json'],expected=None)
 finally:
  subprocess.run(['docker','rm','-f',name],capture_output=True,check=True)

for name in ['baseline','fenced']:
 shutil.copytree(source/'data',out/name)
baseline=legacy('old-reader-positive',out/'baseline');assert baseline.returncode==0,baseline.stderr
registry=out/'fenced/managed/registration.db';freeze=str(uuid.uuid4());target=str(uuid.uuid4())
first=run('freeze',[cli,'freeze',registry,freeze,target]);value=json.loads(first.stdout)
assert value['local_declaration_writer']=='PERMANENTLY_FROZEN'
assert value['platform_acceptance']=='NOT_ESTABLISHED' and value['process_ownership']=='NOT_TRANSFERRED'
source_ids=set(original['registration_ids'].values())
assert {r['document']['value']['id'] for r in value['declarations']}==source_ids
recovered=run('same-request-recovery',[cli,'freeze',registry,freeze,target]);assert recovered.stdout==first.stdout
wrong=run('different-target-refused',[cli,'freeze',registry,freeze,str(uuid.uuid4())],expected=None);assert wrong.returncode!=0
old=legacy('old-reader-refused',out/'fenced');assert old.returncode!=0 and 'newer store schema: downgrade refused' in old.stderr
inspection=run('after-old-reader',[cli,'inspect',registry]);assert inspection.stdout==first.stdout
history=[];after=0
while True:
 page=json.loads(run('history-'+str(after),[cli,'history',registry,freeze,after,2]).stdout)
 if not page:break
 history.extend(page);after=int(page[-1]['seq'])
assert all(int(e['seq'])<=int(value['record']['history_head']) for e in history)
source_after=fingerprint(source/'data');assert source_before==source_after
result={'status':'PASS_SOURCE_FREEZE_AND_LEGACY_REFUSAL','legacy_image':image['Id'],'legacy_daemon_sha256':original['daemon_sha256'],
        'source_registration_ids':sorted(source_ids),'freeze':value,'history_records':len(history),'history_head':str(after),
        'cli_sha256':hashlib.sha256(cli.read_bytes()).hexdigest(),'source_unchanged':source_before==source_after,'original_source_inventory_sha256':hashlib.sha256(json.dumps(source_before,sort_keys=True).encode()).hexdigest(),
        'platform_acceptance':'NOT_PERFORMED','process_ownership_transfer':'NOT_PERFORMED','physical_execution':'NOT_PERFORMED'}
(out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps({'status':result['status'],'evidence':str(out)}))
