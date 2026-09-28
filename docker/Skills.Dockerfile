FROM rust:1.98.1-bookworm@sha256:9a73a5088750b4c95158ab26629c854c3d6fc4b173cb7bc8079ad252d8ed7bfa AS build
WORKDIR /src
COPY --from=platform /Cargo.toml /Cargo.lock ./
COPY --from=platform /crates ./crates
COPY --from=platform /proto ./proto
COPY --from=platform /spec ./spec
RUN --mount=type=cache,id=rx-skills-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=rx-skills-build,target=/src/target \
    cargo build --locked --release -p rx-api --bin rx-skill-server && cp target/release/rx-skill-server /rx-skill-server

FROM python:3.12-slim-bookworm@sha256:392307d22300de8b5986851a12d9176dfc0fc073e65bf6523ebd7dcbeb23564e
LABEL org.opencontainers.image.title="RX local simulation skills" \
      org.opencontainers.image.version="0.3.0-rc.2"
ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1
RUN mkdir -p /data /opt/rx /config && chown 10001:10001 /data
COPY --from=build /rx-skill-server /usr/local/bin/rx-skill-server
COPY deployment/local-skills/worker.py deployment/local-skills/runner.py /opt/rx/
COPY LICENSE NOTICE /opt/rx/
USER 10001:10001
WORKDIR /data
CMD ["rx-skill-server", "/data", "/config/client-token", "/config/worker-token"]
