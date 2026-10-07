#!/usr/bin/env python3
"""Exercise the CLI transport and durable package request boundary without equipment."""
from email.message import Message
from io import BytesIO
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
import urllib.request
import urllib.response

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'deployment/local-skills'))
from definitions_client import LocalAuthoring, Definitions
from runtime_client import RuntimeRejected
import copy
import uuid

class TransportTests(unittest.TestCase):
    def test_public_host_survives_urllib_header_precedence(self):
        with tempfile.TemporaryDirectory() as directory:
            password = Path(directory) / 'password'
            password.write_text('local-test-password')
            password.chmod(0o600)
            observed = []
            def capture(handler, http_class, request, **kwargs):
                headers = dict(request.unredirected_hdrs)
                headers.update({k: v for k, v in request.headers.items() if k not in headers})
                observed.append((request.host, headers))
                response = urllib.response.addinfourl(BytesIO(json.dumps({'principal':'admin','terminal':None}).encode()), Message(), request.full_url, 200)
                response.msg = 'OK'
                return response
            with patch.object(urllib.request.AbstractHTTPHandler, 'do_open', capture):
                client = LocalAuthoring('http://127.0.0.1:8086','http://127.0.0.1:5186','admin',password)
                client.get('/api/v1/session')
            self.assertEqual(len(observed), 2)
            for host, headers in observed:
                self.assertEqual(host,'127.0.0.1:8086')
                self.assertEqual(headers['Host'],'127.0.0.1:5186')
            self.assertEqual(observed[0][1]['Origin'],'http://127.0.0.1:5186')
            self.assertEqual(observed[0][1]['X-rx-client'],'browser-v1')

    def test_local_mode_refuses_remote_endpoint_and_remote_public_origin(self):
        for url, origin in [('http://192.0.2.1:8086','http://127.0.0.1:5186'),
                            ('http://127.0.0.1:8086','http://192.0.2.1:5186')]:
            with self.assertRaises(ValueError):
                LocalAuthoring(url,origin,'admin',Path('/missing'))

class MemoryServer:
    def __init__(self):
        self.fingerprint = 'test-server'
        self.principal = 'engineer'
        self.catalog = {'id': '00000000-0000-4000-8000-000000000010'}
        self.by_request = {}
        self.by_id = {}
        self.drop_reply = True
        self.allowed = True
        self.commits = 0
    def get(self, path, **query):
        if not self.allowed: raise RuntimeRejected(403, {'code':'FORBIDDEN'})
        if path == '/api/v1/definition-catalog': return self.catalog
        return copy.deepcopy(self.by_id[query['id']])
    def request(self, path, value):
        if not self.allowed: raise RuntimeRejected(403, {'code':'FORBIDDEN'})
        key = value['request_key']; command = value['command']
        if key not in self.by_request:
            if command['id'] in self.by_id: raise RuntimeRejected(409, {'code':'STALE_REVISION'})
            ref = {'catalog':command['catalog'],'id':command['id'],'revision':'1','digest':'a'*64}
            result = {'version':{'definition':{'reference':ref,'body':command['body'],'label':command['label']},'archived':False,'updated_by':self.principal},'effective':{}}
            self.by_request[key] = (copy.deepcopy(value),result)
            self.by_id[command['id']] = result
            self.commits += 1
            if self.drop_reply:
                self.drop_reply = False
                raise OSError('injected response loss after commit')
        previous,result = self.by_request[key]
        if previous != value: raise RuntimeRejected(409, {'code':'KEY_CONFLICT'})
        return copy.deepcopy(result)

class PackageTests(unittest.TestCase):
    def test_response_loss_reuses_original_request_and_revocation_blocks_cached_success(self):
        server = MemoryServer()
        package = {'schema':'rx.definition-package.v1','catalog':server.catalog['id'],'title':'test',
            'definitions':[{'key':'property','id':'00000000-0000-4000-8000-000000000011','label':'test','expected':None,'body':{'kind':'PROPERTY'}}]}
        with tempfile.TemporaryDirectory() as directory:
            client = Definitions(server, Path(directory)); request = str(uuid.uuid4())
            with self.assertRaises(OSError): client.apply(package,{},request)
            self.assertEqual(server.commits,1)
            result = client.apply(package,{},request)
            self.assertEqual(server.commits,1)
            self.assertEqual(result['references']['property']['revision'],'1')
            server.allowed = False
            with self.assertRaises(RuntimeRejected): client.apply(package,{},request)
            self.assertEqual(server.commits,1)

    def test_changed_intent_and_mismatched_receipts_are_not_accepted(self):
        server = MemoryServer(); server.drop_reply = False
        package = {'schema':'rx.definition-package.v1','catalog':server.catalog['id'],'title':'test',
            'definitions':[{'key':'property','id':'00000000-0000-4000-8000-000000000011','label':'test','expected':None,'body':{'kind':'PROPERTY'}}]}
        with tempfile.TemporaryDirectory() as directory:
            client = Definitions(server, Path(directory)); request = str(uuid.uuid4())
            client.apply(package,{},request)
            changed = copy.deepcopy(package); changed['definitions'][0]['label']='changed'
            with self.assertRaises(ValueError): client.apply(changed,{},request)
            key = next(iter(server.by_request))
            server.by_request[key][1]['version']['definition']['reference']['revision']='2'
            with self.assertRaisesRegex(ValueError,'response differs'): client.apply(package,{},request)
            self.assertEqual(server.commits,1)

class DiagnosticTests(unittest.TestCase):
    def test_create_conflict_is_explained_without_update_or_new_revision(self):
        server = MemoryServer(); server.drop_reply = False
        package = {'schema':'rx.definition-package.v1','catalog':server.catalog['id'],'title':'test',
            'definitions':[{'key':'tray.sample','id':'00000000-0000-4000-8000-000000000011','label':'Sample tray','expected':None,'body':{'kind':'PROPERTY'}}]}
        with tempfile.TemporaryDirectory() as directory:
            client = Definitions(server, Path(directory)); original = str(uuid.uuid4())
            receipt = client.apply(package, {}, original)
            with self.assertRaises(RuntimeRejected) as caught:
                client.apply(package, {}, str(uuid.uuid4()))
            self.assertEqual(caught.exception.status,409)
            self.assertIn('expected=null is create-only',str(caught.exception))
            self.assertIn('Sample tray',str(caught.exception))
            self.assertIn('r1',str(caught.exception))
            self.assertEqual(server.commits,1)
            self.assertEqual(client.apply(package, {}, original),receipt)
            self.assertIsNone(package['definitions'][0]['expected'])

    def test_diagnostic_read_does_not_guess_when_metadata_is_unavailable(self):
        server = MemoryServer(); server.allowed = False
        with tempfile.TemporaryDirectory() as directory:
            client = Definitions(server, Path(directory))
            command = {'catalog':server.catalog['id'],'id':'00000000-0000-4000-8000-000000000011','expected':None}
            self.assertIsNone(client.conflict_detail(command,'tray.sample'))

if __name__ == '__main__':
    unittest.main()
