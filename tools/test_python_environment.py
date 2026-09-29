#!/usr/bin/env python3
"""Actual offline SDK wheel installation; not Host/device admission evidence."""
import base64
import csv
import hashlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import zipfile

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'deployment/local-skills'))
from python_environment import prepare, verify, inspect_wheel


def wheel(root, extra=None):
    path=root/'rx_fixture_sdk-1.0.0-py3-none-any.whl'
    files={'rx_fixture_sdk/__init__.py':b'def translate(value):\n    return value + 7\n',
        'rx_fixture_sdk-1.0.0.dist-info/METADATA':b'Metadata-Version: 2.1\nName: rx-fixture-sdk\nVersion: 1.0.0\n',
        'rx_fixture_sdk-1.0.0.dist-info/WHEEL':b'Wheel-Version: 1.0\nGenerator: rx-test\nRoot-Is-Purelib: true\nTag: py3-none-any\n'}
    files.update(extra or {})
    rows=[]
    for name,raw in files.items():
        rows.append([name,'sha256='+base64.urlsafe_b64encode(hashlib.sha256(raw).digest()).decode().rstrip('='),str(len(raw))])
    record='rx_fixture_sdk-1.0.0.dist-info/RECORD';rows.append([record,'',''])
    stream=io.StringIO();csv.writer(stream).writerows(rows);files[record]=stream.getvalue().encode()
    with zipfile.ZipFile(path,'w') as archive:
        for name,raw in files.items():archive.writestr(name,raw)
    return path


class Environments(unittest.TestCase):
    def test_offline_sdk_import_and_inventory_tamper(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);source=root/'source';source.mkdir()
            (source/'skill.json').write_text(json.dumps({'name':'translate','version':'1.0.0'}))
            (source/'skill.py').write_text('from rx_fixture_sdk import translate\ndef main(inputs):\n    return {"value":translate(inputs["value"])}\n')
            output=root/'prepared'
            cli=Path(__file__).resolve().parents[1]/'deployment/local-skills/rx'
            result=json.loads(subprocess.check_output([sys.executable,str(cli),'skill','prepare-environment',str(source),
                '--wheel',str(wheel(root)),'--output',str(output)],text=True))
            checked=json.loads(subprocess.check_output([sys.executable,str(cli),'skill','verify-environment',str(output),
                '--digest',result['environment_digest']],text=True))
            self.assertFalse(checked['execution_authorized'])
            self.assertFalse(result['execution_authorized'])
            verify(output,result['environment_digest'])
            # Explicit test execution, separate from preparation. No device is attached.
            code='import runpy,json; print(json.dumps(runpy.run_path('+repr(str(output/'skill.py'))+')["main"]({"value":5})))'
            value=subprocess.check_output([str(output/'venv/bin/python'),'-I','-B','-c',code],text=True)
            self.assertEqual(json.loads(value),{'value':12})
            verify(output,result['environment_digest'])
            with self.assertRaises(FileExistsError):prepare(source,[],output)
            (output/'skill.py').write_text('def main(inputs): return inputs\n')
            with self.assertRaisesRegex(ValueError,'files changed'):verify(output,result['environment_digest'])
    def test_wheel_startup_hooks_and_escaping_paths_are_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp)
            for name in ['launch.pth','sitecustomize.py','../outside.py']:
                with self.subTest(name=name):
                    with self.assertRaises(ValueError):inspect_wheel(wheel(root,{name:b'pass\n'}))


if __name__=='__main__':unittest.main()
