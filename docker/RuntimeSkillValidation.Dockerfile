# Actual P/Host/Executor binaries for a device-free integration acceptance image.
# This is not the complete distribution image or a physical deployment profile.
FROM rust:1.98.1-bookworm@sha256:9a73a5088750b4c95158ab26629c854c3d6fc4b173cb7bc8079ad252d8ed7bfa AS p-build
WORKDIR /source
COPY --from=platform_source /Cargo.toml /Cargo.lock ./
COPY --from=platform_source /crates ./crates
COPY --from=platform_source /proto ./proto
COPY --from=platform_source /spec ./spec
RUN --mount=type=cache,id=rx-runtime-skill-p-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=rx-runtime-skill-p-target,target=/source/target \
    cargo build --release --locked -p rx-platformd && mkdir /out && cp target/release/rx-platformd target/release/rx-package-store /out/
RUN find /source -type f -not -path '/source/target/*' -print0 | sort -z | xargs -0 sha256sum > /out/source.sha256

FROM rust:1.98.1-bookworm@sha256:9a73a5088750b4c95158ab26629c854c3d6fc4b173cb7bc8079ad252d8ed7bfa AS s-build
WORKDIR /source
COPY Cargo.toml Cargo.lock ./
COPY sdk ./sdk
COPY runtime ./runtime
COPY drivers ./drivers
COPY catalogs ./catalogs
COPY native ./native
COPY dependencies ./dependencies
COPY interfaces ./interfaces
RUN --mount=type=cache,id=rx-runtime-skill-s-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=rx-runtime-skill-s-target,target=/source/target \
    cargo build --release --locked -p rx-host -p rx-executor -p rx-process-package && mkdir /out && cp target/release/rx-hostd target/release/rx-executor-service target/release/rx-process-package /out/
RUN --mount=type=cache,id=rx-runtime-skill-s-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=rx-runtime-skill-s-target,target=/source/target \
    cargo test --locked -p rx-host --features test-harness --test process_crash sigkill_at_both_journal_native_boundaries_never_replays_device_effect -- --exact > /out/host-recovery.log && \
    cargo test --locked -p rx-executor --test assignment_journal lost_run_initialization_reply_recovers_binding_without_reinitializing -- --exact > /out/executor-recovery.log && \
    cargo test --locked -p rx-executor --test assignment_journal run_creation_marker_cannot_change_after_header_initialization -- --exact > /out/executor-identity.log
RUN find /source -type f -not -path '/source/target/*' -print0 | sort -z | xargs -0 sha256sum > /out/source.sha256

FROM python:3.12-slim-bookworm@sha256:392307d22300de8b5986851a12d9176dfc0fc073e65bf6523ebd7dcbeb23564e AS bt-build
RUN apt-get update && apt-get install -y --no-install-recommends g++ cmake make nlohmann-json3-dev libzmq3-dev && rm -rf /var/lib/apt/lists/*
COPY --from=btcpp / /btcpp/
COPY native/executor /executor/
RUN --network=none cmake -S /executor -B /build -DCMAKE_BUILD_TYPE=Release -DBTCPP_SOURCE=/btcpp -DRX_BUILD_TEST_HARNESS=OFF && cmake --build /build --target rx-bt-engine -j2

FROM python:3.12-slim-bookworm@sha256:392307d22300de8b5986851a12d9176dfc0fc073e65bf6523ebd7dcbeb23564e AS platform
RUN mkdir -p /data /run/rx && chown -R 10001:10001 /data /run/rx
COPY --from=p-build /out/ /usr/local/bin/
COPY LICENSE NOTICE /opt/rx/
USER 10001:10001
ENTRYPOINT ["/usr/local/bin/rx-platformd"]

FROM python:3.12-slim-bookworm@sha256:392307d22300de8b5986851a12d9176dfc0fc073e65bf6523ebd7dcbeb23564e AS solutions
RUN apt-get update && apt-get install -y --no-install-recommends libstdc++6 libzmq5 && rm -rf /var/lib/apt/lists/* && mkdir -p /data /opt/rx/bin && chown 10001:10001 /data
COPY --from=s-build /out/ /opt/rx/bin/
COPY --from=bt-build /build/rx-bt-engine /opt/rx/bin/
COPY LICENSE NOTICE /opt/rx/
COPY deployment/local-skills/rx deployment/local-skills/runtime_client.py deployment/local-skills/image_identity.py /opt/rx/client/
USER 10001:10001
ENTRYPOINT ["/opt/rx/bin/rx-hostd"]
