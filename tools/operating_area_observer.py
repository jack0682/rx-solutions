#!/usr/bin/env python3
"""Test observer outside the real daemon. Pauses make judgment/use cuts observable."""
import json
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import sys
import time

control = Path('/control')
def publish(name, value):
    temporary = control / ('.' + name + '.pending')
    with temporary.open('w') as output:
        json.dump(value, output)
        output.flush()
        os.fsync(output.fileno())
    os.replace(temporary, control / name)

mode = sys.argv[2]
log = control / 'daemon.log'
with log.open('w') as out:
    child = subprocess.Popen([sys.argv[1], 'run', '/case/config.json', '/case/task.json'], stdout=out, stderr=subprocess.STDOUT)
    paused_request = paused_judgment = False
    try:
        deadline = time.monotonic() + 55
        result = None
        while time.monotonic() < deadline:
            assert child.poll() is None, log.read_text()
            values = []
            for line in log.read_text().splitlines():
                try:
                    values.append(json.loads(line))
                except json.JSONDecodeError:
                    pass
            for value in values:
                if value.get('schema') == 'rx.work-use-result.v1' and value.get('gate') == 'PENDING' and not paused_request:
                    paused_request = True
                    if mode not in ['natural-positive', 'unconnected', 'issuer-scope']:
                        os.kill(child.pid, signal.SIGSTOP)
                        publish('request-ready.json', value)
                        until = time.monotonic() + 20
                        while not (control / 'reply-ready').exists():
                            assert time.monotonic() < until, 'external reply timeout'
                            time.sleep(.01)
                        os.kill(child.pid, signal.SIGCONT)
                    else:
                        publish('request-ready.json', value)
                if value.get('schema') == 'rx.work-use-judgment.v1' and not paused_judgment:
                    paused_judgment = True
                    if mode in ['revoked', 'expiry-after-judgment', 'commit-lock-busy']:
                        os.kill(child.pid, signal.SIGSTOP)
                        publish('judgment-ready.json', value)
                        until = time.monotonic() + 20
                        while not (control / 'commit-ready').exists():
                            assert time.monotonic() < until, 'commit control timeout'
                            time.sleep(.01)
                        os.kill(child.pid, signal.SIGCONT)
                if value.get('schema') == 'rx.work-use-result.v1' and value.get('gate') != 'PENDING':
                    result = value
            if result:
                break
            time.sleep(.01)
        assert result, log.read_text()
        with sqlite3.connect('file:/var/lib/rx-solutions/managed/registration.db?mode=ro', uri=True) as db:
            rows = db.execute("SELECT count(*) FROM entities WHERE key LIKE 'work/%'").fetchone()[0]
        publish('result.json', {'result': result, 'work_rows': rows, 'observer_pid': os.getpid(), 'manager_pid': child.pid, 'judgment_observed': paused_judgment})
    finally:
        if child.poll() is None:
            os.kill(child.pid, signal.SIGCONT)
            child.terminate()
        child.wait(timeout=15)
    assert child.returncode == 0, log.read_text()
