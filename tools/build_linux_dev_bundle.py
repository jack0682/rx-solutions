#!/usr/bin/env python3
"""Export a source-pinned Linux development installer; do not publish or install it."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import gzip

ROOT = Path(__file__).resolve().parents[1]


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args])


def export(root, output, ref="HEAD"):
    commit = git(root, "rev-parse", "--verify", ref + "^{commit}").decode().strip()
    archive = git(root, "archive", commit)
    with tarfile.open(fileobj=io.BytesIO(archive)) as stream:
        for item in stream.getmembers():
            path = Path(item.name)
            if path.is_absolute() or ".." in path.parts or not (item.isfile() or item.isdir()):
                raise ValueError("Source archive contains unsupported member: " + item.name)
        stream.extractall(output, filter="data")
    return commit


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", type=Path, required=True)
    parser.add_argument("--platform-ref", default="HEAD", help="Exact Platform revision to export")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--architecture", choices=["amd64", "arm64"], default="amd64")
    parser.add_argument("--images-state", type=Path, help="Include verified source-matching local Docker images")
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    bundle = out / "rx-linux-dev"
    bundle.mkdir()
    sources = {}
    for name, root in [("platform", args.platform.resolve()), ("solutions", ROOT)]:
        sources[name] = export(root, bundle / "sources" / name, args.platform_ref if name == "platform" else "HEAD")
    # Release contents come from the exported commit, never an uncommitted overlay.
    for file in (bundle / "sources/solutions/deployment/linux-dev").iterdir():
        if file.is_file() and file.suffix != ".pyc":
            shutil.copyfile(file, bundle / file.name)
    for name in ["LICENSE", "NOTICE"]:
        shutil.copyfile(bundle / "sources/solutions" / name, bundle / name)
    for name in ["rx-dev", "install.sh"]:
        (bundle / name).chmod(0o755)
    images = {}
    if args.images_state:
        installed = json.loads((args.images_state / "installation.json").read_text())
        if installed["architecture"] != args.architecture:
            raise ValueError("Installed architecture differs")
        images = installed["images"]
        if set(images) != {"platform", "solutions"}:
            raise ValueError("Exactly Platform and Solutions images are required")
        for role, image in images.items():
            inspected = json.loads(subprocess.check_output(["docker", "image", "inspect", image], text=True))[0]
            if inspected["Id"] != image or inspected["Architecture"] != args.architecture or inspected["Os"] != "linux":
                raise ValueError("Image identity or architecture differs")
            holder = subprocess.check_output(["docker", "create", "--network", "none", "--entrypoint", "/bin/true", image], text=True).strip()
            try:
                manifest_path = "/usr/local/bin/source.sha256" if role == "platform" else "/opt/rx/bin/source.sha256"
                destination = out / (role + "-image-source.sha256")
                subprocess.run(["docker", "cp", holder + ":" + manifest_path, str(destination)], check=True)
                for line in destination.read_text().splitlines():
                    digest, name = line.split("  ", 1)
                    relative = name.removeprefix("/source/")
                    file = bundle / "sources" / role / relative
                    if relative == name or hashlib.sha256(file.read_bytes()).hexdigest() != digest:
                        raise ValueError("Image/source mismatch: " + name)
            finally:
                subprocess.run(["docker", "rm", holder], check=True, stdout=subprocess.DEVNULL)
        with (bundle / "runtime-images.tar.gz").open("wb") as raw:
            child = subprocess.Popen(["docker", "save", *images.values()], stdout=subprocess.PIPE)
            with gzip.GzipFile(fileobj=raw, mode="wb", mtime=0) as compressed:
                shutil.copyfileobj(child.stdout, compressed)
            child.stdout.close()
            if child.wait():
                raise RuntimeError("Docker image export failed")
    files = {}
    for file in sorted(bundle.rglob("*")):
        if file.is_file():
            with file.open("rb") as stream:
                files[str(file.relative_to(bundle))] = hashlib.file_digest(stream, "sha256").hexdigest()
    manifest = {"schema": "rx.linux-dev-bundle.v1", "architecture": args.architecture,
                "sources": sources, "files": files, "profile": "FILE_SIMULATION",
                "physical_execution": "NOT_SUPPORTED", "binary_images_included": bool(images), "images": images,
                "packaging": "installer and sources exported from the pinned commits"}
    (bundle / "bundle.json").write_text(json.dumps(manifest, indent=2) + "\n")
    target = out / f"rx-linux-dev-{sources['platform'][:7]}-{sources['solutions'][:7]}-{args.architecture}.tar.gz"
    with tarfile.open(target, "w:gz") as stream:
        stream.add(bundle, arcname=bundle.name)
    with target.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    (out / "CHECKSUMS.sha256").write_text(f"{digest}  {target.name}\n")
    print(json.dumps({"archive": str(target), "sha256": digest, "sources": sources}, indent=2))


if __name__ == "__main__":
    main()
