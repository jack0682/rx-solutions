# Operator run-start integration

The existing run preparation and run list now connect exact run selection, an explicit production material attempt count, pre-start queries and a confirmation dialog, StartRun acceptance, and start-attempt inspection.
The existing layout, CSS, same-origin API, HttpOnly cookies, and direct-terminal mTLS boundary remain in place.
The browser receives no Host/Executor credentials or native control path.

## Actual APIs and UI semantics

- `GET /api/v1/run/start-context?cell=...&run=...&purpose=PRODUCTION&budget_limit=2`:
  inspect the selected run's current revision, configuration, budget, and blocking reasons at read time.
  The count is a string Counter and must be a positive integer. An existing budget is fixed.
  Passing read checks does not establish qualification or operating permission.
- `POST /api/v1/runs/start`: uses the existing `{request_key, command: StartRun}` contract unchanged.
  The confirmation dialog fixes cell/run revisions, envelope, and count. A changed background read
  requires renewed review; the reviewed request is not silently updated to a new revision.
- `GET /api/v1/run/start-attempt?cell=...&run=...&id=...`:
  inspect the stored PENDING/ARMING/STARTED/REJECTED state and the current Run.
  Do not infer STARTED from the number of Host ACKs. ELAPSED and CLOCK_CHANGED are separate time states;
  they do not turn ARMING into REJECTED or trigger an automatic restart.

The response to new-run preparation selects the exact Run that was created. Read the current state separately to verify start readiness and revision. Existing runs can also be explicitly selected from the list.
The count input starts empty. The most recent run is neither automatically selected nor automatically started.
A previously verified start attempt returns to that exact run under the same installation and account.

## Response correlation and recovery

The existing pending store includes `/api/v1/runs/start` and the reviewed cell context.
Before sending, sessionStorage records the original UUID key, body, principal, installation, store generation,
and cell/epoch/configuration context. An unresolved request blocks other mutations.
After response loss or reload, **Check original request** resends the stored key and body unchanged.
The request is not recovered under a different account, installation, or restore generation.

The existing StartAttempt POST receipt does not contain envelope, purpose, or budget.
First validate cell, run, actor, reviewed revision, epoch, and scope; then compare the Run envelope, recipe,
purpose, and budget in the GET for the same attempt ID before clearing pending state.
A GET failure or correlation mismatch preserves the original request as unresolved. Before clearing pending state,
store the verified attempt ID and original request in a separate sessionStorage inspection marker so that GET
can retrieve the state after reload. This marker is neither operating authority nor a separate business ledger.

Normal state polling uses GET only. A query error or ten seconds since the last successful read marks the
record as historical and blocks new start requests. After a definitive initial rejection, fetch a fresh context
and review again. A later rejection of an already unresolved request is not evidence that the initial request was unprocessed.

The current run state is the selected Run state from the latest overview. Even if the start-attempt GET is less
than ten seconds old, a mismatch in installation, store generation, runtime boot, clock, or the current Run revision
or state immediately downgrades it to the last retrieved record. An older STARTED/EXECUTING read therefore cannot
overwrite run state updated by an operation hold or restart. A subsequent GET failure does not reverse that judgment.
Each Host ACK must match its `host_boots` value, and STARTED requires confirmation from the exact complete Host set.
ARMING is not promoted to confirmed start merely because some or all valid ACKs are present.

## Implementation and verification boundary

- `run-start-schema.ts`: DTO validation, request/response correlation, fixed confirmation content, and start-attempt inspection markers.
- `run-start.tsx`: selection, count, review, and status screens using the existing shared component styles.
- `run-start-schema.test.ts`: invalid Counters, states, installations, Runs, attempts, configurations, and budgets;
  recovery of the original key/body after response loss; renewed review after version changes; Host ACK and deadline counterexamples.

A human-facing per-material production aggregation API or screen is outside this scope.
The existing overview displays Run state, budget usage, operation outcomes, and resource handover separately.
Budget consumption and partial operation lists must not be interpreted as completed material counts or good-part counts.
sessionStorage supports recovery after reload in the same tab. Durable client recovery across browser closure,
new tabs, or device replacement is a separate scope.
Physical-equipment registration status and the current registered terminal come from actual read values.
Connecting the UI does not establish physical operating qualification or completion of physical tests.

After all source changes are frozen, the integration owner runs:
`npm run typecheck`, `npm test`, `npm run format:check`, `npm run build`, and registered-terminal browser acceptance.
