#!/usr/bin/env python3
"""Real subprocess/SDK journal boundaries; separate from P/Host admission tests."""
import hashlib
import json
import fcntl
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest
import uuid

from test_python_environment import wheel
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'deployment/local-skills'))
from python_environment import prepare

RUNNER=Path(__file__).resolve().parents[1]/'deployment/local-skills/host_runner.py'


class RunnerTests(unittest.TestCase):
    def setup_skill(self, root, code):
        source=root/'source';source.mkdir()
        (source/'skill.json').write_text('{"name":"sdk-operation","version":"1.0.0"}')
        (source/'skill.py').write_text(code)
        sdk=b'import os\ndef record(path):\n with open(path,"a") as f:\n  f.write("effect\\n");f.flush();os.fsync(f.fileno())\n return 7\n'
        built=prepare(source,[wheel(root,{'rx_fixture_sdk/__init__.py':sdk})],root/'env')
        state=root/'journal';state.mkdir()
        request={'schema':'rx.python-host-request.v1','operation':str(uuid.uuid4()),'invocation':str(uuid.uuid4()),
            'device_session':str(uuid.uuid4()),'dispatch_clock':'MONOTONIC','dispatch_deadline_ns':str(2**63-1),'intent_digest':'a'*64,'environment':built['path'],'environment_digest':built['environment_digest'],
            'input':{'path':str(root/'effects')}}
        return state,request
    def call(self, action, state, request, expected=0):
        result=subprocess.run([sys.executable,'-I','-S','-B',str(RUNNER),action,str(state)],
            input=json.dumps(request),text=True,capture_output=True,timeout=20)
        self.assertEqual(result.returncode,expected,result.stderr)
        return json.loads(result.stdout) if expected==0 else result.stderr
    def test_returned_output_and_repeat_preserve_one_sdk_effect(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);state,r=self.setup_skill(root,'from rx_fixture_sdk import record\ndef main(inputs):\n return {"value":record(inputs["path"])}\n')
            self.assertEqual(self.call('lookup',state,r)['status'],'NOT_OBSERVED')
            self.assertFalse(list(state.iterdir()))
            first=self.call('execute',state,r)
            self.assertEqual(first['status'],'RETURNED');self.assertEqual(first['output'],{'value':7})
            self.assertEqual(first,self.call('execute',state,r))
            self.assertEqual(first,self.call('lookup',state,r))
            self.assertEqual((root/'effects').read_text(),'effect\n')
            r['invocation']=str(uuid.uuid4())
            self.assertIn('original Python execution request differs',self.call('execute',state,r,1))
    def test_owned_entry_precedes_five_second_completion_and_saved_entry_cannot_reenter(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);state,r=self.setup_skill(root,'from rx_fixture_sdk import record\nimport time\ndef main(inputs):\n record(inputs["path"])\n time.sleep(5)\n return {}\n')
            raw=json.dumps(r)
            child=subprocess.Popen([sys.executable,'-I','-S','-B',str(RUNNER),'execute-entered',str(state)],
                stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
            try:
                started=time.monotonic()
                child.stdin.write(raw);child.stdin.close()
                entry=json.loads(child.stdout.readline())
                self.assertLess(time.monotonic()-started,3)
                self.assertIsNone(child.poll())
                self.assertEqual(entry['schema'],'rx.python-native-entry.v2')
                self.assertEqual(entry['request_sha256'],hashlib.sha256(raw.encode()).hexdigest())
                self.assertEqual(entry,json.loads((state/r['operation']/'entry.json').read_bytes()))
                self.assertEqual(self.call('lookup',state,r)['status'],'UNKNOWN')
                completed=json.loads(child.stdout.read());child.wait(timeout=10)
                self.assertEqual(child.returncode,0,child.stderr.read())
                self.assertGreaterEqual(time.monotonic()-started,5)
                self.assertEqual(completed['status'],'RETURNED')
                self.assertEqual(self.call('execute-entered',state,r),completed)
                self.assertEqual((root/'effects').read_text(),'effect\n')
            finally:
                if child.poll() is None:child.kill();child.wait(timeout=5)
                child.stdout.close();child.stderr.close()

    def test_sigkill_after_sdk_effect_never_replays_unknown(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);state,r=self.setup_skill(root,'from rx_fixture_sdk import record\nimport time\ndef main(inputs):\n record(inputs["path"])\n time.sleep(60)\n return {}\n')
            child=subprocess.Popen([sys.executable,'-I','-S','-B',str(RUNNER),'execute',str(state)],
                stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
            try:
                child.stdin.write(json.dumps(r));child.stdin.close()
                deadline=time.monotonic()+15
                while not (root/'effects').exists() and time.monotonic()<deadline:
                    if child.poll() is not None:self.fail(child.stderr.read())
                    time.sleep(.02)
                self.assertTrue((root/'effects').exists())
                os.kill(child.pid,signal.SIGKILL);child.wait(timeout=5)
                self.assertEqual(self.call('lookup',state,r)['status'],'UNKNOWN')
                self.assertEqual(self.call('execute',state,r)['status'],'UNKNOWN')
                self.assertEqual((root/'effects').read_text(),'effect\n')
            finally:
                if child.poll() is None:child.kill();child.wait(timeout=5)
                child.stdout.close();child.stderr.close()
    def test_busy_before_marker_is_unknown_and_does_not_execute(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);state,r=self.setup_skill(root,'def main(inputs): return {}\n')
            journal=state/r['operation'];journal.mkdir()
            with (journal/'owner.lock').open('w') as lock:
                fcntl.flock(lock,fcntl.LOCK_EX)
                self.assertEqual(self.call('execute',state,r)['status'],'UNKNOWN')
                self.assertFalse((journal/'request.json').exists())

    def test_expired_dispatch_does_not_import_or_execute_sdk(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);state,r=self.setup_skill(root,'from rx_fixture_sdk import record\ndef main(inputs): return {"value":record(inputs["path"])}\n')
            r['dispatch_deadline_ns']='0'
            self.assertIn('dispatch deadline elapsed',self.call('execute',state,r,1))
            self.assertFalse((root/'effects').exists())
            self.assertFalse((state/r['operation']/'request.json').exists())

    def test_sdk_exception_after_effect_is_unknown(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);state,r=self.setup_skill(root,'from rx_fixture_sdk import record\ndef main(inputs):\n record(inputs["path"])\n raise RuntimeError("device reply lost")\n')
            result=self.call('execute',state,r)
            self.assertEqual(result['status'],'UNKNOWN')
            self.assertEqual(result,self.call('execute',state,r))
            self.assertEqual((root/'effects').read_text(),'effect\n')


if __name__=='__main__':unittest.main()
