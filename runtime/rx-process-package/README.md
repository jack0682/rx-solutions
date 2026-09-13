# Process package assembly, external signing and content verification

2026-09-12. `rx-process-package` bundles authored compile input into deterministic Process package candidates and connects external detached signatures to the existing package verifier. It does not generate/store signing keys or perform P trust registration, deployment or execution.

## Stages and commands

```text
rx-process-package assemble BUNDLE RECIPE NEW_CANDIDATE_DIRECTORY
rx-process-package request CANDIDATE KEY_ID NEW_REQUEST_FILE
rx-process-package seal CANDIDATE SIGNATURE POLICY NEW_PACKAGE_DIRECTORY
rx-process-package verify PACKAGE POLICY
rx-process-package compile PACKAGE POLICY NEW_OUTPUT_DIRECTORY
```

- assemble validates digest-matching originals/bindings through the existing S compiler and produces `UNSIGNED_CANDIDATE`.
- request records hex/digest of the actual signing message bound to the key ID and canonical manifest. It can be reviewed by a person or passed to an external signer/HSM. This command neither sends the message externally nor reads private keys.
- seal validates the external signature and explicit local trust policy, rechecks content semantics and publishes the completed package in a new directory.
- verify/compile check signature, current trust, files, target/contracts, locked dependencies and artifacts, then reconstruct the process from verified immutable bytes.

Verification/compilation results are `CONTENT_VERIFIED_NOT_QUALIFIED`. VerifiedPackage or a signature does not mean field qualification, native readiness, operator approval or dispatch permit.

## File layout and determinism

| File | Contents |
|---|---|
| manifest.json | Existing `rx.package.v1` manifest and exact inventory |
| manifest.sig.json | Ed25519 detached signature, present only after sealing |
| process/source.json | Process source |
| process/bindings.json | Host+intent inputs checked for normalization |
| authoring/compile-input.json | Original source/binding/cell/catalog provenance |
| authoring/package-recipe.json | Package/version/publisher/contract/target/dependency/asset selections |
| process/context-requirements.json | Required bindings among profile/site/calibration/tool/mode/stream and catalog digests |

Identical inputs and recipe produce the same manifest digest. Candidate reacquisition compares the reassembled manifest and all file bytes. This correspondence is checked again after signature verification, rejecting packages with valid signatures but inconsistent internal originals/bindings.

Derived resolved/BT files are not embedded in the original package with their own package digest. They are recompiled from the verified package, and resulting package_digest is bound to the actual manifest digest. No circular self-hash is created.

## Permissions and external references

Requested permissions derive from OperationSubmit actually used by the process, ObservationRead schemas for condition/control sources, and ArtifactRead. Process packages contain no NativeEndpoint or executable files. Unused additional action bindings are not silently approved.

ArtifactRefs such as trajectory/program/parameter-set/procedure must be declared with identical metadata in recipe assets. Different metadata for the same digest is rejected. Semantic digests such as profile/site/calibration are not disguised as simple file SHA values; they remain separate context requirements. These must be checked by actual device/profile/site authorities.

Current ActionBinding is Host+intent. Full Cell StepBinding condition/completion/procedure semantics and the path for assembling/activating actual device packages remain future work. This tool does not claim a complete cell package with those checks omitted.

## Verification policy and external signer

Policy files use single-ABI `rx.package-verification-policy.v1` or `rx.package-verification-policy.v2` with an explicit additional-ABI list. They specify contracts/target, explicit key IDs/publishers/public keys/allowed package kinds/permissions, asset files to check and locked dependency paths. This is a local verification basis supplied by the CLI caller; it is not automatically registered as P operational trust. v2 `additional_package_abis` specifies 1–8 unique ABIs beyond the primary ABI. Allowing device ABI v2 and process ABI v1 in one policy retains each signer's kind/permission and content validation.

Asset files are checked for size and actual SHA-256. Dependency directories are read through bounded capability acquisition, checked for designated manifest digests and then verified in order. Cyclic/missing dependencies and revoked/out-of-scope keys do not pass. Current policy checks are not skipped merely because an object was verified previously.

Policy limits are 128 keys, 1,024 assets/256 MiB total and 128 dependencies. This CLI acquires each package within 32 files/4 MiB of content; it does not replace deployment tools for larger device packages/streaming assets.

An actual production signing service and operational trust provisioning/revocation procedures are not yet connected. Tests use fixed test-only keys solely in test code and export only signatures, public policies and expected signing messages. No default product keys or private-key files are generated.

## Acquisition and output

Shared `rx-package::directory::acquire_directory` returns only owned bytes before verification. This type/path cannot be labeled as creating VerifiedPackage. Existing symlink/special-file/path/count/size restrictions remain. Actual verify_directory continues original signature/content verification after acquisition.

Output uses SDK publish_files shared with the device authoring tool. It completes and syncs files in a temporary directory under the same parent, then publishes with Linux/macOS no-replace rename. Existing files/directories/symlinks are not overwritten. Atomic publication backend on Windows and defenses against host-root attacks are outside this stage's verification scope.

## Verification scope

Tests cover deterministic assembly, external signing, recompilation from immutable bytes, signer kinds/permissions/key aliases, file/inventory changes, signed-but-inconsistent content, undeclared artifacts/parallel resource conflicts, current dependency trust, asset content changes and no-overwrite output. Separate evidence also records assembly→signing request→test-only detached-signature sealing→verification→compilation of a previously authored actual process in the final image.

Package review approval, P artifact admission/activation, complete device/site context validation, operator UI validation/signing/deployment flows and site acceptance remain outstanding. The first physical cell is NOT_COMMISSIONED.

P/S local trust-policy acquisition shares SDK `rx-package::policy`. P's separate retention boundary and offline tools are described in [STORE.md](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-package/STORE.md). Successful P retention is not promoted to this crate's process semantic revalidation or user/cell approval.

## Review artifact output

Provides `validator-identity`, `review PACKAGE POLICY REQUEST OUT` and `review-signing-request REPORT KEY_ID OUT`. Actual signed packages are checked through compile_verified to produce canonical review/result artifacts for an external signer to sign the exact bytes. No signing service/private keys are included. For P's independent inspection and approval scope, see [process review](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/PROCESS_REVIEW.md).
