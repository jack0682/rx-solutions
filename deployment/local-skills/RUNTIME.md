# Calling an installed RX process

This unreleased bridge calls an already configured P/Host/Executor installation.
It uses the existing P Run and operation ledger. It does not submit a LOCAL_SIM
Python skill, install the runtime, or register arbitrary device skills.

The current input mode is `BOUND_CONFIGURATION`: the approved process, recipe,
site and envelope are fixed by the installed configuration. Only the material
count is supplied at invocation. Catalog visibility does not grant start permission.
P independently evaluates start admission, current roles, qualification and resources.

## Connection

Use a registered terminal certificate and an Operator principal scoped to the cell.
Create an owner-only JSON file containing absolute file paths:

```json
{
  "schema": "rx.runtime-skill-connection.v1",
  "origin": "https://localhost:8443",
  "ca": "/absolute/path/ca.pem",
  "certificate": "/absolute/path/terminal.pem",
  "private_key": "/absolute/path/terminal-key.pem",
  "principal": "operator",
  "password_file": "/absolute/path/operator-password"
}
```

The connection file, private key and password file require owner-only permissions.
The origin has no trailing slash or path. TLS hostname verification remains enabled;
redirects and proxy forwarding are refused. Passwords and session cookies are not
written to the request journal.

From this source checkout (these commands are not in public rc.2):

```sh
python3 deployment/local-skills/rx runtime --connection /absolute/path/runtime.json skills
python3 deployment/local-skills/rx runtime --connection /absolute/path/runtime.json \
  run PROCESS_NAME --cell CELL_ID --count 1 --request-id CANONICAL_UUID
python3 deployment/local-skills/rx runtime --connection /absolute/path/runtime.json \
  recover CANONICAL_UUID
python3 deployment/local-skills/rx runtime --connection /absolute/path/runtime.json \
  result CANONICAL_UUID
```

`inspect P_RUN_UUID` reads a known P Run directly. `result` uses the existing local
association to that Run without resending mutations. `recover` retains the original
prepare/start request identities and bodies; it does not create a new Run for an
uncertain invocation. Keep the printed runtime request UUID. To select a journal
location, pass `--state-dir /absolute/path/journal` before the action. The default
is `~/.local/share/rx-runtime-client`, separate from LOCAL_SIM storage.

P boot or clock changes block replay of a pending request. Installation/store changes
block reuse of the association. Inspect and reconcile the original Run using existing
P controls; creating a fresh request is not a recovery procedure. `COMPLETED` is P's
Run state, while operation outcomes, resource dispositions and part dispositions
remain explicit in the result. A catalog or result marked truncated is incomplete.

## Validation and remaining work

`tools/test_runtime_skill_client.py` exercises client identity/replay guards.
The cross-repository `rx-platform/tools/test_runtime_skills.py` uses the installed
client extracted from `docker/RuntimeSkillValidation.Dockerfile`, actual mTLS P,
Executor and Host FILE_SIMULATION. It kills the consumer after StartRun returns
but before the reply is journaled, recovers the original Run in a fresh consumer,
and compares P operations to independent native effects. A new request is checked
separately to ensure normal execution remains possible.

These images are validation artifacts. Dynamic business inputs/outputs, device
skill authoring and registration, runtime skill composition/KPIs, P/Host restart
reconciliation and a supported one-command runtime distribution remain incomplete.
The public LOCAL_SIM installer and its process/KPI tests do not establish those
runtime capabilities. No physical equipment qualification is claimed.

## Compose installed steps on the server

Use an Engineer connection for authoring. `steps` reads the existing P binding
catalog; `compose` stores an ordinary process draft and its selected bindings in
P, then exports the matched compile input. The process contains one sequence and
one operation leaf per selected step. Execution contracts remain in the server's
Step definitions, rather than additional author-written BT nodes.

```sh
python3 deployment/local-skills/rx runtime --connection /absolute/path/engineer.json \
  steps --cell CELL_ID
python3 deployment/local-skills/rx runtime --connection /absolute/path/engineer.json \
  compose material-transfer --cell CELL_ID \
  --step REGISTERED_PICK_STEP --step REGISTERED_PLACE_STEP --request-id CANONICAL_UUID
python3 deployment/local-skills/rx runtime --connection /absolute/path/engineer.json \
  compose-recover CANONICAL_UUID
```

The order of `--step` arguments is the execution order. Repeating a step creates
another distinct operation leaf. Select 1–64 steps already present in the catalog.
The original request, catalog and generated draft ID remain pinned through reply
loss; recovering cannot silently change the order or retarget another installation.
P rechecks the current source/catalog when exporting, even if local receipts exist.

The result is `DRAFT_READY_FOR_COMPILER`, with `execution_authorized: false` and
`compile_input`. Supply that input to the existing process-package assembly,
verification/review and activation path. Saving a draft does not replace the
currently installed process. This command does not yet register new native skill
implementations, set dynamic arguments, activate a process or provide branch/parallel
authoring. Those remaining integration steps are not implied by successful export.
