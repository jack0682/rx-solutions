# P-assigned resident execution

`rx-solutionsd` now has an explicit `catalog` / `platform-run` path alongside its
existing startup commands. Both use the compiled release catalog and verification
code; site configuration cannot supply executable paths, effects or requirements.

For catalog enrollment, start with:

```json
{
  "schema": "rx.resident-execution-runtime.v1",
  "state_subdirectory": "resident-managed"
}
```

`rx-solutionsd catalog CONFIG` verifies the actual `/opt/rx` content, records the
installation-wide release floor and opens the chosen registry under
`/var/lib/rx-solutions`. Its output is authoring data for P enrollment, not a live
permission or a deserializable verified catalog. Review and enroll the exact
canonical registry, releases and program digests/effects on P, using a dedicated
Supervisor-only certificate/principal.

After an owner creates a P assignment, add `assignment` and `connection` (the
absolute path to `rx.resident-execution-connection.v1` TLS/bootstrap configuration)
and run `rx-solutionsd platform-run CONFIG`. The command fetches the assignment,
verifies the actual program/plan and prints AWAITING_OWNER_APPROVAL. The owner
approves the exact current preparation through P's authenticated API. The
connection fields are endpoint, server_name, ca, certificate, private_key,
principal, installation, store_generation, shared_clock_id and release_digest
(the expected P context, separate from verified S program-release identities).

S then consumes one live grant through RegisteredSupervisor and the existing OS
backend. It persists P-supplied run/instance IDs before effects and uses the full
catalog requirements. Pending/old JSON grants, report scopes and source-transfer
acknowledgements cannot instantiate a live capability. There is no automatic
restart or local author fallback. Managed declaration snapshots remain attributed
to P and are explicitly not validated as current by a local read.

Use P's stop endpoint or locally send SIGINT/SIGTERM to the owned command. A
separate durable delivery worker handles P traffic; it does not own/call the
process manager. Local stop can finish during a P outage while its original
report remains pending and P keeps the claim. The same original assignment is
not a replayable start after daemon exit. Failures and Unknown outcomes retain
records for investigation instead of adopting a PID or reopening source writes.

This path uses a single shared Linux boot clock and canonical registry namespace.
Nonempty legacy registries require the original frozen cut and completed target
acknowledgement; another empty registry cannot stand in for it. New operational
stores use reader format 9 and refuse older binaries. Whole-file replacement,
active-unknown recovery and general live replacement remain separate requirements.

See [P's workflow](https://github.com/jack0682/rx-platform/blob/develop/docs/resident-execution.md)
and the [execution contract](https://github.com/jack0682/rx_docs/blob/develop/docs/contracts/resident-execution/v1/README.md).
Process execution does not establish business-work permission, physical completion
or functional-safety qualification. New paired installer release and full default
bundle qualification remain unfinished.

## Investigating an original execution

`rx-solutionsd platform-investigate CONFIG` uses the same runtime/connection
configuration and original assignment ID. It opens a new authenticated Supervisor
session, fetches the original P assignment, verifies the actual installed catalog,
and locks the original registry and its derived execution journal. An active owner
holding either store prevents this cold inspection. The command does not construct
a process manager, start or signal a process, or rewrite the source/outbox history.

The output distinguishes committed matching direct-child exit, recorded non-start,
unconsumed pre-start records with an expired original start window, fresh Linux
scoped birth-identity investigation, and conflicting/missing evidence. The original
outcome, source reference, journal revision and digest are retained. A previously
Running source record remains Running history even when a fresh investigation says
the original process is no longer running. Missing birth evidence remains
unverifiable; PID absence cannot backfill it. A changed clock/registry/content or
inconsistent consumption identity is refused rather than silently repaired.

This is source investigation only. It does not acknowledge pending delivery, release
P claims, authorize resumption, confirm descendants/resource handover, or establish
physical completion. The new session changes protocol identity; it does not adopt
the old process. P owner reconciliation and a fresh explicitly approved assignment
remain required follow-up work. Do not feed the serialized diagnostic back as a
live capability. The actual source-backed construction must be used by that future
reconciliation path.
