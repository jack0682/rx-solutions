#!/usr/bin/env python3
"""P5: replay the competing description against R1 before effect classification."""
import errno
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
from unittest.mock import patch

sys.path.insert(0, "/opt/rx/tools/dhi")
import model
import session

OUT = Path("/output")
INSTANCE = "f8b07701-c6e7-4c82-a71c-000000000105"
os.environ.update(
    RX_PROCESS_INSTANCE_ID=INSTANCE,
    ROS_DOMAIN_ID="106",
    RMW_IMPLEMENTATION="rmw_fastrtps_cpp",
    ROS_LOG_DIR="/tmp/ros-log",
)


def main():
    master, slave = pty.openpty()
    tty.setraw(slave)
    outside_path = os.ttyname(slave)
    outside = model.Model(master, lambda row: None)
    os.write(slave, model.packet(1, b"\x01"))
    assert select.select([slave], [], [], 2)[0]
    assert os.read(slave, 128)
    baseline = outside.snapshot()

    stub = session.Session.__new__(session.Session)
    stub.path = outside_path
    foreign_urdf = session.Session.description(stub)
    (OUT / "foreign.urdf").write_text(foreign_urdf)
    publisher_code = """
import rclpy,time
from pathlib import Path
from std_msgs.msg import String
from rclpy.qos import QoSProfile,DurabilityPolicy,ReliabilityPolicy
rclpy.init();n=rclpy.create_node('external_description',namespace='/rx_dhi_f8b07701_c6e7_4c82_a71c_000000000105')
p=n.create_publisher(String,'robot_description',QoSProfile(depth=1,durability=DurabilityPolicy.TRANSIENT_LOCAL,reliability=ReliabilityPolicy.RELIABLE))
end=time.monotonic()+1.0
while time.monotonic()<end:
 p.publish(String(data=Path('/output/foreign.urdf').read_text()));rclpy.spin_once(n,timeout_sec=.05)
while True:rclpy.spin_once(n,timeout_sec=.1)
"""
    rogue = subprocess.Popen(
        [sys.executable, "-c", publisher_code],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    owned = session.Session.__new__(session.Session)
    original_description = session.Session.description

    def delayed_description(current):
        time.sleep(2)
        return original_description(current)

    error = None
    try:
        with patch.object(session.Session, "description", delayed_description):
            owned.__init__(INSTANCE)
        time.sleep(0.2)
        after = outside.snapshot()
        events = list(owned.events)
        refusal_positions = [
            index
            for index, row in enumerate(events)
            if row.get("refusal") == "DHI_UNCUSTODIED_CHARACTER_RESOURCE"
        ]
        grant_positions = [
            index for index, row in enumerate(events) if row.get("event") == "grant"
        ]
        assert after["requests"] - baseline["requests"] == 0, (baseline, after)
        assert after["writes"] - baseline["writes"] == 0, (baseline, after)
        assert refusal_positions and len(grant_positions) == 2, events
        assert refusal_positions[0] < grant_positions[0], events
        assert len(owned.grants) == 2
        try:
            extra = os.open(owned.path, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
        except OSError as caught:
            assert caught.errno == errno.EBUSY
        else:
            os.close(extra)
            raise AssertionError("outside opener entered custody endpoint")
        result = {
            "scope": "P5_ACTUAL_UNMODIFIED_MANAGER_DHI_FRESH_PTY_ONLY",
            "outside_endpoint": outside_path,
            "custody_endpoint": owned.path,
            "outside_baseline": baseline,
            "outside_after": after,
            "outside_new_requests": after["requests"] - baseline["requests"],
            "outside_new_writes": after["writes"] - baseline["writes"],
            "resource_refusal_before_first_grant": True,
            "resource_refusal_count": len(refusal_positions),
            "custody_grants": owned.grants,
            "manager_pid": owned.manager_pid,
            "physical_qualification": "NOT_PERFORMED",
            "effect_classification": "DIRECT_MECHANISM_ONLY; REGISTERED_CLASSIFICATION_VERIFIED_SEPARATELY",
        }
    except Exception as caught:
        error = repr(caught)
        raise
    finally:
        if hasattr(owned, "guardian"):
            owned.close()
        rogue.send_signal(signal.SIGTERM)
        try:
            rogue.wait(timeout=5)
        except subprocess.TimeoutExpired:
            rogue.kill()
            rogue.wait()
        outside.close()
        os.close(slave)
        if error:
            (OUT / "error.txt").write_text(error)
    (OUT / "result.json").write_text(json.dumps(result, indent=2))
    print(json.dumps(result), flush=True)


if __name__ == "__main__":
    main()
