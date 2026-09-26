#!/usr/bin/env python3
import argparse,hashlib,io,json,re,tarfile
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
def h(v):return hashlib.sha256(v).hexdigest()
p=argparse.ArgumentParser();p.add_argument('--archive',required=True,type=Path);a=p.parse_args();pins=json.loads((ROOT/'native/open-manipulator/dependencies.json').read_text());raw=a.archive.read_bytes();assert h(raw)==pins['open_manipulator']['archive_sha256'];files={};body={}
with tarfile.open(fileobj=io.BytesIO(raw),mode='r:gz') as t:
 for m in t.getmembers():
  parts=Path(m.name).parts
  if len(parts)<2 or m.isdir():continue
  data=t.extractfile(m).read();name=Path(*parts[1:]).as_posix();files[name]=h(data);body[name]=data
tree=h(b'RX-OPEN-MANIPULATOR-CONTENT-TREE-v1\0'+json.dumps(files,sort_keys=True,separators=(',',':')).encode());assert tree==pins['open_manipulator']['content_tree_sha256'];assert len(files)==310
docker=body['docker/Dockerfile'].decode();repos=body['open_manipulator_ci.repos'].decode();assert 'S6_OVERLAY_VERSION=3.2.1.0' in docker and 'cyclo_manager' in docker and 'git clone -b jazzy' in docker and 'version: main' in repos
assert all(re.fullmatch('[0-9a-f]{40}',v) for v in pins['git_dependencies'].values());assert all(re.fullmatch('[0-9a-f]{64}',v) for k,v in pins['s6_overlay'].items() if k.endswith('sha256'))
print(json.dumps({'schema':'rx.open-manipulator-source-pin.v1','version':'5.1.2','commit':pins['open_manipulator']['commit'],'archive_sha256':h(raw),'content_tree_sha256':tree,'verified_files':len(files),'pinned_git_dependencies':len(pins['git_dependencies']),'s6_overlay':pins['s6_overlay'],'upstream_floating_refs':'REJECTED_BY_RX_SOURCE_PINS','physical_qualification':'NOT_PERFORMED'},indent=2))
