#!/usr/bin/env python3
"""Observe RX address-space enforcement outside the actual resident supervisor.

Requires the validated runtime image and an explicitly supplied F7 daemon for
the old-registration/new-catalog refusal scene. No image is published. The
unchanged resident passage is run first; test fixtures remain explicitly scoped.
"""
import argparse
import hashlib
import json
import subprocess
import sys
import time
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--image', required=True)
    p.add_argument('--builder', default='rust:1.98.1-slim-bookworm')
    p.add_argument('--baseline-daemon', type=Path, required=True)
    p.add_argument('--evidence', type=Path, required=True)
    a = p.parse_args()
    baseline = a.baseline_daemon.resolve(strict=True)
    evidence = a.evidence.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    commands, names = [], []

    def run(label, argv, check=True):
        r = subprocess.run(argv, capture_output=True, text=True)
        (evidence / (label + '.stdout')).write_text(r.stdout)
        (evidence / (label + '.stderr')).write_text(r.stderr)
        commands.append({'label': label, 'argv': argv, 'exit_code': r.returncode})
        (evidence / 'commands.json').write_text(json.dumps(commands, indent=2) + '\n')
        if check and r.returncode:
            raise RuntimeError(f'{label} failed: {r.returncode}: {r.stderr}')
        return r

    run('resident-passage', [sys.executable, str(ROOT / 'tools/resident_registration_passage.py'),
        '--image', a.image, '--builder', a.builder, '--evidence', str(evidence / 'resident')])
    resident = json.loads((evidence / 'resident/result.json').read_text())
    runtime, builder = resident['runtime_image'], resident['builder_image']
    binary = evidence / 'resident/target/debug/rx-solutionsd'
    for record in resident['unclean_reopen']['state']['records'].values():
        assert record['resources']['lifetime'] == 'UNCONFIRMED'
        assert record['resources']['current_enforcement'] == 'NOT_ESTABLISHED_BY_HISTORY'
        assert record['resources']['last_observed']['soft_bytes'] == '268435456'
    assert all(v['application']['state'] == 'UNCONFIRMED'
        for v in resident['unclean_reopen']['execution_admission'].values())

    build = run('fixture-build', ['docker', 'run', '--rm', '-v', f'{ROOT}:/source:ro',
        '-v', f'{evidence / "resident"}:/evidence', '-w', '/source', '-e', 'CARGO_HOME=/evidence/cargo-home',
        '--entrypoint', 'cargo', builder, 'test', '--locked', '-p', 'rx-supervisor', '--lib',
        '--test', 'resource_enforcement', '--no-run', '--target-dir', '/evidence/target', '--message-format=json'])
    artifacts = [json.loads(line) for line in build.stdout.splitlines() if line.startswith('{')]
    tests = [(v['target']['name'], v['executable']) for v in artifacts
        if v.get('reason') == 'compiler-artifact' and v.get('executable') and v.get('profile', {}).get('test')]
    assert len(tests) == 2
    for name, executable in tests:
        tested = run('fixture-' + name, ['docker', 'run', '--rm', '--network', 'none', '--read-only',
            '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges', '--tmpfs', '/tmp:rw',
            '-v', f'{evidence / "resident"}:/evidence:ro', '--entrypoint', executable, runtime, '--nocapture'])
        if name == 'resource_enforcement':
            assert 'ALLOCATION_ERRNO=12' in tested.stdout
            assert 'outside_supervisor_observation=' in tested.stdout
            assert 'unsupported_bundle=' in tested.stdout

    configuration = json.loads((evidence / 'resident/startup.json').read_text())
    configuration['plan']['id'] = str(uuid.uuid4())
    configuration['plan']['processes'] = configuration['plan']['processes'][:1]
    configuration['plan']['processes'][0]['id'] = 'status'
    config = evidence / 'startup.json'
    config.write_text(json.dumps(configuration, indent=2) + '\n')
    suffix = uuid.uuid4().hex[:10]

    def start(label, executable, data):
        name = 'rx-resources-' + label + '-' + suffix
        names.append(name)
        run(label + '-launcher', ['docker', 'run', '-d', '--name', name, '--network', 'none', '--read-only',
            '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges', '--tmpfs', '/tmp:rw',
            '-v', f'{data}:/var/lib/rx-solutions', '-v', f'{config}:/config/startup.json:ro',
            '-v', f'{executable}:/test/rx-solutionsd:ro', '--entrypoint', '/test/rx-solutionsd', runtime,
            'run', '/config/startup.json'])
        return name

    def inspect(name):
        return json.loads(subprocess.check_output(['docker', 'inspect', name]))[0]

    def ready(label, name):
        deadline = time.monotonic() + 25
        while time.monotonic() < deadline:
            state = inspect(name)
            assert state['State']['Running']
            response = subprocess.run(['docker', 'exec', name, '/usr/bin/python3', '-c',
                'import json,urllib.request; print(json.dumps(json.load(urllib.request.urlopen("http://127.0.0.1:8081/health"))))'],
                capture_output=True, text=True)
            if response.returncode == 0:
                (evidence / (label + '-health.json')).write_text(response.stdout)
                return state
            time.sleep(.1)
        raise AssertionError('status startup timed out')

    def data_dir(label):
        path = evidence / label
        path.mkdir(mode=0o777)
        path.chmod(0o777)
        return path

    def entities(data, filename):
        import sqlite3
        connection = sqlite3.connect(f'file:{data}/managed/{filename}?mode=ro', uri=True)
        try:
            return [(key, revision, json.loads(document)) for key, revision, document
                in connection.execute('select key, revision, document from entities order by key')]
        finally:
            connection.close()

    try:
        fresh = data_dir('fresh-state')
        current = start('current', binary, fresh)
        container = ready('current', current)
        assert container['HostConfig']['Memory'] == 0
        assert container['HostConfig']['NanoCpus'] == 0
        assert not container['HostConfig']['Ulimits']
        assert container['Config']['User'] == '10001:10001'
        observation = run('outside-supervisor', ['docker', 'exec', current, '/usr/bin/python3', '-c',
            "import os,json,sqlite3; c=sqlite3.connect('file:/var/lib/rx-solutions/managed/supervisor.db?mode=ro',uri=True); "
            "s=json.loads(c.execute(\"select document from entities where key='supervisor/state'\").fetchone()[0])['value']; "
            "p=s['records']['status']['pid']; print(json.dumps({'observer_pid':os.getpid(),'supervisor_pid':1,'child_pid':p,"
            "'child_limits':open('/proc/%s/limits'%p).read(),'supervisor_limits':open('/proc/1/limits').read(),"
            "'child_cmdline':open('/proc/%s/cmdline'%p,'rb').read().decode().split(chr(0)),'resource_history':s['records']['status']['resources']}))"])
        observed = json.loads(observation.stdout)
        assert observed['observer_pid'] not in [1, observed['child_pid']]
        row = next(v for v in observed['child_limits'].splitlines() if v.startswith('Max address space'))
        assert row.split()[3:] == ['268435456', '268435456', 'bytes']
        parent = next(v for v in observed['supervisor_limits'].splitlines() if v.startswith('Max address space'))
        assert parent.split()[3:5] == ['unlimited', 'unlimited']
        assert observed['resource_history']['last_observed']['pid'] == observed['child_pid']
        assert '-c' not in observed['child_cmdline']  # Final service, not the exec gate.
        run('current-stop', ['docker', 'stop', '--timeout', '15', current])
        assert inspect(current)['State']['ExitCode'] == 0

        old = data_dir('existing-state')
        previous = start('baseline', baseline, old)
        ready('baseline', previous)
        run('baseline-stop', ['docker', 'stop', '--timeout', '15', previous])
        assert inspect(previous)['State']['ExitCode'] == 0
        before = entities(old, 'registration.db')
        saved_execution = entities(old, 'supervisor.db')
        changed = start('changed-catalog', binary, old)
        deadline = time.monotonic() + 10
        while inspect(changed)['State']['Running']:
            assert time.monotonic() < deadline
            time.sleep(.1)
        assert inspect(changed)['State']['ExitCode'] != 0
        refusal = run('changed-catalog-output', ['docker', 'logs', changed])
        assert 'resident catalog digest changed' in refusal.stderr + refusal.stdout
        assert before == entities(old, 'registration.db')
        assert saved_execution == entities(old, 'supervisor.db')
        result = {'result': 'PASS', 'runtime_image': runtime, 'builder_image': builder,
            'daemon_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
            'supplied_baseline_sha256': hashlib.sha256(baseline.read_bytes()).hexdigest(),
            'external_observation': observed, 'container_resource_options': {'Memory': 0, 'NanoCpus': 0, 'Ulimits': None},
            'existing_registration_changed_catalog_refused': True, 'old_entities_preserved': True,
            'rx_test_child_64MiB_rejected_128MiB_allocation': True,
            'limitations': ['per-process virtual address-space ceiling, not physical memory or capacity reservation',
                'one kernel policy; multi-policy partial application is not established',
                'descendant policy lifetime and physical handover unassessed',
                'stored observations never restore current enforcement ownership',
                'actual guarded services and physical equipment not qualified']}
        (evidence / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps({'result': 'PASS', 'outside_observer_pid': observed['observer_pid'],
            'child_pid': observed['child_pid'], 'evidence': str(evidence)}))
    finally:
        for name in names:
            run('retained-' + name, ['docker', 'logs', name], check=False)
            if inspect(name)['State']['Running']:
                run('cleanup-stop-' + name, ['docker', 'stop', '--timeout', '15', name], check=False)
            run('cleanup-remove-' + name, ['docker', 'rm', name], check=False)


if __name__ == '__main__':
    main()
