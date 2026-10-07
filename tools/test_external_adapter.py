#!/usr/bin/env python3
"""Actual private-process SDK requests; no P authority or full Run acceptance claim."""
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import uuid

SDK = Path(__file__).resolve().parents[1] / 'deployment/external-adapters/rx_external_adapter.py'


@unittest.skipUnless(sys.platform == 'linux', 'provider clock is Linux CLOCK_BOOTTIME')
class ExternalAdapter(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.state = self.root / 'native'
        self.state.mkdir()
        self.session = str(uuid.uuid4())
        (self.state / 'device-session').write_text(self.session)
        spec = importlib.util.spec_from_file_location('external_sdk', SDK)
        self.sdk = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.sdk)
        self.program = self.root / 'adapter.py'
        self.program.write_text('import importlib.util\nfrom pathlib import Path\n'
            f'spec=importlib.util.spec_from_file_location("sdk",{str(SDK)!r})\n'
            'sdk=importlib.util.module_from_spec(spec);spec.loader.exec_module(sdk)\n'
            'class Adapter:\n'
            ' def execute(self,envelope,correlation):\n'
            f'  with open({str(self.root / "effects")!r},"a") as f: f.write(correlation["operation"]+"\\n")\n'
            '  return {"status_schema":"fixture/completed","status":0}\n'
            ' def observe(self,sources): return {s:sdk.sample({"boolean":True}) for s in sources}\n'
            ' def custody(self): return {k:True for k in ("no_pending_commands","control_available","support_stable","safe_to_drop")}\n'
            'sdk.serve(Adapter())\n')
        now = self.sdk.now()
        self.request = {'schema': 'rx.external-process-channel.v1', 'challenge': str(uuid.uuid4()),
                        'profile_digest': 'a'*64, 'device_session': self.session, 'now': now,
                        'sources': [], 'dispatch': {
                            'operation': str(uuid.uuid4()), 'invocation': str(uuid.uuid4()),
                            'intent': {}, 'input': {'parameters': list(b'{"primitive":"echo"}'),
                            'binding': {'selection': {'intent_digest': 'b'*64}}},
                            'device_session': self.session, 'admitted_at': now,
                            'expires_at': {**now, 'ticks_ns': str(int(now['ticks_ns'])+10_000_000_000)}}}

    def call(self, mode, request):
        return subprocess.run([sys.executable, '-I', '-S', '-B', str(self.program), mode, str(self.state)],
                              input=self.sdk.encoded(request), capture_output=True, timeout=15)

    def test_entry_completion_and_lookup_keep_original_fact_without_reissue(self):
        result = self.call('execute', self.request)
        self.assertEqual(result.returncode, 0, result.stderr)
        entry, complete = map(json.loads, result.stdout.splitlines())
        self.assertEqual(entry['operation'], self.request['dispatch']['operation'])
        self.assertEqual(complete['capture']['native_id'], self.request['dispatch']['invocation'])
        query = copy.deepcopy(self.request)
        query['challenge'] = str(uuid.uuid4())
        query['now'] = self.sdk.now()
        found = self.call('lookup', query)
        self.assertEqual(found.returncode, 0, found.stderr)
        self.assertEqual(json.loads(found.stdout)['capture'], complete['capture'])
        repeat = self.call('execute', query)
        self.assertNotEqual(repeat.returncode, 0)
        self.assertEqual((self.root / 'effects').read_text().splitlines(), [entry['operation']])

    def test_foreign_invocation_cannot_read_original_completion(self):
        self.assertEqual(self.call('execute', self.request).returncode, 0)
        query = copy.deepcopy(self.request)
        query['dispatch']['invocation'] = str(uuid.uuid4())
        self.assertNotEqual(self.call('lookup', query).returncode, 0)
        self.assertEqual(len((self.root / 'effects').read_text().splitlines()), 1)

    def test_observation_is_passive_and_preserves_sample_timestamp(self):
        query = copy.deepcopy(self.request)
        query.update(dispatch=None, sources=['ready'])
        observed = self.call('observe', query)
        self.assertEqual(observed.returncode, 0, observed.stderr)
        sample = json.loads(observed.stdout)['samples']['ready']
        self.assertEqual(sample['value'], {'boolean': True})
        self.assertEqual(sample['acquired_at']['clock_id'], self.sdk.now()['clock_id'])
        self.assertFalse((self.root / 'effects').exists())
        self.assertEqual([p.name for p in self.state.iterdir()], ['device-session'])


if __name__ == '__main__': unittest.main()
