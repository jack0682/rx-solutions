FROM ros:jazzy-ros-base@sha256:386d06ec6d4188f731bae5678e07b4cb64a4e4d4152090c0bd1f881dcf7706f5 AS native-dependencies
SHELL ["/bin/bash", "-o", "pipefail", "-c"]
ENV DEBIAN_FRONTEND=noninteractive CMAKE_BUILD_PARALLEL_LEVEL=1 MAKEFLAGS="-j1 -l1"
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential cmake ninja-build curl ca-certificates python3-colcon-common-extensions \
    python3-rosdep python3-yaml libyaml-cpp-dev libeigen3-dev \
    && rm -rf /var/lib/apt/lists/*
COPY --from=robotis /package-manifests.json /opt/rx/input/package-manifests.json
COPY tools/materialize_package_manifests.py /opt/rx/tools/materialize_package_manifests.py
RUN python3 /opt/rx/tools/materialize_package_manifests.py /opt/rx/input/package-manifests.json /workspace/dependencies
ENV ROSDISTRO_INDEX_URL=https://raw.githubusercontent.com/ros/rosdistro/86ac06531559f070ce9ea5ad485714395336ec6f/index-v4.yaml
RUN mkdir -p /etc/ros/rosdep/sources.list.d && \
    printf 'yaml https://raw.githubusercontent.com/ros/rosdistro/86ac06531559f070ce9ea5ad485714395336ec6f/rosdep/base.yaml\nyaml https://raw.githubusercontent.com/ros/rosdistro/86ac06531559f070ce9ea5ad485714395336ec6f/rosdep/python.yaml\n' > /etc/ros/rosdep/sources.list.d/20-default.list
RUN if [ ! -f /etc/ros/rosdep/sources.list.d/20-default.list ]; then rosdep init; fi \
    && rosdep update --rosdistro jazzy \
    && apt-get update \
    && rosdep install --from-paths /workspace/dependencies --ignore-src --rosdistro jazzy -y \
    && apt-get install -y --no-install-recommends ros-jazzy-rmw-zenoh-cpp \
    && mkdir -p /opt/rx/manifests \
    && dpkg-query -W -f='${Package}\t${Version}\t${Architecture}\n' | sort > /opt/rx/manifests/native-packages.tsv \
    && rm -rf /var/lib/apt/lists/*
ARG TARGETARCH
RUN case "$TARGETARCH" in \
      arm64) ORT_ARCH=aarch64; ORT_SHA=7c63c73560ed76b1fac6cff8204ffe34fe180e70d6582b5332ec094810241e5c ;; \
      amd64) ORT_ARCH=x64; ORT_SHA=1fa4dcaef22f6f7d5cd81b28c2800414350c10116f5fdd46a2160082551c5f9b ;; \
      *) exit 1 ;; \
    esac \
    && curl --fail --location --proto '=https' --tlsv1.2 "https://github.com/microsoft/onnxruntime/releases/download/v1.23.2/onnxruntime-linux-${ORT_ARCH}-1.23.2.tgz" -o /tmp/ort.tgz \
    && echo "${ORT_SHA}  /tmp/ort.tgz" | sha256sum --check \
    && mkdir -p /opt/onnxruntime \
    && tar -xzf /tmp/ort.tgz --strip-components=1 -C /opt/onnxruntime \
    && rm /tmp/ort.tgz

FROM native-dependencies AS native-build
WORKDIR /workspace
COPY --from=robotis / /workspace/src/
COPY tools/native_source_inventory.py /opt/rx/tools/native_source_inventory.py
RUN python3 /opt/rx/tools/native_source_inventory.py /workspace/src
RUN --network=none source /opt/ros/jazzy/setup.bash \
    && colcon list --base-paths src --names-only | sort > /opt/rx/manifests/native-source-packages.txt \
    && colcon build --base-paths src --executor sequential --merge-install \
       --install-base /opt/rx/robotis --cmake-args -DCMAKE_BUILD_TYPE=Release -DBUILD_TESTING=OFF -DCMAKE_AUTOGEN_PARALLEL=1 -DONNXRUNTIME_ROOT=/opt/onnxruntime \
    && cp src/source-lock.json /opt/rx/manifests/native-sources.json

FROM native-build AS native-audit
COPY native/support/onnxruntime.conf /etc/ld.so.conf.d/rx-onnxruntime.conf
RUN ldconfig
COPY tools/audit_native_install.py /opt/rx/tools/audit_native_install.py
COPY native/support/ort_inspect.cpp /tmp/ort_inspect.cpp
COPY catalogs/robotis-support.v1.json /opt/rx/manifests/robotis-support.v1.json
RUN --network=none g++ -std=c++17 /tmp/ort_inspect.cpp -I/opt/onnxruntime/include -L/opt/onnxruntime/lib -Wl,-rpath,/opt/onnxruntime/lib -lonnxruntime -o /opt/rx/tools/ort-inspect \
    && source /opt/ros/jazzy/setup.bash && source /opt/rx/robotis/setup.bash \
    && python3 /opt/rx/tools/audit_native_install.py --catalog /opt/rx/manifests/robotis-support.v1.json --output /opt/rx/manifests/native-install-audit.json

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

FROM native-build AS ros-jtc-build
COPY native/ros-jtc /ros-jtc/
COPY catalogs/robotis-support.v1.json /catalogs/robotis-support.v1.json
RUN --network=none source /opt/ros/jazzy/setup.bash \
    && cmake -S /ros-jtc -B /ros-jtc-build -G Ninja -DCMAKE_BUILD_TYPE=Release -DRX_CATALOG=/catalogs/robotis-support.v1.json \
    && cmake --build /ros-jtc-build --target rx-ros-jtc-bridge -j1


FROM native-build AS native-validation
RUN --network=none source /opt/ros/jazzy/setup.bash && source /opt/rx/robotis/setup.bash \
    && colcon build --base-paths /workspace/src --packages-select ai_sapiens_sim2real \
       --executor sequential --build-base /verify-build --install-base /verify-install \
       --cmake-args -DCMAKE_BUILD_TYPE=Release -DBUILD_TESTING=ON -DONNXRUNTIME_ROOT=/opt/onnxruntime
COPY dependencies/ros-schema /opt/rx/ros-schema
ENV XML_CATALOG_FILES=/opt/rx/ros-schema/catalog.xml
RUN --network=none source /opt/ros/jazzy/setup.bash && source /opt/rx/robotis/setup.bash && source /verify-install/setup.bash \
    && rx_test_exit=0 \
    && { colcon test --base-paths /workspace/src --build-base /verify-build --packages-select ai_sapiens_sim2real --return-code-on-test-failure --event-handlers console_direct+ --ctest-args --output-on-failure || rx_test_exit=$?; } \
    && { colcon test-result --test-result-base /verify-build --verbose || true; } \
    && exit "$rx_test_exit"


FROM native-dependencies AS runtime
ARG TARGETARCH
COPY native/support/onnxruntime.conf /etc/ld.so.conf.d/rx-onnxruntime.conf
COPY dependencies/native-apt-locks /opt/rx/apt-locks
RUN ldconfig
ENV PYTHONDONTWRITEBYTECODE=1 ROS_LOG_DIR=/var/lib/rx-solutions/ros-log
COPY --from=native-audit /opt/rx/robotis /opt/rx/robotis
COPY --from=native-audit /opt/rx/manifests /opt/rx/manifests
COPY --from=rust-build /out/ /opt/rx/bin/
COPY --from=bt-build /bt-build/rx-bt-engine /opt/rx/bin/rx-bt-engine
COPY --from=ros-jtc-build /ros-jtc-build/rx-ros-jtc-bridge /opt/rx/bin/rx-ros-jtc-bridge
COPY --from=ui-build /src/dist/ /opt/rx/operator/
COPY catalogs /opt/rx/catalogs
COPY dependencies/native-stack.lock.json /opt/rx/manifests/native-stack.lock.json
COPY native/support/solutions_status.py /opt/rx/tools/solutions_status.py
COPY native/support/entrypoint.sh /opt/rx/entrypoint.sh
COPY tools/write_runtime_inventory.py /opt/rx/tools/write_runtime_inventory.py
RUN cmp /opt/rx/manifests/native-packages.tsv /opt/rx/apt-locks/${TARGETARCH}.tsv \
    && rm -rf /workspace/dependencies \
    && mkdir -p /var/lib/rx-solutions/ros-log /run/rx-solutions \
    && chown -R 10001:10001 /var/lib/rx-solutions /run/rx-solutions \
    && chmod 755 /opt/rx/entrypoint.sh \
    && python3 /opt/rx/tools/write_runtime_inventory.py
RUN mkdir -p /run/rx-host && chown 10001:10001 /run/rx-host
USER 10001:10001
WORKDIR /var/lib/rx-solutions
EXPOSE 8081
STOPSIGNAL SIGTERM
ENTRYPOINT ["/opt/rx/entrypoint.sh"]
CMD ["serve"]
