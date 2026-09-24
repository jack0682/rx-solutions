#!/usr/bin/env python3
"""Actual Linux offline judge and receiving gate. Private key enters only the issuer container.

The dedicated development key is supplied by the maintainer, never generated or
installed by the host. This is not physical/production operating authority.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--image', required=True)
    p.add_argument('--key', required=True, type=Path)
    p.add_argument('--daemon', required=True, type=Path)
    p.add_argument('--judge', required=True, type=Path)
    p.add_argument('--evidence', required=True, type=Path)
    a = p.parse_args()
    e = a.evidence.resolve(); e.mkdir(parents=True, exist_ok=False)
    for name, source in [('rx-solutionsd', a.daemon), ('rx-operating-area-judge', a.judge)]:
        shutil.copy2(source, e / name); (e / name).chmod(0o755)
    shutil.copy2(ROOT / 'tools/operating_area_observer.py', e / 'observer.py')
    commands = []
    def run(label, argv, expected=0):
        r = subprocess.run(argv, capture_output=True, text=True)
        (e / (label + '.stdout')).write_text(r.stdout); (e / (label + '.stderr')).write_text(r.stderr)
        commands.append({'label': label, 'argv': argv, 'exit_code': r.returncode, 'expected': expected})
        (e / 'commands.json').write_text(json.dumps(commands, indent=2))
        if expected is not None:
            assert r.returncode == expected, (label, r.stdout, r.stderr)
        return r
    image = json.loads(run('image', ['docker', 'image', 'inspect', a.image]).stdout)[0]['Id']
    summaries = []
    modes = ['natural-positive', 'unconnected', 'issuer-scope', 'judge-policy-denied', 'signature-mismatch',
             'claimed-area', 'claimed-role', 'claimed-kind', 'unknown-response-key', 'revoked',
             'expiry-after-judgment', 'commit-lock-busy']
    for mode in modes:
        case = e / mode; case.mkdir()
        for name in ['state', 'mailbox', 'control']:
            (case / name).mkdir(); (case / name).chmod(0o777)
        config = {'schema': 'rx.solutions-startup.v1', 'state_subdirectory': 'managed',
                  'operating_area_mailbox': '/exchange', 'plan': {
            'schema': 'rx.solutions-process-plan.v1', 'id': str(uuid.uuid4()), 'environment': 'SIMULATION',
            'profiles': ['SIM-JTC-6DOF'], 'processes': [{'id': 'status', 'program': 'rx/status-work-http',
            'parameters': {'bind': '127.0.0.1', 'port': '8081'}, 'depends_on': [], 'startup_timeout_ms': '10000',
            'shutdown_timeout_ms': '1000', 'restart_limit': '0', 'restart_backoff_ms': '100'}]}}
        task = {'operation': str(uuid.uuid4()), 'selection': 'status', 'operating_area': 'development/support-area',
                'required_native_packages': '1000', 'required_support_profiles': '6'}
        if mode == 'unconnected':
            config.pop('operating_area_mailbox')
        if mode == 'issuer-scope':
            task['operating_area'] = 'outside/authored-area'
        if mode == 'judge-policy-denied':
            task['required_support_profiles'] = '65'
        (case / 'config.json').write_text(json.dumps(config)); (case / 'task.json').write_text(json.dumps(task))
        name = 'rx-g3-' + uuid.uuid4().hex[:12]
        base = ['docker', 'run', '--network', 'none', '--read-only', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges', '--tmpfs', '/tmp:rw']
        host = base + ['-d', '--name', name, '-v', f'{e}:/evidence:ro', '-v', f'{case}:/case:ro',
                      '-v', f'{case}/state:/var/lib/rx-solutions', '-v', f'{case}/mailbox:/exchange',
                      '-v', f'{case}/control:/control', '--entrypoint', '/usr/bin/python3', image,
                      '/evidence/observer.py', '/evidence/rx-solutionsd', mode]
        holder = None
        def wait_for(path, seconds=20):
            until = time.monotonic() + seconds
            while not path.exists():
                if (case / 'control/result.json').exists():
                    raise AssertionError((mode, json.loads((case / 'control/result.json').read_text())))
                assert time.monotonic() < until, (mode, 'timeout', (case / 'control/daemon.log').read_text())
                time.sleep(.02)
            return path
        def judge(action, challenge, parameter):
            argv = base + ['--rm', '-v', f'{e}/rx-operating-area-judge:/judge:ro', '-v', f'{case}/mailbox:/exchange',
                           '-v', f'{a.key.resolve()}:/private/key.pem:ro', '--entrypoint', '/judge', image,
                           action, '/exchange', challenge, '/private/key.pem', parameter]
            return run(mode + '-' + action, argv)
        try:
            run(mode + '-start', host)
            custody = run(mode + '-host-custody', ['docker', 'exec', name, '/usr/bin/python3', '-c',
                "import os,json; from pathlib import Path; assert not Path('/private/key.pem').exists(); processes={}\nfor p in Path('/proc').iterdir():\n if p.name.isdigit() and int(p.name)!=os.getpid():\n  try: processes[p.name]=p.joinpath('cmdline').read_bytes().split(b'\\0')[0].decode(errors='replace')\n  except (FileNotFoundError,PermissionError,ProcessLookupError):pass\nissuers=[v for v in processes.values() if Path(v).name in ['judge','rx-operating-area-judge','openssl']];assert not issuers;print(json.dumps({'uid':os.getuid(),'private_key_path_present':False,'issuer_processes_observed':issuers,'process_snapshot':processes,'scope':'one actual process snapshot plus inspected host launch code'}))"])
            host_description = json.loads(run(mode + '-host-mounts', ['docker', 'inspect', name]).stdout)[0]
            key_path = a.key.resolve()
            assert not any(key_path.is_relative_to(Path(m['Source'])) for m in host_description['Mounts'] if m.get('Source')), 'private key is under a host mount'
            if mode not in ['unconnected', 'issuer-scope']:
                ready = json.loads(wait_for(case / 'control/request-ready.json').read_text())
                challenge = ready['operating_area_provider']['challenge']
                request = case / 'mailbox' / ('request-' + challenge + '.json')
                original_request = request.read_bytes()
                judged = judge('decide', challenge, '1000' if mode == 'expiry-after-judgment' else '30000')
                if mode == 'judge-policy-denied':
                    assert json.loads(judged.stdout)['result'] == 'POLICY_DENIED'
                elif mode in ['signature-mismatch', 'claimed-area', 'claimed-role', 'claimed-kind', 'unknown-response-key']:
                    response = case / 'mailbox' / ('decision-' + challenge + '.json')
                    value = json.loads(response.read_text())
                    if mode == 'signature-mismatch': value['signature']['signature'] = '00' * 64
                    if mode == 'claimed-area': value['claim']['challenge']['operating_area'] = 'outside/area'
                    if mode == 'claimed-role': value['claim']['challenge']['role'] = 'other/role'
                    if mode == 'claimed-kind': value['claim']['challenge']['kind'] = 'REPLACEMENT_BINDING'
                    if mode == 'unknown-response-key': value['signature']['key'] = 'outside/key'
                    # Explicit adversarial fixture, not a cooperating issuer publication.
                    response.write_text(json.dumps(value))
                (case / 'control/reply-ready').touch()
                if mode in ['revoked', 'expiry-after-judgment', 'commit-lock-busy']:
                    judgment = json.loads(wait_for(case / 'control/judgment-ready.json').read_text())
                    assert judgment['work_use_permission']['state'] == 'VERIFIED'
                    if mode == 'revoked': judge('revoke', challenge, 'development/condition-changed')
                    if mode == 'expiry-after-judgment': time.sleep(1.2)
                    if mode == 'commit-lock-busy':
                        hold = "import fcntl,time;from pathlib import Path\nf=open('/exchange/exchange.lock','a+');fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB);Path('/control/lock-held').touch()\nwhile not Path('/control/lock-release').exists():time.sleep(.02)"
                        holder_name = name + '-holder'
                        holder = holder_name
                        run(mode + '-lock-holder', base + ['-d', '--name', holder_name, '-v', f'{case}/mailbox:/exchange', '-v', f'{case}/control:/control', '--entrypoint', '/usr/bin/python3', image, '-c', hold])
                        wait_for(case / 'control/lock-held')
                    (case / 'control/commit-ready').touch()
                assert request.read_bytes() == original_request, 'polling replaced the original challenge'
            waited = run(mode + '-wait', ['docker', 'wait', name]); assert waited.stdout.strip() == '0', run(mode + '-logs', ['docker', 'logs', name]).stdout
            observation = json.loads((case / 'control/result.json').read_text()); result = observation['result']
            if mode == 'natural-positive':
                assert observation['work_rows'] == 2 and result['result']['support_profiles']['shortfall'] == '2', observation
                assert result['result']['current_permission'] == 'NONE; HISTORICAL_WORK_RESULT_ONLY'
            else:
                reasons = {'unconnected': 'operating-area-provider', 'issuer-scope': 'issuer-scope', 'judge-policy-denied': 'judge-policy-denied',
                           'signature-mismatch': 'signature-mismatch', 'claimed-area': 'operating-area-mismatch', 'claimed-role': 'role-kind-mismatch',
                           'claimed-kind': 'role-kind-mismatch', 'unknown-response-key': 'issuer-scope', 'revoked': 'decision/revoked',
                           'expiry-after-judgment': 'decision/expired', 'commit-lock-busy': 'mailbox-unavailable'}
                assert result['gate'] == 'DENIED' and reasons[mode] in result['reason'] and observation['work_rows'] == 0, observation
            summaries.append({'scene': mode, **observation, 'host_custody': json.loads(custody.stdout)})
            (e / 'scenes.json').write_text(json.dumps(summaries, indent=2))
        finally:
            if holder:
                (case / 'control/lock-release').touch()
                run(mode + '-holder-stop', ['docker', 'stop', '--time', '5', holder], expected=None)
                run(mode + '-holder-remove', ['docker', 'rm', holder], expected=None)
            run(mode + '-final-log', ['docker', 'logs', name], expected=None)
            run(mode + '-stop', ['docker', 'stop', '--time', '15', name], expected=None)
            run(mode + '-remove', ['docker', 'rm', name], expected=None)
    result = {'result': 'DEVELOPMENT_OPERATING_AREA_JUDGE_PASS', 'image': image,
              'daemon_sha256': hashlib.sha256((e / 'rx-solutionsd').read_bytes()).hexdigest(),
              'judge_sha256': hashlib.sha256((e / 'rx-operating-area-judge').read_bytes()).hexdigest(),
              'scenes': summaries, 'scope': 'one offline development judge; no physical/production qualification',
              'residuals': ['noncooperating publication not ordered', 'TTL runs during post-cut IO', 'HTTP as-of observation']}
    (e / 'result.json').write_text(json.dumps(result, indent=2)); print(json.dumps({'result': result['result'], 'scenes': len(summaries)}))


if __name__ == '__main__':
    main()
