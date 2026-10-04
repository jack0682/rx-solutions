ARG BASE_IMAGE
FROM ${BASE_IMAGE}
USER 0
RUN python3 -m pip install --no-cache-dir grpcio==1.84.0 grpcio-tools==1.84.0 protobuf==7.36.2
COPY sdk/proto/ /opt/rx/result-link/proto/
RUN mkdir /opt/rx/result-link/generated && python3 -m grpc_tools.protoc -I/opt/rx/result-link/proto --python_out=/opt/rx/result-link/generated $(find /opt/rx/result-link/proto -name '*.proto' | sort)
COPY deployment/simulation/result-link/link.py /opt/rx/result-link/link.py
ENV PYTHONPATH=/opt/rx/result-link/generated
USER 10001:10001
ENTRYPOINT ["python3","/opt/rx/result-link/link.py"]
