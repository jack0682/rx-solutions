#!/usr/bin/env python3
"""Build and run the pre-release R1 discriminating P5 fixture."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--image", required=True)
    parser.add_argument("--evidence", required=True, type=Path)
    args = parser.parse_args()
    evidence = args.evidence.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    evidence.chmod(0o777)
    image = json.loads(
        subprocess.check_output(["docker", "image", "inspect", args.image])
    )[0]["Id"]
    compile_command = [
        "docker",
        "run",
        "--rm",
        "--network",
        "none",
        "-v",
        str(ROOT / "native/dhi") + ":/source:ro",
        "-v",
        str(evidence) + ":/output",
        "--entrypoint",
        "/usr/bin/cc",
        image,
        "-Wall",
        "-Wextra",
        "-Werror",
        "-O2",
        "/source/guardian.c",
        "-o",
        "/output/rx-dhi-custody",
        "-lutil",
    ]
    subprocess.run(compile_command, check=True)
    (evidence / "rx-dhi-custody").chmod(0o755)
    shutil.copy2(ROOT / "native/dhi/guardian.c", evidence / "guardian.c")
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
        (evidence / "rx-dhi-custody").read_bytes()
    ).hexdigest()
    inventory["files"]["tools/dhi/guardian.c"] = hashlib.sha256(
        (evidence / "guardian.c").read_bytes()
    ).hexdigest()
    (evidence / "runtime-files.json").write_text(json.dumps(inventory))
    command = [
        "docker",
        "run",
        "--rm",
        "--read-only",
        "--cap-drop",
        "ALL",
        "--security-opt",
        "no-new-privileges",
        "--user",
        "10001:10001",
        "--network",
        "none",
        "--tmpfs",
        "/tmp:rw,uid=10001,gid=10001",
        "--tmpfs",
        "/run/rx-solutions:rw,uid=10001,gid=10001",
        "-v",
        str(ROOT / "tools") + ":/test-tools:ro",
        "-v",
        str(evidence) + ":/output",
        "-v",
        str(evidence / "rx-dhi-custody") + ":/opt/rx/bin/rx-dhi-custody:ro",
        "-v",
        str(evidence / "guardian.c") + ":/opt/rx/tools/dhi/guardian.c:ro",
        "-v",
        str(evidence / "runtime-files.json")
        + ":/opt/rx/manifests/runtime-files.json:ro",
        "--entrypoint",
        "/bin/bash",
        image,
        "-c",
        "source /opt/ros/jazzy/setup.bash && source /opt/rx/dhi/setup.bash && exec python3 /test-tools/dhi_r1_p5.py",
    ]
    (evidence / "command.json").write_text(json.dumps(command, indent=2))
    with (evidence / "stdout.log").open("w") as stdout, (evidence / "stderr.log").open(
        "w"
    ) as stderr:
        result = subprocess.run(command, stdout=stdout, stderr=stderr, timeout=90)
    (evidence / "exit.json").write_text(json.dumps({"exit_code": result.returncode}))
    print(json.dumps({"image": image, "exit_code": result.returncode}))
    raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
