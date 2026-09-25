#!/usr/bin/env python3
"""Container-side actual-process G6 passages; only fresh PTY simulations."""
import argparse
import errno
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import time
import urllib.error
import urllib.request
from unittest.mock import patch

OUT = Path("/output")
INSTANCE = "f8b07701-c6e7-4c82-a71c-000000000011"


def call(action=None):
    data = None if action is None else json.dumps(action).encode()
    request = urllib.request.Request(
        "http://127.0.0.1:18089/" + ("status" if data is None else "diagnostic"),
        data=data,
        headers={"Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(request, timeout=8) as response:
            return response.status, json.load(response)
    except urllib.error.HTTPError as error:
        return error.code, json.load(error)


def json_rows(path):
    rows = []
    for line in path.read_text().splitlines():
        try:
            rows.append(json.loads(line))
        except ValueError:
            pass
    return rows


def configuration():
    return {
        "schema": "rx.solutions-startup.v1",
        "state_subdirectory": "g6-resident",
        "plan": {
            "schema": "rx.solutions-process-plan.v1",
            "id": "b9c07701-c6e7-4c82-a71c-000000000011",
            "environment": "SIMULATION",
            "profiles": [],
            "processes": [
                {
                    "id": "dhi",
                    "program": "rx/dhi-pty-simulation",
                    "parameters": {"port": "18089"},
                    "depends_on": [],
                    "startup_timeout_ms": "30000",
                    "shutdown_timeout_ms": "15000",
                    "restart_limit": "0",
                    "restart_backoff_ms": "500",
                }
            ],
        },
    }


def resident(loss, traced=False):
    config = OUT / "startup.json"
    config.write_text(json.dumps(configuration()))
    inspect = subprocess.run(
        ["/opt/rx/bin/rx-solutionsd", "inspect", str(config)],
        capture_output=True,
        text=True,
    )
    (OUT / "inspect.stdout").write_text(inspect.stdout)
    (OUT / "inspect.stderr").write_text(inspect.stderr)
    assert inspect.returncode == 0, inspect.stderr
    command = ["/opt/rx/bin/rx-solutionsd", "run", str(config)]
    if traced:
        subprocess.run(
            [
                "cc",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-O2",
                "/test-tools/dhi_write_observer.c",
                "-o",
                "/output/dhi-write-observer",
            ],
            check=True,
        )
        command.insert(0, "/output/dhi-write-observer")
    rows = []
    with (OUT / "supervisor.stdout").open("w") as stdout, (
        OUT / "supervisor.stderr"
    ).open("w") as stderr:
        proc = subprocess.Popen(command, stdout=stdout, stderr=stderr)
        owner = proc.pid
        try:
            if traced:
                for _ in range(100):
                    children = (
                        Path(f"/proc/{proc.pid}/task/{proc.pid}/children")
                        .read_text()
                        .split()
                    )
                    if children:
                        owner = int(children[0])
                        break
                    time.sleep(0.01)
                else:
                    raise AssertionError("traced owner was not created")
            end = time.monotonic() + 30
            while time.monotonic() < end:
                assert proc.poll() is None, "supervisor failed before readiness"
                try:
                    code, status = call()
                    break
                except (OSError, urllib.error.URLError):
                    time.sleep(0.1)
            else:
                raise AssertionError("resident session never ready")
            assert code == 200 and status["component_phase"] == "INACTIVE", status
            assert len(status["grants"]) == 2
            rows.append({"initial": status})
            try:
                fd = os.open(
                    status["endpoint"], os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK
                )
            except OSError as error:
                assert error.errno == errno.EBUSY
                rows.append({"outside_open": "DHI_ENDPOINT_BUSY", "errno": error.errno})
            else:
                os.close(fd)
                raise AssertionError("ordinary second opener admitted")
            code, torque = call({"action": "torque", "enable": True})
            assert (
                code == 200
                and torque["request_accepted"]
                and torque["observed_model"]["model_torque"] == 0
            ), torque
            rows.append({"inactive_torque": torque})
            code, claim = call({"action": "adopt"})
            assert code == 409 and claim["refusal"] == "DHI_OWNER_CLAIM_UNATTESTED"
            rows.append({"unattested": claim})
            code, active = call({"action": "activate"})
            assert code == 200 and active["observed_model"]["model_torque"] == 1, active
            rows.append({"activation": active})
            if loss:
                fd = os.pidfd_open(status["manager_pid"])
                os.kill(status["guardian_pid"], signal.SIGKILL)
                assert select.select([fd], [], [], 5)[0]
                os.close(fd)
                code, lost = call()
                assert code == 200 and lost["component_phase"] == "ATTENTION"
                assert (
                    lost["model"]["model_torque"] == 1
                    and lost["stop_effect"] == "UNCONFIRMED"
                )
                rows.append({"guardian_lost": lost})
                code, denied = call({"action": "activate"})
                assert code == 409
                rows.append({"after_loss": denied})
            else:
                code, stopped = call({"action": "stop"})
                assert code == 200 and stopped["request_accepted"]
                assert stopped["stop_effect"] == "UNCONFIRMED"
                rows.append({"stop_request": stopped})
            code, restart = call({"action": "restart"})
            assert (
                code == 409 and restart["refusal"] == "DHI_PREVIOUS_ENDPOINT_RETAINED"
            )
            rows.append({"restart": restart})
            os.kill(owner, signal.SIGTERM)
            proc.wait(timeout=20)
        finally:
            if proc.poll() is None:
                os.kill(owner, signal.SIGTERM)
                try:
                    proc.wait(timeout=20)
                except subprocess.TimeoutExpired:
                    os.kill(owner, signal.SIGKILL)
                    proc.wait(timeout=5)
    assert (
        proc.returncode == 1
    ), f"aggregate stop must remain unconfirmed: {proc.returncode}"
    final = [
        r
        for r in json_rows(OUT / "supervisor.stdout")
        if r.get("schema") == "rx.supervisor-status.v1"
    ][-1]
    assert (
        final["all_exited"]
        and not final["guarded_shutdown_confirmed"]
        and final["reconciliation_required"]
    )
    assert final["unconfirmed_component_stops"]["dhi"].startswith(
        "DHI_MODEL_STOP_UNCONFIRMED"
    )
    assert final["state"]["records"]["dhi"]["exit_code"] == 0
    residuals = []
    for log in Path("/var/lib/rx-solutions/g6-resident").rglob("*.stdout.log"):
        residuals.extend(
            r
            for r in json_rows(log)
            if r.get("event") in ("session_exit", "model_retired")
        )
    assert residuals and all(
        r["residual"]["stop_effect"] == "UNCONFIRMED" for r in residuals
    )
    if loss:
        assert all(r["residual"]["model_torque"] == 1 for r in residuals)
    rows.append(
        {
            "supervisor_returncode": proc.returncode,
            "final": final,
            "retained_residuals": residuals,
        }
    )
    reopened = subprocess.run(
        ["/opt/rx/bin/rx-solutionsd", "run", str(config)],
        capture_output=True,
        text=True,
        timeout=20,
    )
    (OUT / "reopened.stdout").write_text(reopened.stdout)
    (OUT / "reopened.stderr").write_text(reopened.stderr)
    assert reopened.returncode == 1 and "DHI_MODEL_STOP_UNCONFIRMED" in reopened.stderr
    if traced:
        observations = [
            r
            for r in json_rows(OUT / "supervisor.stderr")
            if r.get("observer") == "ptrace"
        ]
        assert (
            observations[-1]["guardians"] == 1
            and observations[-1]["terminal_write_attempts"] == 0
        )
        rows.append({"independent_write_observer": observations[-1]})
    return rows


def argument_refusals():
    rows = []
    for name in ("endpoint", "owner", "physical", "restart"):
        value = configuration()
        if name in ("endpoint", "owner"):
            value["plan"]["processes"][0]["parameters"][name] = (
                "/dev/ttyUSB0" if name == "endpoint" else "controller_manager"
            )
        elif name == "physical":
            value["plan"]["environment"] = "PHYSICAL"
        else:
            value["plan"]["processes"][0]["restart_limit"] = "1"
        path = OUT / (name + ".json")
        path.write_text(json.dumps(value))
        result = subprocess.run(
            ["/opt/rx/bin/rx-solutionsd", "inspect", str(path)],
            capture_output=True,
            text=True,
        )
        assert result.returncode != 0
        rows.append(
            {"case": name, "returncode": result.returncode, "stderr": result.stderr}
        )
    return rows


def component_fault(kind):
    # Fault injection changes the simulated device input / file acquisition only.
    # It is a direct component test, not a claim of registered release admission.
    sys.path.insert(0, "/opt/rx/tools/dhi")
    import session
    import model

    os.environ["RX_PROCESS_INSTANCE_ID"] = INSTANCE
    before = sorted(p.name for p in Path("/dev/pts").iterdir())
    obj = session.Session.__new__(session.Session)
    original = Path.read_bytes

    def unavailable(path):
        if (
            str(path)
            == "/opt/rx/dhi/share/dynamixel_hardware_interface/param/dxl_model/xl430_w250.model"
        ):
            raise FileNotFoundError("injected model-file acquisition failure")
        return original(path)

    expected = (
        "DHI_MODEL_MISMATCH"
        if kind == "model"
        else "DHI_FIRMWARE_MODEL_FILE_UNAVAILABLE"
    )
    evidence = {
        "scope": "DIRECT_COMPONENT_INPUT_FAULT_NOT_RESIDENT_ADMISSION",
        "expected": expected,
    }
    try:
        with patch.object(
            model, "MODEL_NUMBER", 1070 if kind == "model" else 1060
        ), patch.object(
            Path, "read_bytes", unavailable if kind == "firmware" else original
        ):
            obj.__init__(INSTANCE)
        raise AssertionError("fault was admitted")
    except session.Refusal as error:
        assert str(error) == expected, str(error)
        evidence["observed"] = str(error)
        if obj.model:
            evidence["model"] = obj.model.snapshot()
            evidence["grants"] = obj.grants
    finally:
        obj.close()
    evidence["endpoints_before"] = before
    evidence["endpoints_after"] = sorted(p.name for p in Path("/dev/pts").iterdir())
    assert evidence["endpoints_before"] == evidence["endpoints_after"]
    return evidence


def component_custody():
    sys.path.insert(0, "/opt/rx/tools/dhi")
    import session

    os.environ["RX_PROCESS_INSTANCE_ID"] = INSTANCE
    before = sorted(p.name for p in Path("/dev/pts").iterdir())
    obj = session.Session.__new__(session.Session)
    try:
        obj.__init__(INSTANCE)
        result = {
            "scope": "DIRECT_COMPONENT_TEST_NOT_RESIDENT_ADMISSION",
            "status": obj.status(),
        }
        response = obj.command({"action": "torque", "enable": True})
        assert (
            response["request_accepted"]
            and response["observed_model"]["model_torque"] == 0
        )
        result["inactive_torque"] = response
        return result
    finally:
        obj.close()
        after = sorted(p.name for p in Path("/dev/pts").iterdir())
        print(
            json.dumps({"test_endpoint_cleanup": {"before": before, "after": after}}),
            flush=True,
        )
        assert before == after


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "scene",
        choices=[
            "resident-loss",
            "resident-normal",
            "resident-trace",
            "arguments",
            "fault-model",
            "fault-firmware",
            "component-custody",
        ],
    )
    args = parser.parse_args()
    if args.scene.startswith("resident"):
        result = resident(
            args.scene != "resident-normal", args.scene == "resident-trace"
        )
    elif args.scene == "component-custody":
        result = component_custody()
    elif args.scene == "arguments":
        result = argument_refusals()
    else:
        result = component_fault(args.scene.removeprefix("fault-"))
    report = {
        "scene": args.scene,
        "grade": "A-linux",
        "physical_qualification": "NOT_PERFORMED",
        "result": result,
    }
    (OUT / "result.json").write_text(json.dumps(report, indent=2))
    print(json.dumps({"scene": args.scene, "passed": True}), flush=True)


if __name__ == "__main__":
    main()
