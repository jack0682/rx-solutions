# Signed device packages and product Host startup

Written 2026-09-12. `MELSEC_PACKAGE` has been connected to the fixed factory in the product `rx-hostd`. Package verification, journal initialization and passive open are distinct from the existing qualification acceptance/Arm boundary. Commissioning of the first actual laser cell is not yet complete.

## 1. Product selections

Product configuration has the following three backend forms.

| kind | Current meaning |
|---|---|
| FILE_SIMULATION | Existing file-based device simulation backend |
| MELSEC_PACKAGE | Restricted MELSEC EnsureState backend from a signed DEVICE_REFERENCE package |
| VALIDATED_DRIVER | Previously reserved form. Still rejected; it is not treated as an implemented generic driver registry |

MELSEC_PACKAGE specifies an **absolute package directory, expected manifest digest, and the path and SHA-256 of an independent policy file**. It does not load executables or shared libraries from user paths. The only selectable implementation is the compiled `rx.melsec.ensure-state.v1`, and the actual model list is currently restricted to Q03UDVCPU. This does not mean actual Q03UDVCPU field validation is complete.

`rx-hostd drivers` outputs current implementation descriptors as JSON without a configuration file. `inspect CONFIG` validates the package, policy and static bindings; `init CONFIG` creates a new installation; `run CONFIG` opens only an existing installation. The program cannot execute arbitrary plugin paths.

## 2. Package and independent policy

The new [DEVICE_REFERENCE v2](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-package/DEVICE_REFERENCE.md) uses `rx.package.v2` and `rx.package-abi.v2`. It does not relax the executable requirement of existing DEVICE v1.

The MELSEC resolver accepts three baseline non-executable files. The format produced by the new authoring tool includes a fourth signed file, `authoring/assembly.json`; the Host reassembles the profile from Template/Site and compares it.

| Role | Schema / value | Check |
|---|---|---|
| family | rx.melsec-family.v1 | mitsubishi/melsec-q, Q03UDVCPU, same environment as profile |
| profile | rx.melsec-predicate-profile.v1 | Installation, cell, address, resource, observation, state/completion contracts and exact Intent |
| adapter descriptor | rx.native-driver-reference.v1 | Fixed implementation name and currently compiled source digest |

Detailed `profile` semantics and the 9-word publication prerequisites follow the [adapter specification](MELSEC_ADAPTER.md). A current package contains one Profile fixed to an installation. [rx-device-package](../rx-device-package/README.md) authors separate MELSEC Template and Site inputs and assembles them into a concrete installation package. Authoring schemas for other manufacturers remain future work; this format is not established as the final model for all devices.

The policy is a pinned file outside the package. It reuses the trusted publisher/key/kind/permission and asset acquisition capabilities of existing `rx.package-verification-policy.v1`. The policy's base/cell hashes must match the current baseline embedded in the binary, and the target must be Linux for the current CPU architecture with no ROS requirement. Dependency packages are not currently allowed.

The exact permission set is ArtifactRead, NativeEndpoint(role=melsec-mc3e) and ObservationRead(schema=rx.melsec.status-image.v1). Other permissions or executable payloads are rejected. The content inventory is the baseline three files, or four with the signed assembly; acquisition is limited to at most 8 files and 2 MiB of content. Policy assets are limited to 32 assets, each at most 1 MiB, and actual bytes, size and digest are checked.

Original assets for the publication contract and PLC program evidence are required. A reference without bytes, or changed bytes, causes failure. **The presence of original bytes and a valid signature does not prove that a document's physical claims are true.** Agreement between the program and actual PLC, atomic observation, freshness, debounce and independent protection must be assessed in separate actual qualification. The profile's `plc_program` reference is linked by this resolver to a program evidence artifact; there is no capability to read the PLC program through MC and compare its hash.

## 3. Implementation source pin scope

At build time, the Host/MC driver source files, their Cargo manifests, workspace Cargo.lock, Host build script and SDK source-lock are sorted to produce the source digest. The descriptor must match this digest exactly. A Linux-target package authored/inspected on macOS uses the same digest in the Linux image if its source inventory is identical.

This value is **source identity**. It does not replace an OCI image digest, proof of compiler reproducibility, binary signature or ABI compatibility assessment. An operational release still needs separate image/configuration pins. Matching is currently strict and exact, so relevant source/dependency changes require updating the descriptor and package signature. Automatic compatibility inference and native installation migration are not implemented.

## 4. Initialization and restart

Package/policy and single-cell bindings are checked before initialization. The profile installation ID must equal the Host installation ID, and every allowed Intent must match the profile exactly. Conditions and environment are also compared.

Host journals and the MELSEC native journal are created in a new staging directory, and `installation.json` records both Host journal IDs together with native journal/profile/manifest identity. This stage does not connect to the PLC or send commands. After syncing files and directories, a **no-replace rename** publishes the installation. Even an empty destination directory created concurrently by someone else is not overwritten. Failed unpublished stages are cleaned up; an already published installation is not deleted under the guise of error recovery.

run revalidates the package/policy and compares the existing installation, Host/native journals, manifest/profile identity and ownership locks. It does not create a new ledger to fill in a missing or empty native DB. Passive open does not even establish a TCP connection; connection is deferred until an actual status read.

Native metadata remains optional for existing FILE_SIMULATION installations, which remain readable. That metadata is mandatory for MELSEC installations. There is no path that automatically reuses an existing native installation after changing configuration to a different package/manifest. Updates, restoration and controlled rebind remain future work.

Verification occurs during inspect/init/run in the current process. A trust service that monitors file/key revocation during execution is not yet connected. Operational revocation requires Platform authority revocation/blocking and normal stop procedures; changing a pinned policy is not claimed to immediately stop an already running process.

## 5. Package verification versus operating qualification

A PHYSICAL binding cannot Arm using only the startup `qualification` ID/revision. Qualification acceptance must be delivered by an authenticated Platform for the current Host boot, journals, binding and process context. A separate Arm, current grant/fence/permit and native guard are still required afterward. The same rule applies to other physical NativeAdapters, while the existing bootstrap method for simulation fixtures is retained.

```mermaid
flowchart LR
    A[Check signature, originals and source pin] --> B[Initialize Host and native journals]
    B --> C[SOFTWARE_READY_UNARMED]
    C --> D[Accept current process context]
    D --> E[Accept current qualification]
    E --> F[Separate Arm and work authorization]
    F --> G[Command after final native guard]
```

`physical_backend_available=true` means that the selected package's physical backend configuration is supported. `physical_qualification_verified=false` and `activation_authorized=false` remain displayed as such. The UI must not combine these into “device ready for operation.” Startup inspection itself does not perform expert assessment of the qualification artifact.

## 6. Verification and remaining work

Package verification tests reject incorrect signature/content/policy/keys revoked at verification time/assets/contracts, correctly re-signed packages with incorrect sources or controllers, and installation mismatches. Initialization collision tests confirm preservation even of an empty destination directory created concurrently.

Host library tests use PHYSICAL **metadata** and a loopback PLC to verify Arm rejection with startup IDs alone, then separate Arm and command/completion after current context/qualification acceptance. These materials, signers and qualification requests are explicitly for tests. They grant no PHYSICAL qualification to an actual site.

Product image tests supply a signed simulated package to actual `rx-hostd`. With network-none, non-root and a read-only root, they check init/passive run, native journal identity, mTLS h2, duplicate owner rejection and separate drop proof after SIGTERM. The Python PLC is a loopback helper process started directly by the test, not code executed by the package. This image test performs 0 native writes.

Actual commands, results and source/image hashes are retained in the [phase58 verification record](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase58_checks.json). Physical device publication/program verification and site acceptance, device package authoring for other manufacturers and an authoring UI, runtime trust-revocation monitoring, image/package changes and native journal migration, full disaster recovery/retention policy, and device-specific backend connections remain outstanding.
