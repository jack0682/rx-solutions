"""Offline SDK environment preparation. Does not grant Host execution authority."""
import ast
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import platform
import stat
import subprocess
import sys
import venv
import zipfile


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()


def read_regular(path, limit):
    if path.is_symlink() or not path.is_file() or path.stat().st_size > limit:
        raise ValueError("regular bounded file required: " + str(path))
    raw = path.read_bytes()
    if len(raw) > limit:
        raise ValueError("file grew beyond supported size")
    return raw


def inspect_wheel(path):
    raw = read_regular(path, 128 * 1024 * 1024)
    if path.suffix != ".whl":
        raise ValueError("supply built wheels; source distributions are not installed")
    with zipfile.ZipFile(io.BytesIO(raw)) as archive:
        entries = archive.infolist()
        if len(entries) > 20000 or sum(v.file_size for v in entries) > 512 * 1024 * 1024:
            raise ValueError("wheel content exceeds supported size")
        seen = set()
        for entry in entries:
            name = PurePosixPath(entry.filename)
            if entry.filename in seen or name.is_absolute() or ".." in name.parts or "\\" in entry.filename:
                raise ValueError("wheel contains duplicate or escaping paths")
            seen.add(entry.filename)
            mode = entry.external_attr >> 16
            if stat.S_ISLNK(mode):
                raise ValueError("wheel symlink is not supported")
            if name.suffix == ".pth" or name.name in ("sitecustomize.py", "usercustomize.py"):
                raise ValueError("wheel startup hooks are not allowed before Host admission")
            if name.parts and name.parts[0] in ("pip", "pip.py"):
                raise ValueError("SDK wheel must not replace the environment installer")
        if len([n for n in seen if n.endswith(".dist-info/WHEEL")]) != 1:
            raise ValueError("one wheel metadata directory required")
    return raw


def inventory(root):
    files = {}
    for path in sorted(root.rglob("*")):
        relative = path.relative_to(root).as_posix()
        if relative == "environment.json":
            continue
        if path.is_symlink():
            files[relative] = {"symlink": os.readlink(path)}
        elif path.is_file():
            files[relative] = {"sha256": digest(path.read_bytes()), "size": path.stat().st_size,
                               "executable": bool(path.stat().st_mode & 0o111)}
        elif not path.is_dir():
            raise ValueError("special file in Python environment")
    return files


def prepare(source, wheel_paths, output):
    source = Path(source).absolute()
    output = Path(output).absolute()
    specification = read_regular(source / "skill.json", 65536)
    metadata = json.loads(specification)
    if not isinstance(metadata, dict) or not metadata.get("name") or not metadata.get("version"):
        raise ValueError("skill name and version are required")
    code = read_regular(source / "skill.py", 1024 * 1024)
    tree = ast.parse(code)
    if not any(isinstance(n, ast.FunctionDef) and n.name == "main" for n in tree.body):
        raise ValueError("skill.py must define main(inputs)")
    if len(wheel_paths) > 64:
        raise ValueError("at most 64 wheels are supported")
    wheels = {}
    for value in wheel_paths:
        path = Path(value)
        if path.name in wheels:
            raise ValueError("duplicate wheel filename")
        wheels[path.name] = inspect_wheel(path)
    if sum(len(raw) for raw in wheels.values()) > 512 * 1024 * 1024:
        raise ValueError("total wheel input exceeds 512 MiB")
    # Exclusive destination: an interrupted build has no ready manifest and is never reused.
    output.mkdir(parents=True, exist_ok=False, mode=0o700)
    (output / "skill.py").write_bytes(code)
    (output / "skill.json").write_bytes(specification)
    wheelhouse = output / "wheels"; wheelhouse.mkdir()
    for name, raw in wheels.items():
        (wheelhouse / name).write_bytes(raw)
    environment = output / "venv"
    venv.EnvBuilder(with_pip=True, symlinks=False).create(environment)
    python = environment / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
    if wheels:
        command = [str(python), "-I", "-B", "-m", "pip", "--isolated", "--disable-pip-version-check",
                   "install", "--no-index", "--no-deps", "--no-compile", *[str(wheelhouse / n) for n in sorted(wheels)]]
        result = subprocess.run(command, capture_output=True, text=True, timeout=180,
                                env={"PATH": os.defpath, "PIP_CONFIG_FILE": os.devnull})
        if result.returncode:
            raise ValueError("offline SDK installation failed: " + result.stderr[-3000:])
    body = {"schema": "rx.python-environment.v1", "path": str(output),
            "skill": {"name": metadata["name"], "version": metadata["version"]},
            "python": {"version": sys.version, "platform": sys.platform, "machine": platform.machine(),
                       "base_executable": str(Path(sys.executable).resolve()),
                       "base_executable_sha256": digest(Path(sys.executable).read_bytes())},
            "wheels": {n: digest(raw) for n, raw in sorted(wheels.items())}, "files": inventory(output),
            "execution_authorized": False, "base_runtime_release_verified": False}
    identity = digest(encoded(body))
    record = {"environment_digest": identity, "environment": body}
    with (output / "environment.json").open("xb") as stream:
        stream.write(encoded(record)); stream.flush(); os.fsync(stream.fileno())
    return {"status": "PYTHON_ENVIRONMENT_PREPARED", "path": str(output), "environment_digest": identity,
            "execution_authorized": False}


def verify(root, expected_digest):
    root = Path(root).absolute()
    record = json.loads(read_regular(root / "environment.json", 16 * 1024 * 1024))
    body = record["environment"]
    if record["environment_digest"] != expected_digest or digest(encoded(body)) != expected_digest:
        raise ValueError("Python environment identity differs")
    if body["path"] != str(root) or body["files"] != inventory(root):
        raise ValueError("Python environment files changed or environment moved")
    if digest(Path(body["python"]["base_executable"]).read_bytes()) != body["python"]["base_executable_sha256"]:
        raise ValueError("base Python executable changed")
    return {"status": "PYTHON_ENVIRONMENT_INVENTORY_MATCHES", "environment_digest": expected_digest,
            "execution_authorized": False, "base_runtime_release_verified": False}
