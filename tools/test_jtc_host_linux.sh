#!/bin/bash
set -euo pipefail
workspace="$(cd "$(dirname "$0")/../.." && pwd)"
docker run --rm --network none --read-only --cap-drop ALL --security-opt no-new-privileges \
  --tmpfs /tmp:rw,exec --tmpfs /cargo:rw \
  -v "$workspace/.tools/cargo/registry:/cargo/registry:ro" \
  -v "$workspace/.tools/rustup/toolchains/1.98.1-aarch64-unknown-linux-gnu:/toolchain:ro" \
  -v "$workspace:/rx:ro" -v "$workspace/.tools/linux-executor-build:/build:rw" \
  -v "$workspace/references/implementation:/evidence:rw" \
  -e CARGO_HOME=/cargo -e CARGO_TARGET_DIR=/build -e CARGO_BUILD_JOBS=1 -e CARGO_INCREMENTAL=0 \
  -e RUSTC=/toolchain/bin/rustc -e RUSTDOC=/toolchain/bin/rustdoc \
  -e RMW_IMPLEMENTATION=rmw_fastrtps_cpp -e ROS_AUTOMATIC_DISCOVERY_RANGE=LOCALHOST \
  -e ROS_LOG_DIR=/tmp/ros-log -e RX_JTC_BRIDGE_BINARY=/opt/rx/bin/rx-ros-jtc-bridge \
  -e RX_JTC_SERVER_STATE=/tmp/jtc-state.json \
  --entrypoint /bin/bash rx-solutions:runtime-draft -c '
    set -e
    source /opt/ros/jazzy/setup.bash
    python3 /rx/rx-solutions/native/ros-jtc/mock_for_host.py --state /tmp/jtc-state.json &
    fixture_pid=$!
    trap "kill $fixture_pid 2>/dev/null || true" EXIT
    for attempt in $(seq 1 100); do test -s /tmp/jtc-state.json && break; sleep .05; done
    test -s /tmp/jtc-state.json
    cd /rx/rx-solutions
    /toolchain/bin/cargo test -p rx-host --test ros_jtc --offline --locked actual_rust_host_to_cpp_bridge_to_ros_action_preserves_original_goal -- --ignored --exact --nocapture
    cp /tmp/jtc-state.json /evidence/phase61_ros_server_state.json
  '
