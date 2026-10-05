#!/usr/bin/env python3
"""Original request identity and no-request output refusal for the installed API transport."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import uuid

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT/'deployment/local-skills'))
from runtime_client import RuntimeClient


class Terminal:
    fingerprint = 'first-connection'
    def __init__(self): self.calls = []
    def request(self, path, body=None):
        self.calls.append((path, body))
        return {'observation': len(self.calls)}


class ApiTransport(unittest.TestCase):
    def test_changed_retry_is_rejected_and_same_request_rechecks_server(self):
        with tempfile.TemporaryDirectory() as temp:
            terminal = Terminal()
            client = RuntimeClient(terminal, Path(temp))
            request = str(uuid.uuid4())
            body = {'request_key': str(uuid.uuid4()), 'command': {'value': 1}}
            self.assertEqual(client.api_request('/api/v1/process-changes', body, request), {'observation': 1})
            self.assertEqual(client.api_request('/api/v1/process-changes', body, request), {'observation': 2})
            with self.assertRaises(ValueError): client.api_request('/api/v1/process-changes', {'command': {'value': 2}}, request)
            with self.assertRaises(ValueError): client.api_request('/api/v1/other', body, request)
            terminal.fingerprint = 'other-connection'
            with self.assertRaises(ValueError): client.api_request('/api/v1/process-changes', body, request)
            self.assertEqual(len(terminal.calls), 2)
            saved = json.loads((Path(temp)/request/'api-request.json').read_text())
            self.assertNotIn('body', saved)

    def test_existing_output_is_refused_before_connection(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            output = root/'output.json'; output.write_text('keep')
            process = subprocess.run([sys.executable, str(ROOT/'deployment/local-skills/rx'), 'api',
                '--connection', str(root/'missing-connection.json'), 'get', '/api/v1/overview',
                '--output', str(output)], capture_output=True, text=True)
            self.assertEqual(process.returncode, 1)
            self.assertIn('output already exists', process.stderr)
            self.assertEqual(output.read_text(), 'keep')

    def test_null_post_body_is_refused_before_connection(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp); body = root/'body.json'; body.write_text('null')
            process = subprocess.run([sys.executable, str(ROOT/'deployment/local-skills/rx'), 'api',
                '--connection', str(root/'missing-connection.json'), 'post', '/api/v1/process-changes',
                '--body', str(body)], capture_output=True, text=True)
            self.assertEqual(process.returncode, 1)
            self.assertIn('JSON object', process.stderr)


if __name__ == '__main__': unittest.main()
