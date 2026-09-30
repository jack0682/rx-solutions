# Scoped resident execution reports

`rx_supervisor::reporting` publishes an existing registry execution to Platform's
optional `rx.resident.reporting.v1` service over mTLS. This library is not wired
into the shipped `rx-solutionsd` loop yet.

1. An installation administrator configures a certificate mapped to a dedicated,
   active Observer-only principal on Platform. Use the exact installation, store
   generation, clock, release digest and trusted TLS service origin in `Connection`.
2. `Client::connect` creates a reporting-process incarnation. Give its `peer().id`
   to the component owner. No Engineer password is passed to Supervisor.
3. The owner creates a P declaration with the matching catalog reference and
   issues a reporting scope for the original S registration and revision through
   the component reporting BFF. This does not migrate S's registration writer.
4. Call `inspect_scope` with the returned scope, P component and S registration.
   `from_execution` preserves the registry's original run, selection and instance.
5. Retain a UUID request key and the report before calling `publish`. Start at
   sequence 1 and increment only after accepting a matching receipt. On response
   loss retry the same key and bytes explicitly; the library never retries a
   physical action or automatically advances the sequence.

Receipts are attributed registry snapshots. Their acceptance time is not the
observation time; PID and Running are not proof of current OS ownership. They
cannot authorize a launch, stop, work permit or device command. The owner can
inspect history and revoke a scope. Declaration changes mark the old revision
noncurrent while allowing diagnostic history under an active scope.

A new reporter process or Platform incarnation invalidates the old session.
New scopes and existing-instance reconciliation are required; automatic recovery
and daemon configuration are unfinished. The client cannot silently rebind an
old instance to a new peer.

The `test-harness` feature adds `rx-resident-report-fixture` solely for the Platform
cross-repository runner `tools/test_resident_reporting.py`. It launches a locally
approved Python status child through RegisteredSupervisor, reports Running, loses
an acknowledgement, recovers the same request, stops the owned child normally,
and reports Exited/0. This verifies the transport and retained identities, not
Platform-directed execution, registered-writer migration or physical equipment.
