# RX Solutions

Since 2026-09-13, RX has been a personal project aimed at collaboration among heterogeneous robots and facilities, complete door-to-door tasks, and expansion across districts, villages, and cities. This repository owns device and facility integration, workflows, operator applications, and application packages. The [current project goal](https://github.com/jack0682/rx_docs/blob/main/docs/01_product_definition.md) and [scope](https://github.com/jack0682/rx_docs/blob/main/docs/03_product_scope.md) are maintained in the documentation repository in the same workspace.

It owns RX device Hosts, ROS/native integration, declarative processes and the BT executor, site packages, and configuration and operator web applications. It does not bypass the platform's authoritative ledger or authorization rules.

It uses vendor-neutral device contracts. The default catalog contains simulated JTC declarations for software tests; external devices and facilities are added as packages with source pins and explicit authority, completion, and handover conditions. The current simulation declarations do not constitute completed support for physical devices. The [first DYNAMIXEL adapter](runtime/rx-host/DYNAMIXEL_ADAPTER.md) uses the official SDK 4.1.0 in one Host-owned helper for protocol 2 Ping over a fictional simulated transport; real endpoints are refused. The [DHI foreign component investigation](native/dhi/README.md) keeps the original plugin in an actual controller_manager while RX controls descriptor admission for one fresh PTY model. Its signed resident path now verifies the release-owned catalog and plan before starting the same resource-custody guardian; the classification remains limited to the fresh-PTY simulation and does not authorize physical devices or general Compose/s6 supervision.

**Implementation draft v0.1 was closed on 2026-09-13.** It hands over the validated phase80 runtime; it does not establish physical device operation or completed support for every model. The [draft handoff and critical open items](https://github.com/jack0682/rx_docs/blob/codex/initial-draft/docs/implementation/draft_handoff.md) record the state of the earlier industrial and simulated-cell draft. Unvalidated investigation procedure tools are preserved separately on `codex/investigation-wip`. The overall design follows [rx_docs](https://github.com/jack0682/rx_docs); the scope and status of the earlier implementation draft follow the [implementation records](https://github.com/jack0682/rx_docs/tree/codex/initial-draft/docs/implementation). Distributed and city-scale validation under the new goal remains future work. The ROS-independent core, authoritative ledger, and API belong in [rx-platform](https://github.com/jack0682/rx-platform).

The [operator application](apps/operator/README.md) connects to the platform's local API. The [development cell materials](examples/development/README.md) are used only as an unvalidated simulation configuration. The operator application's request-retrieval records do not replace execution authority or the ledger of device outcomes.

The vendor-neutral solutions image configuration and startup boundary follow [Native image](dependencies/NATIVE_IMAGE.md).

## License

RX Solutions is licensed under [Apache License 2.0](LICENSE). Third-party components retain their own licenses and notices.
