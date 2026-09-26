#!/usr/bin/env python3
"""Measure F12 support refusals in the actual runtime; test fixtures do not confer authority."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import uuid

ROOT = Path(__file__).resolve().parents[1]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--image', required=True)
    p.add_argument('--builder', default='rust:1.98.1-slim-bookworm')
    p.add_argument('--evidence', type=Path, required=True)
    p.add_argument('--host-rounds', type=int, default=50)
    a = p.parse_args()
    if not 1 <= a.host_rounds <= 100:
        p.error('host-rounds must be 1..100')
    e = a.evidence.resolve()
    e.mkdir(parents=True, exist_ok=False)
    commands = []

    def run(label, argv, expect=0):
        r = subprocess.run(argv, capture_output=True, text=True)
        (e / (label + '.stdout')).write_text(r.stdout)
        (e / (label + '.stderr')).write_text(r.stderr)
        commands.append(dict(label=label, argv=argv, exit_code=r.returncode, expected=expect))
        (e / 'commands.json').write_text(json.dumps(commands, indent=2) + '\n')
        if r.returncode != expect:
            raise RuntimeError(f'{label}: {r.returncode}, expected {expect}; evidence retained')
        return r.stdout

    image = json.loads(run('image', ['docker', 'image', 'inspect', a.image]))[0]['Id']
    builder = json.loads(run('builder', ['docker', 'image', 'inspect', a.builder]))[0]['Id']
    build = ['docker', 'run', '--rm', '-v', f'{ROOT}:/source:ro', '-v', f'{e}:/evidence',
             '-w', '/source', '-e', 'CARGO_HOME=/evidence/cargo-home', '-e', 'CARGO_BUILD_JOBS=1', '--entrypoint', 'cargo', builder]
    run('daemon-build', build + ['build', '--locked', '-p', 'rx-supervisor', '--bin', 'rx-solutionsd', '--target-dir', '/evidence/target'])
    binaries = {}
    for package, target, extra in [('rx-supervisor', 'support_limits', []), ('rx-supervisor', 'guarded_os', []), ('rx-supervisor', 'initialization', []), ('rx-host', 'service', ['--all-features'])]:
        output = run('build-' + target, build + ['test', '--locked', '-p', package, '--test', target,
                     '--no-run', '--target-dir', '/evidence/target', '--message-format=json'] + extra)
        artifacts = [json.loads(line) for line in output.splitlines() if line.startswith('{')]
        binaries[target] = next(v['executable'] for v in artifacts if v.get('executable') and v['target']['name'] == target)
    base = ['docker', 'run', '--rm', '--network', 'none', '--read-only', '--cap-drop', 'ALL',
            '--security-opt', 'no-new-privileges', '--tmpfs', '/tmp:rw', '-v', f'{e}:/evidence:ro']
    run('guarded-delivery', base + ['--entrypoint', binaries['support_limits'], image, '--nocapture'])
    run('initialization-refusals', base + ['--entrypoint', binaries['initialization'], image, '--nocapture'])
    run('guarded-force-denial', base + ['--entrypoint', binaries['guarded_os'], image, '--nocapture'])
    run('host-lock-denial', base + ['--entrypoint', binaries['service'], image, '--exact',
        'unavailable_runtime_lock_refuses_service_without_claiming_a_known_owner', '--nocapture'])
    # This is a bounded reproduction attempt, not proof that the old flake is fixed.
    stress = """import subprocess,sys,json
rows=[]
for i in range(int(sys.argv[2])):
 r=subprocess.run([sys.argv[1],'--test-threads=16'],capture_output=True,text=True)
 rows.append({'round':i,'exit_code':r.returncode})
 if r.returncode:print(r.stdout+r.stderr);break
print('host_stress='+json.dumps(rows))
"""
    output = run('host-stress', base + ['--entrypoint', '/usr/bin/python3', image, '-c', stress,
                                      binaries['service'], str(a.host_rounds)])
    rounds = json.loads(next(line.split('=', 1)[1] for line in output.splitlines() if line.startswith('host_stress=')))
    materials = {}
    for label, path in [('inventory', '/opt/rx/manifests/runtime-files.json'), ('script', '/opt/rx/tools/solutions_status.py'), ('catalog', '/opt/rx/catalogs/device-support.v1.json')]:
        materials[label] = run('extract-' + label, ['docker', 'run', '--rm', '--network', 'none', '--read-only', '--entrypoint', '/bin/cat', image, path])
        (e / (label + '-original')).write_text(materials[label])
    changed_script = materials['script'] + '\n# F12 inert source substitution\n'
    changed_catalog = materials['catalog'] + '\n'
    (e / 'changed-script').write_text(changed_script)
    (e / 'changed-catalog').write_text(changed_catalog)
    for kind, content, key in [('script', changed_script, 'tools/solutions_status.py'), ('catalog', changed_catalog, 'catalogs/device-support.v1.json')]:
        inv = json.loads(materials['inventory'])
        inv['files'][key] = hashlib.sha256(content.encode()).hexdigest()
        (e / ('inventory-' + kind)).write_text(json.dumps(inv))
    config = {'schema': 'rx.solutions-startup.v1', 'state_subdirectory': 'f12', 'plan': {
        'schema': 'rx.solutions-process-plan.v1', 'id': str(uuid.uuid4()), 'environment': 'SIMULATION',
        'profiles': ['SIM-JTC-6DOF'], 'processes': [{'id': 'status', 'program': 'rx/status-http',
        'parameters': {'bind': '127.0.0.1', 'port': '8081'}, 'depends_on': [], 'startup_timeout_ms': '5000',
        'shutdown_timeout_ms': '1000', 'restart_limit': '0', 'restart_backoff_ms': '100'}]}}
    (e / 'config.json').write_text(json.dumps(config))
    daemon = '/evidence/target/debug/rx-solutionsd'
    observed = run('daemon-descriptor-observation', base + ['--tmpfs', '/var/lib/rx-solutions:rw,uid=10001,gid=10001,mode=0700',
        '-v', f'{ROOT}/tools/support_limits_observer.py:/observer.py:ro', '--entrypoint', '/usr/bin/python3', image,
        '/observer.py', daemon, '/evidence/config.json'])
    descriptor_observation = json.loads(observed)
    lock_probe = """import fcntl,json,subprocess,sys
from pathlib import Path
base=json.loads(Path(sys.argv[2]).read_text());scenes=[]
for which in ['registration','supervisor']:
 c=dict(base);c['state_subdirectory']='locked-'+which
 path=Path('/var/lib/rx-solutions')/c['state_subdirectory'];path.mkdir()
 config=Path('/tmp')/(which+'.json');config.write_text(json.dumps(c))
 with (path/(which+'.writer.lock')).open('a+') as owner:
  fcntl.flock(owner,fcntl.LOCK_EX|fcntl.LOCK_NB)
  r=subprocess.run([sys.argv[1],'run',str(config)],capture_output=True,text=True)
  assert r.returncode==1,(r.stdout,r.stderr)
  diagnostic=next(json.loads(l) for l in r.stderr.splitlines() if l.startswith('{'))
  assert diagnostic['condition']=='storage/exclusive-writer-not-established',diagnostic
  assert diagnostic['owner_identity']=='NOT_ESTABLISHED',diagnostic
  scenes.append(dict(lock=which,exit_code=r.returncode,diagnostic=diagnostic))
print(json.dumps(scenes))
"""
    storage_refusals = json.loads(run('storage-lock-refusals', base + ['--tmpfs', '/var/lib/rx-solutions:rw,uid=10001,gid=10001,mode=0700',
        '--entrypoint', '/usr/bin/python3', image, '-c', lock_probe, daemon, '/evidence/config.json']))
    schema_probe = """import json,sqlite3,subprocess,sys
from pathlib import Path
c=json.loads(Path(sys.argv[2]).read_text());c['state_subdirectory']='future-schema'
p=Path('/var/lib/rx-solutions/future-schema');p.mkdir();db=p/'registration.db'
with sqlite3.connect(db) as connection:connection.execute('pragma user_version=999')
config=Path('/tmp/future.json');config.write_text(json.dumps(c))
r=subprocess.run([sys.argv[1],'run',str(config)],capture_output=True,text=True)
assert r.returncode==1 and 'newer store schema: downgrade refused' in r.stderr,(r.stdout,r.stderr)
with sqlite3.connect(db) as connection:assert connection.execute('pragma user_version').fetchone()[0]==999
print(json.dumps(dict(exit_code=r.returncode,reason=r.stderr,original_version=999)))
"""
    schema_refusal = json.loads(run('future-schema-refusal', base + ['--tmpfs', '/var/lib/rx-solutions:rw,uid=10001,gid=10001,mode=0700',
        '--entrypoint', '/usr/bin/python3', image, '-c', schema_probe, daemon, '/evidence/config.json']))
    normal = json.loads(run('source-normal', base + ['--entrypoint', daemon, image, 'inspect', '/evidence/config.json']))
    boundary = normal['release_boundary']
    assert boundary['trust'] == 'TRUSTED_INSTALLED_RUST_BINARIES_AND_OS'
    assert boundary['authenticated_immutable_provenance'].startswith('NOT_ESTABLISHED')
    cases = [('script-only', 'script', 'inventory-original'), ('script-plus-inventory', 'script', 'inventory-script'), ('catalog-plus-inventory', 'catalog', 'inventory-catalog')]
    for label, kind, inventory in cases:
        destination = '/opt/rx/tools/solutions_status.py' if kind == 'script' else '/opt/rx/catalogs/device-support.v1.json'
        run('reject-' + label, base + ['-v', f'{e}/changed-{kind}:{destination}:ro', '-v',
            f'{e}/{inventory}:/opt/rx/manifests/runtime-files.json:ro', '--entrypoint', daemon, image,
            'run', '/evidence/config.json'], expect=1)
        error = (e / ('reject-' + label + '.stderr')).read_text()
        assert 'source-pin-mismatch' in error or 'program file integrity differs' in error
    result = dict(result='ENFORCEMENT_PASS_LOCK_LIFETIME_UNRESOLVED', runtime_image=image, builder_image=builder,
        daemon_sha256=hashlib.sha256((e / 'target/debug/rx-solutionsd').read_bytes()).hexdigest(),
        manifest_substitution='NORMAL_ACCEPTED; SCRIPT_ONLY_AND_SCRIPT_PLUS_INVENTORY_AND_CATALOG_PLUS_INVENTORY_REFUSED',
        descriptor_observation=descriptor_observation, storage_refusals=storage_refusals, schema_refusal=schema_refusal,
        boundary=boundary, guarded='FAILED_DELIVERY_NOT_COMPLETION; FINAL_REPORT_AND_OWNED_EXIT_REQUIRED; FORCE_REFUSED',
        host_lock='SERVICE_ADMISSION_REFUSED_WITHOUT_OWNER_IDENTITY_CLAIM; EXPLICIT_RETRY_AFTER_HOLDER_EXIT',
        host_stress=rounds, historical_host_flake='NOT_PROVEN_RESOLVED; ORIGINAL_CAUSE_UNATTRIBUTED',
        evidence_scope='actual Linux processes; explicit delivery failure injection and separate real lock holder; no physical operation')
    (e / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'result': result['result'], 'evidence': str(e), 'stress_failures': sum(r['exit_code'] != 0 for r in rounds)}))


if __name__ == '__main__':
    main()
