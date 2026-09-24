#!/usr/bin/env python3
"""Exercise explicit diagnostic replacement with real Linux HTTP sources.

Includes actual consumer-manager SIGKILL before and after commit, test-only external
signing and fault injection. No production issuer, new daemon or store is installed.
"""
import argparse
import json
import hashlib
import subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--image',required=True)
    p.add_argument('--builder',default='rust:1.98.1-slim-bookworm')
    p.add_argument('--evidence',required=True,type=Path)
    a=p.parse_args();e=a.evidence.resolve();e.mkdir(parents=True,exist_ok=False);commands=[]
    def run(label,argv):
        r=subprocess.run(argv,capture_output=True,text=True)
        (e/(label+'.stdout')).write_text(r.stdout);(e/(label+'.stderr')).write_text(r.stderr)
        commands.append({'label':label,'argv':argv,'exit_code':r.returncode});(e/'commands.json').write_text(json.dumps(commands,indent=2)+'\n')
        if r.returncode:raise RuntimeError(f'{label}: {r.returncode}: {r.stderr}; evidence retained')
        return r
    runtime=json.loads(run('runtime',['docker','image','inspect',a.image]).stdout)[0]['Id']
    builder=json.loads(run('builder',['docker','image','inspect',a.builder]).stdout)[0]['Id']
    built=run('build',['docker','run','--rm','-v',f'{ROOT}:/source:ro','-v',f'{e}:/evidence','-w','/source','-e','CARGO_HOME=/evidence/cargo-home',
        '--entrypoint','cargo',builder,'test','--locked','-p','rx-supervisor','--test','replacement','--no-run','--target-dir','/evidence/target','--message-format=json'])
    artifacts=[json.loads(l) for l in built.stdout.splitlines() if l.startswith('{')]
    exe=[v['executable'] for v in artifacts if v.get('reason')=='compiler-artifact' and v.get('executable') and v['target']['name']=='replacement'];assert len(exe)==1
    tested=run('replacement-tests',['docker','run','--rm','--network','none','--read-only','--cap-drop','ALL','--security-opt','no-new-privileges',
        '--tmpfs','/tmp:rw','-v',f'{e}:/evidence:ro','--entrypoint',exe[0],runtime,'--nocapture'])
    markers=['replacement_midflight','replacement_faults','concurrent_replacement_single_winner','replacement_manager_loss','replacement_loss_impact']
    result={k:json.loads(next(l.split('=',1)[1] for l in tested.stdout.splitlines() if l.startswith(k+'='))) for k in markers}
    assert result['replacement_faults']['active_routes']==1
    scenes=result['replacement_manager_loss']['scenes'];assert {s['phase'] for s in scenes}=={'before','after'}
    assert all(s['observer_pid']!=s['killed_manager_pid'] and s['source_ownership']=='NOT_RESTORED' for s in scenes)
    assert result['replacement_loss_impact']['old_withheld']
    assert 'stale_assignment_refused=true' in tested.stdout
    assert 'replacement/author-policy-absent' in tested.stdout
    result.update(result='PASS',runtime_image=runtime,builder_image=builder,
        test_binary_sha256=hashlib.sha256((e / Path(exe[0]).relative_to('/evidence')).read_bytes()).hexdigest(),
        scope='diagnostic Consumer application API with real Linux processes; explicit test issuer only; no resident routing CLI or operating-area provider integration',
        monitoring='explicit caller checkpoints; no fixed period or bounded detection delay; no monitor added',
        ownership='one future-assignment route per relationship and one binding version per run; no physical/process/resource ownership transfer')
    (e/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'result':'PASS','evidence':str(e)}))


if __name__=='__main__':main()
