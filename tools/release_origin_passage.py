#!/usr/bin/env python3
"""Real Linux release refusals and durable checkpoints. No signing authority is created.

Supply public, pre-signed v2/revoked/v3 metadata for the exact input inventory.
The --image is a signed derivative; --unsigned-image is its original parent.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import uuid

ROOT = Path(__file__).resolve().parents[1]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--image', required=True)
    p.add_argument('--unsigned-image', required=True)
    p.add_argument('--metadata', required=True, type=Path)
    p.add_argument('--builder', default='rust:1.98.1-slim-bookworm')
    p.add_argument('--evidence', required=True, type=Path)
    p.add_argument('--daemon', type=Path, help='Already-built current Linux binary; its hash is recorded')
    a = p.parse_args()
    e = a.evidence.resolve()
    e.mkdir(parents=True, exist_ok=False)
    commands = []
    def run(label, argv, expect=0):
        r = subprocess.run(argv, capture_output=True, text=True)
        (e / (label + '.stdout')).write_text(r.stdout)
        (e / (label + '.stderr')).write_text(r.stderr)
        commands.append(dict(label=label, argv=argv, exit_code=r.returncode, expected=expect))
        (e / 'commands.json').write_text(json.dumps(commands, indent=2) + '\n')
        assert r.returncode == expect, (label, r.returncode, r.stdout, r.stderr)
        return r.stdout, r.stderr
    image = json.loads(run('image', ['docker', 'image', 'inspect', a.image])[0])[0]['Id']
    original = json.loads(run('unsigned-image', ['docker', 'image', 'inspect', a.unsigned_image])[0])[0]['Id']
    if a.daemon:
        shutil.copyfile(a.daemon, e / 'rx-solutionsd')
        os.chmod(e / 'rx-solutionsd', 0o755)
    else:
        builder = json.loads(run('builder', ['docker', 'image', 'inspect', a.builder])[0])[0]['Id']
        run('build', ['docker', 'run', '--rm', '-v', f'{ROOT}:/source:ro', '-v', f'{e}:/evidence',
            '-w', '/source', '-e', 'CARGO_HOME=/evidence/cargo-home', '-e', 'CARGO_BUILD_JOBS=1', '--entrypoint', 'cargo', builder,
            'build', '--locked', '-p', 'rx-supervisor', '--bin', 'rx-solutionsd', '--target-dir', '/evidence/target'])
        shutil.copyfile(e / 'target/debug/rx-solutionsd', e / 'rx-solutionsd')
        os.chmod(e / 'rx-solutionsd', 0o755)
    daemon = '/evidence/rx-solutionsd'
    shutil.copytree(a.metadata, e / 'metadata')
    config = {'schema': 'rx.solutions-startup.v1', 'state_subdirectory': 'first', 'plan': {
        'schema': 'rx.solutions-process-plan.v1', 'id': str(uuid.uuid4()), 'environment': 'SIMULATION',
        'profiles': ['SIM-JTC-6DOF'], 'processes': [{'id': 'status', 'program': 'rx/status-http',
        'parameters': {'bind': '127.0.0.1', 'port': '8081'}, 'depends_on': [], 'startup_timeout_ms': '5000',
        'shutdown_timeout_ms': '1000', 'restart_limit': '0', 'restart_backoff_ms': '100'}]}}
    (e / 'config.json').write_text(json.dumps(config))
    # The state is preserved across separate containers. No deletion/reset to pass a refusal.
    state = e / 'state'; state.mkdir(); state.chmod(0o777)
    base = ['docker', 'run', '--rm', '--network', 'none', '--read-only', '--cap-drop', 'ALL',
            '--security-opt', 'no-new-privileges', '--tmpfs', '/tmp:rw', '-v', f'{e}:/evidence:ro',
            '-v', f'{state}:/var/lib/rx-solutions']
    def meta(label):
        return sum((['-v', f'{e}/metadata/{label}/{name}:/opt/rx/manifests/{name}:ro'] for name in ['release.json', 'revocations.json']), [])
    refusals = {}
    def refuse(label, options, reason, selected=image, verb='run'):
        _, err = run(label, base + options + ['--entrypoint', daemon, selected, verb, '/evidence/config.json'], expect=1)
        assert reason in err, (label, err)
        refusals[label] = reason
    refuse('unsigned-original', [], 'release/unsigned', selected=original)
    original_release = json.loads(run('extract-release', ['docker', 'run', '--rm', '--entrypoint', '/bin/cat', image, '/opt/rx/manifests/release.json'])[0])
    bad = json.loads(json.dumps(original_release)); bad['signature']['signature'] = '00' * 64
    (e / 'bad-signature.json').write_text(json.dumps(bad))
    refuse('invalid-signature', ['-v', f'{e}/bad-signature.json:/opt/rx/manifests/release.json:ro'], 'release/invalid-signature')
    bad = json.loads(json.dumps(original_release)); bad['signature']['key'] = 'attacker/key'
    (e / 'unknown-key.json').write_text(json.dumps(bad))
    refuse('unknown-key', ['-v', f'{e}/unknown-key.json:/opt/rx/manifests/release.json:ro'], 'release/unknown-key')
    # Real attacker signature with the same public fixture seed used by the pre-change policy probe.
    # This private key is intentionally public test material; it is never a trusted root.
    (e / 'attacker.der').write_bytes(bytes.fromhex('302e020100300506032b657004220420') + bytes([42]) * 32)
    key_id = 'test-key'
    message = b'RX-RELEASE-v1\0' + key_id.encode() + b'\0' + json.dumps(original_release['manifest'], sort_keys=True, separators=(',', ':')).encode()
    (e / 'attacker.message').write_bytes(message)
    sig = subprocess.check_output(['openssl', 'pkeyutl', '-sign', '-rawin', '-keyform', 'DER', '-inkey', str(e / 'attacker.der'), '-in', str(e / 'attacker.message')])
    public = subprocess.check_output(['openssl', 'pkey', '-inform', 'DER', '-in', str(e / 'attacker.der'), '-pubout', '-outform', 'DER'])[-32:].hex()
    bad['signature'] = {'key': key_id, 'signature': sig.hex()}
    (e / 'attacker-release.json').write_text(json.dumps(bad))
    policy = {'schema': 'rx.package-verification-policy.v1', 'keys': [{'id': key_id, 'verifying_key': public}]}
    (e / 'attacker-policy.json').write_text(json.dumps(policy))
    (e / 'attacker-public.key').write_text(public)
    attacks = ['-v', f'{e}/attacker-release.json:/opt/rx/manifests/release.json:ro',
               '-v', f'{e}/attacker-policy.json:/opt/rx/manifests/policy.json:ro',
               '-v', f'{e}/attacker-public.key:/opt/rx/manifests/release-root.key:ro',
               '-e', 'RX_RELEASE_KEY=/opt/rx/manifests/release-root.key', '-e', 'RX_RELEASE_POLICY=/opt/rx/manifests/policy.json']
    refuse('disk-and-env-key-injection', attacks, 'release/unknown-key')
    # Changing only the compiled key ID in the forged envelope does not make the signature valid.
    bad['signature']['key'] = original_release['signature']['key']
    (e / 'root-id-impersonation.json').write_text(json.dumps(bad))
    refuse('root-id-impersonation', ['-v', f'{e}/root-id-impersonation.json:/opt/rx/manifests/release.json:ro'], 'release/invalid-signature')
    _, err = run('cli-key-injection', base + ['--entrypoint', daemon, image, 'inspect', '/evidence/config.json', '--release-key', '/evidence/attacker-public.key'], expect=1)
    assert 'usage:' in err
    (e / 'altered-script').write_text('# inert unauthenticated replacement\n')
    refuse('content-mismatch', ['-v', f'{e}/altered-script:/opt/rx/tools/solutions_status.py:ro'], 'release/content-mismatch')
    inventory = json.loads(run('extract-inventory', ['docker', 'run', '--rm', '--entrypoint', '/bin/cat', image, '/opt/rx/manifests/runtime-files.json'])[0])
    inventory['files']['tools/solutions_status.py'] = hashlib.sha256((e / 'altered-script').read_bytes()).hexdigest()
    (e / 'forged-inventory.json').write_text(json.dumps(inventory))
    refuse('forged-inventory', ['-v', f'{e}/altered-script:/opt/rx/tools/solutions_status.py:ro', '-v', f'{e}/forged-inventory.json:/opt/rx/manifests/runtime-files.json:ro'], 'release/content-mismatch')
    # A Host binary is not one of F12's two compiled source pins. Its digest
    # and a forged matching inventory must still be refused by G2 itself.
    (e / 'altered-host').write_text('inert replacement of indexed Host bytes\n')
    refuse('uncompiled-host-content', ['-v', f'{e}/altered-host:/opt/rx/bin/rx-hostd:ro'], 'release/content-mismatch')
    inventory = json.loads((e / 'extract-inventory.stdout').read_text())
    inventory['files']['bin/rx-hostd'] = hashlib.sha256((e / 'altered-host').read_bytes()).hexdigest()
    (e / 'forged-host-inventory.json').write_text(json.dumps(inventory))
    refuse('uncompiled-host-plus-inventory', ['-v', f'{e}/altered-host:/opt/rx/bin/rx-hostd:ro', '-v', f'{e}/forged-host-inventory.json:/opt/rx/manifests/runtime-files.json:ro'], 'release/content-mismatch')
    # External observer records PROCESS_READY and explicitly stops the real RX child manager.
    observer = '''import json,subprocess,time,sys
from pathlib import Path
config=json.loads(Path('/evidence/config.json').read_text());config['state_subdirectory']=sys.argv[2]
Path('/tmp/config.json').write_text(json.dumps(config));log=Path('/tmp/manager.log')
with log.open('w') as out:
 manager=subprocess.Popen([sys.argv[1],'run','/tmp/config.json'],stdout=out,stderr=subprocess.STDOUT)
 try:
  deadline=time.monotonic()+30;ready=False
  while time.monotonic()<deadline:
   assert manager.poll() is None,log.read_text()
   for line in log.read_text().splitlines():
    try:value=json.loads(line)
    except json.JSONDecodeError:continue
    records=value.get('state',{}).get('records',{})
    if records and all(r['phase']=='PROCESS_READY' for r in records.values()):ready=True
   if ready:break
   time.sleep(.05)
  assert ready,log.read_text()
 finally:
  if manager.poll() is None:manager.terminate()
  manager.wait(timeout=15)
 assert manager.returncode==0,log.read_text()
 print(json.dumps({'result':'ACCEPTED_AND_PROCESS_READY','state_subdirectory':sys.argv[2],'manager_exit':manager.returncode,'observed_by':'outside-manager-process'}))
 print(log.read_text())
'''
    for label, options in [('v1', []), ('v2', meta('v2'))]:
        run('accept-' + label, base + options + ['--entrypoint', '/usr/bin/python3', image, '-c', observer, daemon, label])
    # Different plan state directory cannot reset the global floor.
    refuse('restart-rollback', [], 'release/rollback')
    refuse('revoked-release', meta('revoked'), 'release/revoked')
    refuse('revocation-replay-after-restart', meta('v2'), 'release/rollback')
    run('accept-v3-after-revocation', base + meta('v3') + ['--entrypoint', '/usr/bin/python3', image, '-c', observer, daemon, 'v3'])
    result = {'result': 'DEVELOPMENT_RELEASE_AUTHENTICATION_PASS', 'image': image, 'unsigned_image': original,
              'daemon_sha256': hashlib.sha256((e / 'rx-solutionsd').read_bytes()).hexdigest(), 'refusals': refusals,
              'positive_versions': [1, 2, 3], 'root': 'COMPILED_SOURCE_LITERAL',
              'whole_state_rollback_or_deletion': 'NOT_DETECTED', 'product_custody': 'NOT_ESTABLISHED',
              'offline_revocation_freshness': 'NOT_ESTABLISHED', 'physical_qualification': 'NOT_PERFORMED'}
    (e / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
