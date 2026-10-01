# Registration source preparation

The source-side migration boundary freezes legacy declarations while preserving
original IDs/revisions and a bounded history cut. It is preparation for P intake;
P acceptance and P-directed execution are not implemented by this tool.

Build `rx-registration-transfer` from the `rx-supervisor` package. The existing
installer does not invoke or install this development tool. The explicit commands
are:

```text
rx-registration-transfer freeze REGISTRY_DB REQUEST_ID TARGET_INSTALLATION
rx-registration-transfer inspect REGISTRY_DB
rx-registration-transfer history REGISTRY_DB FREEZE_ID AFTER LIMIT
```

Use an existing reviewed source database; a missing/empty/symlink database is
refused. The repository lock must be available. `freeze` is permanent through
this API: source registration/selection namespaces and its freeze marker are
sealed in SQLite and its reader version becomes at least 7 (an existing version 8
is preserved). Older schema-6 binaries cannot
open it. The exact request is recoverable after reply loss or process restart.
History pages retain original events only through the stored cut (limit 1..128).
The preparation output explicitly leaves Platform acceptance unestablished and
process ownership untransferred. The paired Platform change adds an explicit configured-source receiver.
The explicit reconcile command below records authenticated acceptance; there is no rollback;
this development phase is exercised on copies, not an automatic live migration.

New code also refuses local declaration changes, new execution assignments,
resume permits and new work commits. Existing observation writes and historical
queries remain available. The registry view exposes declaration_authority on its
own axis; a recovery gate or stored Accepted declaration cannot override the
source fence.

The SQL fence does not prevent privileged schema replacement, an older database
being restored over the source, or whole-installation rollback. A JSON export is
not remote proof of the live fence. The paired P intake path verifies its
configured source and retains original identities/history before enabling P writes.
See [the revised transfer contract](https://github.com/jack0682/rx_docs/blob/develop/docs/contracts/registration-transfer/v1/README.md).

FreezeRequest and FreezeRecord now come from the shared rx-domain SDK; this module
re-exports them without changing their JSON shape. The SDK reads target format 8
while ordinary stores still initialize at 6. The test-harness-only
`rx-registration-source-fixture` edits and freezes a private source copy for the
Platform cross-repository HTTP test; it is not an installed product command.

`tools/test_registration_transfer.py` takes a completed R4 Linux daemon scene,
copies its data, runs the exact old daemon as a positive control, freezes a second
copy with the new CLI, and requires the old reader's schema refusal. It checks
same-request recovery, unchanged original source bytes and retained history.
It attaches no devices and grants no process or work authority.

## Reconcile P acceptance

After target intake, keep the source sealed and use a current scoped mTLS peer:

```text
rx-registration-transfer reconcile REGISTRY_DB CONNECTION_FILE
```

CONNECTION_FILE uses the existing `rx.resident-report-connection.v1` reporting
configuration (TLS identity and pinned installation/store/release context). The
command opens one fresh reporting-process session and prints AWAITING_OWNER_SCOPE
with its peer and frozen source cut. In another authenticated owner/admin session,
issue an ordinary reporting scope for an imported canonical component ID and this
reporter session. Enter one JSON line on the waiting command's stdin:

```json
{"component":"<original imported component UUID>","scope":"<owner-issued scope UUID>"}
```

The command inspects that scope, reads its historical target acceptance over mTLS,
checks the exact original freeze/cut and source membership, and records the first
acceptance atomically. It does not accept a receipt JSON or an `accepted` boolean
from stdin. A new invocation has a new peer boot and needs a current owner-issued
scope; it never reuses an old process boot. Transport failure leaves the source
frozen; reconnect and reconcile with a current scope rather than creating a new
registration or execution attempt.

`inspect` then reports RECORDED_FROM_AUTHENTICATED_PLATFORM and the original local
evidence. Same-decision retries retain that evidence without another history event,
including after source-side restart; the command also displays the current query
observation separately. Different target decisions are refused for investigation.
The frozen original history cursor does not grow when this later event is stored.

This is historical custody acknowledgement only. Source declarations and local
assignments stay refused. Program content acceptance, P execution assignment and
process ownership are separate unfinished work. The CLI remains a development
tool, not an automatic installer migration or daemon launch operation. Reporting
binding revision 3 requires paired P/S upgrade; old binding hashes are refused.
