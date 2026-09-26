#!/usr/bin/env python3
"""Container-side P-REG resident-path counterexample and negative control."""
import argparse
import json
import os
from pathlib import Path
import pty
import select
import signal
import subprocess
import sys
import time
import tty
import urllib.error
import urllib.request

sys.path.insert(0, "/opt/rx/tools/dhi")
import model
import session

OUT = Path("/output")
STATE = Path("/var/lib/rx-solutions")
PORT = 18089


def configuration(scene):
    suffix = "201" if scene == "preserved" else "202"
    return {
        "schema": "rx.solutions-startup.v1",
        "state_subdirectory": "g8-registered-" + scene,
        "plan": {
            "schema": "rx.solutions-process-plan.v1",
            "id": "b9c07701-c6e7-4c82-a71c-000000000" + suffix,
            "environment": "SIMULATION",
            "profiles": [],
            "processes": [
                {
                    "id": "dhi",
                    "program": "rx/dhi-pty-simulation",
                    "parameters": {"port": str(PORT)},
                    "depends_on": [],
                    "startup_timeout_ms": "30000",
                    "shutdown_timeout_ms": "15000",
                    "restart_limit": "0",
                    "restart_backoff_ms": "500",
                }
            ],
        },
    }


def json_rows(path):
    result = []
    if not path.exists():
        return result
    for line in path.read_text(errors="replace").splitlines():
        try:
            result.append(json.loads(line))
        except ValueError:
            pass
    return result


def status():
    with urllib.request.urlopen(
        f"http://127.0.0.1:{PORT}/status", timeout=1
    ) as response:
        return json.load(response)


def session_process(deadline):
    while time.monotonic() < deadline:
        for entry in Path("/proc").iterdir():
            if not entry.name.isdigit():
                continue
            try:
                command = (entry / "cmdline").read_bytes().replace(b"\0", b" ")
                if b"/opt/rx/tools/dhi/session.py serve" not in command:
                    continue
                pid = int(entry.name)
                environment = dict(
                    item.split(b"=", 1)
                    for item in (entry / "environ").read_bytes().split(b"\0")
                    if b"=" in item
                )
                instance = environment[b"RX_PROCESS_INSTANCE_ID"].decode()
                return pid, instance
            except (FileNotFoundError, ProcessLookupError, KeyError, PermissionError):
                continue
        time.sleep(0.001)
    raise AssertionError("resident DHI child was not intercepted before publication")


def stop_session_after_guardian_started(session_pid, deadline):
    while time.monotonic() < deadline:
        for entry in Path("/proc").iterdir():
            if not entry.name.isdigit():
                continue
            try:
                command = (entry / "cmdline").read_bytes().replace(b"\0", b" ")
                if b"/opt/rx/bin/rx-dhi-custody" in command:
                    os.kill(session_pid, signal.SIGSTOP)
                    return
            except (FileNotFoundError, ProcessLookupError, PermissionError):
                continue
        time.sleep(0.001)
    raise AssertionError("guardian did not start before resident publication gate")


def foreign_description(path):
    stub = session.Session.__new__(session.Session)
    stub.path = path
    return session.Session.description(stub)


def publisher(instance, description):
    path = OUT / "foreign.urdf"
    path.write_text(description)
    code = r'''import os,sys,time
from pathlib import Path
import rclpy
from std_msgs.msg import String
from rclpy.qos import QoSProfile,DurabilityPolicy,ReliabilityPolicy
os.environ.update(ROS_DOMAIN_ID="106",ROS_LOCALHOST_ONLY="1",RMW_IMPLEMENTATION="rmw_fastrtps_cpp",ROS_LOG_DIR="/tmp/rogue-ros-log")
rclpy.init(); node=rclpy.create_node("external_description",namespace=sys.argv[1])
qos=QoSProfile(depth=1,durability=DurabilityPolicy.TRANSIENT_LOCAL,reliability=ReliabilityPolicy.RELIABLE)
pub=node.create_publisher(String,"robot_description",qos); value=String(data=Path(sys.argv[2]).read_text())
end=time.monotonic()+1.0
while time.monotonic()<end:
 pub.publish(value); rclpy.spin_once(node,timeout_sec=.01); time.sleep(.01)
while True: rclpy.spin_once(node,timeout_sec=.1)
'''
    namespace = "/rx_dhi_" + instance.replace("-", "_")
    return subprocess.Popen(
        [sys.executable, "-c", code, namespace, str(path)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def custody_events():
    events = []
    for path in STATE.rglob("*.stdout.log"):
        for row in json_rows(path):
            if row.get("event") == "custody_observation" and isinstance(
                row.get("observation"), dict
            ):
                events.append(row["observation"])
    return events


def run_scene(scene):
    config = OUT / "startup.json"
    config.write_text(json.dumps(configuration(scene)))
    inspect = subprocess.run(
        ["/opt/rx/bin/rx-solutionsd", "inspect", str(config)],
        capture_output=True,
        text=True,
        timeout=30,
    )
    (OUT / "inspect.stdout").write_text(inspect.stdout)
    (OUT / "inspect.stderr").write_text(inspect.stderr)
    assert inspect.returncode == 0, inspect.stderr
    inspection = next(
        row
        for row in json_rows(OUT / "inspect.stdout")
        if row.get("schema") == "rx.solutions-plan-inspection.v1"
    )
    boundary = inspection["release_boundary"]
    assert boundary["dhi_registered_release_admission_verified"] is True
    assert boundary["dhi_effect_classification"] == (
        "NONACTUATING_FRESH_PTY_REGISTERED_RELEASE_VERIFIED"
    )

    master, slave = pty.openpty()
    tty.setraw(slave)
    outside_path = os.ttyname(slave)
    outside = model.Model(master, lambda _: None)
    os.write(slave, model.packet(1, b"\x01"))
    assert select.select([slave], [], [], 2)[0]
    assert os.read(slave, 128)
    baseline = outside.snapshot()
    daemon = None
    rogue = None
    child = None
    with (OUT / "supervisor.stdout").open("w") as stdout, (
        OUT / "supervisor.stderr"
    ).open("w") as stderr:
        daemon = subprocess.Popen(
            ["/opt/rx/bin/rx-solutionsd", "run", str(config)],
            stdout=stdout,
            stderr=stderr,
        )
        try:
            child, instance = session_process(time.monotonic() + 15)
            stop_session_after_guardian_started(child, time.monotonic() + 15)
            rogue = publisher(instance, foreign_description(outside_path))
            time.sleep(1.25)
            os.kill(child, signal.SIGCONT)
            child = None
            observed_status = None
            deadline = time.monotonic() + 40
            while time.monotonic() < deadline:
                snapshot = outside.snapshot()
                if scene == "bypass" and snapshot["writes"] > baseline["writes"]:
                    break
                if scene == "preserved":
                    try:
                        candidate = status()
                        if candidate.get("component_phase") == "INACTIVE":
                            observed_status = candidate
                            break
                    except (OSError, urllib.error.URLError, json.JSONDecodeError):
                        pass
                if daemon.poll() is not None:
                    break
                time.sleep(0.05)
            after = outside.snapshot()
            if scene == "preserved":
                assert observed_status is not None, (daemon.poll(), after)
            else:
                assert after["writes"] > baseline["writes"], (baseline, after)
        finally:
            if child is not None:
                try:
                    os.kill(child, signal.SIGCONT)
                except ProcessLookupError:
                    pass
            if rogue is not None:
                rogue.terminate()
                try:
                    rogue.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    rogue.kill()
                    rogue.wait()
            if daemon is not None and daemon.poll() is None:
                daemon.terminate()
                try:
                    daemon.wait(timeout=25)
                except subprocess.TimeoutExpired:
                    daemon.kill()
                    daemon.wait(timeout=5)
            outside.close()
            os.close(slave)

    supervisor = json_rows(OUT / "supervisor.stdout")
    registrations = [
        row
        for row in supervisor
        if row.get("schema")
        in ("rx.resident-reconciliation.v1", "rx.resident-registration-observation.v1")
    ]
    assert registrations and registrations[-1]["registrations"], supervisor
    events = custody_events()
    refusals = [
        index
        for index, row in enumerate(events)
        if row.get("refusal") == "DHI_UNCUSTODIED_CHARACTER_RESOURCE"
    ]
    grants = [index for index, row in enumerate(events) if row.get("event") == "grant"]
    result = {
        "schema": "rx.dhi-registered-admission-passage.v1",
        "scene": scene,
        "registered_path": {
            "release_verified": True,
            "catalog_constructed_and_plan_validated": True,
            "plan_digest": inspection["plan_digest"],
            "registration_observed": True,
        },
        "outside_endpoint": outside_path,
        "outside_baseline": baseline,
        "outside_after": after,
        "outside_new_requests": after["requests"] - baseline["requests"],
        "outside_new_writes": after["writes"] - baseline["writes"],
        "resource_refusal_count": len(refusals),
        "resource_refusal_before_first_grant": bool(
            refusals and grants and refusals[0] < grants[0]
        ),
        "custody_grants": [events[index] for index in grants],
        "supervisor_returncode": daemon.returncode,
        "physical_qualification": "NOT_PERFORMED",
        "product_signing_custody": boundary["product_release_custody_and_rotation"],
        "offline_revocation_freshness": boundary["offline_revocation_freshness"],
        "sdk_baseline_complete": boundary["sdk_baseline_complete"],
        "robotis_bundle_complete": boundary["robotis_bundle_complete"],
    }
    if scene == "preserved":
        assert result["outside_new_requests"] == 0, result
        assert result["outside_new_writes"] == 0, result
        assert result["resource_refusal_before_first_grant"], result
        assert len(result["custody_grants"]) == 2, result
    else:
        assert result["outside_new_writes"] > 0, result
    (OUT / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result), flush=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("scene", choices=["preserved", "bypass"])
    args = parser.parse_args()
    run_scene(args.scene)


if __name__ == "__main__":
    main()
