#!/usr/bin/env python3
"""Release-pinned L3 service-generation gate for the AI Worker simulation."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import resource
import signal
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, HTTPServer

ROOT = Path("/opt/rx")
SERVICE = "ai-worker/2.2.7/l3-simulation"
SOURCE_FILES = ("tools/ai-worker/l3_guard.py", "tools/ai-worker/dependencies.json")


class Refusal(Exception):
    pass


def append_event(state, event, **fields):
    row = {
        "schema": "rx.l3-supervision-observation.v1",
        "service_generation": SERVICE,
        "event": event,
        "recorded_at_ns": time.monotonic_ns(),
        "confirmed_stop": False,
        "physical_qualification": "NOT_PERFORMED",
        **fields,
    }
    descriptor = os.open(
        state / "observations.jsonl", os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600
    )
    with os.fdopen(descriptor, "a") as output:
        output.write(json.dumps(row, sort_keys=True) + "\n")
        output.flush()
        os.fsync(output.fileno())
    print(json.dumps(row, sort_keys=True), flush=True)
    return row


def installed_content():
    try:
        inventory = json.loads((ROOT / "manifests/runtime-files.json").read_text())
        pins = json.loads((ROOT / "tools/ai-worker/dependencies.json").read_text())
    except (OSError, ValueError):
        raise Refusal("AI_WORKER_RELEASE_METADATA_UNAVAILABLE") from None
    if (
        pins.get("schema") != "rx.ai-worker-dependencies.v1"
        or pins.get("ai_worker", {}).get("version") != "2.2.7"
        or pins.get("ai_worker", {}).get("commit")
        != "897ef342ff0e1b2ef59d6afbb5195b410d57b85f"
        or pins.get("upstream_floating_refs") != "REJECTED_BY_RX_SOURCE_PINS"
    ):
        raise Refusal("AI_WORKER_DEPENDENCY_PIN_MISMATCH")
    for relative in SOURCE_FILES:
        try:
            actual = hashlib.sha256((ROOT / relative).read_bytes()).hexdigest()
        except OSError:
            raise Refusal("release/content-mismatch: " + relative) from None
        if inventory.get("files", {}).get(relative) != actual:
            raise Refusal("release/content-mismatch: " + relative)
    return pins


def atomic_json(path, value):
    temporary = path.with_suffix(".tmp")
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "w") as output:
            output.write(json.dumps(value, sort_keys=True) + "\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise


def next_generation(state):
    path = state / "generation.json"
    try:
        current = json.loads(path.read_text())["generation"]
    except FileNotFoundError:
        current = 0
    generation = current + 1
    atomic_json(path, {"generation": generation})
    return generation


def inspect_requirement(requirement):
    installed_content()
    if requirement == "zed":
        resources = Path("/usr/local/zed/resources")
        devices = list(Path("/dev").glob("video*"))
        if not resources.is_dir() or not any(resources.iterdir()) or not devices:
            raise Refusal("AI_WORKER_ZED_ASSETS_OR_DEVICE_UNAVAILABLE")
        result = "AI_WORKER_ZED_ASSETS_AND_DEVICE_PRESENT_UNQUALIFIED"
    else:
        soft, hard = resource.getrlimit(resource.RLIMIT_RTPRIO)
        status = Path("/proc/self/status").read_text()
        effective = int(
            next(line.split()[1] for line in status.splitlines() if line.startswith("CapEff:")),
            16,
        )
        if soft < 99 or hard < 99 or not effective & (1 << 23):
            raise Refusal("AI_WORKER_RT_AUTHORITY_UNAVAILABLE")
        result = "AI_WORKER_RT_AUTHORITY_PRESENT_UNQUALIFIED"
    print(
        json.dumps(
            {
                "schema": "rx.ai-worker-requirement-inspection.v1",
                "requirement": requirement,
                "result": result,
                "physical_qualification": "NOT_PERFORMED",
            },
            sort_keys=True,
        )
    )


def child_command():
    code = """import signal,time
stop=False
signal.signal(signal.SIGTERM,lambda *_:globals().__setitem__('stop',True))
while not stop: time.sleep(.05)
"""
    return [sys.executable, "-I", "-B", "-c", code]


def serve(state, owner, port):
    installed_content()
    state.mkdir(parents=True, exist_ok=True)
    lock = (state / "owner.lock").open("a+")
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        append_event(
            state,
            "L3_SECOND_SUPERVISOR_REFUSED_ACTIVE_OWNER",
            attempted_owner=owner,
            disposition="BLOCKED",
        )
        raise SystemExit(73)
    fence = state / "rx-stop-fence.json"
    if fence.exists():
        append_event(
            state,
            "L3_FOREIGN_RESTART_AFTER_RX_STOP_REFUSED",
            attempted_owner=owner,
            disposition="BLOCKED",
            stop_fence=json.loads(fence.read_text()),
        )
        raise SystemExit(74)
    generation = next_generation(state)
    child = subprocess.Popen(child_command(), env={"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"})
    append_event(
        state,
        "L3_SERVICE_GENERATION_STARTED",
        owner=owner,
        generation=generation,
        gate_pid=os.getpid(),
        child_pid=child.pid,
        disposition="ADMITTED",
    )
    stopping = threading.Event()
    signal.signal(signal.SIGTERM, lambda *_: stopping.set())
    signal.signal(signal.SIGINT, lambda *_: stopping.set())

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_GET(self):
            if self.path not in ("/health", "/status"):
                self.send_error(404)
                return
            body = json.dumps(
                {
                    "schema": "rx.ai-worker-l3-status.v1",
                    "service_generation": SERVICE,
                    "owner": owner,
                    "generation": generation,
                    "child_pid": child.pid,
                    "child_running": child.poll() is None,
                    "l3_owner_definition": "AUTHORITY_TO_CREATE_REPLACEMENT_ROOT_PROCESS_TREE_FOR_STABLE_SERVICE_GENERATION",
                    "zed_assets": "NOT_USED_BY_SIMULATION",
                    "rt_authority": "NOT_USED_BY_SIMULATION",
                    "physical_qualification": "NOT_PERFORMED",
                    "work_permission": "NOT_GRANTED",
                }
            ).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    server = HTTPServer(("127.0.0.1", port), Handler)
    server.timeout = 0.1
    while not stopping.is_set() and child.poll() is None:
        server.handle_request()
    if stopping.is_set():
        stop = {
            "requested_by": "RX",
            "owner": owner,
            "generation": generation,
            "stop_response": "REQUEST_ACCEPTED",
            "confirmed_stop": False,
            "reason": "L3_EXTERNAL_RESTART_CAPABILITY_UNRECONCILED",
        }
        atomic_json(fence, stop)
        append_event(state, "L3_STOP_REQUEST_ACCEPTED", **stop)
        child.terminate()
        try:
            child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait()
        append_event(
            state,
            "L3_OWNED_PROCESS_TREE_EXIT_OBSERVED",
            owner=owner,
            generation=generation,
            child_exit=child.returncode,
            disposition="STOP_UNCONFIRMED",
            reason="L3_EXTERNAL_RESTART_CAPABILITY_UNRECONCILED",
        )
        return
    append_event(
        state,
        "L3_OWNED_PROCESS_TREE_EXIT_UNEXPECTED",
        owner=owner,
        generation=generation,
        child_exit=child.returncode,
        disposition="UNKNOWN",
    )
    raise SystemExit(75)


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    inspect = sub.add_parser("inspect")
    inspect.add_argument("requirement", choices=("zed", "rt"))
    run = sub.add_parser("serve")
    run.add_argument("--state", required=True, type=Path)
    run.add_argument("--owner", required=True, choices=("rx", "compose"))
    run.add_argument("--port", required=True, type=int)
    args = parser.parse_args()
    if args.command == "inspect":
        inspect_requirement(args.requirement)
    elif not 1024 <= args.port <= 65535:
        raise Refusal("AI_WORKER_L3_PORT_INVALID")
    else:
        serve(args.state, args.owner, args.port)


if __name__ == "__main__":
    try:
        main()
    except Refusal as error:
        print(
            json.dumps(
                {
                    "schema": "rx.ai-worker-refusal.v1",
                    "condition": str(error),
                    "current_permission": "NOT_GRANTED",
                    "physical_qualification": "NOT_PERFORMED",
                },
                sort_keys=True,
            ),
            file=sys.stderr,
        )
        raise SystemExit(76) from None
