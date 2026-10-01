# Resident execution reporting

`rx-solutionsd` can automatically publish existing registry snapshots through an
optional reporting worker. Its dedicated thread owns a separate `reporting.db`
journal and mTLS client. The local 50ms process/stop loop never waits for a report
RPC. After local children finish, the daemon allows a bounded delivery tail;
an incomplete tail is reported explicitly and the source registry is retained.

## Configure a running installation

Add the following optional field to the existing startup configuration. Existing
configurations without it retain their behavior.

```json
"reporting": {
  "connection": "/etc/rx-solutions/report-connection.json",
  "scopes": "/etc/rx-solutions/report-scopes.json"
}
```

The connection file uses schema `rx.resident-report-connection.v1` and fields:
endpoint (an exact `https://host:port` origin), server_name, ca, certificate,
private_key (the last three are PEM file paths), principal, installation,
store_generation, shared_clock_id and release_digest. Use actual installation
values and a certificate registered on Platform for a dedicated active
Observer-only principal. Never pass the component author's Engineer credentials.
All configuration/PEM files are bounded regular files; symlink inputs are refused.

1. Run the existing verified release/startup procedure. The reporter prints
   `rx.resident-reporting-status.v1`; its `peer.id` is the session the owner needs.
   Missing scopes or a failed connection do not authorize or inhibit local work.
2. The component owner creates a P declaration with the same catalog reference
   and issues a reporting scope through POST `/api/v1/components/reporting`.
   The original S registration/revision is visible in the resident registration
   output. This relation does not migrate the registration writer.
3. Write the scopes file atomically with schema `rx.resident-report-scopes.v1` and
   a `selections` object. Each plan selection maps to component (P ID), registration
   (original S ID), and scope (the issued scope ID). The worker rereads this file.
4. Read delivery status, pending request counts and errors. Receipt ownership is
   always NOT_ESTABLISHED_BY_REPORT and work use remains NOT_EVALUATED.

The handoff retains the latest source snapshot per selection and may coalesce
intermediate snapshots before request creation. The source registry remains the
record of local observations. The worker commits an exact report, UUID request
key and peer before transmission. Once created, a pending request is never
replaced by a newer snapshot, and the receipt must match before advancing.
At most 128 pending instances may be queued; refusal leaves existing requests
intact and surfaces attention. Historical rows remain retained separately. Unsent snapshots from previous runs
are rediscovered from the journal even when absent from the current plan. The
worker examines at most 128 retained entries plus current selections per cycle;
`held_snapshots` exposes additional unsent history.

## Reconnect and restart

A transport reconnect within the same reporting process reuses its peer boot.
A new reporting process generates a new boot; P restart likewise invalidates the
old session. The owner calls POST `/api/v1/components/reporting/continue` with
scope, expected_revision and the new reporter_session, then updates the scopes
file with the returned scope. An optional `instances` map uses the execution
instance ID to select a scope for retained history from an older run; it overrides
the selection mapping. P atomically revokes the predecessor and preserves
its source identities and historical receipts. Revoked scopes cannot branch.

The new worker reads the scoped P head. An identical receipt can recover a lost
acknowledgement. Otherwise an old pending request is archived as
PRIOR_DELIVERY_UNRESOLVED, never marked successful by a new report. A new snapshot
uses the lineage's next sequence. If a known accepted head regressed, reporting
requires explicit store reconciliation. Neither continuation nor a stored PID
adopts a process or changes the outcome of a physical operation.

Library callers may use `Client`, `Outbox` and `Resident` directly. Only actual
transport-verified scopes should feed the delivery journal. The journal is
local trusted application state, not an authority for process/work admission.

## Verification boundary

The Platform runner `tools/test_resident_reporting.py` builds the separate
`rx-resident-report-fixture` under the test-harness feature. It exercises the
same Resident worker used by the daemon with RegisteredSupervisor, its SQLite
registry, a real software status child and actual mTLS. Scenes include lost
acknowledgement, delayed reporting RPCs while the child stops, durable undelivered
state, and owner-approved reporting restart. The installed daemon command and
platform-directed execution require their own integration evidence; this scene
has no physical equipment, content intake or registration-writer migration.
