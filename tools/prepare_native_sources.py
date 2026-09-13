#!/usr/bin/env python3
"""Materialize locked Git trees for image build without editing or executing original sources."""
import argparse,hashlib,io,json,os,shutil,subprocess,tarfile,tempfile,xml.etree.ElementTree as ET
from pathlib import Path
from native_source_inventory import tree_digest
parser=argparse.ArgumentParser();parser.add_argument('--local-root',type=Path);parser.add_argument('--output',type=Path,default=Path('.cache/native'));a=parser.parse_args()
root=Path(__file__).resolve().parents[1];lock=json.loads((root/'dependencies/native-stack.lock.json').read_text());output=a.output.absolute()
repositories=lock['repositories']
if len({r['name'] for r in repositories})!=len(repositories):raise SystemExit('duplicate repository name')
import re
for r in repositories:
    if not re.fullmatch(r'[A-Za-z0-9_-]+',r['name']) or not re.fullmatch(r'[0-9a-f]{40}',r['commit']):raise SystemExit('repository must have a full immutable commit')
    if not r['url'].startswith('https://') or len(r['url']) <= 8 or any(c.isspace() for c in r['url']):raise SystemExit('repository requires an HTTPS source URL')
for asset in lock['required_external_assets'] + lock['policy_assets']:
    parts=Path(asset['path']).parts
    if asset['repository'] not in {r['name'] for r in repositories} or not parts or Path(asset['path']).is_absolute() or '..' in parts or '\\' in asset['path']:
        raise SystemExit('asset must be relative to a locked repository')
cache=root/'.cache/git';cache.mkdir(parents=True,exist_ok=True);output.parent.mkdir(parents=True,exist_ok=True)
if output.exists():
    recorded=json.loads((output/'source-lock.json').read_text())
    if recorded.get('schema')!='rx.native-source-inventory.v1':raise SystemExit('output is not owned source materialization')
    if recorded.get('source_tree_sha256') is not None and recorded['source_tree_sha256']!=tree_digest(output):raise SystemExit('existing prepared source was changed; preserve/review it before replacement')
def git(path,*args):return subprocess.check_output(['git','-C',str(path),*args])
with tempfile.TemporaryDirectory(prefix='.native-',dir=output.parent) as temporary:
    stage=Path(temporary)/'source';stage.mkdir();sources=[]
    for r in lock['repositories']:
        local=a.local_root/r['name'] if a.local_root else None
        if local and (local/'.git').exists(): repository=local
        else:
            repository=cache/r['name']
            if not (repository/'.git').exists():
                subprocess.run(['git','init','-q',str(repository)],check=True)
                subprocess.run(['git','-C',str(repository),'remote','add','origin',r['url']],check=True)
            if subprocess.run(['git','-C',str(repository),'cat-file','-e',r['commit']+'^{commit}'],capture_output=True).returncode:
                subprocess.run(['git','-C',str(repository),'fetch','--quiet','--depth=1','origin',r['commit']],check=True)
        commit=git(repository,'rev-parse',r['commit']+'^{commit}').decode().strip();assert commit==r['commit']
        entries=git(repository,'ls-tree','-r',commit).decode().splitlines()
        if any(line.startswith('160000 ') for line in entries):raise SystemExit('unlocked submodule in '+r['name'])
        archive=git(repository,'archive','--format=tar',commit);dest=stage/r['name'];dest.mkdir()
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:tar.extractall(dest,filter='data')
        for path in dest.rglob('*'):
            if path.is_file() and path.stat().st_size<1024 and path.read_bytes().startswith(b'version https://git-lfs.github.com/spec/v1'):raise SystemExit('LFS asset unresolved: '+str(path))
        sources.append({**r,'tree':git(repository,'rev-parse',commit+'^{tree}').decode().strip(),'archive_sha256':hashlib.sha256(archive).hexdigest()})
    packages={};external=set()
    for xml in stage.rglob('package.xml'):
        e=ET.parse(xml).getroot();name=e.findtext('name');deps=sorted({x.text for x in e if x.tag in ('depend','build_depend','buildtool_depend','build_export_depend','exec_depend')})
        if name in packages:raise SystemExit('duplicate ROS package: '+name)
        packages[name]={'path':str(xml.parent.relative_to(stage)),'dependencies':deps};external.update(deps)
    report={'schema':'rx.native-source-inventory.v1','sources':sources,'packages':packages,'external_dependencies':sorted(external-packages.keys()),'missing_assets':[p for p in lock['required_external_assets'] if not (stage/p['repository']/p['path']).exists()]}
    if report['missing_assets']:raise SystemExit('required external assets are missing')
    report['source_tree_sha256']=tree_digest(stage)
    report['policy_assets']=lock['policy_assets']
    for asset in lock['policy_assets']:
        data=(stage/asset['repository']/asset['path']).read_bytes()
        if len(data)!=asset['bytes'] or hashlib.sha256(data).hexdigest()!=asset['sha256']:raise SystemExit('policy asset differs from pin')
    manifests={name:(stage/p['path']/'package.xml').read_text() for name,p in packages.items()}
    (stage/'package-manifests.json').write_text(json.dumps(manifests,sort_keys=True,indent=2)+'\n')
    (stage/'source-lock.json').write_text(json.dumps(report,indent=2)+'\n')
    if output.exists():
        old=output.with_name(output.name+'.previous')
        if old.exists():raise SystemExit('previous staged directory already exists')
        output.rename(old);stage.rename(output);shutil.rmtree(old)
    else:stage.rename(output)
print(json.dumps({'sources':len(sources),'packages':len(packages),'external_dependencies':len(report['external_dependencies']),'missing_assets':report['missing_assets']}))
