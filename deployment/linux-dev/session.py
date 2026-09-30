"""Retained, isolated simulation sessions built on RX's public delivery harness."""
import hashlib
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import time
import uuid
import fcntl


def shutdown(folder):
    """Serialize cooperative shutdown, including after loss of the session runner."""
    with (folder / "shutdown.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        record = json.loads((folder / "session.json").read_text())
        names = record["containers"]
        prefix = "rx-dev-" + folder.name[:12]
        ordered = [prefix + "-" + suffix for suffix in ["e", "p", "h"]]
        ordered += [name for name in reversed(names) if name not in ordered]
        results = []
        def inspect(name):
            value = json.loads(subprocess.check_output(["docker", "inspect", name], text=True))[0]
            if value["Config"].get("Labels", {}).get("rx.dev.session") != folder.name:
                raise ValueError("Refusing unrelated container: " + name)
            return value["State"]
        for name in ordered:
            if name not in names:
                continue
            try:
                state = inspect(name)
                if state["Running"]:
                    subprocess.run(["docker", "kill", "--signal", "TERM", name], check=True, capture_output=True)
                    deadline = time.monotonic() + 30
                    while inspect(name)["Running"] and time.monotonic() < deadline:
                        time.sleep(.2)
                state = inspect(name)
                results.append({"container": name, "running": state["Running"], "exit_code": state["ExitCode"]})
            except Exception as error:
                results.append({"container": name, "error": str(error)})
        return results


def start(root, folder, config, *, demo):
    sys.path.insert(0, str(root / "sources/platform/tools"))
    from cell_delivery.docker import Docker
    from cell_delivery.materials import Materials
    from cell_delivery.installation import exercise
    from test_runtime_skills import recovery_evidence, author_installed_process, invoke_scene

    os.umask(0o077)
    evidence = folder / "evidence"
    private = folder / "private"
    evidence.mkdir(mode=0o700)
    private.mkdir(mode=0o700)
    record = {"schema": "rx.dev-session.v1", "phase": "PREPARING", "containers": [],
              "volumes": [], "networks": [], "images": config["images"],
              "profile": "FILE_SIMULATION", "physical_execution": "NOT_SUPPORTED"}

    def save():
        temporary = folder / (".session-" + uuid.uuid4().hex)
        temporary.write_text(json.dumps(record, indent=2) + "\n")
        temporary.replace(folder / "session.json")

    stopping = False

    def request_stop(*_):
        nonlocal stopping
        stopping = True

    signal.signal(signal.SIGTERM, request_stop)
    signal.signal(signal.SIGINT, request_stop)

    class RetainedDocker(Docker):
        def put(self, image, volume, source, path="/"):
            if volume not in record["volumes"]:
                raise ValueError("Refusing writes to an unowned volume")
            super().put(image, volume, source, path)
            # docker cp creates root-owned files. Keep restrictive file modes while
            # making each newly published artifact readable by the runtime UID.
            self.run("run", "--rm", "--network", "none", "--read-only", "--user", "0",
                     "--cap-drop", "ALL", "--cap-add", "CHOWN", "--cap-add", "DAC_OVERRIDE",
                     "--security-opt", "no-new-privileges", "-v", volume + ":/copy:rw",
                     "--entrypoint", "/bin/chown", image, "-R", "10001:10001", "/copy")

        def run(self, *args, capture=True):
            args = list(args)
            if args[0] in ["create", "start", "run"]:
                if stopping or (folder / "stop-request").exists():
                    raise RuntimeError("Session stop requested; no new containers will be created")
            if args[0] == "create":
                args[1:1] = ["--label", "rx.dev.session=" + folder.name]
            output = super().run(*args, capture=capture)
            if args[0] == "create" and "--name" in args:
                record["containers"].append(args[args.index("--name") + 1])
                save()
            if args[:2] == ["volume", "create"]:
                record["volumes"].append(output)
                save()
            if args[:2] == ["network", "create"]:
                record["networks"].append(args[-1])
                save()
            return output

    docker = RetainedDocker(evidence)
    docker.prefix = "rx-dev-" + folder.name[:12]
    docker.network = docker.prefix + "-network"
    save()

    class InstalledMaterials(Materials):
        def exporter(self, test, environment, label):
            args = ["docker", "run", "--rm", "--network", "none", "--read-only",
                    "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
                    "--user", f"{os.getuid()}:{os.getgid()}", "--tmpfs", "/tmp:rw,mode=1777",
                    "-v", f"{private}:{private}:rw", "--entrypoint", "/opt/rx/dev/delivery-fixture"]
            for key, value in environment.items():
                args += ["-e", key + "=" + value]
            args += [config["images"]["platform"], test, "--ignored", "--exact"]
            result = subprocess.run(args, capture_output=True, text=True)
            (evidence / (label + ".log")).write_text(result.stdout + result.stderr)
            if result.returncode or "1 passed; 0 failed" not in result.stdout:
                raise RuntimeError("Fixture preparation failed; see " + label + ".log")

    failed = None
    try:
        p = docker.image(config["images"]["platform"])
        s = docker.image(config["images"]["solutions"])
        if p["Architecture"] != s["Architecture"] or p["Architecture"] != config["architecture"]:
            raise ValueError("Installed image architectures differ")
        recovery, client = recovery_evidence(docker, s, root / "sources/solutions", private)
        holder = docker.holder(p["Id"], [])
        manifest = private / "platform-source.sha256"
        docker.run("cp", holder + ":/usr/local/bin/source.sha256", str(manifest))
        for line in manifest.read_text().splitlines():
            digest, path = line.split("  ", 1)
            relative = path.removeprefix("/source/")
            file = root / "sources/platform" / relative
            if relative == path or hashlib.sha256(file.read_bytes()).hexdigest() != digest:
                raise ValueError("Platform image does not match source: " + relative)
        ui = private / "operator"
        holder = docker.holder(s["Id"], [])
        docker.run("cp", holder + ":/opt/rx/operator", str(ui))
        request = str(uuid.uuid4())
        draft = str(uuid.uuid5(uuid.UUID(request), "rx.runtime-skill.draft"))
        materials = InstalledMaterials(root / "sources/platform", private, evidence, docker, s["Id"])
        materials.create_seed(s["Architecture"], draft)
        package, compiled, compiler = materials.compile()
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        validator = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
        final = materials.finalize(package, compiled, compiler, port, ui, validator)

        def after(context, active):
            context["installed_client"] = client
            record["phase"] = "VERIFYING"
            save()
            invoke_scene(context, active)
            record["recorded_verification"] = {
                "status": "PASS", "evidence": str(evidence / "result.json"),
                "scope": "FILE_SIMULATION P/Host/Executor, original-request recovery; not physical qualification"}
            record["phase"] = "DEMO_READY" if demo else "VERIFIED"
            record["origin"] = context["browser"]["origin"]
            record["private_access_file"] = str(final / "browser-fixture.json")
            save()
            print(json.dumps({"status": record["phase"], "session": folder.name,
                              "origin": record["origin"], "evidence": str(evidence),
                              "access_instructions": str(root / "README.md"),
                              "scope": "FILE_SIMULATION only; independent P/Host/Executor"}), flush=True)
            while demo and not stopping and not (folder / "stop-request").exists():
                for name in [context["p"], context["h"], context["e"]]:
                    if not docker.state(name)["State"]["Running"]:
                        raise RuntimeError("Development process exited: " + name)
                time.sleep(1)

        exercise(docker, materials, final, ui, p, s, port, validator, recovery, "independent",
                 after_commissioning=after,
                 before_commissioning=lambda c: author_installed_process(c, client, request))
    except BaseException as error:
        failed = error
        record["phase"] = "FAILED"
        record["error"] = str(error)
        save()
    finally:
        # Preserve all volumes, configurations and evidence. Never force-kill or auto-replay.
        shutdown_results = shutdown(folder)
        docker.capture_logs()
        record["shutdown"] = shutdown_results
        record["phase"] = "FAILED" if failed else "STOPPED"
        incomplete = any(v.get("running") or "error" in v or v.get("exit_code", 0) != 0 for v in shutdown_results)
        if incomplete:
            record["phase"] = "STOP_ATTENTION"
        save()
        print(json.dumps({"session": folder.name, "phase": record["phase"],
                          "records_and_volumes_preserved": True}), flush=True)
    if failed:
        raise failed
    if incomplete:
        raise RuntimeError("Shutdown needs attention; inspect session.json. No process was forced off.")
