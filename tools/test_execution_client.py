#!/usr/bin/env python3
"""CLI custody and exact-byte export checks; not simulated-device acceptance."""
import copy
import hashlib
import io
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import uuid
sys.path.insert(0, str(Path(__file__).resolve().parents[1]/'deployment/local-skills'))
from execution_client import ExecutionClient, run


class ExecutionClientTests(unittest.TestCase):
    def test_original_object_request_replay_and_changed_input_refusal(self):
        calls = []
        command = {'run':str(uuid.uuid4()), 'ordinal':'1', 'object':{'id':'object'}}
        def request(route, body):
            calls.append(copy.deepcopy(body))
            if len(calls) == 1: raise OSError('reply lost')
            return body['command']
        server = SimpleNamespace(fingerprint='server', request=request)
        with tempfile.TemporaryDirectory() as d:
            client = ExecutionClient(server, Path(d)); key = str(uuid.uuid4())
            with self.assertRaises(OSError): client.command('bind-object', command, key)
            self.assertEqual(client.command('bind-object', command, key), command)
            self.assertEqual(calls[0], calls[1])
            changed = dict(command, ordinal='2')
            with self.assertRaises(ValueError): client.command('bind-object', changed, key)
            server.fingerprint = 'other'
            with self.assertRaises(ValueError): client.command('bind-object', command, key)
            self.assertEqual(len(calls), 2)

    def test_output_exists_is_checked_before_login(self):
        with tempfile.TemporaryDirectory() as d:
            output = Path(d)/'receipt'; output.write_text('original')
            with patch('execution_client.Terminal') as terminal:
                with self.assertRaisesRegex(ValueError, 'no server request'):
                    run(SimpleNamespace(output=output, action='create-run'))
                terminal.assert_not_called()
            self.assertEqual(output.read_text(), 'original')

    def test_export_preserves_canonical_bytes_and_checks_hash(self):
        # JSON spelling must not be changed by a parse/encode round trip.
        raw = b'{"number":0.000001}'
        pin = {'size_bytes':str(len(raw)), 'sha256':hashlib.sha256(raw).hexdigest()}
        preview = {'reference':{'catalog':str(uuid.uuid4()), 'id':str(uuid.uuid4()), 'revision':'1', 'digest':'a'*64},
                   **{k:pin for k in ('policy','inputs','index')}}
        server = SimpleNamespace(origin='https://example.invalid', opener=SimpleNamespace(open=lambda *a, **k:io.BytesIO(raw)))
        with tempfile.TemporaryDirectory() as d:
            root = Path(d); client = ExecutionClient(server, root/'state')
            client.export(preview, root/'material')
            self.assertEqual((root/'material/inputs-0.json').read_bytes(), raw)
            material = json.loads((root/'material/host-material.json').read_bytes())
            self.assertEqual(material['inputs']['reference'], pin)
            changed = copy.deepcopy(preview); changed['inputs']['sha256'] = '0'*64
            with self.assertRaisesRegex(ValueError, 'digest/size'):
                client.export(changed, root/'changed')
            self.assertFalse((root/'changed/host-material.json').exists())


if __name__ == '__main__': unittest.main()
