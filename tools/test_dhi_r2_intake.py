#!/usr/bin/env python3
"""Run R2 measurements with current R1 guardian, without using R2 as authority."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--image", required=True)
    parser.add_argument("--evidence", required=True, type=Path)
    args = parser.parse_args()
    evidence = args.evidence.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    image = json.loads(
        subprocess.check_output(["docker", "image", "inspect", args.image])
    )[0]["Id"]
    build = evidence / "build"
    build.mkdir(mode=0o777)
    build.chmod(0o777)
    compile_command = [
        "docker",
        "run",
        "--rm",
        "--network",
        "none",
        "-v",
        str(ROOT / "native/dhi") + ":/source:ro",
        "-v",
        str(build) + ":/output",
        "--entrypoint",
        "/usr/bin/cc",
        image,
        "-Wall",
        "-Wextra",
        "-Werror",
        "-O2",
        "/source/guardian.c",
        "-o",
        "/output/guardian",
        "-lutil",
    ]
    subprocess.run(compile_command, check=True)
    (build / "guardian").chmod(0o755)
    source = (ROOT / "native/dhi/guardian.c").read_bytes()
    (build / "guardian.c").write_bytes(source)
    inventory = json.loads(
        subprocess.check_output(
            [
                "docker",
                "run",
                "--rm",
                "--network",
                "none",
                "--entrypoint",
                "/bin/cat",
                image,
                "/opt/rx/manifests/runtime-files.json",
            ]
        )
    )
    inventory["files"]["bin/rx-dhi-custody"] = hashlib.sha256(
        (build / "guardian").read_bytes()
    ).hexdigest()
    inventory["files"]["tools/dhi/guardian.c"] = hashlib.sha256(source).hexdigest()
    (build / "runtime-files.json").write_text(json.dumps(inventory))
    summary = []
    for scene in ["parameter", "late-description"]:
        output = evidence / scene
        output.mkdir(mode=0o777)
        output.chmod(0o777)
        command = [
            "docker",
            "run",
            "--rm",
            "--read-only",
            "--cap-drop",
            "ALL",
            "--security-opt",
            "no-new-privileges",
            "--security-opt",
            "seccomp=unconfined",
            "--user",
            "10001:10001",
            "--network",
            "none",
            "--tmpfs",
            "/tmp:rw,uid=10001,gid=10001",
            "--tmpfs",
            "/run/rx-solutions:rw,uid=10001,gid=10001",
            "--tmpfs",
            "/var/lib/rx-solutions:rw,uid=10001,gid=10001",
            "-v",
            str(ROOT / "tools") + ":/test-tools:ro",
            "-v",
            str(output) + ":/output",
            "-v",
            str(build / "guardian") + ":/opt/rx/bin/rx-dhi-custody:ro",
            "-v",
            str(build / "guardian.c") + ":/opt/rx/tools/dhi/guardian.c:ro",
            "-v",
            str(build / "runtime-files.json")
            + ":/opt/rx/manifests/runtime-files.json:ro",
            "--entrypoint",
            "/bin/bash",
            image,
            "-c",
            "source /opt/ros/jazzy/setup.bash && source /opt/rx/dhi/setup.bash && exec python3 /test-tools/dhi_r2_intake.py "
            + scene,
        ]
        (output / "command.json").write_text(json.dumps(command, indent=2))
        with (output / "stdout.log").open("w") as stdout, (output / "stderr.log").open(
            "w"
        ) as stderr:
            result = subprocess.run(command, stdout=stdout, stderr=stderr, timeout=60)
        (output / "exit.json").write_text(json.dumps({"exit_code": result.returncode}))
        item = {"scene": scene, "exit_code": result.returncode}
        summary.append(item)
        print(json.dumps(item), flush=True)
    (evidence / "summary.json").write_text(json.dumps(summary, indent=2))
    raise SystemExit(any(item["exit_code"] for item in summary))


if __name__ == "__main__":
    main()
