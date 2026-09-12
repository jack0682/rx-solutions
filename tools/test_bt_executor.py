#!/usr/bin/env python3
"""Build the pinned BT validation target and run it without network/device/write access."""
from pathlib import Path
import argparse,hashlib,json,re,subprocess,tempfile

parser=argparse.ArgumentParser()
parser.add_argument('--evidence-dir',type=Path,required=True)
args=parser.parse_args()
root=Path(__file__).resolve().parents[1];out=args.evidence_dir.absolute();out.mkdir(parents=True,exist_ok=True)
repository=root/'vendor/BehaviorTree.CPP'
lock=(root/'dependencies/behaviortree_cpp.repos').read_text()
expected=re.search(r'version:\s*([0-9a-f]{40})',lock).group(1)
actual=subprocess.check_output(['git','-C',str(repository),'rev-parse','HEAD'],text=True).strip()
assert actual==expected, 'BT source revision differs from dependency lock'
assert not subprocess.check_output(['git','-C',str(repository),'status','--porcelain','--untracked-files=all'],text=True).strip(), 'BT source tree was modified'

def run(command,log):
    result=subprocess.run(command,cwd=root,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
    (out/log).write_text(result.stdout)
    if result.returncode:print(result.stdout[-8000:]);raise SystemExit(result.returncode)
    return result.stdout

run(['docker','build','-f','docker/ExecutorValidation.Dockerfile','-t','rx-solutions:executor-validation','.'],'build.log')
with tempfile.TemporaryDirectory(prefix='rx-bt-fixture-') as temporary:
    fixture=Path(temporary)/'compiled'
    run(['./tools/cargo','run','-p','rx-process','--bin','rx-process-compile','--locked','--','examples/process/material-supply.source.json','examples/process/material-supply.bindings.json',str(fixture)],'compiler.log')
    output=run(['docker','run','--rm','--network','none','--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','-v',f'{fixture}:/fixture:ro','rx-solutions:executor-validation','/fixture/resolved.json','/fixture/process.bt.xml'],'tests.log')
    print(output.strip())
    files={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in fixture.iterdir() if p.is_file()}
image=subprocess.check_output(['docker','image','inspect','rx-solutions:executor-validation','--format','{{.Id}}'],text=True).strip()
result={'schema':'rx.bt-executor-check.v1','status':'PASS','btcpp_commit':actual,'btcpp_version':'4.8.3','image':image,'compiled_artifacts':files,
    'scope':'Actual BT.CPP node execution with synthetic validated-view fixtures; no P network client, ROS, native device or operating qualification.'}
(out/'result.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
print(json.dumps(result,ensure_ascii=False))
