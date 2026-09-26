#!/usr/bin/env python3
"""Build and execute the signed G8 resident-path P-REG passages."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-image", required=True)
    parser.add_argument("--builder", default="rust:1.98.1-slim-bookworm")
    parser.add_argument("--platform-root", required=True, type=Path)
    parser.add_argument("--encrypted-key", required=True, type=Path)
    parser.add_argument("--custody-record", required=True, type=Path)
    parser.add_argument("--old-metadata", required=True, type=Path)
    parser.add_argument("--target-cache", required=True, type=Path)
    parser.add_argument("--output-image", default="rx-g8-solutions-signed:latest")
    parser.add_argument("--evidence", required=True, type=Path)
    args = parser.parse_args()
    evidence = args.evidence.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    target = args.target_cache.resolve()
    target.mkdir(parents=True, exist_ok=True)
    if target == Path("/") or not any(
        token in target.name.lower() for token in ("target", "cache")
    ):
        parser.error("target cache must be an explicit target/cache directory")
    commands = []

    def run(label, command, *, expected=0, timeout=None):
        result = subprocess.run(command, capture_output=True, text=True, timeout=timeout)
        (evidence / f"{label}.stdout").write_text(result.stdout)
        (evidence / f"{label}.stderr").write_text(result.stderr)
        commands.append(
            {
                "label": label,
                "argv": [str(item) for item in command],
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

    base = json.loads(
        run(
            "base-image",
            ["docker", "image", "inspect", args.base_image],
        ).stdout
    )[0]["Id"]
    builder = json.loads(
        run("builder-image", ["docker", "image", "inspect", args.builder]).stdout
    )[0]["Id"]
    rust_host = next(
        line.split(":", 1)[1].strip()
        for line in run(
            "builder-rust-host",
            ["docker", "run", "--rm", "--network", "none", "--entrypoint", "rustc", builder, "-vV"],
        ).stdout.splitlines()
        if line.startswith("host:")
    )
    source_hashes = {
        path: digest(ROOT / path)
        for path in (
            "native/dhi/guardian.c",
            "native/dhi/endpoint-channels.json",
            "runtime/rx-supervisor/src/builtin.rs",
            "runtime/rx-supervisor/src/builtin/dhi.rs",
        )
    }

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
    clean = clean[: clean.index("--locked")] + [
        "-p",
        "rx-supervisor",
        "--target-dir",
        "/target",
    ]
    run("clean-rx-supervisor-before-current", clean, timeout=300)
    run("build-current-daemon", build, timeout=1800)
    current = evidence / "current"
    current.mkdir()
    shutil.copy2(target / "debug/rx-solutionsd", current / "rx-solutionsd")
    os.chmod(current / "rx-solutionsd", 0o755)
    shutil.copy2(ROOT / "native/dhi/endpoint-channels.json", current / "endpoint-channels.json")

    guardian_source = (ROOT / "native/dhi/guardian.c").read_text()
    before = '''        } else if (resource.st_rdev != custody.st_rdev) {
          refuse(r, q, "DHI_UNCUSTODIED_CHARACTER_RESOURCE", requested,
                 &resource);
          resource_refusals++;
        } else {'''
    after = '''        } else if (resource.st_rdev != custody.st_rdev) {
          /* G8 private negative control: remove only resource admission. */
          r->flags = SECCOMP_USER_NOTIF_FLAG_CONTINUE;
        } else {'''
    assert guardian_source.count(before) == 1
    mutant = evidence / "resource-admission-off"
    mutant.mkdir()
    (mutant / "guardian.c").write_text(guardian_source.replace(before, after))
    run(
        "compile-resource-admission-off-guardian",
        [
            "docker",
            "run",
            "--rm",
            "--network",
            "none",
            "-v",
            f"{mutant}:/output",
            "--entrypoint",
            "/usr/bin/cc",
            base,
            "-Wall",
            "-Wextra",
            "-Werror",
            "-O2",
            "/output/guardian.c",
            "-o",
            "/output/rx-dhi-custody",
            "-lutil",
        ],
        timeout=300,
    )
    os.chmod(mutant / "rx-dhi-custody", 0o755)
    run("clean-rx-supervisor-before-mutant", clean, timeout=300)
    mutant_build = build[:]
    mount_index = mutant_build.index("-w")
    mutant_build[mount_index:mount_index] = [
        "-v",
        f"{mutant / 'guardian.c'}:/source/native/dhi/guardian.c:ro",
    ]
    run("build-resource-admission-off-daemon", mutant_build, timeout=1800)
    shutil.copy2(target / "debug/rx-solutionsd", mutant / "rx-solutionsd")
    os.chmod(mutant / "rx-solutionsd", 0o755)

    inventory = json.loads(
        run(
            "extract-base-inventory",
            [
                "docker",
                "run",
                "--rm",
                "--entrypoint",
                "/bin/cat",
                base,
                "/opt/rx/manifests/runtime-files.json",
            ],
        ).stdout
    )
    inventory["files"]["bin/rx-solutionsd"] = digest(current / "rx-solutionsd")
    inventory["files"]["tools/dhi/endpoint-channels.json"] = digest(
        current / "endpoint-channels.json"
    )
    (current / "runtime-files.json").write_text(json.dumps(inventory, indent=2) + "\n")
    mutant_inventory = json.loads(json.dumps(inventory))
    mutant_inventory["files"]["bin/rx-solutionsd"] = digest(mutant / "rx-solutionsd")
    mutant_inventory["files"]["bin/rx-dhi-custody"] = digest(
        mutant / "rx-dhi-custody"
    )
    mutant_inventory["files"]["tools/dhi/guardian.c"] = digest(mutant / "guardian.c")
    (mutant / "runtime-files.json").write_text(
        json.dumps(mutant_inventory, indent=2) + "\n"
    )

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
        result = subprocess.run(command, capture_output=True, text=True, timeout=120)
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

    sign("sign-current-release", current, 2)
    sign("sign-resource-admission-off-release", mutant, 3)
    (current / "Dockerfile").write_text(
        "ARG BASE_IMAGE\n"
        "FROM ${BASE_IMAGE}\n"
        "COPY --chmod=0755 rx-solutionsd /opt/rx/bin/rx-solutionsd\n"
        "COPY endpoint-channels.json /opt/rx/tools/dhi/endpoint-channels.json\n"
        "COPY runtime-files.json /opt/rx/manifests/runtime-files.json\n"
        "COPY metadata/release.json /opt/rx/manifests/release.json\n"
        "COPY metadata/revocations.json /opt/rx/manifests/revocations.json\n"
    )
    run(
        "build-current-signed-image",
        [
            "docker",
            "build",
            "--build-arg",
            f"BASE_IMAGE={args.base_image}",
            "-t",
            args.output_image,
            str(current),
        ],
        timeout=600,
    )
    signed = json.loads(
        run(
            "signed-image",
            ["docker", "image", "inspect", args.output_image],
        ).stdout
    )[0]["Id"]

    def passage(label, image, mounts=()):
        output = evidence / label
        output.mkdir(mode=0o777)
        output.chmod(0o777)
        state = output / "state"
        state.mkdir(mode=0o777)
        state.chmod(0o777)
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
            f"{ROOT / 'tools'}:/test-tools:ro",
            "-v",
            f"{output}:/output",
            "-v",
            f"{state}:/var/lib/rx-solutions",
        ]
        for source, destination in mounts:
            command += ["-v", f"{source.resolve()}:{destination}:ro"]
        command += [
            "--entrypoint",
            "/bin/bash",
            image,
            "-c",
            "source /opt/ros/jazzy/setup.bash && source /opt/rx/dhi/setup.bash && exec python3 /test-tools/dhi_registered_passage.py \"$1\"",
            "g8-registered",
            "preserved" if label == "p-reg2-preserved" else "bypass",
        ]
        run(label, command, timeout=120)
        return json.loads((output / "result.json").read_text())

    preserved = passage("p-reg2-preserved", signed)
    bypass = passage(
        "p-reg3-admission-off",
        signed,
        [
            (mutant / "rx-solutionsd", "/opt/rx/bin/rx-solutionsd"),
            (mutant / "rx-dhi-custody", "/opt/rx/bin/rx-dhi-custody"),
            (mutant / "guardian.c", "/opt/rx/tools/dhi/guardian.c"),
            (mutant / "runtime-files.json", "/opt/rx/manifests/runtime-files.json"),
            (mutant / "metadata/release.json", "/opt/rx/manifests/release.json"),
            (mutant / "metadata/revocations.json", "/opt/rx/manifests/revocations.json"),
        ],
    )

    old = evidence / "p-reg4-old-root"
    old.mkdir(mode=0o777)
    old.chmod(0o777)
    config = evidence / "p-reg2-preserved/startup.json"
    old_command = [
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
        f"{config}:/config.json:ro",
        "-v",
        f"{args.old_metadata.resolve() / 'release.json'}:/opt/rx/manifests/release.json:ro",
        "-v",
        f"{args.old_metadata.resolve() / 'revocations.json'}:/opt/rx/manifests/revocations.json:ro",
        "--entrypoint",
        "/opt/rx/bin/rx-solutionsd",
        signed,
        "inspect",
        "/config.json",
    ]
    old_result = run("p-reg4-old-root", old_command, expected=1, timeout=60)
    assert "release/unknown-key" in old_result.stdout + old_result.stderr

    run("clean-rx-supervisor-after-builds", clean, timeout=300)
    assert all(digest(ROOT / path) == value for path, value in source_hashes.items())
    summary = {
        "schema": "rx.dhi-registered-admission.v1",
        "base_image": base,
        "builder_image": builder,
        "signed_image": signed,
        "p_reg1_signed_registered_entry": preserved["registered_path"],
        "p_reg2": {
            key: preserved[key]
            for key in (
                "outside_new_requests",
                "outside_new_writes",
                "resource_refusal_count",
                "resource_refusal_before_first_grant",
                "custody_grants",
            )
        },
        "p_reg3": {
            "registered_path": bypass["registered_path"],
            "outside_new_requests": bypass["outside_new_requests"],
            "outside_new_writes": bypass["outside_new_writes"],
            "resource_refusal_count": bypass["resource_refusal_count"],
            "mutation": "ONLY_NONCUSTODY_CHARACTER_RESOURCE_REFUSAL_REMOVED",
        },
        "p_reg4_old_root": "release/unknown-key",
        "physical_qualification": "NOT_PERFORMED",
        "product_signing_custody": "NOT_ESTABLISHED",
        "offline_revocation_freshness": "NOT_ESTABLISHED",
        "sdk_baseline_complete": False,
        "robotis_bundle_complete": False,
        "source_tree_unchanged_by_private_mutation": True,
    }
    (evidence / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
