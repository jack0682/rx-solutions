# Authoring device templates, site settings and signed packages

2026-09-12. `rx-device-package` separates reusable device semantics from site connection values and assembles them into immutable DEVICE_REFERENCE packages read by the product Host. It currently authors restricted EnsureState packages for MELSEC Q03UDVCPU and ROS position JTC packages. MELSEC inputs are described below; JTC Template/Site, operations/outcomes, Jazzy target and the unconnected execution-provider boundary follow the [JTC package specification](../rx-host/JTC_PACKAGE.md). It is not marked complete as a generic schema capable of authoring all other robots/devices.

This tool only reads and writes files. It does not connect to PLCs/robots, read private keys, send signing messages externally or install trust, qualification or operating permissions.

phase65 generates device package software reports through `validator-identity`, `review PACKAGE POLICY REQUEST OUT` and `review-signing-request REPORT KEY_ID OUT_FILE`. It performs original reassembly/declaration checks through the actual decoder and binds the original P request to package/catalog. Signing is separate, and physical validation is NOT_PERFORMED. The policy fingerprint is compared with the original import policy; file acquisition additionally limits payloads to 8 and content to 2 MiB. Follow the [validation, signing and independent approval API](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/DEVICE_REVIEW.md).

## Inputs managed separately by authors

| Input | Contains | Does not contain |
|---|---|---|
| Template | Model, version, logical resource roles/command slots, condition/observation names and bit semantics, publication contract, completion/settle rules | Site IP, PLC M/D addresses, installation/cell IDs |
| Site | Selected template digest, installation/cell/target/site configuration/calibration, role→actual resource, slot→M address, status D address, connection settings and PLC program evidence | Arbitrary overrides of common completion semantics |
| Recipe | Package name/version/publisher and Linux target | Private keys, trust registration, dispatch authority |

Template `resource_roles` and predicate `command_slot` must each map exactly once in Site. Missing/extra entries, duplicate actual resources and overlapping command addresses are rejected. Profiles are not produced through generic string replacement or arbitrary JSON patches.

The Template digest is a semantic digest of the normalized structure. It differs from a simple SHA-256 of the original file; obtain it with the command below. `sha256` for publication/program artifacts is the SHA-256 of actual bytes. Site `site_config` and calibration digests are references to be checked by a separate authority; this tool does not prove actual site configuration/calibration.

The same Template can be retained while changing Site endpoint, addresses and actual resource names. New Template semantics require Site to explicitly name the new digest. Semantically unordered resource-role, predicate, calibration and target sets are normalized so ordering changes alone do not change the signing message.

## Command sequence

```text
rx-device-package driver-identity
rx-device-package template-digest TEMPLATE
rx-device-package assemble TEMPLATE SITE RECIPE NEW_CANDIDATE_DIRECTORY
rx-device-package request CANDIDATE KEY_ID NEW_REQUEST_FILE
rx-device-package seal CANDIDATE SIGNATURE POLICY NEW_PACKAGE_DIRECTORY
rx-device-package verify PACKAGE POLICY
rx-device-package inspect PACKAGE POLICY
```

1. Use `driver-identity` to check the fixed implementation referenced by the current tool/Host release. Authoring with the tool included in the target image uses a release with the same source identity.
2. Author Template and place the `template-digest` result in Site `template_digest`. `TEMPLATE_STRUCTURE_VALID` means structural validation passed, not physical qualification.
3. Author Site and Recipe and run `assemble`. Existing Host Profile validation checks addresses, environment, bit/settle, resources and time limits, producing `UNSIGNED_CANDIDATE`.
4. `request` stores hex and digest of the signing message bound to the key ID and canonical manifest in a new file. An actual external signer/HSM must review and sign that message.
5. `seal` uses an externally received `SignatureEnvelope` and independent local policy. It checks signature, publisher/kind/permissions, contracts/target and original assets, then publishes a `CONTENT_VERIFIED_NOT_QUALIFIED` package in a new directory.
6. `verify` rechecks current policy and bytes. `inspect` performs the same verification, then displays the interpreted profile and digest, installation/cell/environment.

The external signer signs **all original message bytes** obtained by hex-decoding the request's `message_hex` using Ed25519. It does not sign the hex string itself or the `message_digest` value. `message_digest` is SHA-256 for checking message agreement during transfer. The signing message binds the key ID and canonical manifest, so the returned key ID must match the request.

The returned file is `SignatureEnvelope` JSON with the following two fields. `signature` represents the 64 raw Ed25519 signature bytes as **128 lowercase hexadecimal characters**. The angle-bracketed portion below describes the format and is not an actual valid signature.

```json
{"key":"publisher/key-id","signature":"<128 lowercase hexadecimal characters>"}
```

Do not include private keys, base64, DER or additional schema fields. The public key and publisher/kind/permissions must be registered under the same key ID in a separate Policy. The signer is responsible for reviewing the generated candidate and exact request content; this CLI does not replace that review.

Existing result directories or signature-request files are not overwritten. Directory output uses synchronized no-replace publication through `rx-package::directory::publish_files`, shared with process packages. Simple relative output names in the current directory, such as `candidate`, are supported. Atomic publication backends beyond Linux/macOS are not yet supported.

## Signed files and validation

| File | Role |
|---|---|
| manifest.json | package v2 identity, ABI, target, permissions/assets and file inventory |
| manifest.sig.json | External Ed25519 signature after sealing |
| family.json | Model/environment |
| profile.json | Existing Host Profile generated from Template and Site |
| adapter.json | Product-pinned implementation name and source digest |
| authoring/assembly.json | Normalized original Template and Site; included in signed inventory |

Only the candidate directory additionally contains `candidate-recipe.json`. Rereading a candidate reassembles every file and manifest from this Recipe and the signed assembly and compares actual bytes. The sealed result omits the candidate Recipe; package name/version/publisher/target remain in the signed manifest itself.

The Host resolver reinterprets `authoring/assembly.json` in the 4-file format and compares it with family/profile and asset metadata. **Original/profile disagreement is rejected even with a valid signature.** Originals are not retained merely as reference comments. Existing 3-file DEVICE_REFERENCE format remains interpretable, but differs in having no reassembly originals.

The generated profile digest still covers the concrete profile, including installation/addresses. It is not interchangeable with the Template digest. Applying the same Template to a different site retains the Template digest but changes the concrete profile/package digest. Existing Host/Intent protocol digest semantics were not changed to fit this tool.

## Policy, originals and guarantee scope

Policy is existing `rx.package-verification-policy.v1`, an explicit input outside the package. It requires at most 128 keys, 32 assets of at most 1 MiB each, no package dependencies, a Linux target without a ROS requirement, current base/cell hashes and package ABI v2. Actual asset bytes/size/hash are checked. Authoring checks packages against the policy target; actual Host startup also compares the running CPU architecture.

Checking the presence/signatures of publication contracts and PLC program evidence differs from proving their physical truth. Atomic status images, sensor freshness, PLC debounce, independent protection and actual program agreement remain site qualification matters. Test-only inputs/keys must not be registered as production trust.

The tool has no default operating key, automatic signing, automatic trust updates/deployment, Host initialization, Arm or command capability. Successful authoring/inspection derives no operating permission, and output `activation_authorized` is false. Follow [Host package startup](../rx-host/DEVICE_PACKAGE_STARTUP.md) to connect a completed package to the product.

## Verification and follow-up

Tests cover reuse of one Template for two Sites, exact role/slot scope, order normalization, candidate reacquisition, current keys/assets/permissions, signed original/profile mismatch, actual CLI assembly→signing request→external test signer→seal→inspection and no-overwrite. Image tests run actual `/opt/rx/bin/rx-device-package` with network-none, non-root and a read-only root. Keys exist only in the test harness; the product CLI receives only signatures.

Execution results and source/image hashes are retained in the [phase59 verification record](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase59_checks.json). Editing UI, generic manufacturer template registry, actual device/mode-specific authoring, P device review/deployment/change/recovery flows and site acceptance remain outstanding. The first physical cell is NOT_COMMISSIONED.
