#!/usr/bin/env python3
"""Build a source-pinned local-simulation installer bundle; never publish it."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parents[1]


def command(*args, **kwargs):
    return subprocess.check_output(args, text=True, **kwargs).strip()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--platform", required=True, type=Path)
    p.add_argument("--output", required=True, type=Path)
    p.add_argument("--architecture", choices=("arm64", "amd64"), required=True)
    p.add_argument("--version", default="0.4.0-dev.1")
    args = p.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    image = "rx-local-skills:" + args.version + "-" + args.architecture
    subprocess.run(["docker", "build", "--platform", "linux/" + args.architecture,
                    "--build-context", "platform=" + str(args.platform.resolve()),
                    "-f", str(ROOT / "docker/Skills.Dockerfile"), "-t", image, str(ROOT)], check=True)
    bundle = args.output / "bundle"
    shutil.copytree(ROOT / "deployment/local-skills", bundle,
                    ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
    for name in ("LICENSE", "NOTICE"):
        shutil.copyfile(ROOT / name, bundle / name)
    (bundle / "rx").chmod(0o755)
    archive = bundle / "runtime.tar.gz"
    import gzip
    with archive.open("wb") as target:
        child = subprocess.Popen(["docker", "save", image], stdout=subprocess.PIPE)
        with gzip.GzipFile(fileobj=target, mode="wb", mtime=0) as stream:
            shutil.copyfileobj(child.stdout, stream)
        child.stdout.close()
        if child.wait(): raise RuntimeError("docker save failed")
    manifest = {"schema": "rx.local-sim.release.v1", "version": args.version,
                "architecture": args.architecture, "image": image,
                "image_id": command("docker", "image", "inspect", image, "--format", "{{.Id}}"),
                "image_sha256": hashlib.file_digest(archive.open("rb"), "sha256").hexdigest(),
                "platform_commit": command("git", "-C", str(args.platform), "rev-parse", "HEAD"),
                "solutions_commit": command("git", "-C", str(ROOT), "rev-parse", "HEAD"),
                "source_dirty": bool(command("git", "-C", str(ROOT), "status", "--porcelain") or command("git", "-C", str(args.platform), "status", "--porcelain")),
                "physical_execution": "NOT_SUPPORTED"}
    (bundle / "release.json").write_text(json.dumps(manifest, indent=2) + "\n")
    name = "rx-local-skills-" + args.version + "-linux-" + args.architecture + ".tar.gz"
    with tarfile.open(args.output / name, "w:gz") as stream:
        stream.add(bundle, arcname="rx-local-skills")
    digest = hashlib.file_digest((args.output / name).open("rb"), "sha256").hexdigest()
    (args.output / "CHECKSUMS.sha256").write_text(digest + "  " + name + "\n")
    print(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    main()
