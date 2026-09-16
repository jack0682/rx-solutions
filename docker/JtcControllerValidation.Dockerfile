# Validation only: official controller software with GenericSystem mock hardware.
# The product image's dormant entrypoint and Authority boundary stay unchanged.
ARG RX_BASE_IMAGE=rx-solutions:runtime-draft
FROM ${RX_BASE_IMAGE}
USER root
RUN apt-get update && apt-get install -y --no-install-recommends \
    ros-jazzy-controller-manager ros-jazzy-ros2controlcli \
    && dpkg-query -W -f='${Package}\t${Version}\t${Architecture}\n' | sort > /opt/rx/manifests/n5-packages.tsv \
    && rm -rf /var/lib/apt/lists/*
COPY native/ros-jtc/controller_validation/run.py \
     native/ros-jtc/controller_validation/controllers.yaml \
     native/ros-jtc/controller_validation/mock.urdf \
     native/ros-jtc/controller_validation/goal.json /opt/rx/validation/n5/
USER 10001:10001
WORKDIR /tmp
ENTRYPOINT ["/bin/bash", "-c", "source /opt/ros/jazzy/setup.bash && exec python3 /opt/rx/validation/n5/run.py"]
