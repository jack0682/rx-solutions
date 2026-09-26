#!/usr/bin/env python3
import argparse,hashlib,io,json,tarfile
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];p=argparse.ArgumentParser();p.add_argument('--archive',required=True,type=Path);a=p.parse_args();v=json.loads((ROOT/'native/ai-sapiens/dependencies.json').read_text());raw=a.archive.read_bytes();assert hashlib.sha256(raw).hexdigest()==v['archive_sha256'];f={}
with tarfile.open(fileobj=io.BytesIO(raw),mode='r:gz') as t:
 for m in t.getmembers():
  q=Path(m.name).parts
  if len(q)<2 or m.isdir():continue
  f[Path(*q[1:]).as_posix()]=hashlib.sha256(t.extractfile(m).read()).hexdigest()
tree=hashlib.sha256(b'RX-AI-SAPIENS-CONTENT-TREE-v1\0'+json.dumps(f,sort_keys=True,separators=(',',':')).encode()).hexdigest();assert tree==v['content_tree_sha256'] and len(f)==v['file_count']==300
for name,d in v['policy_assets'].items():assert f['ai_sapiens_sim2real/assets/k1/'+name+'/exported/policy.onnx']==d
print(json.dumps({'schema':'rx.ai-sapiens-source-pin.v1','version':v['version'],'commit':v['commit'],'verified_files':len(f),'policies':len(v['policy_assets']),'onnx_runtime':v['onnx_runtime'],'physical_qualification':'NOT_PERFORMED'},indent=2))
