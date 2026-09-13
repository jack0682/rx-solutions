# Operator app delivery across two images

Phase73. The operator app is production-built in S and served by P's direct-terminal HTTPS endpoint. The development Vite proxy and port 5173 are not part of product delivery.

`npm run build` produces `dist/` and `operator-bundle.json`. The manifest contains the API schema and sorted index/asset paths with exact byte counts and SHA-256 hashes. It rejects root or leaf symlinks, reserved names, path aliases, unsupported or empty files, files larger than 8 MiB, bundles larger than 64 MiB, and more than 1,024 files. `npm run test:bundle` checks generator reproducibility and rejection cases.

The S image contains the completed bundle at `/opt/rx/operator/`. Installation extracts that bundle from the selected S image digest, publishes it to an immutable deployment directory, and connects it to P's `operator_ui` configuration and read-only mount. S does not overwrite P's UI volume during product operation. No third image is introduced.

P first verifies the manifest pin and the complete file list, hashes, and sizes, then serves only the bytes it holds. Follow the [P serving boundary](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-api/OPERATOR_UI.md). The mTLS identity used to retrieve the page is distinct from the current terminal, person, and role checks for sign-in and API requests. Certificate-header proxies and business authorization through static paths are not used.

The operator app uses fixed CSS classes for graph indentation. Vite's `assetsInlineLimit=0` exports even small font assets as separate files, preserving the UI's self-only script/style/font CSP. The data-font CSP violation discovered during real image testing was fixed without weakening the policy. No external CDN is required.

`rx-platform/tools/test_operator_delivery.py` is a browser acceptance test using a fresh temporary installation, real P/S images, a registered terminal certificate, and the production bundle. It checks bundled font rendering, desktop/mobile layouts, missing API/asset paths, certificate-free access and unregistered-certificate sign-in rejection, a lost CreateRun commit response, and recovery of the same request. Python TLS validates the server CA. Browser handling of the temporary self-signed CA is recorded separately from production certificate installation.

At phase73, this path provided existing CreateRun operations and configuration/review screens. StartRun ARMING/STARTED presentation, continuous executor assignment, and resumption after faults remained subsequent work; the later [run-start UI](RUN_START_UI.md) document covers the connected start flow. S was used as the immutable UI source in this test. The result does not establish full-cell acceptance with Host/controller processes running together.
