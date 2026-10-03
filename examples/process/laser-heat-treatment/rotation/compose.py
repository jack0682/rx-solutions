import json,uuid,sys
from pathlib import Path
base,task,out=map(Path,sys.argv[1:])
w=json.loads(base.read_text());addition=json.loads(task.read_text())
w['id']=str(uuid.uuid5(uuid.NAMESPACE_URL,'rx-f0/rotation-workflow'));w['expected']=None
w['label']='0F tending with package-defined rotation alignment · SIMULATION'
w['spec']['tasks']['rotate-align']=addition['task']
w['spec']['rules'].update(addition['rules'])
w['spec']['defaults']['part']=[{'$ref':'part.f0-align'}]
w['spec']['defaults']['jig']=[{'$ref':'fixture.f0-align-site'}]
w['spec']['steps'].insert(2,{'id':'rotate-align','task':'rotate-align'})
out.write_text(json.dumps(w,indent=2,ensure_ascii=False)+'\n')
