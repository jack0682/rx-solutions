# Read-only simulated DYNAMIXEL Ping

This example selects the Host adapter described in
[the DYNAMIXEL boundary](../../../runtime/rx-host/DYNAMIXEL_ADAPTER.md).
No hardware connection, torque, register write or motion is supported.

Obtain the descriptor from the exact installed Host build:

```sh
/opt/rx/bin/rx-hostd drivers dynamixel > dynamixel-driver.json
python3 -c 'import json; d=json.load(open("dynamixel-driver.json")); print(json.dumps({"kind":"VALIDATED_DRIVER","profile":d["profile"],"driver_digest":d["source_digest"],"endpoint":d["endpoint"]},indent=2))'
```

Use that backend in a new reviewed Host startup configuration. The descriptor's exact
`materials` and `intent_contract` must be bound in the reviewed process/cell configuration
before package signing and compilation. The fixed endpoint is `simulation/dynamixel/id-1`;
substituting `/dev/ttyUSB0` is an error, not hardware enablement. Preserve the normal TLS,
installation, release and binding pins; do not invent or copy an old installation identity.

The helper binary and descriptor require the G2 authenticated release inventory. Inspect
and initialize remain passive. Current cell commissioning and operation admission are
still required, even though the model is simulated. The installed Python and C++ examples
in the platform client package supply the same `Cell.SubmitOperation` request; neither
example contains a device driver or calls the helper directly.

For reproducible software acceptance, the platform harness accepts the descriptor above:

```sh
python3 tools/test_clients.py --language python --mode loss \
  --adapter-descriptor /absolute/path/dynamixel-driver.json \
  --platform-image SELECTED_PLATFORM_IMAGE \
  --solutions-image SIGNED_SOLUTIONS_IMAGE \
  --client-image INSTALLED_CLIENT_IMAGE \
  --release-evidence /absolute/path/release-evidence.json \
  --evidence /absolute/path/new-evidence-directory
```

Run from a platform checkout with matching client/source artifacts. Repeat with `cpp` and
with `--mode unknown` for the separate observed helper-loss scenes. Release evidence must
contain the actual existing recovery-test results for the selected image; it is not a
manual permission receipt. The harness uses offline test signers and explicitly simulated
qualification inputs, never an approval for physical equipment.
