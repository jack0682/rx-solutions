# Isolated protocol validation target. This is not the RX product deployment image.
FROM ubuntu:24.04 AS protocol-validation
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential cmake ninja-build protobuf-compiler libprotobuf-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY interfaces /src/interfaces
COPY native/protocol /src/native/protocol
RUN cmake -S native/protocol -B /build -G Ninja -DCMAKE_BUILD_TYPE=Release \
    && cmake --build /build -j2
ENTRYPOINT ["/build/rx-protocol-interop"]
