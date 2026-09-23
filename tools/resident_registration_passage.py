#!/usr/bin/env python3
"""Run the actual resident daemon with two release-owned read-only services.

Builds only a test binary overlay; publishes no image and attaches no devices.
Raw daemon logs, commands and all persistent state remain in a fresh evidence
directory. This is separate from the library registration_passage procedure.
"""
import argparse
import hashlib
import json
import subprocess
import time
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image', required=True)
    parser.add_argument('--builder', default='rust:1.98.1-slim-bookworm')
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    evidence = args.evidence.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    commands, names = [], []

    def run(label, argv, check=True):
        result = subprocess.run(argv, capture_output=True, text=True)
        (evidence / (label + '.stdout')).write_text(result.stdout)
        (evidence / (label + '.stderr')).write_text(result.stderr)
        commands.append({'label': label, 'argv': argv, 'exit_code': result.returncode})
        (evidence / 'commands.json').write_text(json.dumps(commands, indent=2) + '\n')
        if check and result.returncode:
            raise RuntimeError(f'{label}: {result.returncode}: {result.stderr}')
        return result

    runtime = json.loads(run('image', ['docker', 'image', 'inspect', args.image]).stdout)[0]['Id']
    builder = json.loads(run('builder', ['docker', 'image', 'inspect', args.builder]).stdout)[0]['Id']
    run('build', ['docker', 'run', '--rm', '-v', f'{ROOT}:/source:ro', '-v', f'{evidence}:/evidence',
        '-w', '/source', '-e', 'CARGO_HOME=/evidence/cargo-home', '--entrypoint', 'cargo', builder,
        'build', '--locked', '-p', 'rx-supervisor', '--bin', 'rx-solutionsd', '--target-dir', '/evidence/target'])
    binary = evidence / 'target/debug/rx-solutionsd'
    config = {'schema': 'rx.solutions-startup.v1', 'state_subdirectory': 'managed',
        'plan': {'schema': 'rx.solutions-process-plan.v1', 'id': str(uuid.uuid4()),
            'environment': 'SIMULATION', 'profiles': ['SIM-JTC-6DOF'], 'processes': [
                {'id': selection, 'program': 'rx/status-http', 'parameters': {'bind': '127.0.0.1', 'port': str(port)},
                    'depends_on': [], 'startup_timeout_ms': '10000', 'shutdown_timeout_ms': '5000',
                    'restart_limit': '0', 'restart_backoff_ms': '500'}
                for selection, port in [('first', 8081), ('second', 8082)]]}}
    config_path = evidence / 'startup.json'
    config_path.write_text(json.dumps(config, indent=2) + '\n')
    data = evidence / 'data'
    data.mkdir(mode=0o777)
    data.chmod(0o777)
    suffix = uuid.uuid4().hex[:12]

    def start(label, mode):
        name = 'rx-resident-' + label + '-' + suffix
        names.append(name)
        launch = run(label + '-launcher', ['docker', 'run', '-d', '--name', name, '--network', 'none',
            '--read-only', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges', '--tmpfs', '/tmp:rw',
            '-v', f'{data}:/var/lib/rx-solutions', '-v', f'{config_path}:/config/startup.json:ro',
            '-v', f'{binary}:/test/rx-solutionsd:ro', '--entrypoint', '/test/rx-solutionsd',
            runtime, mode, '/config/startup.json'])
        assert launch.returncode == 0  # The management client has already exited.
        return name

    def inspect(name):
        return json.loads(subprocess.check_output(['docker', 'inspect', name]))[0]

    def logs(label, name):
        output = run(label + '-logs', ['docker', 'logs', name]).stdout
        return [json.loads(line) for line in output.splitlines() if line.startswith('{')]

    def ready(label, name):
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            current = inspect(name)
            if not current['State']['Running']:
                raise AssertionError(run(label + '-failed', ['docker', 'logs', name]).stdout)
            report = subprocess.run(['docker', 'exec', name, '/usr/bin/python3', '-c',
                'import json,urllib.request; print(json.dumps([json.load(urllib.request.urlopen("http://127.0.0.1:%s/health"%p)) for p in (8081,8082)]))'], capture_output=True, text=True)
            if report.returncode == 0:
                (evidence / (label + '-health.json')).write_text(report.stdout)
                run(label + '-owner', ['docker', 'exec', name, '/usr/bin/python3', '-c',
                    'from pathlib import Path; print(Path("/proc/1/cmdline").read_bytes().decode().replace(chr(0)," "))'])
                assert current['Config']['User'] == '10001:10001'
                assert current['HostConfig']['ReadonlyRootfs'] and not current['HostConfig']['Devices']
                return json.loads(report.stdout)
            time.sleep(.1)
        raise AssertionError('daemon services did not become ready')

    def stopped(name):
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            state = inspect(name)['State']
            if not state['Running']:
                return state['ExitCode']
            time.sleep(.1)
        raise AssertionError('daemon did not exit')

    def startup(output):
        matches = [row for row in output if row.get('schema') == 'rx.resident-reconciliation.v1']
        assert len(matches) == 1
        return matches[0]

    try:
        first = start('first', 'run')
        first_health = ready('first', first)
        run('first-stop', ['docker', 'stop', '--time', '15', first])
        assert stopped(first) == 0
        first_output = logs('first', first)
        first_start = startup(first_output)
        identities = {k: v['registration']['registration']['id'] for k, v in first_start['registrations'].items()}
        assert len(set(identities.values())) == 2
        final_observation = [r for r in first_output if r.get('schema') == 'rx.resident-registration-observation.v1'][-1]
        assert all(v['executions'][-1]['last_observed']['state'] == 'EXITED' for v in final_observation['registrations'].values())

        second = start('reopen', 'run')
        assert stopped(second) == 0
        reopened = startup(logs('reopen', second))
        for selection, component in identities.items():
            view = reopened['registrations'][selection]
            assert view['registration']['registration']['id'] == component
            assert view['executions'][-1]['last_observed']['state'] == 'EXITED'
            assert reopened['state']['records'][selection]['phase'] == 'EXITED'

        third = start('activate', 'activate')
        next_health = ready('activate', third)
        assert all(a['supervisor_instance'] != b['supervisor_instance'] for a, b in zip(first_health, next_health))
        run('crash', ['docker', 'kill', '--signal', 'KILL', third])
        logs('activate', third)
        fourth = start('unknown', 'activate')
        assert stopped(fourth) != 0
        unknown = startup(logs('unknown', fourth))
        for selection, component in identities.items():
            assert unknown['registrations'][selection]['registration']['registration']['id'] == component
            instance = unknown['state']['records'][selection]['instance']
            assert unknown['state']['records'][selection]['phase'] == 'UNKNOWN'
            execution = next(e for e in unknown['registrations'][selection]['executions'] if e['binding']['instance'] == instance)
            assert execution['last_observed']['state'] == 'UNKNOWN'
        files = sorted(p.name for p in (data / 'managed').iterdir())
        assert sorted(p for p in files if p.endswith('.db')) == ['registration.db', 'supervisor.db']
        result = {'result': 'PASS', 'runtime_image': runtime, 'builder_image': builder,
            'daemon_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
            'registration_ids': identities, 'state_files': files, 'first': first_start, 'reopened': reopened,
            'unclean_reopen': unknown,
            'scope': 'actual Linux resident daemon; two release-owned non-actuating services; launcher exits before observations; same registration across normal restart and crash; no PID adoption',
            'not_established': ['physical qualification', 'resource enforcement', 'functional readiness',
                'work-use decisions', 'dependency replacement', 'manager-total-loss recovery',
                'actual guarded Host/Executor process integration']}
        (evidence / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps({'result': 'PASS', 'registration_ids': identities, 'evidence': str(evidence)}))
    finally:
        for name in names:
            run('retained-' + name, ['docker', 'logs', name], check=False)
            if inspect(name)['State']['Running']:
                run('cleanup-stop-' + name, ['docker', 'stop', '--time', '15', name], check=False)
            run('cleanup-remove-' + name, ['docker', 'rm', name], check=False)


if __name__ == '__main__':
    main()
