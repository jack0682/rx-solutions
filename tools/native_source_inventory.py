#!/usr/bin/env python3
"""Git-tree materialization digest, independent of mtimes and generated root inventory files."""
import hashlib,json,os,stat
from pathlib import Path
EXCLUDED={'source-lock.json','package-manifests.json'}
def tree_digest(root):
    rows=[]
    for path in sorted(Path(root).rglob('*')):
        rel=path.relative_to(root).as_posix()
        if rel in EXCLUDED:continue
        mode=path.lstat().st_mode
        if stat.S_ISLNK(mode):rows.append([rel,'link',os.readlink(path)])
        elif stat.S_ISREG(mode):rows.append([rel,'file',bool(mode&0o111),hashlib.sha256(path.read_bytes()).hexdigest()])
        elif not stat.S_ISDIR(mode):raise ValueError('unsupported source entry: '+rel)
    return hashlib.sha256(json.dumps(rows,separators=(',',':'),ensure_ascii=False).encode()).hexdigest()
if __name__=='__main__':
    import argparse
    p=argparse.ArgumentParser();p.add_argument('root',type=Path);a=p.parse_args()
    lock=json.loads((a.root/'source-lock.json').read_text())
    if lock.get('schema')!='rx.native-source-inventory.v1' or lock.get('source_tree_sha256')!=tree_digest(a.root):raise SystemExit('native source tree differs from prepared inventory')
    print('Native source tree verified: '+lock['source_tree_sha256'])
