#!/usr/bin/env python3
"""Verify the neutral image's installed RX binaries and dynamic links without device access."""
import argparse
import hashlib
import json
import subprocess
from pathlib import Path

REQUIRED = ('rx-hostd', 'rx-executor-service', 'rx-process-compile', 'rx-solutionsd',
            'rx-process-package', 'rx-device-package', 'rx-bt-engine', 'rx-ros-jtc-bridge', 'rx-dynamixel-ping')

def audit(prefix, catalog_path):
    catalog = json.loads(catalog_path.read_text())
    if catalog['schema'] != 'rx.device-support.v1':
        raise ValueError('unsupported device catalog')
    libraries = []
    for name in REQUIRED:
        path = prefix / 'bin' / name
        if path.read_bytes()[:4] != b'\x7fELF':
            raise ValueError(f'missing ELF executable: {name}')
        result = subprocess.run(['ldd', str(path)], capture_output=True, text=True)
        if 'not found' in result.stdout + result.stderr or result.returncode:
            raise ValueError(f'unresolved executable dependency: {name}: {result.stdout}{result.stderr}')
        libraries.append({'path': f'bin/{name}', 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()})
    packages = (prefix / 'manifests/native-packages.tsv').read_text().splitlines()
    return {'schema': 'rx.native-install-audit.v1', 'status': 'PASS', 'elf': libraries,
            'native_packages': len(packages), 'catalog_sha256': hashlib.sha256(catalog_path.read_bytes()).hexdigest(),
            'hardware_plugins_instantiated': False, 'ros_nodes_started': False,
            'physical_qualification': 'NOT_PERFORMED'}

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--prefix', type=Path, default=Path('/opt/rx'))
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--catalog', type=Path, required=True)
    args = parser.parse_args()
    report = audit(args.prefix, args.catalog)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'status': 'PASS', 'executables': len(report['elf'])}))
