#!/usr/bin/env python3
import hashlib,json
from pathlib import Path
root=Path('/opt/rx');files={name:hashlib.sha256((root/name).read_bytes()).hexdigest() for name in ['LICENSE','NOTICE']}
for folder in ['bin','catalogs','operator','tools','manifests','licenses','dhi']:
    for file in sorted((root/folder).rglob('*')):
        if file.is_file() and file.name != 'runtime-files.json':files[file.relative_to(root).as_posix()]=hashlib.sha256(file.read_bytes()).hexdigest()
(root/'manifests/runtime-files.json').write_text(json.dumps({'schema':'rx.solutions-runtime-files.v1','files':files,'external_files':{p:hashlib.sha256(Path(p).read_bytes()).hexdigest() for p in ['/usr/bin/python3']}},indent=2)+'\n')
