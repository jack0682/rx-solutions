#!/usr/bin/env python3
"""Named denials for unqualified OpenMANIPULATOR physical inputs."""
import argparse, json, os, platform, sys
from pathlib import Path

class Refusal(Exception): pass

def main():
    p=argparse.ArgumentParser();p.add_argument("requirement",choices=("realsense","architecture","mode","maintenance"));p.add_argument("--expected-architecture");a=p.parse_args()
    if a.requirement=="realsense":
        if not list(Path("/dev").glob("video*")): raise Refusal("OPEN_MANIPULATOR_REALSENSE_UNAVAILABLE")
    elif a.requirement=="architecture":
        expected=a.expected_architecture or "UNSELECTED"
        if expected not in ("aarch64","x86_64") or platform.machine()!=expected: raise Refusal("OPEN_MANIPULATOR_ARCHITECTURE_PROFILE_MISMATCH")
    elif a.requirement=="mode":
        if os.environ.get("RX_OPEN_MANIPULATOR_MODE") not in ("SIMULATION","PHYSICAL"): raise Refusal("OPEN_MANIPULATOR_MODE_DEPENDENCIES_UNAVAILABLE")
    elif not Path("/var/lib/rx-solutions/open-manipulator-maintenance-handoff.json").is_file(): raise Refusal("OPEN_MANIPULATOR_MAINTENANCE_HANDOFF_UNAVAILABLE")
    print(json.dumps({"schema":"rx.open-manipulator-requirement.v1","result":"PRESENT_UNQUALIFIED","physical_qualification":"NOT_PERFORMED"}))

if __name__=="__main__":
    try: main()
    except Refusal as e:
        print(json.dumps({"schema":"rx.open-manipulator-refusal.v1","condition":str(e),"current_permission":"NOT_GRANTED","physical_qualification":"NOT_PERFORMED"}),file=sys.stderr);raise SystemExit(76)
