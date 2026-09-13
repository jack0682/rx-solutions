# Solutions process management example

`solutions-startup.json` is a simulation plan for testing managed mode and durable startup/shutdown. It validates device profile IDs but starts only one release-owned status HTTP service. It does not establish device/controller readiness or qualification.

Provide the state volume at `/var/lib/rx-solutions` and the configuration file at `/config/solutions-startup.json:ro`. Keep the image root containing the programs and related files read-only. Follow the [management module's boundaries and execution instructions](../../runtime/rx-supervisor/README.md).
