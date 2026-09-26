# Vendor-neutral solutions image

Configuration dated 2026-09-14. Includes ROS Jazzy, a position JTC bridge, the BT engine, Rust Host, Executor, Supervisor and package tools, and the operator UI. Build and installation test results for the earlier vendor-specific image remain in Git history and are not evidence of success for the new image. Physical device qualification is `NOT_PERFORMED`.

## Sources and build

ROS, Rust, and Node base images are pinned by digest in the Dockerfile. Prepare BehaviorTree.CPP at the existing pinned commit specified in `dependencies/behaviortree_cpp.repos`. The default device catalog is a newly written `SIMULATION_FIXTURE`, not physical-model data with its names replaced. The lists of external device repositories and policy assets are empty.

```sh
vcs import vendor < dependencies/behaviortree_cpp.repos
python3 tools/check_device_catalog_sources.py
python3 tools/prepare_dhi_sources.py --output .cache/dhi-sources
python3 tools/prepare_dhi_sources.py --output .cache/dhi-sources --check
docker build --build-context btcpp=vendor/BehaviorTree.CPP \
  --build-context dhi=.cache/dhi-sources \
  -f docker/Solutions.Dockerfile --target runtime \
  -t rx-solutions:runtime-draft .
python3 tools/test_solutions_image.py --image rx-solutions:runtime-draft --evidence /tmp/rx-solutions-image.json
```

The SDK must match the platform's current export. Network access is disabled during the ROS JTC and BT C++ compilation stages. Installation checks verify the presence, ELF format, and dynamic linking of required RX executables; they do not start controllers or hardware plugins.

Actual APT package versions and architectures are recorded in each image's `native-packages.tsv` and included in the runtime inventory. **An APT repository snapshot and approved inventories per architecture have not yet been pinned.** Base image digests alone do not guarantee a fully reproducible build. APT locks from the earlier vendor configuration are not reused for the new configuration.

When optional external sources are needed, record HTTPS URLs and 40-character commits under repositories in `native-stack.lock.json` and use `tools/prepare_native_sources.py`. This tool prepares Git archives in a separate location without modifying the originals and rejects submodules, unresolved LFS objects, and asset hash mismatches. Installation and execution paths for optional sources require separate changes and validation.

## Default execution and validation boundary

The default entrypoint sets up the ROS environment and runs read-only software diagnostics only. It runs as user `10001:10001` with state `SOFTWARE_READY_UNCOMMISSIONED`. `/health` and `/api/v1/solution/support` expose installation and simulation declaration counts; control POST requests are rejected with `CONTROL_NOT_EXPOSED`.

At startup, SHA-256 values are checked for executables, the catalog, UI, diagnostic code, and installation inventory. Default startup does not start a controller manager, device driver, BT engine, Host, Executor, or ROS router. The UI bundle is included, but the diagnostic server does not replace a production operator API.

The image smoke test checks diagnostics, control rejection, SIGTERM shutdown, and catalog-tampering rejection with a read-only root, cap-drop ALL, no-new-privileges, and no connected devices. Physical devices, site handover, real-time performance, multi-domain operation, and a production Authority provider remain separate open items. Explicit `supervise` and `host` execution follows the contracts of the respective services.

The [DHI PTY investigation](../native/dhi/README.md) builds three immutable source archives and exact controller-manager package4.48.0. Upstream floating `.repos` entries are not used. The image build rechecks the prepared source inventory before compiling. Direct component tests and the signed registered resident path separately exercise the simulated ownership mechanism; the latter verifies release loading, catalog construction, plan validation and the same guardian's resource admission. This is not physical support or general Compose/s6 integration.

The [AI Worker L3 simulation](../native/ai-worker/README.md) copies only its
release-pinned ownership gate and dependency manifest into the native image. It
does not build or start the upstream privileged AI Worker image. The fixed
source audit records the upstream `restart: always`, s6 service tree, ZED mounts,
`SYS_NICE` and `rtprio: 99`, then refuses the physical profile while the ZED,
RT, base-image and download-digest closure remains unqualified.
