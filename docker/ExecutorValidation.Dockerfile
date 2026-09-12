# Validation-only target. No ROS drivers, device permissions or production startup.
FROM rx-solutions:protocol-validation AS executor-validation
WORKDIR /src
COPY vendor/BehaviorTree.CPP /src/vendor/BehaviorTree.CPP
COPY native/executor /src/native/executor
RUN cmake -S native/executor -B /build/executor -G Ninja -DCMAKE_BUILD_TYPE=Release -DRX_BUILD_TEST_HARNESS=ON \
    && cmake --build /build/executor -j2
ENTRYPOINT ["/build/executor/rx-bt-tests"]
