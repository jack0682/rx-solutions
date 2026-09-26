#!/usr/bin/env python3
"""Actual Linux multi-area judgments and work commits; issuer keys never enter host.

Both compiled recipes share one Plan/manager in each scene. Replay cases use the
exact bytes that previously committed in their own area. Adversarial raw signing
runs separately from the host and deliberately bypasses the issuer's rule, to
isolate receiver signature enforcement. It does not add a product signing path.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
AREAS = {
    'A': {'area': 'development/support-area', 'program': 'rx/status-work-http',
          'role': 'work/support-gap-report', 'ttl': 30000},
    'B': {'area': 'development/compact-support-area', 'program': 'rx/status-compact-work-http',
          'role': 'work/compact-support-gap-report', 'ttl': 15000},
}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for arg in ['image', 'daemon', 'judge', 'key-a', 'key-b', 'evidence']:
        parser.add_argument('--' + arg, required=True)
    a = parser.parse_args(); e = Path(a.evidence).resolve(); e.mkdir(parents=True, exist_ok=False)
    for name, path in [('daemon', a.daemon), ('judge', a.judge)]:
        shutil.copy2(path, e / name); (e / name).chmod(0o755)
    shutil.copy2(ROOT / 'tools/operating_area_observer.py', e / 'observer.py')
    keys = {'A': Path(a.key_a).resolve(), 'B': Path(a.key_b).resolve()}
    commands = []; results = []; accepted = {}
    def run(label, cmd, expected=0):
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=100)
        (e / (label + '.stdout')).write_text(r.stdout); (e / (label + '.stderr')).write_text(r.stderr)
        commands.append({'label': label, 'argv': cmd, 'exit_code': r.returncode, 'expected': expected})
        (e / 'commands.json').write_text(json.dumps(commands, indent=2))
        if expected is not None: assert r.returncode == expected, (label, r.stdout, r.stderr)
        return r
    image = json.loads(run('image', ['docker', 'image', 'inspect', a.image]).stdout)[0]['Id']
    base = ['docker', 'run', '--network', 'none', '--read-only', '--cap-drop', 'ALL',
            '--security-opt', 'no-new-privileges', '--tmpfs', '/tmp:rw']
    cases = [(area, mode) for mode in ['positive', 'replay-other', 'native-5000', 'profiles-32', 'ttl-25000',
             'own-native-denied', 'own-profiles-denied', 'task-other-area', 'foreign-role', 'foreign-program',
             'foreign-key-signature'] for area in AREAS]
    for area, mode in cases:
        label = area + '-' + mode; c = e / label; c.mkdir(); spec = AREAS[area]; other = 'B' if area == 'A' else 'A'
        for directory in ['state', 'mailbox', 'control']:
            (c / directory).mkdir(); (c / directory).chmod(0o777)
        cfg = {'schema': 'rx.solutions-startup.v1', 'state_subdirectory': 'managed',
               'operating_area_mailbox': '/exchange', 'plan': {
            'schema': 'rx.solutions-process-plan.v1', 'id': str(uuid.uuid4()), 'environment': 'SIMULATION',
            'profiles': ['SIM-JTC-6DOF'], 'processes': [
                {'id': 'status-' + code.lower(), 'program': sp['program'],
                 'parameters': {'bind': '127.0.0.1', 'port': str(8081 + i)}, 'depends_on': [],
                 'startup_timeout_ms': '10000', 'shutdown_timeout_ms': '1000',
                 'restart_limit': '0', 'restart_backoff_ms': '100'}
                for i, (code, sp) in enumerate(AREAS.items())]}}
        task = {'operation': str(uuid.uuid4()), 'selection': 'status-' + area.lower(), 'operating_area': spec['area'],
                'required_native_packages': '1000', 'required_support_profiles': '6'}
        ttl = spec['ttl']; denied = None
        if mode == 'native-5000': task['required_native_packages'] = '5000'
        if mode == 'profiles-32': task['required_support_profiles'] = '32'
        if mode == 'ttl-25000': ttl = 25000
        if mode in ['native-5000', 'profiles-32', 'ttl-25000'] and area == 'B': denied = 'judge-policy-denied'
        if mode == 'own-native-denied':
            task['required_native_packages'] = '10001' if area == 'A' else '2001'; denied = 'judge-policy-denied'
        if mode == 'own-profiles-denied':
            task['required_support_profiles'] = '65' if area == 'A' else '9'; denied = 'judge-policy-denied'
        if mode == 'task-other-area': task['operating_area'] = AREAS[other]['area']; denied = 'issuer-scope'
        if mode in ['foreign-role', 'foreign-program']: denied = 'judge-policy-denied'
        if mode == 'replay-other': denied = 'issuer-scope'
        if mode == 'foreign-key-signature': denied = 'signature-mismatch'
        (c / 'config.json').write_text(json.dumps(cfg)); (c / 'task.json').write_text(json.dumps(task))
        name = 'rx-g4-' + uuid.uuid4().hex[:12]
        host = base + ['-d', '--name', name, '-v', f'{e}:/evidence:ro', '-v', f'{c}:/case:ro',
                      '-v', f'{c}/state:/var/lib/rx-solutions', '-v', f'{c}/mailbox:/exchange',
                      '-v', f'{c}/control:/control', '--entrypoint', '/usr/bin/python3', image,
                      '/evidence/observer.py', '/evidence/daemon', 'issuer-scope' if mode == 'task-other-area' else 'controlled']
        try:
            run(label + '-start', host)
            desc = json.loads(run(label + '-mounts', ['docker', 'inspect', name]).stdout)[0]
            assert not any(k.is_relative_to(Path(m['Source'])) for k in keys.values() for m in desc['Mounts'] if m.get('Source'))
            original = None; signed_bytes = None
            if mode != 'task-other-area':
                deadline = time.monotonic() + 25
                while not (c / 'control/request-ready.json').exists():
                    assert time.monotonic() < deadline, (c / 'control/daemon.log').read_text()
                    time.sleep(.02)
                ready = json.loads((c / 'control/request-ready.json').read_text())
                cid = ready['operating_area_provider']['challenge']; request = c / 'mailbox' / ('request-' + cid + '.json')
                original = request.read_bytes(); info = json.loads(original)
                assert info['owner']['program'] == spec['program'] and info['role'] == spec['role'] and info['operating_area'] == spec['area']
                response = c / 'mailbox' / ('decision-' + cid + '.json')
                if mode == 'replay-other':
                    signed_bytes = accepted[other]; response.write_bytes(signed_bytes)
                    assert hashlib.sha256(signed_bytes).hexdigest() == results[['A-positive', 'B-positive'].index(other + '-positive')]['decision_sha256']
                elif mode == 'foreign-key-signature':
                    claim = {'schema': 'rx.external-decision.v1', 'verdict': 'APPROVE', 'challenge': info,
                             'decision': str(uuid.uuid4()), 'ttl_ms': str(ttl)}
                    # All fixture strings are ASCII and counters strings: sorted
                    # compact JSON is exactly JCS here. Retain message and OpenSSL
                    # verification under the foreign key as independent evidence.
                    message = b'RX-EXTERNAL-DECISION-v1\0' + json.dumps([info['key'], claim], sort_keys=True, separators=(',', ':')).encode()
                    (c / 'message').write_bytes(message)
                    cmd = base + ['--rm', '-v', f'{c}:/attack', '-v', f'{keys[other]}:/private/key.pem:ro',
                                  '--entrypoint', '/bin/sh', image, '-ec',
                                  'openssl pkeyutl -sign -rawin -inkey /private/key.pem -in /attack/message -out /attack/signature; openssl pkey -in /private/key.pem -pubout -out /attack/public.pem; openssl pkeyutl -verify -rawin -pubin -inkey /attack/public.pem -in /attack/message -sigfile /attack/signature']
                    run(label + '-adversarial-sign-and-verify', cmd)
                    signed = {'claim': claim, 'signature': {'key': info['key'], 'signature': (c / 'signature').read_bytes().hex()}}
                    response.write_text(json.dumps(signed)); signed_bytes = response.read_bytes()
                else:
                    if mode == 'foreign-role': info['role'] = AREAS[other]['role']
                    if mode == 'foreign-program': info['owner']['program'] = AREAS[other]['program']
                    issuer_mailbox = c / 'mailbox'
                    if mode in ['foreign-role', 'foreign-program']:
                        # Isolate issuer rule evaluation on a hostile copy. Altering
                        # the live request instead is caught earlier by immutable
                        # publication, not by the issuer rule under examination.
                        issuer_mailbox = c / 'adversarial-issuer-input'
                        issuer_mailbox.mkdir(); issuer_mailbox.chmod(0o777)
                        (issuer_mailbox / request.name).write_text(json.dumps(info))
                    cmd = base + ['--rm', '-v', f'{e}/judge:/judge:ro', '-v', f'{issuer_mailbox}:/exchange',
                                  '-v', f'{keys[area]}:/private/key.pem:ro', '--entrypoint', '/judge', image,
                                  'decide', '/exchange', cid, '/private/key.pem', str(ttl)]
                    judged = json.loads(run(label + '-judge', cmd).stdout)
                    assert judged['result'] == ('POLICY_DENIED' if denied else 'SIGNED_DEVELOPMENT_APPROVAL'), judged
                    if denied and mode in ['foreign-role', 'foreign-program']:
                        assert judged['denial']['reason'] == 'judge/role-kind-program'
                        shutil.copy2(issuer_mailbox / ('denial-' + cid + '.json'), c / 'mailbox' / ('denial-' + cid + '.json'))
                    if not denied: signed_bytes = response.read_bytes()
                (c / 'control/reply-ready').touch()
                assert request.read_bytes() == original
            waited = run(label + '-wait', ['docker', 'wait', name]); assert waited.stdout.strip() == '0', (c / 'control/daemon.log').read_text()
            observed = json.loads((c / 'control/result.json').read_text()); result = observed['result']
            if denied:
                assert result['gate'] == 'DENIED' and denied in result['reason'] and observed['work_rows'] == 0, observed
            else:
                assert observed['work_rows'] == 2 and observed['judgment_observed'], observed
                assert result['result']['current_permission'] == 'NONE; HISTORICAL_WORK_RESULT_ONLY'
                assert result['result']['support_profiles']['shortfall'] == str(max(0, int(task['required_support_profiles']) - 4))
            if mode == 'positive': accepted[area] = signed_bytes
            results.append({'scene': label, 'area': area, 'expected_refusal': denied,
                            'decision_sha256': hashlib.sha256(signed_bytes).hexdigest() if signed_bytes else None,
                            'replayed_from': other + '-positive' if mode == 'replay-other' else None, **observed})
            (e / 'scenes.json').write_text(json.dumps(results, indent=2))
        finally:
            run(label + '-logs', ['docker', 'logs', name], None)
            run(label + '-stop', ['docker', 'stop', '--time', '15', name], None)
            run(label + '-remove', ['docker', 'rm', name], None)
    summary = {'result': 'COMPILED_MULTI_AREA_PASS', 'image': image, 'scenes': results,
               'daemon_sha256': hashlib.sha256((e / 'daemon').read_bytes()).hexdigest(),
               'judge_sha256': hashlib.sha256((e / 'judge').read_bytes()).hexdigest(),
               'scope': 'two compiled development areas; one manager, two recipes; no physical/network/product key-custody qualification'}
    (e / 'result.json').write_text(json.dumps(summary, indent=2)); print(json.dumps({'result': summary['result'], 'scenes': len(results)}))

if __name__ == '__main__':
    main()
