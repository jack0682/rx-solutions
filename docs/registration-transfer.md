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
sealed in SQLite and its reader version becomes 7. Older schema-6 binaries cannot
open it. The exact request is recoverable after reply loss or process restart.
History pages retain original events only through the stored cut (limit 1..128).
The preparation output explicitly leaves Platform acceptance unestablished and
process ownership untransferred. There is no P receiver or rollback command yet;
this development phase is exercised on copies, not an automatic live migration.

New code also refuses local declaration changes, new execution assignments,
resume permits and new work commits. Existing observation writes and historical
queries remain available. The registry view exposes declaration_authority on its
own axis; a recovery gate or stored Accepted declaration cannot override the
source fence.

The SQL fence does not prevent privileged schema replacement, an older database
being restored over the source, or whole-installation rollback. A JSON export is
not remote proof of the live fence. The next P intake path must verify its
registered source and retain original identities/history before enabling P writes.

`tools/test_registration_transfer.py` takes a completed R4 Linux daemon scene,
copies its data, runs the exact old daemon as a positive control, freezes a second
copy with the new CLI, and requires the old reader's schema refusal. It checks
same-request recovery, unchanged original source bytes and retained history.
It attaches no devices and grants no process or work authority.
