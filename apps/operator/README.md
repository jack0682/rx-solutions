# RX operator UI draft

This React/TypeScript interface connects to the real platform API. It provides sign-in, role-based cell state, preparation of new runs, operation holds, and views of records, configuration, and account access. The UI does not maintain a separate operating-authority or outcome ledger.

The initial local development scope did not connect actual run start, site terminal authentication, a workflow editor, or the complete recovery, verification, deployment, and account-administration flows. The later sections and linked implementation documents describe the subsequently connected features. Planned features are not presented as working controls.

## Source layout

| File | Responsibility |
|---|---|
| `schema.ts` | Validate API read models and stored request formats; never display an unknown state as normal |
| `api.ts` | Same-origin HTTP and cookies; distinguish timeouts, errors, and undetermined outcomes |
| `pending.ts` | Store requests in sessionStorage before sending; recover the same key and content; bind them to account, installation, and store generation |
| `App.tsx` | Sign-in and query lifecycle, fixed reviewed revisions, and connections between user requests and the UI |
| `views.tsx` | Run and operation records, and configuration read views |
| `labels.ts` | User-facing state terminology and English date formatting |
| `style.css` | Shared layout, responsive behavior, and keyboard focus styling |

The app follows [React's standalone app guidance](https://react.dev/learn/build-a-react-app-from-scratch) and the [Vite proxy](https://vite.dev/config/server-options) approach. The npm-pinned IBM Plex Sans KR font supports the English interface and is bundled as local assets. No external CDN is required.

## State and request rules

- An unregistered qualification appears as awaiting verification. Network connectivity or an existing configuration does not establish operating readiness.
- Display operation observations, outcomes, integrity, and resource disposition separately. Confirmed completion does not mean that resource handover is complete.
- The current implementation polls snapshots every three seconds. A query error, or ten seconds since the last successful check, blocks new requests and marks the data as historical. This is not an SSE implementation.
- Opening a confirmation dialog fixes the cell revision and request content. Background updates do not silently replace the reviewed content. Review again if the server rejects a stale revision.
- Record the original key and content before sending. Recover the same request after a lost response, including after reload. An unresolved request blocks new mutations.
- Bind request records to principal, installation, and store generation. Do not blindly resubmit after an account or restore-generation change. An explicit operator/support reconciliation screen for this case remains future work.
- Do not store passwords or session tokens in localStorage or sessionStorage. Authentication uses an HttpOnly cookie. Storage contains unresolved business requests.
- Do not silently reset damaged or unavailable sessionStorage. Restrict the UI to read access. Durable client recovery across browser closure, new tabs, and device replacement remains separate work.
- Role-based disabling in the UI is a convenience. The platform independently checks authority, terminal identity, qualification, revision, and idempotency.

## Development and verification

```text
npm ci --ignore-scripts
npm run dev
npm run build
npm test
```

First run the platform's `rx-platform-local` on 127.0.0.1:8080 with its public origin set to `http://127.0.0.1:5173`. The UI address is `http://127.0.0.1:5173`. Vite is a development server and does not replace the product image's web ingress.

`tests/browser_smoke.py` signs in to the real API and registers the unqualified `examples/development/cell-demo.json` fixture. It disconnects an HTTP response after a real server commit to check recovery of the original request, and checks revision changes during review, connection loss, mobile overflow, and sign-out. It does not substitute successful fixtures for database state or committed responses.

Python test dependencies are pinned in `tests/requirements-browser.txt`. Tests use headless Chromium. Install them in an isolated virtual environment following [Playwright Python](https://playwright.dev/python/docs/library).

`tests/run_browser.py --platform-executable PATH --evidence-dir PATH` creates a fresh temporary installation and manages the two development services through the bundled `tests/with_servers.py` helper. The optional `--server-runner PATH` overrides that helper. No agent tooling or personal configuration is required. Alternatively, prepare a fresh development installation and both servers manually, then run `RX_BROWSER_EVIDENCE=OUTPUT python tests/browser_smoke.py`. The initial test password is `browser-fixture-password`, used only in the temporary installation.

The package and device review launchers require `--evidence-dir PATH` and use the same bundled server runner by default. Pass `--review-ui` to `tests/run_device_browser.py` to exercise device software review. These launchers use generated test identities and signed fixtures.

Server command arguments are parsed without a shell. The runner waits for local port readiness, preserves the check exit status, and cleans up only the process groups it starts. If a port is already occupied, the launcher fails without terminating the existing process. Run `python3 tests/test_with_servers.py` to verify server lifecycle handling. pytest/Playwright results cover the UI draft and do not replace product acceptance or physical testing.

The intervention case list and notification acknowledgment connect to the real development API. An acknowledgment does not clear the case or production blocks. A lost response is recovered with the existing sessionStorage pending key. Case creation currently uses the API/engineering configuration path. The procedure catalog, creation UI, and physical procedure, recovery, and restart flows remain future work. Follow the [specification](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/INTERVENTION_CASES.md).

## Operating conditions and observation evidence

The **Operating conditions** view shows source values, acquisition time and age, quality, generations, registered Host leases, and start, sustain, and operation conditions. Refresh after the display validity period expires. PASS does not grant operating permission. Follow the [diagnostic model and verification scope](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/OPERATOR_CONDITIONS.md).

`tests/run_browser.py` generates PASS, FAIL, and expired read-model fixtures through P application tests. These three browser scenarios verify presentation at the response boundary; they do not inject native observations into the running service. Use a fresh evidence directory for each run.

## Workflow design

Engineers and Verifiers can read workflow drafts. Engineers can create drafts, edit nodes and control flow, save, copy, and compare versions. Incomplete content is saved with errors and does not change the installed cell. Advanced JSON input remains in a buffer until applied. Follow the [draft storage and execution boundary](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/PROCESS_DRAFTS.md).

The [package intake and review UI](PACKAGE_REVIEW_UI.md) connects to the real API and global pending-request recovery. It distinguishes historical revisions from the current approval target. Software approval does not start operation.

# Serving the app from product images

The phase73 [two-image, direct-terminal HTTPS delivery](DELIVERY.md) path is connected separately from the development server. That document records delivery, file verification, mTLS browser testing, and the StartRun/executor-assignment work remaining at that phase. The later [run-start UI](RUN_START_UI.md) document describes the connected start flow.

The Host recovery binding in the configuration view connects proposal, review, and approval by a ReleaseManager on a registered terminal, advancement of the existing Fence request, and receipt/outcome inspection for known operations. It preserves the server-returned digest and all cell revisions, and records the original key/body in sessionStorage before proposal or approval. It recovers the same request after response loss or refresh. Existing records can also be discovered from a fresh view without a previously saved UUID. RECOVERY_ONLY does not restore operating registration, grants, qualification, Arm, or Run resumption. A missing application receipt in Host Inspect is not displayed as a current connection failure. The phase-specific evidence in rx_docs defines the actual exercised scope.
