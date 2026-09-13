# Core dependencies and optional device sources

The default solutions image contains ROS Jazzy, generic JointTrajectoryController communication, BehaviorTree.CPP, RX executables, and the operator application. Vendor-specific SDKs, bringup stacks, and policy models are not core dependencies.

`native-stack.lock.json` records the ROS base image digest and dependency scope. The external device repository list in `native.repos` is currently empty. Adding a device requires recording an immutable Git commit for each repository and the actual SHA-256 of required assets, followed by separate qualification. Inclusion in a repository does not grant operating authority.

Materialization of optional sources can be checked with `tools/prepare_native_sources.py` and `tools/native_source_inventory.py`. The current default image does not require this external source context. See [Native image](NATIVE_IMAGE.md) for the specific build and diagnostic boundaries and reproducibility limitations.
