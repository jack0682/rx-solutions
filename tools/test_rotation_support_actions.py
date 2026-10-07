#!/usr/bin/env python3
"""Package behavior only; no Run admission or fresh support observation claim."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch
import uuid

SOURCE = Path(__file__).resolve().parents[1] / 'examples/process/laser-heat-treatment/rotation'


class SupportActions(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.device = root / 'device'
        self.device.mkdir()
        shutil.copyfile(SOURCE / 'skill.py', root / 'skill.py')
        (root / 'skill.json').write_text(json.dumps({'simulation': {
            'environment': 'FILE_SIMULATION', 'state_directory': str(self.device), 'initial_slots': [0]}}))
        spec = importlib.util.spec_from_file_location('rotation_support', root / 'skill.py')
        self.skill = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.skill)
        def quantity(kind, value, unit, frame=None):
            data = {'kind': kind}
            if kind == 'NUMBER': data['range'] = {'min': value, 'max': value}
            elif kind == 'VECTOR': data['ranges'] = [{'min': x, 'max': x} for x in value]
            else: data['value'] = value
            return {'value': {'unit': unit, 'data': data}, 'frame': frame}
        values = {'timeout_s': quantity('NUMBER', 30, 's'),
                  'frame': quantity('TEXT', 'cell', 'unitless'),
                  'target_position': quantity('VECTOR', [0, 0, 0], 'mm', 'cell'),
                  'approach_position': quantity('VECTOR', [0, 0, 10], 'mm', 'cell'),
                  'orientation': quantity('VECTOR', [0, 0, 0, 1], 'unitless', 'cell'),
                  'width': quantity('NUMBER', 47, 'mm'), 'force': quantity('NUMBER', 25, 'N')}
        self.inputs = {'schema': 'rx.workflow-parameters.v2', 'inputs': 'test-input-identity',
                       'templates_digest': 'test-template-identity', 'candidate': 0, 'slot': 0,
                       'node': 'acquire-support', 'task': 'test-task', 'primitive': 'acquire-support',
                       'values': values, 'done': {'observation': 'gripper.part_held',
                       'equals': quantity('BOOLEAN', True, 'unitless')['value']},
                       'on_failure': 'STOP', 'on_unknown': 'HOLD_AND_RECONCILE'}
        selection = {k: self.inputs[k] for k in ('inputs', 'templates_digest', 'candidate', 'slot')}
        self.state = {'schema': 'rx.tending-simulation-state.v1', 'environment': 'FILE_SIMULATION',
                      'phase': 'OPEN', 'clamped': True, 'holding': False, 'process_done': True,
                      'door_closed': False, 'robot_clear': True, 'active_selection': selection,
                      'active_slot': 0, 'effects': 0}
        self.save(self.state)
        self.ids = {'RX_HOST_OPERATION_ID': str(uuid.uuid4()), 'RX_HOST_INVOCATION_ID': str(uuid.uuid4())}

    def save(self, state):
        (self.device / 'state.json').write_text(json.dumps(state))

    def invoke(self, action):
        data = copy.deepcopy(self.inputs)
        data.update(node=action, primitive=action)
        if action in ('withdraw', 'unload'): data['done']['observation'] = 'jig.part_unloaded'
        with patch.dict(os.environ, self.ids): return self.skill.main(data)

    def test_acquire_does_not_unclamp_and_withdraw_waits(self):
        result = self.invoke('acquire-support')
        state = self.skill.read(self.device / 'state.json')
        self.assertTrue(state['clamped'] and state['holding'])
        self.assertFalse(state['robot_clear'])
        self.assertEqual(result['support_evidence'], {'basis': 'OPERATION_ORDER_ONLY', 'fresh_observation': False})
        with self.assertRaises(ValueError): self.invoke('withdraw')
        self.assertEqual(self.skill.read(self.device / 'state.json'), state)
        # Native unclamp is external to these two actions, modeled only for this unit check.
        state['clamped'] = False
        self.save(state)
        self.invoke('withdraw')
        self.assertEqual(self.skill.read(self.device / 'state.json')['phase'], 'UNLOADED')
        effects = [json.loads(line) for line in (self.device / 'effects.jsonl').read_text().splitlines()]
        self.assertEqual([e['primitive'] for e in effects], ['acquire-support', 'withdraw'])
        self.assertTrue(all(not e['support_evidence']['fresh_observation'] for e in effects))

    def test_wrong_selection_cannot_acquire(self):
        self.inputs['slot'] = 1
        with self.assertRaises(ValueError): self.invoke('acquire-support')
        self.assertFalse((self.device / 'effects.jsonl').exists())

    def test_withdraw_without_acquire_is_rejected(self):
        self.state['clamped'] = False
        self.save(self.state)
        with self.assertRaises(ValueError): self.invoke('withdraw')
        self.assertFalse((self.device / 'effects.jsonl').exists())

    def test_historical_unload_unchanged(self):
        result = self.invoke('unload')
        state = self.skill.read(self.device / 'state.json')
        self.assertTrue(state['holding'] and state['robot_clear'])
        self.assertFalse(state['clamped'])
        self.assertNotIn('support_evidence', result)


if __name__ == '__main__': unittest.main()
