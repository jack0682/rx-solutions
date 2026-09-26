#!/usr/bin/env python3
"""Run P6-P8 resource aliases, syscall surfaces and late-fork controls."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
SCENES = [
    "surfaces",
    "alias-symlink",
    "alias-relative",
    "late-fork",
    "late-fork-bypass",
]


def replace_once(source, before, after):
    if source.count(before) != 1:
        raise ValueError("private mutation source anchor differs: " + before)
    return source.replace(before, after)


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
    base_inventory = json.loads(
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
    original = (ROOT / "native/dhi/guardian.c").read_text()
    summary = []
    for scene in SCENES:
        output = evidence / scene
        output.mkdir(mode=0o777)
        output.chmod(0o777)
        source = original
        define = None
        preload = ""
        if scene == "surfaces":
            define = "TEST_SURFACES"
            preload = '    setenv("LD_PRELOAD", "/mutation/probe.so", 1);\n'
        elif scene.startswith("late-fork"):
            define = "TEST_LATE_FORK"
            preload = (
                '    setenv("LD_PRELOAD", "/mutation/probe.so", 1);\n'
                '    setenv("DHI_TEST_CUSTODY", path, 1);\n'
            )
        if preload:
            source = replace_once(
                source,
                "    execl(MANAGER, MANAGER,",
                preload + "    execl(MANAGER, MANAGER,",
            )
        if scene == "late-fork-bypass":
            source = replace_once(
                source,
                "          reason = clone_disposition(q, &allow_thread);",
                "          {\n            allow_thread = 1;\n            reason = NULL;\n          }",
            )
        (output / "guardian.c").write_text(source)
        compile_lines = [
            "set -e",
            "cc -Wall -Wextra -Werror -Wno-unused-function -O2 /output/guardian.c -o /output/guardian -lutil",
        ]
        if define:
            compile_lines.append(
                "cc -Wall -Wextra -Werror -O2 -fPIC -shared "
                f"-D{define} /test-tools/dhi_r1_claim_probe.c "
                "-o /output/probe.so -ldl"
            )
        compile_command = [
            "docker",
            "run",
            "--rm",
            "--network",
            "none",
            "-v",
            str(ROOT / "tools") + ":/test-tools:ro",
            "-v",
            str(output) + ":/output",
            "--entrypoint",
            "/bin/bash",
            image,
            "-c",
            "; ".join(compile_lines),
        ]
        with (output / "compile.log").open("w") as log:
            subprocess.run(
                compile_command, stdout=log, stderr=subprocess.STDOUT, check=True
            )
        inventory = json.loads(json.dumps(base_inventory))
        inventory["files"]["bin/rx-dhi-custody"] = hashlib.sha256(
            (output / "guardian").read_bytes()
        ).hexdigest()
        inventory["files"]["tools/dhi/guardian.c"] = hashlib.sha256(
            (output / "guardian.c").read_bytes()
        ).hexdigest()
        (output / "runtime-files.json").write_text(json.dumps(inventory))
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
            str(output) + ":/mutation:ro",
            "-v",
            str(output / "guardian") + ":/opt/rx/bin/rx-dhi-custody:ro",
            "-v",
            str(output / "guardian.c") + ":/opt/rx/tools/dhi/guardian.c:ro",
            "-v",
            str(output / "runtime-files.json")
            + ":/opt/rx/manifests/runtime-files.json:ro",
            "--entrypoint",
            "/bin/bash",
            image,
            "-c",
            "source /opt/ros/jazzy/setup.bash && source /opt/rx/dhi/setup.bash && exec python3 /test-tools/dhi_r1_component.py "
            + scene,
        ]
        (output / "command.json").write_text(json.dumps(command, indent=2))
        with (output / "stdout.log").open("w") as stdout, (output / "stderr.log").open(
            "w"
        ) as stderr:
            result = subprocess.run(command, stdout=stdout, stderr=stderr, timeout=90)
        (output / "exit.json").write_text(json.dumps({"exit_code": result.returncode}))
        stderr_text = (output / "stderr.log").read_text()
        if scene == "late-fork":
            assert '"r1_probe":"grant_fd_cloexec","result":1' in stderr_text
            assert '"r1_probe":"late_fork","result":-1,"errno":1' in stderr_text
        if scene == "late-fork-bypass":
            assert '"r1_probe":"grant_fd_cloexec","result":1' in stderr_text
            assert '"r1_probe":"inherited_fd_write","result":1' in stderr_text
        item = {"scene": scene, "exit_code": result.returncode}
        summary.append(item)
        print(json.dumps(item), flush=True)
    (evidence / "summary.json").write_text(json.dumps(summary, indent=2))
    raise SystemExit(any(item["exit_code"] for item in summary))


if __name__ == "__main__":
    main()
