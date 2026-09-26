#!/usr/bin/env python3
"""Container-side direct R1 surfaces and inheritance tests."""
import argparse
import json
import os
from pathlib import Path
import pty
import sys
from unittest.mock import patch

sys.path.insert(0, "/opt/rx/tools/dhi")
import model
import session

OUT = Path("/output")
INSTANCE = "f8b07701-c6e7-4c82-a71c-000000000106"


def create_session(description=None):
    current = session.Session.__new__(session.Session)
    if description:
        with patch.object(session.Session, "description", description):
            current.__init__(INSTANCE)
    else:
        current.__init__(INSTANCE)
    return current


def surface_scene():
    master, slave = pty.openpty()
    outside_path = os.ttyname(slave)
    outside = model.Model(master, lambda row: None)
    baseline = outside.snapshot()
    os.environ["DHI_TEST_ENDPOINT"] = outside_path
    owned = create_session()
    try:
        after = outside.snapshot()
        refusals = [row.get("refusal") for row in owned.events if row.get("refusal")]
        required = {
            "DHI_UNCUSTODIED_CHARACTER_RESOURCE",
            "DHI_OPENAT2_RESOLVE_UNSUPPORTED",
            "DHI_OPEN_BY_HANDLE_UNSUPPORTED",
            "DHI_IO_URING_UNSUPPORTED",
            "DHI_PIDFD_IMPORT_UNSUPPORTED",
        }
        assert required <= set(refusals), refusals
        assert after["requests"] == baseline["requests"]
        assert after["writes"] == baseline["writes"]
        assert len(owned.grants) == 2
        return {
            "outside_endpoint": outside_path,
            "outside_new_requests": after["requests"] - baseline["requests"],
            "outside_new_writes": after["writes"] - baseline["writes"],
            "refusals": refusals,
            "custody_grants": owned.grants,
        }
    finally:
        owned.close()
        outside.close()
        os.close(slave)


def alias_scene(kind):
    original = session.Session.description

    def description(current):
        if kind == "symlink":
            alias = "/tmp/dhi-r1-custody-link"
        else:
            alias = "dhi-r1-custody-relative"
        try:
            os.unlink(alias)
        except FileNotFoundError:
            pass
        os.symlink(current.path, alias)
        return original(current).replace(current.path, alias)

    owned = create_session(description)
    try:
        requested = [grant["requested_path"] for grant in owned.grants]
        expected = (
            "/tmp/dhi-r1-custody-link"
            if kind == "symlink"
            else "dhi-r1-custody-relative"
        )
        assert requested == [expected, expected], requested
        return {"kind": kind, "requested_paths": requested, "grants": owned.grants}
    finally:
        owned.close()
        try:
            os.unlink(
                "/tmp/dhi-r1-custody-link"
                if kind == "symlink"
                else "dhi-r1-custody-relative"
            )
        except FileNotFoundError:
            pass


def fork_scene(expect_bypass):
    owned = session.Session.__new__(session.Session)
    error = None
    try:
        owned.__init__(INSTANCE)
    except Exception as caught:
        error = repr(caught)
    events = list(getattr(owned, "events", []))
    refusals = [row for row in events if row.get("refusal")]
    if expect_bypass:
        assert error is not None
        snapshot = owned.model.snapshot() if getattr(owned, "model", None) else None
        manager_exit = next(
            (row for row in events if row.get("event") == "manager_exit"), None
        )
        assert manager_exit and manager_exit["wait_status"] != 0, manager_exit
        assert not any(
            row.get("refusal") == "DHI_PROCESS_FORK_UNSUPPORTED" for row in refusals
        )
    else:
        assert error is None, error
        assert any(
            row.get("refusal") == "DHI_PROCESS_FORK_UNSUPPORTED" for row in refusals
        ), refusals
        assert len(owned.grants) == 2
        snapshot = owned.model.snapshot()
        assert snapshot["error"] is None
    result = {
        "expect_bypass": expect_bypass,
        "startup_error": error,
        "refusals": refusals,
        "model": snapshot,
        "grants": list(getattr(owned, "grants", [])),
        "manager_exit": next(
            (row for row in events if row.get("event") == "manager_exit"), None
        ),
    }
    if hasattr(owned, "guardian"):
        owned.close()
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "scene",
        choices=[
            "surfaces",
            "alias-symlink",
            "alias-relative",
            "late-fork",
            "late-fork-bypass",
        ],
    )
    args = parser.parse_args()
    os.environ.update(
        RX_PROCESS_INSTANCE_ID=INSTANCE,
        ROS_DOMAIN_ID="106",
        RMW_IMPLEMENTATION="rmw_fastrtps_cpp",
        ROS_LOG_DIR="/tmp/ros-log",
    )
    if args.scene == "surfaces":
        result = surface_scene()
    elif args.scene.startswith("alias-"):
        result = alias_scene(args.scene.removeprefix("alias-"))
    else:
        result = fork_scene(args.scene.endswith("bypass"))
    report = {
        "scene": args.scene,
        "scope": "DIRECT_COMPONENT_R1_TEST_NOT_REGISTERED_RELEASE_ADMISSION",
        "physical_qualification": "NOT_PERFORMED",
        "result": result,
    }
    (OUT / "result.json").write_text(json.dumps(report, indent=2))
    print(json.dumps({"scene": args.scene, "passed": True}), flush=True)


if __name__ == "__main__":
    main()
