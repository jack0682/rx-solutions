# Intake, review, and software approval UI

2026-09-12. **Package review** was added to the existing operator app. Engineers and Verifiers inspect intake records, review requests, source, compilation results, and verification evidence for authorized cells. This screen does not activate operation.

## Screen flow

1. **Intake records**: an Engineer imports a package using its relative path in the server intake directory, content/signature identifiers, and title. The current implementation uses prepared server files; it does not imitate browser file uploads.
2. **New review request**: bind package operation names to exact device operations in the current cell. Creation is disabled if verification signers are not configured. Download the request JSON for the S verification tool.
3. **Verification evidence**: register the evidence directory and report digest produced by an external tool and signer. Display the results, issues, signer, and review identifier verified by P.
4. **Review**: show the workflow source flow in a bounded number of rows, with expandable views and downloads for the full source/conditions, compiled output, current cell snapshot, selections, and verification evidence. Use the full JSON or download for long source documents. The summary does not claim to verify every workflow semantic.
5. **Decision**: only a current Verifier may approve or reject. Approval by the submitting account is disabled. After reviewing source, results, cell bindings, and version, enter a note and record the decision in a dialog showing the target title, revision, and full review digest.

Intake and review lists fetch the next page using an ID cursor. Each intake lists only its own review requests. Historical verification revisions are read only and distinct from the current verification revision. Historical views do not permit approval or registration of a new report. The backend also always checks the latest review revision and CAS.

## Freshness and response loss

- Selected evidence is refreshed approximately every three seconds while the screen is active. New actions are blocked if ten seconds have elapsed since the last query started or if a query fails. This does not guarantee real-time device state.
- Bind the acknowledgment stamp to a snapshot of displayed source, results, configuration, verification, and decision metadata. A changed snapshot clears the acknowledgment and any open approval dialog.
- At final confirmation, compare the current snapshot, revision, digest, account role, and latest-version status again. The server independently performs the final authority, policy, and content checks.
- Intake, review creation, report registration, and decisions reuse the existing global pending-request store. Record the same key, body, account, installation, and store generation in sessionStorage before transmission.
- Distinguish processing from an unverified outcome. When the outcome is unknown, recover with the original request key and content instead of creating a new request. Preserve this across refresh. Do not recover under another account or installation.
- Responses must match the original request's target cell, IDs, review digest, revision, note, choice, and other correlation fields. Do not clear pending state as successful after a mismatched response. Compare Counters with BigInt.
- When a historical decision is retrieved, separately indicate whether it applies to the current review. **Software approval recorded** is neither operating qualification nor a device-readiness indicator.

Screen inputs and selections are held in memory for the current account, installation, and cell. A full-page refresh does not restore ordinary unsent forms, but does restore transmitted pending requests. Separate the editing context when the signed-in account or installation changes.

## API additions

P adds `GET /api/v1/process-reviews?cell=...&intake=...&after=...` and an optional `revision` for single-review inspection. The current intake context also reports whether review authority is configured. Historical queries do not change the latest-revision basis used by the approval endpoint.

The current list uses the existing document-repository scan. Output pages contain 50 entries. Database indexing and server paging optimization for large, long-lived archives remain future work.

## Verification and delivery boundary

Unit tests cover a separate verifier, freshness and historical guards, acknowledgment stamps invalidated by evidence changes, request/response matching, and large integer revisions. Browser tests use the real P API, a separate S compiler, and evidence produced by a test-only signer. They check intake, operation binding, request export, report registration, rejection of self-approval, acknowledgment/dialog invalidation on a new revision, historical reads, restart and same-key recovery after response loss, and mobile layout.

The local development service for browser testing may use an explicit private `package-service.json`. This development fixture path is separate from production daemon mTLS configuration and does not automatically add production signers or device authority. Keys and accounts in the fixture are test only.

Automation of S CLI execution, signing, and file transfer; expanded raw-source preview after review failure; comparison of decision history; activation/change-plan screens; and physical site acceptance remain future work. The first physical cell is NOT_COMMISSIONED.

# Phase64 device declaration inspection

Phase66 adds the [device software review UI](DEVICE_REVIEW_UI.md). That document distinguishes the report and independent-approval actions added after the phase64 declaration view below, together with their limits.

The intake list recognizes DEVICE_REFERENCE and shows device operation declarations in a separate panel for inspection and download. Compare the selected receipt's cell, intake, object, and reference against the query result. Device packages do not display a workflow review-request button. Declaration inspection is not device verification approval and does not apply the current configuration or activate operation. Follow the [device operation declaration specification](https://github.com/jack0682/rx-platform/blob/codex/initial-draft/crates/rx-application/DEVICE_CATALOG.md) for API details and storage/freshness boundaries.
