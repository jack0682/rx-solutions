#!/usr/bin/env python3
"""Private native implementation mutations, distinct from registered admission.

Changed binaries/inventories are mounted only in disposable test containers.
The original signed image and product source are never changed. These direct
component tests intentionally do not claim authenticated resident admission.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def change(source, before, after):
    assert source.count(before) == 1, before
    return source.replace(before, after)


def rows(text):
    result = []
    for line in text.splitlines():
        try:
            result.append(json.loads(line))
        except ValueError:
            pass
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--image", required=True)
    parser.add_argument("--evidence", required=True, type=Path)
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=False)
    root = args.evidence.resolve()
    image = json.loads(
        subprocess.check_output(["docker", "image", "inspect", args.image])
    )[0]["Id"]
    source = (ROOT / "native/dhi/guardian.c").read_text()
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
    source_hash = hashlib.sha256(source.encode()).hexdigest()
    assert (
        inventory["files"]["tools/dhi/guardian.c"] == source_hash
    ), "fixture must match image source"
    (root / "source-and-image.json").write_text(
        json.dumps(
            {
                "image": image,
                "source_sha256": source_hash,
                "scope": "DIRECT_COMPONENT_MUTATION_NOT_REGISTERED_RELEASE_ADMISSION",
            },
            indent=2,
        )
    )
    injection = '    setenv("LD_PRELOAD", "/mutation/probe.so", 1);\n    setenv("DHI_TEST_ENDPOINT", path, 1);\n'
    hooked = change(
        source, "    execl(MANAGER, MANAGER,", injection + "    execl(MANAGER, MANAGER,"
    )
    peer_bad, count = re.subn(
        r"keeper\s*=\s*ioctl\(master, TIOCGPTPEER,.*?\);",
        "errno = ENOTTY; keeper = -1;",
        source,
        flags=re.S,
    )
    assert count == 1
    variants = {
        "excess-refused": (hooked, "excess", True),
        "excess-bypass": (
            change(hooked, "grants >= 2", "grants >= 3"),
            "excess",
            False,
        ),
        "guardian-write": (
            change(
                source,
                "  pass(observer, master, path);",
                '  if (write(keeper, "X", 1) != 1) fail("TEST_WRITE_FAILED");\n  pass(observer, master, path);',
            ),
            None,
            False,
        ),
        "peer-unsupported": (peer_bad, None, False),
        "fence-bypass": (change(source, "ioctl(keeper, TIOCEXCL)", "0"), None, False),
    }
    summary = []
    for name, (code, hook, success) in variants.items():
        output = root / name
        output.mkdir(mode=0o777)
        output.chmod(0o777)
        (output / "guardian.c").write_text(code)
        compile_command = [
            "docker",
            "run",
            "--rm",
            "--network",
            "none",
            "--user",
            "10001:10001",
            "-v",
            str(output) + ":/output",
            "-v",
            str(ROOT / "tools") + ":/test-tools:ro",
            "--entrypoint",
            "/bin/bash",
            image,
            "-c",
            "set -e; cc -Wall -Wextra -Werror -Wno-unused-function -O2 /output/guardian.c -o /output/guardian -lutil; "
            "cc -Wall -Wextra -Werror -O2 /test-tools/dhi_write_observer.c -o /output/observer; "
            + (
                (
                    "cc -Wall -Wextra -Werror -O2 -fPIC -shared "
                    "/test-tools/dhi_claim_probe.c -o /output/probe.so -ldl"
                )
                if hook
                else "true"
            ),
        ]
        with (output / "compile.log").open("w") as log:
            subprocess.run(
                compile_command, stdout=log, stderr=subprocess.STDOUT, check=True
            )
        changed = json.loads(json.dumps(inventory))
        changed["files"]["bin/rx-dhi-custody"] = hashlib.sha256(
            (output / "guardian").read_bytes()
        ).hexdigest()
        (output / "inventory.json").write_text(json.dumps(changed))
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
            str(output) + ":/mutation:ro",
            "-v",
            str(output / "guardian") + ":/opt/rx/bin/rx-dhi-custody:ro",
            "-v",
            str(output / "inventory.json") + ":/opt/rx/manifests/runtime-files.json:ro",
            "-e",
            "ROS_LOG_DIR=/tmp/dhi-ros-log",
            "--entrypoint",
            "/bin/bash",
            image,
            "-c",
            "source /opt/ros/jazzy/setup.bash && source /opt/rx/dhi/setup.bash && exec /output/observer /usr/bin/python3 /test-tools/dhi_passage.py component-custody",
        ]
        (output / "command.json").write_text(json.dumps(command, indent=2))
        with (output / "stdout.log").open("w") as stdout, (output / "stderr.log").open(
            "w"
        ) as stderr:
            result = subprocess.run(command, stdout=stdout, stderr=stderr, timeout=60)
        stdout = (output / "stdout.log").read_text()
        stderr = (output / "stderr.log").read_text()
        observed = rows(stderr)
        (output / "exit.json").write_text(json.dumps({"exit_code": result.returncode}))
        assert (result.returncode == 0) == success, (
            name,
            result.returncode,
            stderr[-1000:],
        )
        if hook:
            probe = next(r for r in observed if r.get("probe") == "excess_grant")
            assert probe["opened"] is (not success), (name, probe)
            if success:
                assert "DHI_REOPEN_BUDGET_EXCEEDED" in stdout
        if name == "guardian-write":
            assert result.returncode == 42
            assert observed[-1]["terminal_write_attempts"] == 1
        if name == "peer-unsupported":
            assert (
                "DHI_PEER_FD_UNSUPPORTED" in stderr
                and '"event": "custody"' not in stdout
            )
        if name == "fence-bypass":
            assert "DHI_FENCE_BROKEN" in stderr
        summary.append(
            {
                "case": name,
                "expected_control_satisfied": True,
                "returncode": result.returncode,
                "observer": observed[-1],
            }
        )
        print(json.dumps(summary[-1]), flush=True)
    summary.append(
        {
            "case": "forked-new-open-tgid-membership",
            "classification": "REDUNDANT_AFTER_PROCESS_FORK_DENIAL",
            "executed_as_independent_control": False,
            "evidence": "P8_LATE_FORK_AND_LATE_FORK_BYPASS",
        }
    )
    print(json.dumps(summary[-1]), flush=True)
    assert (
        hashlib.sha256((ROOT / "native/dhi/guardian.c").read_bytes()).hexdigest()
        == source_hash
    )
    (root / "summary.json").write_text(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
