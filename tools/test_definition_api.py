#!/usr/bin/env python3
"""M1 API/CLI acceptance against a fresh real platform process and SQLite store."""
import argparse
import copy
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'deployment/local-skills'))
from definitions_client import Definitions, LocalAuthoring
from runtime_client import RuntimeRejected


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()
    package = json.loads((ROOT / 'examples/definitions/0f-laser-simulation/cell.json').read_text())
    new_tray = json.loads((ROOT / 'examples/definitions/0f-laser-simulation/new-tray.json').read_text())
    checks = []
    with tempfile.TemporaryDirectory(prefix='rx-definition-api-') as temporary:
        root = Path(temporary)
        password = root / 'password'
        password.write_text(secrets.token_urlsafe(24)); password.chmod(0o600)
        with password.open() as stream:
            subprocess.run([str(binary), 'init', str(root / 'installation')], stdin=stream, check=True, capture_output=True)
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0)); port = listener.getsockname()[1]
        origin = f'http://127.0.0.1:{port}'
        process = None

        def start():
            nonlocal process
            process = subprocess.Popen([str(binary), 'serve', str(root / 'installation'), f'127.0.0.1:{port}', origin],
                                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise RuntimeError(process.stderr.read().decode())
                try:
                    with urllib.request.urlopen(origin + '/api/v1/health', timeout=1) as response:
                        if response.status == 200: return
                except OSError:
                    time.sleep(.05)
            raise RuntimeError('platform did not become ready')

        def client():
            return Definitions(LocalAuthoring(origin, origin, 'admin', password), root / 'client')

        def model(delta, values):
            data = copy.deepcopy(new_tray)
            entry = data['definitions'][0]
            entry['key'] = delta; entry['id'] = str(uuid.uuid4())
            entry['body']['values'].update(values)
            return data

        try:
            start(); c = client()
            request = str(uuid.uuid4())
            receipt = c.apply(package, {}, request); refs = receipt['references']
            assert len(receipt['applied']) == len(package['definitions'])
            assert c.apply(package, {}, request) == receipt
            checks.append('full package applied and replayed without new revisions')
            instance = c.show(refs['tray.supply'])
            assert instance['effective']['values']['origin']['declared_by'] == refs['tray.supply']
            assert instance['effective']['shadowed']['origin'][0]['declared_by'] == refs['tray.standard']
            assert instance['effective']['values']['rows']['declared_by'] == refs['tray.standard']
            points = c.points(refs['tray.supply'], refs['rule.tray-slots'])
            assert points['total'] == '24' and points['points'][0]['position'] == [100,200,760]
            assert points['orientation_xyzw'] == [0,0,0,1]
            checks.append('instance inheritance, overridden model provenance and complete poses')
            all_indices = []
            offset = 0
            while True:
                page = c.points(refs['tray.dense'], refs['rule.tray-slots'], offset)
                all_indices.extend(p['index'] for p in page['points'])
                if page['next'] is None: break
                offset = int(page['next'])
            assert all_indices == [str(i) for i in range(2400)]
            assert page['points'][-1]['position'] == [1180,780,0]
            checks.append('2400 dense slots paged exactly once in declared order')
            added = c.apply(new_tray, refs, str(uuid.uuid4()))['references']
            assert c.points(added['tray.new'], refs['rule.tray-slots'])['total'] == '15'
            rotated = model('rotated', {'orientation': {'vector':[0,0,2**-.5,2**-.5]}})
            rotation = c.apply(rotated, refs, str(uuid.uuid4()))['references']['rotated']
            last = c.points(rotation, refs['rule.tray-slots'])['points'][-1]['position']
            assert all(abs(a-b)<1e-8 for a,b in zip(last,[-120,240,0]))
            checks.append('new and rotated tray models added as data with unchanged code/rule')
            invalid = model('invalid-orientation', {'orientation': {'vector':[0,0,0,0]}})
            invalid_ref = c.apply(invalid, refs, str(uuid.uuid4()))['references']['invalid-orientation']
            blocked = c.points(invalid_ref, refs['rule.tray-slots'])
            assert not blocked['points'] and blocked['violations'][0]['code'] == 'ORIENTATION_REQUIRED'
            checks.append('invalid quaternion blocks point generation with a source location')

            class LostReply:
                def __init__(self, terminal):
                    self.terminal = terminal; self.fingerprint = terminal.fingerprint
                    self.principal = terminal.principal; self.once = True
                def get(self, *a, **kw): return self.terminal.get(*a, **kw)
                def request(self, path, body):
                    value = self.terminal.request(path, body)
                    if self.once and path == '/api/v1/definitions':
                        self.once = False
                        raise OSError('injected response loss after real server commit')
                    return value
            lost = Definitions(LostReply(c.terminal), root / 'client')
            delta = model('lost-reply', {})
            req = str(uuid.uuid4())
            try:
                lost.apply(delta, refs, req)
                raise AssertionError('response loss did not fire')
            except OSError:
                pass
            recovered = lost.apply(delta, refs, req)['references']['lost-reply']
            history = c.terminal.get('/api/v1/definition-history', catalog=recovered['catalog'], id=recovered['id'])
            assert len(history['versions']) == 1 and recovered['revision'] == '1'
            checks.append('real committed response loss recovered with the original request and one revision')
            tampered = {**refs['tray.supply'], 'digest':'0'*64}
            try:
                c.points(tampered, refs['rule.tray-slots'])
                raise AssertionError('tampered reference accepted')
            except RuntimeRejected as error:
                assert error.status == 400
            checks.append('server refuses tampered pinned definition digest')
            before = c.points(refs['tray.supply'], refs['rule.tray-slots'])
            process.terminate(); process.wait(timeout=10); process.stderr.close()
            start(); c = client()
            assert c.points(refs['tray.supply'], refs['rule.tray-slots']) == before
            assert c.show(refs['part.ECC_51-14'])['version']['definition']['reference'] == refs['part.ECC_51-14']
            checks.append('server restart preserves definitions, provenance and identical pinned point report')
        finally:
            if process is not None:
                if process.poll() is None: process.terminate(); process.wait(timeout=10)
                process.stderr.close()
    result = {'status':'PRECHECK_PASSED', 'profile':'SIMULATION', 'definitions':len(package['definitions']),
              'checks':checks, 'not_performed':['user acceptance','workflow execution resolution','preview','publish','run','physical devices']}
    args.output.write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result,indent=2))

if __name__ == '__main__':
    main()
