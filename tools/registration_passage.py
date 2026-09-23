#!/usr/bin/env python3
"""Exercise registration, recovery, dependencies and verified external test decisions.

Requires Docker and a previously validated RX runtime image. No image is published,
no device is attached, and all new build/state artifacts stay in --evidence.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image', required=True, help='validated RX runtime image or immutable ID')
    parser.add_argument('--builder', default='rust:1.98.1-slim-bookworm')
    parser.add_argument('--evidence', type=Path, required=True, help='fresh directory; never reuses state')
    args = parser.parse_args()
    evidence = args.evidence.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    commands = []

    def run(label, argv):
        print('$ ' + shlex.join(argv), flush=True)
        result = subprocess.run(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        (evidence / (label + '.stdout')).write_text(result.stdout)
        (evidence / (label + '.stderr')).write_text(result.stderr)
        commands.append({'label': label, 'argv': argv, 'exit_code': result.returncode})
        (evidence / 'commands.json').write_text(json.dumps(commands, indent=2) + '\n')
        if result.returncode:
            sys.stdout.write(result.stdout)
            sys.stderr.write(result.stderr)
            raise RuntimeError(f'{label} failed: {result.returncode}; evidence retained')
        return result.stdout

    runtime = json.loads(run('runtime-image', ['docker', 'image', 'inspect', args.image]))[0]
    builder = json.loads(run('builder-image', ['docker', 'image', 'inspect', args.builder]))[0]
    runtime_id, builder_id = runtime['Id'], builder['Id']
    run('release-inspection', ['docker', 'run', '--rm', '--network', 'none', '--read-only',
        '--cap-drop', 'ALL', '--entrypoint', '/usr/bin/python3', runtime_id,
        '/opt/rx/tools/solutions_status.py', 'inspect'])
    installed_hash = run('release-source-hash', ['docker', 'run', '--rm', '--network', 'none',
        '--read-only', '--entrypoint', '/usr/bin/python3', runtime_id, '-c',
        'import hashlib; from pathlib import Path; print(hashlib.sha256(Path("/opt/rx/tools/solutions_status.py").read_bytes()).hexdigest())']).strip()
    source_hash = hashlib.sha256((ROOT / 'native/support/solutions_status.py').read_bytes()).hexdigest()
    if installed_hash != source_hash:
        raise RuntimeError('runtime status service differs from current source; build/review a matching image')
    output = run('build', ['docker', 'run', '--rm', '-v', f'{ROOT}:/source:ro',
        '-v', f'{evidence}:/evidence', '-w', '/source', '-e', 'CARGO_HOME=/evidence/cargo-home',
        '--entrypoint', 'cargo', builder_id, 'test', '-p', 'rx-supervisor', '--locked',
        '--test', 'registration_passage', '--no-run', '--target-dir', '/evidence/target',
        '--message-format=json'])
    artifacts = [json.loads(line) for line in output.splitlines() if line.startswith('{')]
    executables = [a['executable'] for a in artifacts if a.get('reason') == 'compiler-artifact'
        and a.get('target', {}).get('name') == 'registration_passage' and a.get('executable')]
    if len(executables) != 1:
        raise RuntimeError('expected one compiled passage test executable')
    state = evidence / 'state'
    state.mkdir()
    result = run('passage', ['docker', 'run', '--rm', '--network', 'none', '--read-only',
        '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges', '--tmpfs', '/tmp:rw',
        '--user', f'{os.getuid()}:{os.getgid()}', '-v', f'{evidence}:/evidence',
        '-e', 'RX_PASSAGE_DATA=/evidence/state', '--entrypoint', executables[0], runtime_id,
        '--ignored', '--exact', 'real_registration_passage', '--nocapture'])
    sys.stdout.write(result)
    print(json.dumps({'result': 'PASS', 'runtime_image': runtime_id, 'builder_image': builder_id,
        'status_source_sha256': source_hash, 'evidence': str(evidence),
        'scope': 'actual Linux registration/recovery, scoped self-report comparison and diagnostic dependency consumption; externally signed test decisions only, actual operating-area service integration and default production anchors absent; no resource enforcement or physical qualification'}))


if __name__ == '__main__':
    main()
