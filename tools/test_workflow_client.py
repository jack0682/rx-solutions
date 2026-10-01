#!/usr/bin/env python3
"""Verify workflow CLI request custody and response correlation without devices."""
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'deployment/local-skills'))
from workflow_client import Workflows, quantity
from runtime_client import RuntimeRejected


def ref(identifier):
    return {'catalog': '00000000-0000-4000-8000-000000000010',
            'id': identifier, 'revision': '1', 'digest': 'a' * 64}


class Server:
    def __init__(self):
        self.principal = 'engineer'
        self.fingerprint = 'workflow-test-server'
        self.saved = {}
        self.calls = []
        self.allowed = True
        self.drop = False
        self.alter = lambda value: value

    def request(self, route, body):
        self.calls.append(copy.deepcopy((route, body)))
        if not self.allowed:
            raise RuntimeRejected(403, {'code': 'FORBIDDEN'})
        key = body['request_key']
        if key not in self.saved:
            command = body['command']
            if route == '/api/v1/workflow-models':
                result = {'reference': ref(command['id']), 'spec': command['spec'],
                          'label': command['label'], 'updated_by': self.principal}
            else:
                result = {'reference': ref(str(uuid.uuid4())), 'created_by': self.principal,
                          'report': {'schema': 'rx.workflow-resolution.v1', 'request': command,
                                     'valid': True, 'concrete': True, 'violations': [],
                                     'status': 'RESOLVED_NOT_QUALIFIED'}}
            self.saved[key] = copy.deepcopy((route, body, result))
            if self.drop:
                self.drop = False
                raise OSError('lost committed reply')
        previous_route, previous_body, result = self.saved[key]
        if (route, body) != (previous_route, previous_body):
            raise RuntimeRejected(409, {'code': 'KEY_CONFLICT'})
        return self.alter(copy.deepcopy(result))

    def get(self, route, **query):
        if not self.allowed:
            raise RuntimeRejected(403, {'code': 'FORBIDDEN'})
        return copy.deepcopy(next(v[2] for v in self.saved.values()
                                  if v[2]['reference']['id'] == query['id']))


class WorkflowClientTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.server = Server()
        self.client = Workflows(self.server, Path(self.directory.name))
        self.key = str(uuid.uuid4())
        self.request = {'workflow': ref('00000000-0000-4000-8000-000000000011'),
                        'contexts': {}, 'property_sets': [], 'overrides': {},
                        'inputs': {}, 'slot_index': '0'}

    def test_lost_reply_recovers_original_request_and_rechecks_access(self):
        self.server.drop = True
        with self.assertRaises(OSError):
            self.client.resolve(self.request, self.key)
        restarted = Workflows(self.server, Path(self.directory.name))
        result = restarted.recover(self.key)
        self.assertEqual(restarted.resolve(self.request, self.key), result)
        self.assertEqual(len(self.server.saved), 1)
        self.assertTrue(all(call == self.server.calls[0] for call in self.server.calls))
        self.server.allowed = False
        with self.assertRaises(RuntimeRejected):
            restarted.recover(self.key)

    def test_changed_intent_and_connection_are_refused_before_send(self):
        self.client.resolve(self.request, self.key)
        changed = copy.deepcopy(self.request)
        changed['slot_index'] = '1'
        with self.assertRaises(ValueError):
            self.client.resolve(changed, self.key)
        self.server.fingerprint = 'another-installation'
        with self.assertRaisesRegex(ValueError, 'connection differs'):
            self.client.recover(self.key)
        self.assertEqual(len(self.server.calls), 1)

    def test_mismatched_receipts_are_not_journaled(self):
        def changed(value, path, replacement):
            target = value
            for key in path[:-1]:
                target = target[key]
            target[path[-1]] = replacement
            return value
        cases = [(('created_by',), 'other-author'),
                 (('reference', 'catalog'), str(uuid.uuid4())),
                 (('reference', 'revision'), '2'),
                 (('report', 'request', 'slot_index'), '1'),
                 (('report', 'status'), 'PUBLISHED'),
                 (('report', 'valid'), False),
                 (('report', 'concrete'), 'true'),
                 (('report', 'violations'), [{'location': 'nodes/pick'}])]
        for path, replacement in cases:
            with self.subTest(path=path):
                request_id = str(uuid.uuid4())
                self.server.alter = lambda v: changed(v, path, replacement)
                with self.assertRaises(ValueError):
                    self.client.resolve(self.request, request_id)
                self.assertFalse((Path(self.directory.name) / request_id / 'workflow-reply.json').exists())

    def test_report_read_allows_another_author_but_requires_exact_reference(self):
        result = self.client.resolve(self.request, self.key)
        self.server.principal = 'authorized-reader'
        self.assertEqual(self.client.report(result['reference']), result)
        changed = copy.deepcopy(result['reference'])
        changed['digest'] = 'b' * 64
        with self.assertRaisesRegex(ValueError, 'stored resolution differs'):
            self.client.report(changed)

    def test_model_apply_replay_and_recovery_preserve_spec(self):
        package = {'schema': 'rx.workflow-model-package.v1',
                   'catalog': self.request['workflow']['catalog'],
                   'id': self.request['workflow']['id'], 'expected': None,
                   'label': 'Test workflow', 'spec': {'schema': 'rx.workflow-model.v1'}}
        self.server.drop = True
        with self.assertRaises(OSError):
            self.client.apply(package, {}, self.key)
        recovered = self.client.recover(self.key)
        receipt = self.client.apply(package, {}, self.key)
        self.assertEqual(receipt['workflow'], recovered['reference'])
        self.assertEqual(len(self.server.saved), 1)
        self.server.alter = lambda v: {**v, 'spec': {'schema': 'changed'}}
        with self.assertRaisesRegex(ValueError, 'save receipt differs'):
            self.client.recover(self.key)

    def test_explicit_units_false_zero_ranges_and_nonfinite_inputs(self):
        self.assertEqual(quantity('false:unitless')['data'], {'kind': 'BOOLEAN', 'value': False})
        self.assertEqual(quantity('0:N')['data']['range'], {'min': 0, 'max': 0})
        self.assertEqual(quantity('20..40:N')['data']['range'], {'min': 20, 'max': 40})
        self.assertEqual(len(quantity('[1,2,3]:mm')['data']['ranges']), 3)
        for value in ['60', '60:', 'NaN:N', '0..Infinity:N', '[true]:mm']:
            with self.subTest(value=value), self.assertRaises(ValueError):
                quantity(value)


if __name__ == '__main__':
    unittest.main()
