#!/usr/bin/env python3
"""Check catalogue claims against immutable Git blobs, without launching any ROS code."""
from pathlib import Path,PurePosixPath
import argparse,hashlib,json,re,subprocess

parser=argparse.ArgumentParser()
parser.add_argument('--source-root',type=Path,required=True)
parser.add_argument('--output',type=Path)
args=parser.parse_args()
root=Path(__file__).resolve().parents[1]
catalog=json.loads((root/'catalogs/robotis-support.v1.json').read_text())
required={'DynamixelSDK','dynamixel_hardware_interface','open_manipulator','ai_worker','ai_sapiens'}
pins={row['repository']:row for row in catalog['repositories']}
assert set(pins)==required and len(catalog['repositories'])==5
for repository,pin in pins.items():
    assert re.fullmatch('[0-9a-f]{40}',pin['commit'])
    assert pin['url']==f'https://github.com/ROBOTIS-GIT/{repository}.git'
    actual=subprocess.check_output(['git','-C',str(args.source_root/repository),'rev-parse',f"{pin['commit']}^{{commit}}"],text=True).strip()
    assert actual==pin['commit']
files={}
for profile in catalog['profiles']:
    for source in profile['sources']:
        repository=source['repository'];assert repository in pins and source['commit']==pins[repository]['commit']
        path=PurePosixPath(source['path'])
        assert not path.is_absolute() and '..' not in path.parts and '\\' not in source['path'] and ':' not in source['path']
        key=(repository,source['commit'],source['path'])
        if key not in files:
            blob=subprocess.check_output(['git','-C',str(args.source_root/repository),'cat-file','blob',f"{source['commit']}:{source['path']}"])
            files[key]=hashlib.sha256(blob).hexdigest()
        assert files[key]==source['sha256'],f"catalog source mismatch: {repository}/{source['path']}"
result={'schema':'rx.catalog-source-check.v1','status':'PASS','mandatory_repositories':len(pins),'profiles':len(catalog['profiles']),'unique_source_files':len(files),
    'catalog_sha256':hashlib.sha256((root/'catalogs/robotis-support.v1.json').read_bytes()).hexdigest(),
    'scope':'Pinned Git commit and source file identity only; not build, control behavior, image or hardware qualification.'}
if args.output:args.output.write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
print(json.dumps(result,ensure_ascii=False))
