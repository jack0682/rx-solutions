"""Signed device candidate -> fresh P process review. No hardware or activation."""
import json
import os
import subprocess
import uuid
from pathlib import Path


def exercise_process_review(author, limited, verifier, origin, out, info, plan, receipt, context, package):
    headers = {'Origin': origin, 'Content-Type': 'application/json', 'X-RX-Client': 'browser-v1'}

    def post(client, path, command, expected=200, key=None):
        body = {'request_key': key or str(uuid.uuid4()), 'command': command}
        response = client.post(origin + path, headers=headers, data=body)
        assert response.status == expected, (path, response.status, response.text())
        return response.json()

    ref = {k: plan[k] for k in ('id', 'revision', 'plan_digest')}
    create = {'id': str(uuid.uuid4()), 'intake': receipt['id'], 'cell': 'cell/demo',
              'configuration_digest': context['configuration_digest'],
              'policy_generation': context['registration']['generation'],
              'binding_selections': {'load': 'robot/supply'}, 'device_plans': [ref]}
    post(limited, '/api/v1/process-reviews', create, 403)
    create_key = str(uuid.uuid4())
    job = post(author, '/api/v1/process-reviews', create, key=create_key)
    assert job == post(author, '/api/v1/process-reviews', create, key=create_key)
    assert job['request']['schema'] == 'rx.process-review-request.v2'
    assert job['request']['device_context_digest']
    assert job['device_context']['dependencies'][0]['plan'] == plan
    request_file = out / 'device-process-review-request.json'
    request_file.write_text(json.dumps(job['request']))
    report_dir = package.parent / 'device-process-report'
    command = [os.environ['RX_PROCESS_PACKAGE_BIN'], 'review', str(package), info['policy'], str(request_file), str(report_dir)]
    result = subprocess.run(command, capture_output=True, text=True)
    assert result.returncode == 0, result.stderr
    report = json.loads((report_dir / 'verification.json').read_text())
    assert not report['issues'] and report['resolved']
    repo = Path(os.environ['RX_DEVICE_REVIEW_SOLUTIONS'])
    env = dict(os.environ, RX_PROCESS_DEVICE_REPORT=str(report_dir / 'verification.json'),
               RX_PROCESS_DEVICE_SIGNATURE=str(report_dir / 'verification.sig.json'))
    subprocess.run([str(repo / 'tools/cargo'), 'test', '-p', 'rx-process-package', '--test', 'package',
                    'sign_device_process_review', '--locked', '--offline', '--', '--ignored', '--exact'],
                   cwd=repo, env=env, check=True, capture_output=True)
    digest = json.loads(result.stdout)["report_digest"]
    submit = {'review': create['id'], 'cell': 'cell/demo', 'expected': None,
              'directory': 'device-process-report', 'report_digest': digest}
    post(limited, '/api/v1/process-review/reports', submit, 403)
    key = str(uuid.uuid4())
    version = post(author, '/api/v1/process-review/reports', submit, key=key)
    assert version == post(author, '/api/v1/process-review/reports', submit, key=key)
    assert version['ready_for_software_approval'] and not version['platform_issues'], version
    decide = {'review': create['id'], 'cell': 'cell/demo', 'report_revision': version['revision'],
              'review_digest': version['review_digest'], 'expected': None, 'choice': 'APPROVE',
              'note': 'Software candidate source and conditions checked; Host application remains separate'}
    post(author, '/api/v1/process-review/decisions', decide, 403)
    post(limited, '/api/v1/process-review/decisions', decide, 403)
    # An already recorded device approval cannot hide modification of its authority file.
    authority = Path(os.environ['RX_DEVICE_REVIEW_AUTHORITY'])
    original = authority.read_bytes()
    try:
        authority.write_bytes(original + b' ')
        response = verifier.post(origin + '/api/v1/process-review/decisions', headers=headers,
                                 data={'request_key': str(uuid.uuid4()), 'command': decide})
        assert not response.ok, response.text()
    finally:
        authority.write_bytes(original)
    key = str(uuid.uuid4())
    decision = post(verifier, '/api/v1/process-review/decisions', decide, key=key)
    assert decision == post(verifier, '/api/v1/process-review/decisions', decide, key=key)
    detail_url = origin + '/api/v1/process-review?cell=cell%2Fdemo&id=' + create['id']
    assert limited.get(detail_url).status == 403
    detail = verifier.get(detail_url).json()
    assert detail['approval_matches_current_review'] and not detail['activation_authorized']
    change = {'id': str(uuid.uuid4()), 'cell': 'cell/demo', 'reason': 'Must require Host binding application',
              'review': {'id': create['id'], 'revision': version['revision'], 'review_digest': version['review_digest'],
                         'decision_revision': decision['revision']}}
    response = author.post(origin + '/api/v1/process-changes', headers=headers,
                           data={'request_key': str(uuid.uuid4()), 'command': change})
    if os.environ.get('RX_HOST_BINDING_PLAN_ENABLED')=='1':
        assert response.ok, response.text()
        from host_binding_plan import exercise_host_plan
        exercise_host_plan(author, verifier, origin, headers, out, info, response.json())
    else:
        # Device proposals can now be recorded; physical preparation/application remains unsupported.
        assert response.ok, response.text()
        assert response.json()['host_binding_plan']
    # Retire the underlying approval through the actual device-review decision API.
    dependency = job['device_context']['dependencies'][0]
    dv, dd = dependency['version'], dependency['decision']
    post(verifier, '/api/v1/device-review/decisions', {
        'review': dv['review'], 'cell': 'cell/demo', 'report_revision': dv['revision'],
        'review_digest': dv['review_digest'], 'expected': dd['revision'], 'choice': 'REJECT',
        'note': 'Invalidate the source approval to verify dependent review freshness'})
    stale = verifier.get(detail_url).json()
    assert not stale['context_current'] and not stale['approval_matches_current_review']
    assert stale['verification']['revision'] == version['revision']
    # Historical receipt is recoverable only to an actor retaining the whole affected scope.
    assert decision == post(verifier, '/api/v1/process-review/decisions', decide, key=key)
    response = verifier.post(origin + '/api/v1/process-review/decisions', headers=headers,
                             data={'request_key': str(uuid.uuid4()), 'command': dict(decide, expected=decision['revision'])})
    assert not response.ok
    (out / 'device-process-review.json').write_text(json.dumps({
        'status': 'PASS', 'job': create['id'], 'request': job['request'],
        'checks': ['actual signed JTC/process packages and S compiler report', 'exact candidate conditions and provenance',
                   'all affected cells required for creation/report/decision/history', 'independent approval',
                   'fresh device authority file required at approval', 'same request recovers one decision',
                   'underlying approval revocation invalidates dependent review', 'Host application remains blocked',
                   'no active configuration or qualification change']}, indent=2) + '\n')
