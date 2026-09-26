FROM ros:jazzy-ros-base@sha256:386d06ec6d4188f731bae5678e07b4cb64a4e4d4152090c0bd1f881dcf7706f5 AS native-dependencies
SHELL ["/bin/bash", "-o", "pipefail", "-c"]
ENV DEBIAN_FRONTEND=noninteractive CMAKE_BUILD_PARALLEL_LEVEL=1 MAKEFLAGS="-j1 -l1"
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential cmake ninja-build ca-certificates python3 \
    nlohmann-json3-dev libzmq3-dev libsqlite3-dev \
    ros-jazzy-rclcpp ros-jazzy-control-msgs ros-jazzy-controller-manager-msgs \
    ros-jazzy-action-msgs ros-jazzy-joint-trajectory-controller \
    ros-jazzy-controller-manager=4.48.0-1noble.20260904.013127 \
    ros-jazzy-position-controllers ros-jazzy-rmw-zenoh-cpp \
    && mkdir -p /opt/rx/manifests \
    && dpkg-query -W -f='${Package}\t${Version}\t${Architecture}\n' | sort > /opt/rx/manifests/native-packages.tsv \
    && rm -rf /var/lib/apt/lists/*

FROM rust:1.98.1-bookworm@sha256:9a73a5088750b4c95158ab26629c854c3d6fc4b173cb7bc8079ad252d8ed7bfa AS rust-build
ENV CARGO_BUILD_JOBS=1
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY sdk ./sdk
COPY runtime ./runtime
COPY drivers ./drivers
COPY catalogs ./catalogs
COPY native/ros-jtc ./native/ros-jtc
COPY native/dynamixel ./native/dynamixel
COPY native/support ./native/support
COPY native/dhi ./native/dhi
COPY native/ai-worker ./native/ai-worker
COPY native/ai-sapiens ./native/ai-sapiens
COPY native/open-manipulator ./native/open-manipulator
COPY dependencies ./dependencies
COPY interfaces ./interfaces
RUN --mount=type=cache,id=rx-solutions-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=rx-solutions-release,target=/src/target \
    cargo build --release --locked --workspace && mkdir -p /out && cp target/release/rx-hostd target/release/rx-executor-service target/release/rx-process-compile target/release/rx-solutionsd target/release/rx-process-package target/release/rx-device-package /out/

FROM node:26.5.0-bookworm-slim@sha256:2d49d876e96237d76de412761cf05dbfe5aee325cc4406a4d41d5824c5bb8beb AS ui-build
WORKDIR /src
COPY apps/operator/package.json apps/operator/package-lock.json ./
RUN npm ci
COPY apps/operator/ ./
RUN npm run build

FROM native-dependencies AS bt-build
COPY --from=btcpp / /btcpp/
COPY native/executor /executor/
RUN --network=none cmake -S /executor -B /bt-build -G Ninja -DCMAKE_BUILD_TYPE=Release -DBTCPP_SOURCE=/btcpp -DRX_BUILD_TEST_HARNESS=OFF \
    && cmake --build /bt-build --target rx-bt-engine -j1

FROM native-dependencies AS ros-jtc-build
COPY native/ros-jtc /ros-jtc/
COPY catalogs/device-support.v1.json /catalogs/device-support.v1.json
RUN --network=none source /opt/ros/jazzy/setup.bash \
    && cmake -S /ros-jtc -B /ros-jtc-build -G Ninja -DCMAKE_BUILD_TYPE=Release -DRX_CATALOG=/catalogs/device-support.v1.json \
    && cmake --build /ros-jtc-build --target rx-ros-jtc-bridge -j1

FROM native-dependencies AS dynamixel-build
COPY native/dynamixel /dynamixel/
RUN --network=none cmake -S /dynamixel -B /dynamixel-build -DCMAKE_BUILD_TYPE=Release \
    && cmake --build /dynamixel-build --target rx-dynamixel-ping -j1

FROM native-dependencies AS dhi-build
COPY --from=dhi / /dhi-input/
COPY native/dhi/dependencies.json /source/native/dhi/dependencies.json
COPY tools/prepare_dhi_sources.py /source/tools/prepare_dhi_sources.py
RUN --network=none python3 /source/tools/prepare_dhi_sources.py --check --output /dhi-input \
    && source /opt/ros/jazzy/setup.bash \
    && colcon build --merge-install --base-paths /dhi-input/src --build-base /dhi-build --install-base /opt/rx/dhi \
       --packages-select dynamixel_sdk dynamixel_interfaces dynamixel_hardware_interface \
       --executor sequential --event-handlers console_direct+ \
       --cmake-args -DCMAKE_BUILD_TYPE=Release -DBUILD_TESTING=OFF
COPY native/dhi /source/native/dhi/
RUN --network=none cmake -S /source/native/dhi -B /dhi-guardian-build -DCMAKE_BUILD_TYPE=Release \
    && cmake --build /dhi-guardian-build -j1

FROM native-dependencies AS runtime
ENV PYTHONDONTWRITEBYTECODE=1 ROS_LOG_DIR=/var/lib/rx-solutions/ros-log
COPY LICENSE NOTICE /opt/rx/
COPY --from=btcpp /LICENSE /opt/rx/licenses/BehaviorTree.CPP/LICENSE
COPY --from=btcpp /3rdparty/tinyxml2/LICENSE.txt /opt/rx/licenses/tinyxml2/LICENSE.txt
COPY --from=btcpp /3rdparty/minitrace/LICENSE /opt/rx/licenses/minitrace/LICENSE
COPY --from=btcpp /3rdparty/lexy/LICENSE /opt/rx/licenses/lexy/LICENSE
COPY --from=btcpp /3rdparty/minicoro/LICENSE /opt/rx/licenses/minicoro/LICENSE
COPY --from=btcpp /3rdparty/cppzmq/LICENSE /opt/rx/licenses/cppzmq/LICENSE
COPY --from=rust-build /out/ /opt/rx/bin/
COPY --from=dhi-build /opt/rx/dhi/ /opt/rx/dhi/
COPY --from=dhi-build /dhi-guardian-build/rx-dhi-custody /opt/rx/bin/rx-dhi-custody
COPY --from=dhi-build /opt/ros/jazzy/lib/controller_manager/ros2_control_node /opt/rx/bin/rx-dhi-controller-manager
COPY --from=dhi-build /dhi-input/source-lock.json /opt/rx/manifests/dhi-source-lock.json
COPY native/dhi/session.py native/dhi/model.py native/dhi/guardian.c native/dhi/dependencies.json native/dhi/endpoint-channels.json /opt/rx/tools/dhi/
COPY native/ai-worker/l3_guard.py native/ai-worker/dependencies.json /opt/rx/tools/ai-worker/
COPY native/ai-sapiens/asset_gate.py native/ai-sapiens/dependencies.json /opt/rx/tools/ai-sapiens/
COPY native/open-manipulator/dependencies.json native/open-manipulator/requirements.py /opt/rx/tools/open-manipulator/
COPY --from=dhi /src/dynamixel_hardware_interface/LICENSE /opt/rx/licenses/dynamixel_hardware_interface/LICENSE
COPY --from=dhi /src/dynamixel_interfaces/LICENSE /opt/rx/licenses/dynamixel_interfaces/LICENSE
COPY --from=dynamixel-build /dynamixel-build/rx-dynamixel-ping /opt/rx/bin/rx-dynamixel-ping
COPY native/dynamixel/vendor/LICENSE /opt/rx/licenses/DynamixelSDK/LICENSE
COPY --from=bt-build /bt-build/rx-bt-engine /opt/rx/bin/rx-bt-engine
COPY --from=ros-jtc-build /ros-jtc-build/rx-ros-jtc-bridge /opt/rx/bin/rx-ros-jtc-bridge
COPY --from=ui-build /src/dist/ /opt/rx/operator/
COPY catalogs /opt/rx/catalogs
COPY dependencies/native-stack.lock.json /opt/rx/manifests/native-stack.lock.json
COPY native/support/solutions_status.py /opt/rx/tools/solutions_status.py
COPY native/support/entrypoint.sh /opt/rx/entrypoint.sh
COPY tools/write_runtime_inventory.py /opt/rx/tools/write_runtime_inventory.py
COPY tools/audit_native_install.py /opt/rx/tools/audit_native_install.py
RUN source /opt/ros/jazzy/setup.bash \
    && python3 /opt/rx/tools/audit_native_install.py --catalog /opt/rx/catalogs/device-support.v1.json --output /opt/rx/manifests/native-install-audit.json \
    && mkdir -p /var/lib/rx-solutions/ros-log /run/rx-solutions /run/rx-host \
    && chown -R 10001:10001 /var/lib/rx-solutions /run/rx-solutions /run/rx-host \
    && chmod 755 /opt/rx/entrypoint.sh \
    && /opt/rx/bin/rx-hostd drivers dynamixel > /opt/rx/manifests/dynamixel-driver.json \
    && python3 /opt/rx/tools/write_runtime_inventory.py
USER 10001:10001
WORKDIR /var/lib/rx-solutions
EXPOSE 8081
STOPSIGNAL SIGTERM
ENTRYPOINT ["/opt/rx/entrypoint.sh"]
CMD ["serve"]
