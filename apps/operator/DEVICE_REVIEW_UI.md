# Device software review UI

Phase66. Verification requests, reports, and independent review decisions are connected below the existing device declaration view. The API contract follows [device review](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/DEVICE_REVIEW.md). This screen does not apply physical cell configuration, create operating qualification, or move robots.

## User flow

1. Select an imported device package and inspect its declared operations, conditions, and sources.
2. Create a review request when the current configuration context and device verification signer settings are available.
3. Download the request, then generate and sign a report with the external verification tool.
4. Register the report using its relative path and identifier. The UI displays three software checks and their issues.
5. A Verifier other than the package submitter inspects the current report, source, signatures, and scope.
6. Enter a review note, acknowledge the review, and record approval or rejection in a dialog showing the exact version and identifiers.

Reports and review evidence can be downloaded. Historical report versions are read only. The scope is fixed to DEVICE_PACKAGE_SOFTWARE, with physical verification and operating qualification explicitly separate. Approval is disabled for evidence with failed or unperformed checks, self-approval by the submitter, old reports, or evidence that does not match the current context. Under backend policy, rejection requires a current Verifier and the latest target; it does not manufacture the source proof required for positive approval.

## Response correlation and acknowledgment invalidation

Lists and details are checked against the selected cell, intake, and review, and package manifest, signature, and catalog references. If the detailed Job, Report, Decision, state, scope, and current-version indicators disagree, the displayed evidence is not accepted as a current approval basis.

The normalized evidence stamp includes the report, signatures, decision, current configuration, registration, device authority, and account context. Polls with unchanged content preserve the acknowledgment. New reports or context changes, query errors or expiry, an inactive screen, and unavailable actions clear the acknowledgment and any open confirmation dialog. Queries discard late responses through abort/generation handling. The final button also checks the same stamp, and the server separately revalidates current sources and authority.

The confirmation dialog fixes the selected report revision, review digest, expected decision revision, choice, and note. It does not silently send input changed after the dialog opened. The scope is package software review and is not described as physical qualification.

## Response loss

The three device mutation paths use the existing sessionStorage pending system. Before sending, store the same request key, command, account, installation, and store generation. Preserve them if the outcome is unknown or response correlation fails. After refresh, **Check original request** recovers the original outcome. Do not automatically resend under another account, installation, or store generation.

Precisely compare the response's review, cell, report revision, digest, choice, note, and incremented revision. Treat 64-bit revisions as strings/BigInt. Distinguish recovery of a historical approval receipt from whether approval applies in the current view. Selecting or querying evidence never sends a new decision.

## List size

P returns only 50 Summary records per page. They contain requester/creation time, the latest report revision/digest, approval-candidate state, decision summary, and freshness. Reports, signatures, and long notes come from a separate detail endpoint. This replaces the initial layout that copied full reports into every list row.

Summaries loaded through **Load more** are retained during ordinary refreshes, with duplicate IDs merged. Validate the scope of query results. Backend full-Job scans and long-term archive/maximum-load validation remain future work. This change bounds the response payload structure; it is not full repository performance acceptance.

## Verification

Use the real S JTC CLI, an external test signer, and the P API to check request creation, report registration, disabled author approval, independent approval, new/historical versions, invalidation of open dialogs, and same-request recovery after response loss through the UI. Authority-context changes are separate UI counterexamples using injected responses. Actual backend rejection after authority-file changes remains covered by phase65 API verification.

The UI and actual outcomes are linked in the [phase66 record](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase66_checks.json). Configuration changes, qualification after device software approval, and physical device verification are subsequent work.
