#!/usr/bin/env python3
"""Build and exercise AI Worker L3 ownership against real Compose restart."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def rows(path):
    result = []
    if not path.exists():
        return result
    for line in path.read_text(errors="replace").splitlines():
        try:
            result.append(json.loads(line))
        except ValueError:
            pass
    return result


def wait_for(path, predicate, timeout=30):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        for row in rows(path):
            if predicate(row):
                return row
        time.sleep(0.05)
    raise AssertionError(f"timed out waiting for L3 evidence in {path}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-image", required=True)
    parser.add_argument("--builder", default="rust:1.98.1-slim-bookworm")
    parser.add_argument("--platform-root", required=True, type=Path)
    parser.add_argument("--encrypted-key", required=True, type=Path)
    parser.add_argument("--custody-record", required=True, type=Path)
    parser.add_argument("--ai-worker-archive", required=True, type=Path)
    parser.add_argument("--target-cache", required=True, type=Path)
    parser.add_argument("--current-image", default="rx-g9-ai-worker-l3:latest")
    parser.add_argument("--mutant-image", default="rx-g9-ai-worker-l3-admission-off:latest")
    parser.add_argument("--evidence", required=True, type=Path)
    args = parser.parse_args()
    evidence = args.evidence.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    target = args.target_cache.resolve()
    target.mkdir(parents=True, exist_ok=True)
    if target == Path("/") or not any(word in target.name.lower() for word in ("target", "cache")):
        parser.error("target cache must be an explicit target/cache directory")
    commands = []

    def run(label, command, *, expected=0, timeout=None, record=True):
        result = subprocess.run(command, capture_output=True, text=True, timeout=timeout)
        (evidence / f"{label}.stdout").write_text(result.stdout)
        (evidence / f"{label}.stderr").write_text(result.stderr)
        if record:
            commands.append(
                {
                    "label": label,
                    "argv": [str(value) for value in command],
                    "exit_code": result.returncode,
                    "expected": expected,
                }
            )
            (evidence / "commands.json").write_text(json.dumps(commands, indent=2) + "\n")
        assert result.returncode == expected, (
            label,
            result.returncode,
            result.stdout[-2000:],
            result.stderr[-2000:],
        )
        return result

    run("docker-ps-first", ["docker", "ps"])
    base = json.loads(run("base-image", ["docker", "image", "inspect", args.base_image]).stdout)[0]["Id"]
    builder = json.loads(run("builder-image", ["docker", "image", "inspect", args.builder]).stdout)[0]["Id"]
    rust_host = next(
        line.split(":", 1)[1].strip()
        for line in run(
            "builder-rust-host",
            ["docker", "run", "--rm", "--network", "none", "--entrypoint", "rustc", builder, "-vV"],
        ).stdout.splitlines()
        if line.startswith("host:")
    )
    source_hashes = {
        relative: digest(ROOT / relative)
        for relative in (
            "native/ai-worker/l3_guard.py",
            "native/ai-worker/dependencies.json",
            "runtime/rx-supervisor/src/builtin.rs",
            "runtime/rx-supervisor/src/builtin/ai_worker.rs",
            "runtime/rx-supervisor/src/supervisor.rs",
        )
    }
    pins = evidence / "source-pins.json"
    run(
        "source-pins",
        [
            sys.executable,
            str(ROOT / "tools/test_ai_worker_pins.py"),
            "--archive",
            str(args.ai_worker_archive.resolve()),
            "--evidence",
            str(pins),
        ],
    )

    build = [
        "docker",
        "run",
        "--rm",
        "--network",
        "none",
        "-v",
        f"{ROOT}:/source:ro",
        "-v",
        f"{target}:/target",
        "-w",
        "/source",
        "-e",
        "CARGO_HOME=/target/cargo-home",
        "-e",
        "CARGO_BUILD_JOBS=1",
        "-e",
        "CARGO_NET_OFFLINE=true",
        "-e",
        f"RUSTUP_TOOLCHAIN=1.98.1-{rust_host}",
        "--entrypoint",
        "cargo",
        builder,
        "build",
        "--locked",
        "-p",
        "rx-supervisor",
        "--bin",
        "rx-solutionsd",
        "--target-dir",
        "/target",
    ]
    clean = build[:]
    clean[clean.index("build")] = "clean"
    clean = clean[: clean.index("--locked")] + ["-p", "rx-supervisor", "--target-dir", "/target"]

    def sign(label, directory, version):
        output = directory / "metadata"
        command = [
            sys.executable,
            str(args.platform_root.resolve() / "tools/sign_release.py"),
            "--encrypted-key",
            str(args.encrypted_key.resolve()),
            "--custody-record",
            str(args.custody_record.resolve()),
            "--inventory",
            str(directory / "runtime-files.json"),
            "--version",
            str(version),
            "--revocation-version",
            "1",
            "--output",
            str(output),
        ]
        result = subprocess.run(command, capture_output=True, text=True, timeout=180)
        (evidence / f"{label}.stdout").write_text(result.stdout)
        (evidence / f"{label}.stderr").write_text(result.stderr)
        commands.append(
            {
                "label": label,
                "argv": [
                    sys.executable,
                    "tools/sign_release.py",
                    "--encrypted-key",
                    "<REDACTED_CUSTODY_ENVELOPE>",
                    "--custody-record",
                    str(args.custody_record.resolve()),
                    "--inventory",
                    str(directory / "runtime-files.json"),
                    "--version",
                    str(version),
                    "--revocation-version",
                    "1",
                    "--output",
                    str(output),
                ],
                "exit_code": result.returncode,
                "expected": 0,
            }
        )
        (evidence / "commands.json").write_text(json.dumps(commands, indent=2) + "\n")
        assert result.returncode == 0, (label, result.stderr)

    run("clean-before-current", clean, timeout=300)
    run("build-current-daemon", build, timeout=1800)
    current = evidence / "current"
    current.mkdir()
    shutil.copy2(target / "debug/rx-solutionsd", current / "rx-solutionsd")
    os.chmod(current / "rx-solutionsd", 0o755)
    shutil.copy2(ROOT / "native/ai-worker/l3_guard.py", current / "l3_guard.py")
    shutil.copy2(ROOT / "native/ai-worker/dependencies.json", current / "dependencies.json")

    inventory = json.loads(
        run(
            "extract-base-inventory",
            ["docker", "run", "--rm", "--entrypoint", "/bin/cat", base, "/opt/rx/manifests/runtime-files.json"],
        ).stdout
    )
    inventory["files"]["bin/rx-solutionsd"] = digest(current / "rx-solutionsd")
    inventory["files"]["tools/ai-worker/l3_guard.py"] = digest(current / "l3_guard.py")
    inventory["files"]["tools/ai-worker/dependencies.json"] = digest(current / "dependencies.json")
    (current / "runtime-files.json").write_text(json.dumps(inventory, indent=2) + "\n")
    sign("sign-current", current, 4)

    mutation_before = "    if fence.exists():\n"
    mutation_after = "    if False and fence.exists():  # G9 private admission-off control\n"
    source = (ROOT / "native/ai-worker/l3_guard.py").read_text()
    assert source.count(mutation_before) == 1
    mutant = evidence / "admission-off"
    mutant.mkdir()
    (mutant / "l3_guard.py").write_text(source.replace(mutation_before, mutation_after))
    shutil.copy2(ROOT / "native/ai-worker/dependencies.json", mutant / "dependencies.json")
    run("clean-before-mutant", clean, timeout=300)
    mutant_build = build[:]
    insertion = mutant_build.index("-w")
    mutant_build[insertion:insertion] = [
        "-v",
        f"{mutant / 'l3_guard.py'}:/source/native/ai-worker/l3_guard.py:ro",
    ]
    run("build-mutant-daemon", mutant_build, timeout=1800)
    shutil.copy2(target / "debug/rx-solutionsd", mutant / "rx-solutionsd")
    os.chmod(mutant / "rx-solutionsd", 0o755)
    mutant_inventory = json.loads(json.dumps(inventory))
    mutant_inventory["files"]["bin/rx-solutionsd"] = digest(mutant / "rx-solutionsd")
    mutant_inventory["files"]["tools/ai-worker/l3_guard.py"] = digest(mutant / "l3_guard.py")
    (mutant / "runtime-files.json").write_text(json.dumps(mutant_inventory, indent=2) + "\n")
    sign("sign-admission-off", mutant, 5)

    def image(directory, tag, label):
        (directory / "Dockerfile").write_text(
            "ARG BASE_IMAGE\n"
            "FROM ${BASE_IMAGE}\n"
            "COPY --chmod=0755 rx-solutionsd /opt/rx/bin/rx-solutionsd\n"
            "COPY l3_guard.py /opt/rx/tools/ai-worker/l3_guard.py\n"
            "COPY dependencies.json /opt/rx/tools/ai-worker/dependencies.json\n"
            "COPY runtime-files.json /opt/rx/manifests/runtime-files.json\n"
            "COPY metadata/release.json /opt/rx/manifests/release.json\n"
            "COPY metadata/revocations.json /opt/rx/manifests/revocations.json\n"
        )
        run(
            label,
            [
                "docker",
                "build",
                "--build-arg",
                f"BASE_IMAGE={args.base_image}",
                "-t",
                tag,
                str(directory),
            ],
            timeout=600,
        )
        return json.loads(run(label + "-inspect", ["docker", "image", "inspect", tag]).stdout)[0]["Id"]

    current_image = image(current, args.current_image, "build-current-image")
    mutant_image = image(mutant, args.mutant_image, "build-mutant-image")

    def requirement(name, expected):
        result = run(
            "requirement-" + name,
            [
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
                "--entrypoint",
                "/usr/bin/python3",
                current_image,
                "/opt/rx/tools/ai-worker/l3_guard.py",
                "inspect",
                name,
            ],
            expected=76,
            timeout=30,
        )
        refusal = next(json.loads(line) for line in result.stderr.splitlines() if line.startswith("{"))
        assert refusal["condition"] == expected
        return refusal

    zed = requirement("zed", "AI_WORKER_ZED_ASSETS_OR_DEVICE_UNAVAILABLE")
    rt = requirement("rt", "AI_WORKER_RT_AUTHORITY_UNAVAILABLE")

    def config(scene):
        return {
            "schema": "rx.solutions-startup.v1",
            "state_subdirectory": "g9-" + scene,
            "plan": {
                "schema": "rx.solutions-process-plan.v1",
                "id": "b9c07701-c6e7-4c82-a71c-000000000" + ("301" if scene == "preserved" else "302"),
                "environment": "SIMULATION",
                "profiles": [],
                "processes": [
                    {
                        "id": "ai-worker",
                        "program": "rx/ai-worker-l3-simulation",
                        "parameters": {"port": "18119"},
                        "depends_on": [],
                        "startup_timeout_ms": "30000",
                        "shutdown_timeout_ms": "15000",
                        "restart_limit": "0",
                        "restart_backoff_ms": "500",
                    }
                ],
            },
        }

    inspection_config = evidence / "inspection.json"
    inspection_config.write_text(json.dumps(config("preserved")))
    for label, selected_image in (("current", current_image), ("admission-off", mutant_image)):
        inspected = run(
            "signed-inspect-" + label,
            [
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
                "-v",
                f"{inspection_config}:/config.json:ro",
                "--entrypoint",
                "/opt/rx/bin/rx-solutionsd",
                selected_image,
                "inspect",
                "/config.json",
            ],
            timeout=60,
        )
        row = next(json.loads(line) for line in inspected.stdout.splitlines() if line.startswith("{"))
        assert row["schema"] == "rx.solutions-plan-inspection.v1"
        assert row["release_boundary"]["ai_worker_compose_restart_admission_verified"] is True

    def passage(scene, foreign_image):
        output = evidence / ("p-l3-preserved" if scene == "preserved" else "p-l3-admission-off")
        output.mkdir(mode=0o777)
        output.chmod(0o777)
        state = output / "state"
        state.mkdir(mode=0o777)
        state.chmod(0o777)
        (output / "startup.json").write_text(json.dumps(config(scene)))
        event_log = state / "ai-worker-l3/observations.jsonl"
        primary_name = "rx-g9-primary-" + scene
        project = "rx-g9-" + ("p" if scene == "preserved" else "n")
        compose_file = output / "compose.json"
        compose_file.write_text(
            json.dumps(
                {
                    "services": {
                        "foreign": {
                            "image": foreign_image,
                            "restart": "always",
                            "read_only": True,
                            "network_mode": "none",
                            "user": "10001:10001",
                            "cap_drop": ["ALL"],
                            "security_opt": ["no-new-privileges:true"],
                            "tmpfs": ["/tmp:rw,uid=10001,gid=10001"],
                            "volumes": [f"{state}:/var/lib/rx-solutions"],
                            "entrypoint": ["/usr/bin/python3"],
                            "command": [
                                "/opt/rx/tools/ai-worker/l3_guard.py",
                                "serve",
                                "--state",
                                "/var/lib/rx-solutions/ai-worker-l3",
                                "--owner",
                                "compose",
                                "--port",
                                "18119",
                            ],
                        }
                    }
                },
                indent=2,
            )
            + "\n"
        )
        primary_command = [
            "docker",
            "run",
            "--rm",
            "--name",
            primary_name,
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
            f"{output}:/output:ro",
            "-v",
            f"{state}:/var/lib/rx-solutions",
            "--entrypoint",
            "/opt/rx/bin/rx-solutionsd",
            current_image,
            "run",
            "/output/startup.json",
        ]
        commands.append({"label": "primary-" + scene, "argv": primary_command})
        (evidence / "commands.json").write_text(json.dumps(commands, indent=2) + "\n")
        with (output / "supervisor.stdout").open("w") as stdout, (output / "supervisor.stderr").open("w") as stderr:
            primary = subprocess.Popen(primary_command, stdout=stdout, stderr=stderr)
            try:
                started = wait_for(event_log, lambda row: row.get("event") == "L3_SERVICE_GENERATION_STARTED" and row.get("owner") == "rx")
                compose = ["docker", "compose", "-p", project, "-f", str(compose_file)]
                run("compose-up-" + scene, compose + ["up", "-d", "--no-build"], timeout=60)
                active_refusal = wait_for(event_log, lambda row: row.get("event") == "L3_SECOND_SUPERVISOR_REFUSED_ACTIVE_OWNER")
                run("primary-stop-" + scene, ["docker", "kill", "--signal", "TERM", primary_name], timeout=30)
                primary.wait(timeout=30)
                if scene == "preserved":
                    terminal = wait_for(event_log, lambda row: row.get("event") == "L3_FOREIGN_RESTART_AFTER_RX_STOP_REFUSED")
                else:
                    terminal = wait_for(
                        event_log,
                        lambda row: row.get("event") == "L3_SERVICE_GENERATION_STARTED"
                        and row.get("owner") == "compose"
                        and row.get("generation") == 2,
                    )
                container = run("compose-id-" + scene, compose + ["ps", "-q", "--all", "foreign"]).stdout.strip()
                inspected = json.loads(run("compose-inspect-" + scene, ["docker", "inspect", container]).stdout)[0]
                restart_count = inspected["RestartCount"]
                assert restart_count >= 1, restart_count
                supervisor_rows = rows(output / "supervisor.stdout")
                final = [row for row in supervisor_rows if row.get("schema") == "rx.supervisor-status.v1"][-1]
                assert final["unconfirmed_component_stops"]["ai-worker"].startswith("AI_WORKER_L3_STOP_UNCONFIRMED")
                result = {
                    "schema": "rx.ai-worker-l3-passage.v1",
                    "scene": scene,
                    "l3_owner_definition": "AUTHORITY_TO_CREATE_REPLACEMENT_ROOT_PROCESS_TREE_FOR_STABLE_SERVICE_GENERATION",
                    "rx_generation": started,
                    "active_owner_refusal": active_refusal,
                    "after_rx_stop": terminal,
                    "compose_restart_count": restart_count,
                    "blocked": scene == "preserved",
                    "observed": True,
                    "foreign_generation_started": scene != "preserved",
                    "confirmed_stop": False,
                    "unconfirmed_component_stops": final["unconfirmed_component_stops"],
                    "supervisor_returncode": primary.returncode,
                    "physical_qualification": "NOT_PERFORMED",
                }
                (output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
                return result
            finally:
                subprocess.run(
                    ["docker", "compose", "-p", project, "-f", str(compose_file), "down", "--remove-orphans"],
                    capture_output=True,
                    text=True,
                    timeout=60,
                )
                if primary.poll() is None:
                    subprocess.run(["docker", "kill", primary_name], capture_output=True, text=True)
                    primary.wait(timeout=10)

    preserved = passage("preserved", current_image)
    negative = passage("admission-off", mutant_image)
    run("clean-after-builds", clean, timeout=300)
    assert all(digest(ROOT / relative) == value for relative, value in source_hashes.items())
    summary = {
        "schema": "rx.ai-worker-l3-acceptance.v1",
        "base_image": base,
        "current_image": current_image,
        "admission_off_image": mutant_image,
        "source_pins": json.loads(pins.read_text()),
        "p_l3_1_2_4": preserved,
        "p_l3_3": negative,
        "p_a1": zed,
        "p_a2": rt,
        "dhi_compose_s6_reuse": "NOT_ESTABLISHED",
        "open_manipulator_l3_reuse": "NOT_ESTABLISHED",
        "product_signing_custody": "NOT_ESTABLISHED",
        "offline_revocation_freshness": "NOT_ESTABLISHED",
        "robotis_bundle_complete": False,
        "physical_qualification": "NOT_PERFORMED",
        "source_tree_unchanged_by_private_mutation": True,
    }
    (evidence / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
