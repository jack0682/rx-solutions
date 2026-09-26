#!/usr/bin/env python3
"""Run retained, actual-process DHI PTY evidence in a supplied release image."""
import argparse
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
SCENES = [
    "resident-loss",
    "resident-normal",
    "resident-trace",
    "arguments",
    "fault-model",
    "fault-firmware",
]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--image", required=True)
    parser.add_argument("--evidence", required=True, type=Path)
    parser.add_argument("--scene", choices=SCENES + ["all"], default="all")
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=False)
    evidence = args.evidence.resolve()
    image = json.loads(
        subprocess.check_output(["docker", "image", "inspect", args.image])
    )[0]["Id"]
    (evidence / "image.json").write_text(
        json.dumps({"tag": args.image, "id": image}, indent=2)
    )
    results = []
    for scene in SCENES if args.scene == "all" else [args.scene]:
        output = evidence / scene
        output.mkdir(mode=0o777)
        output.chmod(0o777)
        (output / "data").mkdir(mode=0o777)
        (output / "data").chmod(0o777)
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
            str(output) + ":/output",
            "-v",
            str(output / "data") + ":/var/lib/rx-solutions",
            "--entrypoint",
            "/bin/bash",
            image,
            "-c",
            'source /opt/ros/jazzy/setup.bash && source /opt/rx/dhi/setup.bash && exec python3 /test-tools/dhi_passage.py "$1"',
            "dhi-test",
            scene,
        ]
        (output / "command.json").write_text(json.dumps(command, indent=2))
        with (output / "stdout.log").open("w") as stdout, (output / "stderr.log").open(
            "w"
        ) as stderr:
            result = subprocess.run(command, stdout=stdout, stderr=stderr, timeout=120)
        (output / "exit.json").write_text(json.dumps({"exit_code": result.returncode}))
        results.append({"scene": scene, "exit_code": result.returncode})
        print(json.dumps(results[-1]), flush=True)
    (evidence / "summary.json").write_text(json.dumps(results, indent=2))
    raise SystemExit(any(r["exit_code"] != 0 for r in results))


if __name__ == "__main__":
    main()
