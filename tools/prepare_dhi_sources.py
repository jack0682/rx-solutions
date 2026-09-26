#!/usr/bin/env python3
"""Prepare exactly the three approved immutable DHI archives for an offline build.

No source is executed and upstream .repos files are never imported. Existing
outputs are checked, never replaced or repaired in place.
"""
import argparse
import hashlib
import io
import json
from pathlib import Path
import re
import tarfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def content_tree(files):
    return digest(
        b"RX-DHI-CONTENT-TREE-v1\0"
        + json.dumps(files, sort_keys=True, separators=(",", ":")).encode()
    )


def check(output, pins):
    lock = json.loads((output / "source-lock.json").read_text())
    if (
        lock.get("schema") != "rx.dhi-source-lock.v1"
        or lock.get("sources") != pins["sources"]
    ):
        raise ValueError("DHI_DEPENDENCY_PIN_MISMATCH")
    actual = {
        str(p.relative_to(output)): digest(p.read_bytes())
        for p in (output / "src").rglob("*")
        if p.is_file()
    }
    if (
        any(p.is_symlink() for p in (output / "src").rglob("*"))
        or actual != lock["files"]
    ):
        raise ValueError("DHI_SOURCE_TREE_CHANGED")
    for name, pin in pins["sources"].items():
        prefix = "src/" + name + "/"
        files = {
            path.removeprefix(prefix): value
            for path, value in actual.items()
            if path.startswith(prefix)
        }
        if content_tree(files) != pin["content_tree_sha256"]:
            raise ValueError("DHI_SOURCE_CONTENT_PIN_MISMATCH: " + name)
    return len(actual)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--archives", type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    pins = json.loads((ROOT / "native/dhi/dependencies.json").read_text())
    if set(pins["sources"]) != {
        "dynamixel_hardware_interface",
        "dynamixel_sdk",
        "dynamixel_interfaces",
    }:
        raise ValueError("DHI_DEPENDENCY_SET_UNSUPPORTED")
    for name, pin in pins["sources"].items():
        if not re.fullmatch("[0-9a-f]{64}", pin.get("content_tree_sha256", "")):
            raise ValueError("DHI_SOURCE_CONTENT_PIN_REQUIRED")
        if not re.fullmatch("[0-9a-f]{40}", pin["commit"]) or not re.fullmatch(
            "[0-9a-f]{64}", pin["archive_sha256"]
        ):
            raise ValueError("DHI_FLOATING_DEPENDENCY_REFUSED")
        if not re.fullmatch(r"ROBOTIS-GIT/[A-Za-z0-9_-]+", pin["repository"]):
            raise ValueError("DHI_SOURCE_ORIGIN_UNSUPPORTED")
    if args.check or args.output.exists():
        print(json.dumps({"verified_files": check(args.output, pins)}))
        return
    # Validate every archive and member before publishing any prepared output.
    content = {}
    for name, pin in pins["sources"].items():
        if args.archives:
            archive = (args.archives / (name + ".tar.gz")).read_bytes()
        else:
            url = (
                "https://codeload.github.com/"
                + pin["repository"]
                + "/tar.gz/"
                + pin["commit"]
            )
            with urllib.request.urlopen(url, timeout=60) as response:
                archive = response.read(64 * 1024 * 1024 + 1)
        if digest(archive) != pin["archive_sha256"]:
            raise ValueError("DHI_ARCHIVE_CONTENT_MISMATCH: " + name)
        with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as tar:
            for member in tar.getmembers():
                parts = Path(member.name).parts
                if len(parts) < 2 or member.isdir():
                    continue
                if (
                    not member.isfile()
                    or member.name.startswith("/")
                    or ".." in parts
                    or member.size > 16 * 1024 * 1024
                ):
                    raise ValueError("DHI_ARCHIVE_MEMBER_UNSUPPORTED")
                relative = Path("src") / name / Path(*parts[1:])
                if relative in content:
                    raise ValueError("DHI_ARCHIVE_DUPLICATE_PATH")
                content[relative] = tar.extractfile(member).read()
    args.output.mkdir(parents=True, exist_ok=False)
    for relative, data in content.items():
        path = args.output / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    lock = {
        "schema": "rx.dhi-source-lock.v1",
        "sources": pins["sources"],
        "files": {str(path): digest(data) for path, data in sorted(content.items())},
    }
    (args.output / "source-lock.json").write_text(
        json.dumps(lock, sort_keys=True, indent=2) + "\n"
    )
    print(json.dumps({"verified_files": check(args.output, pins)}))


if __name__ == "__main__":
    main()
