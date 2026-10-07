# Simulation result-loss link

This is an external transmission fixture for the F1 checkpoint-3 scenario. It is
not a Host backend, workflow runner, authority issuer or physical operating profile.
It has no access to P/Host databases, device files or signing seeds. Deploy only on
the isolated simulation network, with role-specific TLS files and an owned control
directory. Both inbound roles are pinned to the expected peer certificate.

Build with the exact simulation release image that supplies Python 3.12:

```sh
docker build --build-arg BASE_IMAGE=SIM_RELEASE_IMAGE \
  -f deployment/simulation/result-link/Dockerfile -t rx-sim-result-link .
```

The two relays forward raw unary gRPC request/response payloads unchanged: P→Host
and Host publisher→P. Existing source observations, qualification, fences and grants
remain normal traffic. A selector names the published workflow, Part ordinal and
node. The matching Prepare binds the original operation; after the actual Authorize
response confirms native entry, its return is withheld. Evidence publication and
receipt/evidence queries for that operation are also held. No result is fabricated,
no timeout is extended and no native command is originated by the relay. Cancelled
caller waits do not terminate the already entered native invocation.

The fixture keeps a separate non-authoritative control/audit file. Run these commands
inside its container; the CLI has no remote control endpoint:

```sh
python3 /opt/rx/result-link/link.py --state /data/control arm \
  --publication PUBLICATION_UUID --ordinal 2 --node rotate-align
python3 /opt/rx/result-link/link.py --state /data/control status
python3 /opt/rx/result-link/link.py --state /data/control release
```

Check BLOCKED and its original operation/invocation, then use the product execution
CLI to observe UNKNOWN, resource/slot custody and the absent third Part. Release
only restores communication. Existing P/Host receipt recovery and the Executor's
reconciliation settle the original operation; the relay does not settle it. Continue
the original execution CLI request to supply the remaining object. The product Run,
parameter/report records and correlated SIM effects are the acceptance evidence.

Configuration is a SIMULATION object with two `relays`. Each specifies listen and
upstream addresses, expected server_name, CA, server key/certificate, upstream
client key/certificate and allowed_peer certificate. These files are local
installation inputs, not repository artifacts. `transport.jsonl` records withheld
methods and upstream responses; it is diagnostic evidence, not execution authority.
