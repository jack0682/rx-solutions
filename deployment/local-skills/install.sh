#!/bin/sh
# Pinned developer-preview release. Requires Python 3.11+ and a running Linux Docker engine.
set -eu
command -v python3 >/dev/null || { echo 'Python 3.11+ is required.' >&2; exit 1; }
command -v docker >/dev/null || { echo 'Docker Engine or Docker Desktop is required.' >&2; exit 1; }
python3 - "$@" <<'PY'
import argparse, hashlib, json, os, shutil, subprocess, sys, tarfile, tempfile, urllib.request
from pathlib import Path
if sys.version_info < (3,11): raise SystemExit('Python 3.11+ is required')
p=argparse.ArgumentParser(description='Install RX local simulation skills')
p.add_argument('--profile',choices=['local-sim'],default='local-sim')
p.add_argument('--port',type=int,default=8766)
p.add_argument('--bundle',type=Path,help='Use an already downloaded, extracted bundle')
p.add_argument('--prefix',type=Path,default=Path.home()/'.local')
a=p.parse_args()
version='0.3.0-rc.1'
root=a.prefix.resolve()/'lib/rx-skills'/version
if a.bundle:
    root=a.bundle.resolve()
else:
    info=json.loads(subprocess.check_output(['docker','info','--format','{{json .}}']))
    if info['OSType']!='linux':raise SystemExit('Linux Docker engine required')
    arch={'aarch64':'arm64','x86_64':'amd64'}.get(info['Architecture'],info['Architecture'])
    if arch not in ('arm64','amd64'):raise SystemExit('Unsupported Docker architecture')
    root=root/arch
    name=f'rx-local-skills-{version}-linux-{arch}.tar.gz'
    base=f'https://github.com/jack0682/rx-solutions/releases/download/v{version}/'
    if not root.exists():
        root.parent.mkdir(parents=True,exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='.rx-install-',dir=root.parent) as temp:
            temp=Path(temp)
            with urllib.request.urlopen(base+'CHECKSUMS.sha256',timeout=30) as r:checks=r.read(16384).decode()
            expected={line.split()[1]:line.split()[0] for line in checks.splitlines()}[name]
            archive=temp/name
            with urllib.request.urlopen(base+name,timeout=60) as r,archive.open('wb') as f:shutil.copyfileobj(r,f)
            with archive.open('rb') as f:actual=hashlib.file_digest(f,'sha256').hexdigest()
            if actual!=expected:raise SystemExit('Release bundle checksum mismatch')
            extracted=temp/'extracted';extracted.mkdir()
            with tarfile.open(archive) as t:
                for m in t.getmembers():
                    path=Path(m.name)
                    if not (m.isfile() or m.isdir()) or path.is_absolute() or '..' in path.parts or not path.parts or path.parts[0]!='rx-local-skills':raise SystemExit('Invalid archive member')
                t.extractall(extracted,filter='data')
            os.rename(extracted/'rx-local-skills',root)
subprocess.run([sys.executable,str(root/'rx'),'install','--profile',a.profile,'--port',str(a.port)],check=True)
bin_dir=a.prefix.resolve()/'bin';bin_dir.mkdir(parents=True,exist_ok=True)
link=bin_dir/'rx'
if link.exists() or link.is_symlink():
    if not link.is_symlink() or (link.resolve()!=root/'rx' and not link.resolve().is_relative_to(a.prefix.resolve()/'lib/rx-skills')):raise SystemExit('RX is installed, but an unrelated rx command was preserved')
    link.unlink()
link.symlink_to(root/'rx')
print('CLI: '+str(link))
if str(bin_dir) not in os.environ.get('PATH','').split(os.pathsep):print('Add this directory to PATH for the short rx command: '+str(bin_dir))
PY
