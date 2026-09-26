#!/usr/bin/env python3
"""Verify the exact AI Worker archive and reject its floating source graph."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import re
import tarfile

ROOT = Path(__file__).resolve().parents[1]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--archive", required=True, type=Path)
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    pins = json.loads((ROOT / "native/ai-worker/dependencies.json").read_text())
    archive = args.archive.read_bytes()
    assert digest(archive) == pins["ai_worker"]["archive_sha256"]
    files = {}
    content = {}
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as source:
        for member in source.getmembers():
            parts = Path(member.name).parts
            if len(parts) < 2 or member.isdir():
                continue
            assert member.isfile() and not member.name.startswith("/") and ".." not in parts
            relative = Path(*parts[1:]).as_posix()
            data = source.extractfile(member).read()
            files[relative] = digest(data)
            content[relative] = data
    tree = digest(
        b"RX-AI-WORKER-CONTENT-TREE-v1\0"
        + json.dumps(files, sort_keys=True, separators=(",", ":")).encode()
    )
    assert tree == pins["ai_worker"]["content_tree_sha256"]
    assert len(files) == pins["ai_worker"]["file_count"] == 380
    assert files["LICENSE"] == "c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4"

    compose = content["docker/docker-compose.yml"].decode()
    assert "restart: always" in compose
    assert "- SYS_NICE" in compose and "rtprio: 99" in compose
    assert "/usr/local/zed/resources/" in compose and "privileged: true" in compose
    repos = content["ai_worker_ci.repos"].decode()
    dockerfiles = "\n".join(
        content[name].decode()
        for name in ("docker/Dockerfile.amd64", "docker/Dockerfile.arm64")
    )
    assert "version: main" in repos and "git clone -b jazzy" in dockerfiles
    required = {
        "DynamixelSDK",
        "Lakibeam_ROS2_Driver",
        "cyclo_control",
        "cyclo_manager",
        "dual_laser_merger",
        "dynamixel_hardware_interface",
        "dynamixel_interfaces",
        "librealsense",
        "realsense-ros",
        "robotis_hand",
        "robotis_interfaces",
        "zed-ros2-wrapper",
    }
    assert set(pins["git_dependencies"]) == required
    assert all(re.fullmatch(r"[0-9a-f]{40}", value) for value in pins["git_dependencies"].values())
    assert pins["upstream_floating_refs"] == "REJECTED_BY_RX_SOURCE_PINS"
    assert set(pins["unqualified_build_inputs"].values()) == {"NOT_ESTABLISHED"}
    result = {
        "schema": "rx.ai-worker-source-pin-verification.v1",
        "project": "ai_worker",
        "version": pins["ai_worker"]["version"],
        "commit": pins["ai_worker"]["commit"],
        "archive_sha256": digest(archive),
        "content_tree_sha256": tree,
        "verified_files": len(files),
        "pinned_git_dependencies": len(required),
        "upstream_floating_refs": "REJECTED_BY_RX_SOURCE_PINS",
        "upstream_restart": "always",
        "zed_and_rt_requirements_present": True,
        "physical_qualification": "NOT_PERFORMED",
    }
    if args.evidence:
        args.evidence.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
