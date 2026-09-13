FROM ros:jazzy-ros-base@sha256:386d06ec6d4188f731bae5678e07b4cb64a4e4d4152090c0bd1f881dcf7706f5 AS native-dependencies
SHELL ["/bin/bash", "-o", "pipefail", "-c"]
ENV DEBIAN_FRONTEND=noninteractive CMAKE_BUILD_PARALLEL_LEVEL=1 MAKEFLAGS="-j1 -l1"
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential cmake ninja-build ca-certificates python3 \
    nlohmann-json3-dev libzmq3-dev libsqlite3-dev \
    ros-jazzy-rclcpp ros-jazzy-control-msgs ros-jazzy-controller-manager-msgs \
    ros-jazzy-action-msgs ros-jazzy-joint-trajectory-controller \
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
    && python3 /opt/rx/tools/write_runtime_inventory.py
USER 10001:10001
WORKDIR /var/lib/rx-solutions
EXPOSE 8081
STOPSIGNAL SIGTERM
ENTRYPOINT ["/opt/rx/entrypoint.sh"]
CMD ["serve"]
