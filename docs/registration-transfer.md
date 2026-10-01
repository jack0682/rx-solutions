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
This CLI still does not reconcile its acceptance receipt or provide rollback;
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
