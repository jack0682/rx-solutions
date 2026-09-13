# Native contract binding foundation

This target builds the exported RX v1 Protobuf message types and strict wire parser.
The CMake configuration verifies the exported bundle's file hashes before compilation.

All inbound bytes must use rx::wire::parse before application validation. The ordinary generated parser alone merges duplicate singular values and cannot enforce the RX contract. Structural validity does not establish authentication, request idempotency, a valid permit, or a physical result.

The isolated docker/ProtocolValidation.Dockerfile target is used for cross-language tests without device mounts or network. It is not a product image and does not replace the ROS/native image build.

Runtime gRPC services, Host journals, native capability/profile binding and C++ Intent canonical hashing are subsequent work. Current interop proves message meaning survives Rust→C++→Rust; the final digest comparison is performed by Rust.
