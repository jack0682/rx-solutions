#!/bin/bash
set -eo pipefail
if [ "${1:-}" = "host" ]; then
    shift
    exec /opt/rx/bin/rx-hostd "$@"
fi
source /opt/ros/jazzy/setup.bash
source /opt/rx/robotis/setup.bash
set -u
if [ "${1:-}" = "supervise" ]; then
    shift
    exec /opt/rx/bin/rx-solutionsd run "$@"
fi
exec python3 /opt/rx/tools/solutions_status.py "$@"
