#!/usr/bin/env python3
"""Exercise a clean installed local-simulation release using an external skill."""
import argparse
from http.client import HTTPConnection
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
import urllib.error
import uuid


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--bundle", required=True, type=Path)
    p.add_argument("--evidence", required=True, type=Path)
    args = p.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=False)
    checks = []
    with tempfile.TemporaryDirectory(prefix="rx-installed-acceptance-") as temp:
        root = Path(temp)
        state = root / "state"
        env = dict(os.environ, RX_SKILLS_HOME=str(state))
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0)); port = sock.getsockname()[1]
        cli = [sys.executable, str(args.bundle.resolve() / "rx")]
        def call(*words, code=0):
            r = subprocess.run([*cli, *words], env=env, capture_output=True, text=True, timeout=100)
            assert r.returncode == code, (words, r.returncode, r.stdout, r.stderr)
            return r.stdout
        def http(path, data=None, token=True, expected=200):
            headers = {"Content-Type": "application/json"}
            if token: headers["Authorization"] = "Bearer " + (state / "client-token").read_text().strip()
            req = urllib.request.Request("http://127.0.0.1:" + str(port) + path,
                                         None if data is None else json.dumps(data).encode(), headers)
            try:
                with urllib.request.urlopen(req, timeout=10) as r:
                    assert r.status == expected
                    return json.load(r)
            except urllib.error.HTTPError as exc:
                assert exc.code == expected, (path, exc.code, exc.read())
        def done(id):
            deadline = time.monotonic() + 15
            while True:
                r = http("/v1/runs/" + id)
                if r["operation"]["outcome"] != "NONE": return r
                assert time.monotonic() < deadline, r
                time.sleep(.1)
        def register(name, code, outputs=None, timeout=2000):
            directory = root / name; directory.mkdir()
            manifest = {"name": name, "version": "1.0.0", "environment": "LOCAL_SIM",
                        "inputs": {"values": "array"}, "outputs": outputs or {"total": "number", "receipt": "string"}, "timeout_ms": timeout}
            (directory / "skill.json").write_text(json.dumps(manifest))
            (directory / "skill.py").write_text(code)
            return json.loads(call("skill", "add", str(directory)))
        c = None
        try:
            command = ["sh", str(args.bundle.resolve() / "install.sh"), "--bundle", str(args.bundle.resolve()),
                       "--prefix", str(root / "prefix"), "--port", str(port)]
            r = subprocess.run(command, env=env, capture_output=True, text=True, timeout=100)
            assert r.returncode == 0, (r.stdout, r.stderr)
            c = json.loads((state / "installation.json").read_text())
            assert (root / "prefix/bin/rx").is_symlink()
            cli = [str(root / "prefix/bin/rx")]
            checks.append("clean-installer-and-cli-link")
            http("/v1/skills", token=False, expected=401)
            http("/internal/claim", str(uuid.uuid4()), expected=401)
            checks.append("client-worker-authentication-separation")
            skill = register("external-sum", "import uuid\ndef main(inputs):\n    return {'total': sum(inputs['values']), 'receipt': str(uuid.uuid4())}\n")
            request_id = str(uuid.uuid4())
            original = json.loads(call("run", "external-sum", "--input", '{"values":[2,4,8]}', "--request-id", request_id))
            assert original["output"]["total"] == 14
            replay = json.loads(call("run", "external-sum", "--input", '{"values":[2,4,8]}', "--request-id", request_id))
            assert replay == original
            logs = subprocess.check_output(["docker", "logs", c["name"] + "-worker"], text=True)
            assert logs.count(request_id) == 1, logs
            checks.append("external-code-executed-once-and-original-request-replayed")
            changed = dict(original["request"], input={"values": [9]})
            http("/v1/runs", changed, expected=409)
            altered = dict(skill["package"], code="def main(x): return {}")
            http("/v1/skills", altered, expected=409)
            physical = dict(skill["package"], name="physical", environment="PHYSICAL")
            http("/v1/skills", physical, expected=400)
            checks.append("key-version-and-physical-scope-refusals")
            lost_id = str(uuid.uuid4())
            body = {"request_id": lost_id, "skill": "external-sum", "version": "1.0.0", "input": {"values": [5, 6]}}
            conn = HTTPConnection("127.0.0.1", port, timeout=10)
            conn.request("POST", "/v1/runs", json.dumps(body), {"Authorization": "Bearer " + (state / "client-token").read_text().strip(), "Content-Type": "application/json"})
            assert conn.getresponse().status == 200
            conn.close()  # Deliberately discard the committed response body.
            lost = done(lost_id)
            assert lost["output"]["total"] == 11
            assert http("/v1/runs", body) == lost
            checks.append("lost-admission-body-recovered-by-original-id")
            register("broken", "def main(inputs):\n    raise ValueError('intentional failure')\n")
            failure = json.loads(call("run", "broken", "--input", '{"values":[]}', code=2))
            assert failure["operation"]["outcome"] == "FAILED" and "intentional failure" in failure["error"]
            register("wrong-output", "def main(inputs): return {'total': 'not numeric', 'receipt': 'x'}\n")
            assert json.loads(call("run", "wrong-output", "--input", '{"values":[]}', code=2))["operation"]["outcome"] == "FAILED"
            register("slow", "import time\ndef main(inputs):\n    time.sleep(10)\n    return {'total': 0, 'receipt': 'late'}\n", timeout=100)
            assert json.loads(call("run", "slow", "--input", '{"values":[]}', code=2))["operation"]["outcome"] == "UNRESOLVED"
            checks.append("real-exception-output-schema-and-timeout")
            call("down"); call("up")
            assert json.loads(call("result", request_id)) == original
            checks.append("persistent-installation-restart")
            register("interrupted", "import time\ndef main(inputs):\n    time.sleep(10)\n    return {'total': 1, 'receipt': 'late'}\n", timeout=20000)
            interrupted = str(uuid.uuid4())
            call("run", "interrupted", "--input", '{"values":[]}', "--request-id", interrupted, "--no-wait")
            deadline = time.monotonic() + 10
            while http("/v1/runs/" + interrupted)["started_ms"] is None:
                assert time.monotonic() < deadline
                time.sleep(.1)
            subprocess.run(["docker", "kill", c["name"] + "-worker"], check=True, capture_output=True)
            call("up")
            assert done(interrupted)["operation"]["outcome"] == "UNRESOLVED"
            assert json.loads(call("run", "external-sum", "--input", '{"values":[7]}'))["output"]["total"] == 7
            checks.append("killed-worker-quarantined-original-and-accepted-distinct-next-run")
            for role in ("server", "worker"):
                item = json.loads(subprocess.check_output(["docker", "inspect", c["name"] + "-" + role]))[0]
                assert item["Config"]["User"] == "10001:10001"
                assert item["HostConfig"]["ReadonlyRootfs"] and not item["HostConfig"]["Devices"]
                assert all(m["Destination"] != "/var/run/docker.sock" for m in item["Mounts"])
            checks.append("non-root-readonly-no-devices-no-docker-socket")
            call("down")
            def data_command(code):
                return subprocess.run(["docker", "run", "--rm", "--network", "none", "--read-only",
                                       "-v", c["name"] + "-server-data:/data", c["image"], "python3", "-c", code],
                                      check=True, capture_output=True, text=True)
            data_command("from pathlib import Path; Path('/data/skills.db').rename('/data/preserved.db')")
            refused = subprocess.run(["docker", "start", "--attach", c["name"] + "-server"], capture_output=True, text=True, timeout=20)
            assert refused.returncode != 0 and "database missing" in refused.stderr, refused
            data_command("from pathlib import Path; assert not Path('/data/skills.db').exists(); Path('/data/preserved.db').rename('/data/skills.db')")
            call("up")
            assert json.loads(call("result", request_id)) == original
            checks.append("lost-database-refused-and-preserved-original-restored")
            call("down")
            subprocess.run(["docker", "rm", c["name"] + "-server"], check=True, capture_output=True)
            subprocess.run(["docker", "volume", "rm", c["name"] + "-server-data"], check=True, capture_output=True)
            missing = subprocess.run([*cli, "up"], env=env, capture_output=True, text=True, timeout=20)
            assert missing.returncode == 1 and "volume missing" in missing.stderr, missing
            assert subprocess.run(["docker", "volume", "inspect", c["name"] + "-server-data"], capture_output=True).returncode != 0
            checks.append("lost-volume-refused-without-recreating-ledger")
            result = {"status": "PASS", "checks": checks, "release": json.loads((args.bundle / "release.json").read_text()),
                      "normal_result": original, "physical_execution": "NOT_PERFORMED"}
            (args.evidence / "result.json").write_text(json.dumps(result, indent=2) + "\n")
            print(json.dumps({"status": "PASS", "checks": checks}))
        finally:
            if c is None and (state / "installation.json").exists(): c = json.loads((state / "installation.json").read_text())
            if c:
                for role in ("worker", "server"):
                    r = subprocess.run(["docker", "logs", c["name"] + "-" + role], capture_output=True, text=True)
                    (args.evidence / (role + ".log")).write_text(r.stdout + r.stderr)
                    subprocess.run(["docker", "rm", "-f", c["name"] + "-" + role], capture_output=True)
                subprocess.run(["docker", "network", "rm", c["name"]], capture_output=True)
                subprocess.run(["docker", "network", "rm", c["name"] + "-web"], capture_output=True)
                for role in ("worker", "server"):
                    for kind in ("config", "data"):
                        subprocess.run(["docker", "volume", "rm", c["name"] + "-" + role + "-" + kind], capture_output=True)


if __name__ == "__main__":
    main()
