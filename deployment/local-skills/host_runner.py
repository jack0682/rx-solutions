"""Private Python skill boundary for a Host-owned process; not a public admission API.

The Host must validate execution authority before launching execute. This journal
records Python execution facts only. It cannot establish physical completion/stop.
"""
import contextlib
import fcntl
import hashlib
import json
import os
from pathlib import Path
import sys
import uuid

# This directory and both helper files must come from the verified Host release.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from python_environment import encoded, read_regular, verify


def publish(path, value):
    raw = encoded(value)
    temporary = path.with_name(path.name + "." + uuid.uuid4().hex + ".tmp")
    try:
        with temporary.open("xb") as stream:
            stream.write(raw); stream.flush(); os.fsync(stream.fileno())
        os.link(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY)
        try: os.fsync(directory)
        finally: os.close(directory)
    finally:
        temporary.unlink(missing_ok=True)


def identifier(value):
    if str(uuid.UUID(value)) != value:
        raise ValueError("canonical execution identity required")
    return value


def validate(request):
    fields = {"schema", "operation", "invocation", "intent_digest", "environment", "environment_digest", "input"}
    if set(request) != fields or request["schema"] != "rx.python-host-request.v1":
        raise ValueError("Python Host request schema differs")
    identifier(request["operation"]); identifier(request["invocation"])
    for name in ("intent_digest", "environment_digest"):
        value = request[name]
        if not isinstance(value, str) or len(value) != 64 or any(c not in "0123456789abcdef" for c in value):
            raise ValueError("invalid execution digest")
    if len(encoded(request)) > 131072 or not isinstance(request["input"], dict):
        raise ValueError("bounded object input required")


def receipt(request, status, **values):
    return {"schema": "rx.python-host-receipt.v1", "operation": request["operation"],
            "invocation": request["invocation"], "intent_digest": request["intent_digest"],
            "environment_digest": request["environment_digest"], "status": status, **values}


def stored(root, request):
    marker = root / "request.json"
    if not marker.exists():
        return receipt(request, "NOT_OBSERVED")
    if read_regular(marker, 131072) != encoded(request):
        raise ValueError("original Python execution request differs")
    result = root / "receipt.json"
    if not result.exists():
        return receipt(request, "UNKNOWN")
    value = json.loads(read_regular(result, 131072))
    for key in ("operation", "invocation", "intent_digest", "environment_digest"):
        if value[key] != request[key]:
            raise ValueError("Python receipt identity differs")
    return value


def handle(action, state, request):
    validate(request)
    state = Path(state)
    if not state.is_absolute() or state.is_symlink() or not state.is_dir():
        raise ValueError("Host-owned absolute journal directory required")
    root = state / request["operation"]
    if root.is_symlink(): raise ValueError("execution journal symlink refused")
    if action == "lookup":
        return stored(root, request)  # Read only: never loads the SDK or writes a marker.
    if action != "execute": raise ValueError("unsupported Python Host action")
    root.mkdir(mode=0o700, exist_ok=True)
    lock = os.open(root / "owner.lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    try:
        try: fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            current = stored(root, request)
            return receipt(request, "UNKNOWN") if current["status"] == "NOT_OBSERVED" else current
        if (root / "request.json").exists():
            return stored(root, request)  # Existing UNKNOWN is not a resubmit instruction.
        environment = Path(request["environment"])
        verify(environment, request["environment_digest"])
        record = json.loads(read_regular(environment / "environment.json", 16 * 1024 * 1024))["environment"]
        if record["python"]["version"] != sys.version:
            raise ValueError("running Python version differs from prepared environment")
        if hashlib.sha256(Path(sys.executable).read_bytes()).hexdigest() != record["python"]["base_executable_sha256"]:
            raise ValueError("running interpreter differs from prepared environment")
        paths = list((environment / "venv/lib").glob("python*/site-packages"))
        if len(paths) != 1 or paths[0].is_symlink():
            raise ValueError("one Unix SDK site-packages directory required")
        code = read_regular(environment / "skill.py", 1048576)
        if hashlib.sha256(code).hexdigest() != record["files"]["skill.py"]["sha256"]:
            raise ValueError("skill source changed")
        # The fully synced marker precedes module import, which itself may call an SDK.
        publish(root / "request.json", request)
        try:
            sys.path.append(str(paths[0]))  # No .pth or site customization processing.
            with contextlib.redirect_stdout(sys.stderr):
                namespace = {"__name__": "rx_skill", "__file__": str(environment / "skill.py")}
                exec(compile(code, str(environment / "skill.py"), "exec"), namespace)
                output = namespace["main"](request["input"])
            if not isinstance(output, dict) or len(encoded(output)) > 65536:
                raise ValueError("bounded object output required")
            result = receipt(request, "RETURNED", output=output)
        except BaseException as exc:
            # A Python exception can follow a native side effect. Never label it NOT_EXECUTED.
            result = receipt(request, "UNKNOWN", error_type=type(exc).__name__)
        publish(root / "receipt.json", result)
        return result
    finally:
        os.close(lock)


if __name__ == "__main__":
    try:
        raw = sys.stdin.buffer.read(131073)
        if len(raw) > 131072: raise ValueError("request too large")
        value = handle(sys.argv[1], sys.argv[2], json.loads(raw))
        print(json.dumps(value, allow_nan=False, separators=(",", ":")), flush=True)
    except (ValueError, OSError, KeyError) as exc:
        print("Python Host boundary: " + str(exc), file=sys.stderr)
        sys.exit(1)
