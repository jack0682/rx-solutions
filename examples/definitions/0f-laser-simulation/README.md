# 0F cell model — M1 developer quickstart

This is the **SIMULATION authoring milestone**, not the final Linux RC or a
device-control installation. The platform stores and resolves all definitions;
the existing `rx` CLI and Definitions screen are clients of the same API.
Task execution-value resolution, publication, Preview and operation are M2/M3.

`cell.json` contains 58 definitions: properties, inherited object/resource types,
models, site instances, family property sets and a pinned Cartesian pose rule.
ECC_51-14, ECC_99-14 and PART1/14 names come from the existing
[tooling reference](../../process/laser-heat-treatment/tooling-reference.json).
**Every dimension, mass, force, pose and tray count here is simulated.** The 14/17
form labels are identifiers, not millimetre measurements. These are not physical
equipment specifications or operating qualification.

## Start the platform API

Prerequisites: the matching platform and solutions feature/integration sources,
the repository Rust toolchain, Python 3, and Node/npm for the optional UI.
Use an unused local state directory; initialization refuses to overwrite one.

From the platform checkout:

```sh
./tools/cargo build -p rx-api --bin rx-platform-local --locked
export RX_MODEL_HOME="$HOME/.local/state/rx-model-m1"
mkdir -p "$RX_MODEL_HOME"
python3 - <<'PY'
import os, secrets
from pathlib import Path
p = Path(os.environ['RX_MODEL_HOME']) / 'password'
with p.open('x') as stream:
    os.chmod(p, 0o600)
    stream.write(secrets.token_urlsafe(24) + '\n')
PY
target/debug/rx-platform-local init "$RX_MODEL_HOME/platform" < "$RX_MODEL_HOME/password"
target/debug/rx-platform-local serve "$RX_MODEL_HOME/platform" 127.0.0.1:8080 http://127.0.0.1:5173
```

The last command remains running. It uses the real writer and SQLite store with
device control disabled. The explicit public origin is shared with the UI; CLI
requests connect directly to port 8080 without needing a browser or UI server.

## Apply and inspect with the existing rx CLI

In another shell, from the solutions checkout:

```sh
export RX_MODEL_HOME="$HOME/.local/state/rx-model-m1"
rxdef() {
  python3 deployment/local-skills/rx definitions \
    --local-url http://127.0.0.1:8080 --public-origin http://127.0.0.1:5173 \
    --password-file "$RX_MODEL_HOME/password" --state-dir "$RX_MODEL_HOME/client" "$@"
}
rxdef apply examples/definitions/0f-laser-simulation/cell.json --output "$RX_MODEL_HOME/cell.receipt.json"
rxdef --references "$RX_MODEL_HOME/cell.receipt.json" show tray.standard
rxdef --references "$RX_MODEL_HOME/cell.receipt.json" show tray.supply
rxdef --references "$RX_MODEL_HOME/cell.receipt.json" points tray.supply --rule rule.tray-slots
rxdef --references "$RX_MODEL_HOME/cell.receipt.json" points tray.dense --rule rule.tray-slots --offset 2300
```

The instance inherits 4 rows, 6 columns and 80 mm pitches. Its origin is
`[100,200,760]` mm; the report preserves the overridden model origin `[0,0,0]`.
Surface height is 760 mm, with model value 900 mm retained as shadowed provenance.
There are 24 poses; the first is `[100,200,760]` and the last `[500,440,760]` mm.
The parent frame is `SIMULATION/world` and every pose uses the explicitly supplied
unit quaternion `[0,0,0,1]` in `[x,y,z,w]` order. Missing orientation is an error,
not an assumed identity rotation. Directions in the rule are local axes rotated
by that quaternion into the parent frame.

The dense model has 40 × 60 = 2,400 poses. Pages contain at most 100 poses;
use the response's `next` as the next `--offset`. The final dense pose is
`[1180,780,0]` mm. No stored list of thousands of manually typed positions exists.

`effective.values` identifies the exact source revision for each value;
`effective.shadowed` preserves overridden values. The pose report includes the
same input provenance, subject and rule references, units, frame and orientation.

## Add a tray as data

```sh
cp examples/definitions/0f-laser-simulation/new-tray.json "$RX_MODEL_HOME/new-tray.json"
# Optionally edit rows, columns, pitches or origin in this JSON file.
rxdef --references "$RX_MODEL_HOME/cell.receipt.json" apply "$RX_MODEL_HOME/new-tray.json" --output "$RX_MODEL_HOME/new.receipt.json"
rxdef --references "$RX_MODEL_HOME/new.receipt.json" points tray.new --rule rule.tray-slots
```

The supplied model has 3 × 5 = 15 poses. Its type and rule are reused unchanged.
To create another model, give the entry a new UUID and a distinct `key`. To revise
an existing definition, keep its UUID and supply its current `expected` revision.
References returned by the server pin exact revisions; they never follow latest
implicitly. `show ... --latest` or `points ... --latest` explicitly selects the
current revisions, including changes made in the UI.

Application is a sequence of individually durable definition transactions. A
failed package may be partially applied; the CLI keeps the original requests in
`--state-dir`. Rerun the **same command and unchanged package** to recover the
original receipts. Changing an in-flight package requires a separate request and
explicit revisions. An access revocation is checked by the server on replay.
This applies authoring data, not a signed executable release or device permission.

## View the same data in the existing UI

```sh
cd apps/operator
npm ci
RX_DEV_API_PORT=8080 npm run dev
```

1. Open `http://127.0.0.1:5173`. Sign in as `admin` with the local password file.
2. Open **Definitions** → **0F Laser cell · SIMULATION**.
3. Click **Load more definitions and reference choices** if needed. Open
   **Resource model** → **tray.standard**, then **Resource instance** → **tray.supply**.
4. Inspect **Saved field values and sources**, including **Overridden** values.
5. Under **Derived points**, select **rule.tray-slots** and click **Calculate points**.
6. Edit an instance value and **Save new revision**. Recalculate and refresh;
   the saved revision survives. Unsaved edits do not change the displayed report.
7. Open **Resource model** → **New tray model** and calculate its 15 poses.

The UI does no pose calculation. It displays `/api/v1/definition` and
`/api/v1/definition-points` responses. No canvas or styling changes are required.

## Repeatable prechecks

```sh
python3 tools/test_definition_client.py
python3 tools/test_definition_api.py --binary /absolute/path/to/rx-platform-local --output /tmp/rx-m1-api-check.json
```

The API precheck creates a private temporary installation and tests real server
storage, source pins, all dense pages, rotated data, invalid orientation, a lost
reply after commit and process restart. It runs no devices. User acceptance of
M1 is still required before starting M2.

## Readable provenance and create conflicts

Add `--format text` before `show` or `points` for named sources, for example:

```sh
rxdef --format text --references "$RX_MODEL_HOME/cell.receipt.json" show tray.supply
```

Names are read at the exact referenced revision. JSON remains the default and
retains full UUID/revision/digest references. The UI shows pinned labels and
revisions; hover over a name to inspect its full reference.

`expected=null` requests creation only. If that ID already exists under a new
request key, `STALE_REVISION` is intentional. The CLI/UI supplements this with a
current authorized read and identifies the create-only conflict or expected vs
current revision. That read is not an atomic snapshot of the earlier rejection.
No revision is substituted automatically and no update is retried. If the read
is unavailable, the original error remains without an invented explanation.

The legacy `surface_height` declaration in M1 is not consumed by the pose rule.
It is not a second z offset. M2 must derive slot surface position from the full
pose and validate any separately retained measurement in the same frame/unit.
Existing saved M1 revisions and their calculated poses remain unchanged.
