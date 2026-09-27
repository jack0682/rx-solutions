"""Serial local-simulation worker. Results go through the RX state writer."""
import json
import os
from pathlib import Path
import selectors
import signal
import subprocess
import sys
import tempfile
import time
import urllib.request
import urllib.error
import uuid


def api(endpoint, token, path, value):
    req = urllib.request.Request(endpoint + path, json.dumps(value).encode(),
                                 {"Authorization": "Bearer " + token,
                                  "Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=5) as reply:
        return json.load(reply)


def execute(run, skill):
    started = time.monotonic()
    print(json.dumps({"event": "EXECUTION_STARTED", "run": run["request"]["request_id"]}), flush=True)
    result = {"run": run["request"]["request_id"], "worker": run["worker"],
              "outcome": "FAILED", "output": None, "error": None, "duration_ms": 0}
    package = skill["package"]
    with tempfile.TemporaryDirectory(prefix="rx-skill-") as temporary:
        root = Path(temporary)
        (root / "skill.py").write_text(package["code"], encoding="utf-8")
        (root / "input.json").write_text(json.dumps(run["request"]["input"]), encoding="utf-8")
        child = subprocess.Popen([sys.executable, "-I", "-S", "-u",
                                  str(Path(__file__).with_name("runner.py")),
                                  str(root / "skill.py"), str(root / "input.json")],
                                 stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                 stderr=subprocess.PIPE, cwd=root, start_new_session=True,
                                 env={"PATH": "/usr/local/bin:/usr/bin:/bin", "HOME": temporary})
        streams = {child.stdout: bytearray(), child.stderr: bytearray()}
        issue = None
        try:
            with selectors.DefaultSelector() as selector:
                for stream in streams:
                    selector.register(stream, selectors.EVENT_READ)
                deadline = started + package["timeout_ms"] / 1000
                while selector.get_map():
                    if time.monotonic() >= deadline:
                        issue = "execution deadline exceeded; result unknown"
                        break
                    for key, _ in selector.select(min(.1, max(0, deadline-time.monotonic()))):
                        chunk = os.read(key.fileobj.fileno(), 8192)
                        if not chunk:
                            selector.unregister(key.fileobj)
                            continue
                        streams[key.fileobj].extend(chunk)
                        if len(streams[key.fileobj]) > 65_536:
                            issue = "output limit exceeded; result unknown"
                            break
                    if issue:
                        break
        finally:
            if not issue:
                try:
                    child.wait(timeout=1)
                except subprocess.TimeoutExpired:
                    issue = "process completion unconfirmed; result unknown"
            # The process group belongs to this exact child, never a saved PID.
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait(timeout=5)
            for stream in streams:
                stream.close()
        if issue:
            result.update(outcome="UNKNOWN", error=issue)
        elif child.returncode:
            # Exit only describes this software computation, not physical state.
            result["error"] = bytes(streams[child.stderr]).decode("utf-8", "replace")[-3500:] or "skill process failed"
        else:
            try:
                output = json.loads(streams[child.stdout], parse_constant=lambda _: (_ for _ in ()).throw(ValueError("non-finite JSON")))
                validate_output(package["outputs"], output)
                result.update(outcome="SUCCEEDED", output=output)
            except (ValueError, TypeError) as exc:
                result["error"] = "invalid skill output: " + str(exc)[:2000]
    result["duration_ms"] = int((time.monotonic() - started) * 1000)
    return result


def validate_output(spec, output):
    if not isinstance(output, dict) or set(output) != set(spec):
        raise ValueError("output fields differ from declared schema")
    types = {"string": lambda v: isinstance(v, str),
             "integer": lambda v: type(v) is int,
             "number": lambda v: type(v) in (int, float),
             "boolean": lambda v: type(v) is bool,
             "object": lambda v: isinstance(v, dict),
             "array": lambda v: isinstance(v, list)}
    for name, kind in spec.items():
        if not types[kind](output[name]):
            raise ValueError(name + " requires " + kind)
    if len(json.dumps(output, ensure_ascii=False).encode()) > 65_536:
        raise ValueError("output exceeds 64 KiB")


def main():
    endpoint, token_file, state = sys.argv[1:]
    token = Path(token_file).read_text().strip()
    state = Path(state)
    state.mkdir(parents=True, exist_ok=True)
    pending = state / "pending-result.json"
    previous = state / "worker-id"
    # Retry a captured immutable receipt before reporting continuity loss.
    if pending.exists():
        try:
            api(endpoint, token, "/internal/finish", json.loads(pending.read_text()))
            pending.rename(state / ("settled-" + str(uuid.uuid4()) + ".json"))
        except urllib.error.HTTPError as exc:
            if exc.code not in (400, 409):
                raise
            pending.rename(state / ("unresolved-" + str(uuid.uuid4()) + ".json"))
    if previous.exists():
        api(endpoint, token, "/internal/abandon", previous.read_text().strip())
    boot = str(uuid.uuid4())
    durable(previous, boot)
    while True:
        claim = api(endpoint, token, "/internal/claim", boot)
        if claim is None:
            time.sleep(.2)
            continue
        result = execute(*claim)
        durable(pending, json.dumps(result, allow_nan=False))
        while True:
            try:
                api(endpoint, token, "/internal/finish", result)
                pending.rename(state / ("settled-" + result["run"] + ".json"))
                break
            except urllib.error.HTTPError:
                raise
            except (OSError, urllib.error.URLError):
                time.sleep(1)


def durable(path, text):
    temporary = path.with_suffix(".new")
    with temporary.open("w", encoding="utf-8") as stream:
        stream.write(text)
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, path)
    fd = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


if __name__ == "__main__":
    main()
