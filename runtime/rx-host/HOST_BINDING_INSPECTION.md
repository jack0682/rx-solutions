# Manual inspection of Host startup configuration changes

phase72 [durable preparation, lookup and cancellation](HOST_MAINTENANCE_PREPARATION.md) follow this comparison. Neither comparison nor preparation replaces an actual installation or performs native operation.

phase71. The command reads and compares files without Host initialization, driver startup, device connection, native submit or installation file replacement.

```text
rx-hostd inspect-binding-change PLAN CURRENT_CONFIG PROPOSED_CONFIG
```

PLAN is P's `rx.host-binding-plan.v1`. Both configurations retain the existing `rx.host-startup.v1`, pinned-file and TLS checks. Inspection produces `rx.host-binding-inspection.v1` JSON. Format, trust or package acquisition failures terminate with an error; change-scope mismatches between valid inputs return `software_matches=false` and issues.

It compares the following.

- Installation and Host identity, the Host in P's plan, and the full affected-cell cohort.
- The target cell's exact Intent, condition set, definition, envelope, scope and environment.
- Bindings of other affected cells must remain identical to current.
- Qualification ID/revision, purpose and platform peer remain unchanged. File comparison does not issue new qualification.
- Startup configuration outside backend/bindings remains identical. Changes to data/runtime locations, release, TLS, publisher or network require a separate deployment plan.
- The proposed backend is checked through the same product's release-owned factory and actual signed package decoder.
- The device package manifest/signature and normalized catalog hash/size/schema match P's requirements.

The current native factory requires one device package and one exact cell binding per Host. Multiple packages or additional cell changes are not marked as supported. This is a current backend limit, distinct from the maximum scope of the platform's common artifact.

The result records the plan digest, current/proposed installation identity and proposed bindings pin. `runtime_provider_available` indicates factory availability in that release, not device readiness or qualification. It is false for JTC even when the software matches, because no production control provider exists.

`installation_changed=false`, `activation_authorized=false` and `native_processes_started=0` are retained. This JSON is neither an independently signed verification report nor a durable Host application receipt. Subsequent application must recheck files, policy, the current installation and journals; it must not assume files remain identical immediately after inspection.

Comparison integration involving an actual JTC package, P change proposal and product CLI, and rejection of qualification/storage changes have been tested. Separate Host fixture tests reject scope expansion through duplicate operations, missing cohorts and different installations, and confirm that no data directory is created. Physical equipment, production TLS management operations, Host reload and restoration remain unverified.
