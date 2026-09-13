# Host deployment input template

`rx-hostd` is the Host executable included in the `rx-solutions` image. This example selects explicit Host mode in that same image. It does not automatically start default diagnostic mode or all device ROS launches.

Fill in the IDs, paths, and pins in `startup.template.json` with values that have actually been reviewed to create `config/startup.json`. Hashes consisting of 0s are not valid installation evidence. The binding array's host/platform/cell, definition/envelope, qualification, and allowed intents/conditions/scopes must match the P installation. The server SAN must match the Host name used by P, and the client certificate fingerprint is the DER SHA-256 of the registered P certificate. Supply private keys with owner-only permissions.

The current release's builtin backend is FILE_SIMULATION. Selecting VALIDATED_DRIVER for a robot or PLC is rejected because no implementation is registered. Bundling SDK/ROS libraries does not establish that the driver's startup/shutdown has been validated.

First run `host inspect /config/startup.json` using the same config/data mounts and non-root permissions, then run `host init /config/startup.json` **exactly once**. Init creates a new Host ledger and installation identity without opening a device. Do not repeat init against existing data or automatically recreate a lost ledger. Then use compose's `host run`. Prepare the data root during installation so that UID10001 can write to it. Do not share P's authority DB in this volume.

The Host checks the installation identity, ledger generation, and runtime lock, and starts the actual Linux boottime clock and mTLS service. Check this process's readiness through the instance, Host boot, phase, and admission state in `/run/rx-host/host-status.json`. `SOFTWARE_READY_UNARMED` does not establish device operating readiness or qualification. When configuring the remote P Host endpoint and publisher, align the clock/network, server name, certificates, and contract conditions on the same Linux PC.

SIGTERM first blocks new admission and checks the adapter's explicit safe-to-drop evidence. Without that evidence, normal shutdown is deferred. Do not use a kill after an arbitrary timeout as a physical shutdown procedure. Automatic restart is disabled in the template. Forcefully terminating or replacing a running control process requires a separately validated procedure.

The current supervisor's SoftwareOnly recipe does not automatically manage a Host control process. This deployment scope covers direct Host mode and its ledger/process lifecycle boundary. Authority coordination for a multi-Host supervisor, real driver backends, device/fieldbus permissions, and update/restore procedures remain future work.
