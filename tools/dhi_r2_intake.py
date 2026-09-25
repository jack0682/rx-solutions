#!/usr/bin/env python3
"""Measure controller_manager configuration intake; never an R1 substitute."""
import argparse
import json
import os
from pathlib import Path
import pty
import signal
import subprocess
import sys
import time

sys.path.insert(0, "/opt/rx/tools/dhi")
import model
import session

OUT = Path("/output")
INSTANCE = "f8b07701-c6e7-4c82-a71c-000000000109"


def description(path):
    stub = session.Session.__new__(session.Session)
    stub.path = path
    return session.Session.description(stub)


def parameter_scene():
    master, slave = pty.openpty()
    path = os.ttyname(slave)
    device = model.Model(master, lambda row: None)
    urdf = description(path)
    namespace = "/rx_dhi_parameter_measurement"
    command = [
        "/opt/rx/bin/rx-dhi-controller-manager",
        "--ros-args",
        "-r",
        "__ns:=" + namespace,
        "-p",
        "robot_description:=" + urdf,
    ]
    with (OUT / "parameter-manager.log").open("w") as log:
        process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
        time.sleep(4)
        snapshot = device.snapshot()
        process.send_signal(signal.SIGINT)
        try:
            process.wait(timeout=8)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
    device.close()
    os.close(slave)
    log_text = (OUT / "parameter-manager.log").read_text()
    return {
        "manager_returncode": process.returncode,
        "model": snapshot,
        "subscribed_to_topic": "Subscribing to" in log_text,
        "parameter_warning": "robot_description" in log_text,
        "classification": (
            "PARAMETER_REACHED_SDK"
            if snapshot["requests"]
            else "PARAMETER_DID_NOT_REACH_SDK_IN_MEASURED_MANAGER"
        ),
    }


def late_description_scene():
    owned = session.Session(INSTANCE)
    master, slave = pty.openpty()
    outside_path = os.ttyname(slave)
    outside = model.Model(master, lambda row: None)
    baseline = outside.snapshot()
    before = len(
        [
            row
            for row in owned.events
            if row.get("refusal") == "DHI_UNCUSTODIED_CHARACTER_RESOURCE"
        ]
    )
    foreign = description(outside_path)
    publisher_code = """
import rclpy,time
from pathlib import Path
from std_msgs.msg import String
from rclpy.qos import QoSProfile,DurabilityPolicy,ReliabilityPolicy
rclpy.init();n=rclpy.create_node('late_external_description',namespace='/rx_dhi_f8b07701_c6e7_4c82_a71c_000000000109')
p=n.create_publisher(String,'robot_description',QoSProfile(depth=1,durability=DurabilityPolicy.TRANSIENT_LOCAL,reliability=ReliabilityPolicy.RELIABLE))
for _ in range(20):p.publish(String(data=Path('/output/late.urdf').read_text()));rclpy.spin_once(n,timeout_sec=.05)
"""
    (OUT / "late.urdf").write_text(foreign)
    published = subprocess.run([sys.executable, "-c", publisher_code], timeout=10)
    assert published.returncode == 0
    time.sleep(1)
    after = outside.snapshot()
    refusals = [
        row
        for row in owned.events
        if row.get("refusal") == "DHI_UNCUSTODIED_CHARACTER_RESOURCE"
    ]
    result = {
        "outside_new_requests": after["requests"] - baseline["requests"],
        "outside_new_writes": after["writes"] - baseline["writes"],
        "new_resource_refusals": len(refusals) - before,
        "classification": (
            "LATE_DESCRIPTION_TRIGGERED_RESOURCE_GATE"
            if len(refusals) > before
            else "LATE_DESCRIPTION_IGNORED_AFTER_INITIALIZATION_IN_MEASURED_MANAGER"
        ),
        "custody_grants": owned.grants,
    }
    owned.close()
    outside.close()
    os.close(slave)
    assert result["outside_new_requests"] == 0
    assert result["outside_new_writes"] == 0
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("scene", choices=["parameter", "late-description"])
    args = parser.parse_args()
    os.environ.update(
        RX_PROCESS_INSTANCE_ID=INSTANCE,
        ROS_DOMAIN_ID="106",
        RMW_IMPLEMENTATION="rmw_fastrtps_cpp",
        ROS_LOG_DIR="/tmp/ros-log",
    )
    result = (
        parameter_scene() if args.scene == "parameter" else late_description_scene()
    )
    report = {
        "scene": args.scene,
        "role": "R2_MEASUREMENT_ONLY_NOT_OWNERSHIP_MECHANISM",
        "physical_qualification": "NOT_PERFORMED",
        "result": result,
    }
    (OUT / "result.json").write_text(json.dumps(report, indent=2))
    print(json.dumps(report), flush=True)


if __name__ == "__main__":
    main()
