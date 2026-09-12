#!/usr/bin/env python3
"""Inspect package/ELF/asset availability; never instantiate a hardware plugin or ROS node."""
import argparse,hashlib,json,subprocess,re,xml.etree.ElementTree as ET
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--prefix',type=Path,default=Path('/opt/rx/robotis'));p.add_argument('--manifest',type=Path,default=Path('/opt/rx/manifests/native-sources.json'));p.add_argument('--model-checker',type=Path,default=Path('/opt/rx/tools/ort-inspect'));p.add_argument('--output',type=Path,required=True);p.add_argument('--catalog',type=Path,required=True);a=p.parse_args()
m=json.loads(a.manifest.read_text());packages=sorted(m['packages']);index=a.prefix/'share/ament_index/resource_index/packages'
missing=[name for name in packages if not (index/name).is_file()]
if missing:raise SystemExit('native packages not installed: '+','.join(missing))
libraries=[]
for path in sorted(a.prefix.rglob('*')):
    if not path.is_file() or path.is_symlink():continue
    with path.open('rb') as f:magic=f.read(4)
    if magic!=b'\x7fELF':continue
    r=subprocess.run(['ldd',str(path)],capture_output=True,text=True)
    if 'not found' in r.stdout+r.stderr:raise SystemExit('unresolved native dependency: '+str(path)+'\n'+r.stdout+r.stderr)
    libraries.append({'path':str(path.relative_to(a.prefix)),'sha256':hashlib.sha256(path.read_bytes()).hexdigest()})
assets=[]
for asset in m['policy_assets']:
    path=a.prefix/'share'/asset['path'];data=path.read_bytes()
    if len(data)!=asset['bytes'] or hashlib.sha256(data).hexdigest()!=asset['sha256']:raise SystemExit('installed policy asset differs: '+str(path))
    item=dict(asset)
    if path.suffix=='.onnx':
        result=subprocess.run([str(a.model_checker),str(path)],check=True,capture_output=True,text=True);item['cpu_session_load']=json.loads(result.stdout)
    assets.append(item)
plugins={}
for prefix in [Path('/opt/ros/jazzy'),a.prefix]:
    root=prefix/'share/ament_index/resource_index'
    for group in [root/'controller_interface__pluginlib__plugin',root/'hardware_interface__pluginlib__plugin']:
        if not group.is_dir():continue
        for marker in group.iterdir():
            for relative in re.split(r'[;\n]',marker.read_text()):
                if not relative.strip():continue
                xml=prefix/relative.strip()
                if not xml.is_file():raise SystemExit('plugin manifest missing: '+str(xml))
                try: parsed=ET.parse(xml).getroot()
                except ET.ParseError as error: raise SystemExit('invalid plugin XML '+str(xml)+': '+str(error))
                for cls in parsed.iter('class'):
                    plugins[cls.get('name',cls.get('type'))]={'type':cls.get('type'),'base':cls.get('base_class_type'),'xml':str(xml)}
catalog=json.loads(a.catalog.read_text())
commits={r['name']:r['commit'] for r in m['sources']}
for repo in catalog['repositories']:
    if commits.get(repo['repository'])!=repo['commit']:raise SystemExit('support catalog commit differs from build source')
for profile in catalog['profiles']:
    for source in profile['sources']:
        if commits.get(source['repository'])!=source['commit']:raise SystemExit('profile source commit mismatch')
        path=Path('/workspace/src')/source['repository']/source['path']
        if hashlib.sha256(path.read_bytes()).hexdigest()!=source['sha256']:raise SystemExit('support catalog source bytes differ')
required={c['plugin'] for p in catalog['profiles'] for c in p['controllers']}
required.add('dynamixel_hardware_interface/DynamixelHardware')
missing=sorted(required-plugins.keys())
if missing:raise SystemExit('required catalog plugin unavailable: '+','.join(missing))
result={'required_plugins':{p:plugins[p] for p in sorted(required)},'schema':'rx.native-install-audit.v1' ,'status':'PASS','packages':packages,'elf':libraries,'policy_assets':assets,'source_tree_sha256':m['source_tree_sha256'],'hardware_plugins_instantiated':False,'ros_nodes_started':False,'physical_qualification':'NOT_PERFORMED'}
a.output.parent.mkdir(parents=True,exist_ok=True);a.output.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'packages':len(packages),'elf':len(libraries),'policy_assets':len(assets),'model_sessions':sum('cpu_session_load'in x for x in assets)}))
