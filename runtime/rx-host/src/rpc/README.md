# Host mTLS adapter

The server requires a client certificate issued by the configured CA and an explicitly registered leaf-certificate fingerprint. PeerHello must match the installation, release, clock and base manifest. CellHello separately selects a supported CellDefinition and exact cell manifest.

Only the Host-owned surfaces are implemented here. Platform business operations on CellService are deliberately refused; registering CellService exposes Cell.Open for the second negotiation stage, not a second business authority. Legacy Prepare/Authorize cannot bypass the cell permit.

RPC keys and stable effect identifiers are durably bound to normalized semantic bodies. CallContext and expected revisions are excluded; grant, permit, condition evidence, target scopes and clearance references remain bound. The gate retains the complete wire-permit fingerprint as well as its local execution fields.

All potentially blocking Host work uses a bounded blocking-worker path. Session handshakes are serialized, and the bind/publish step continues atomically with respect to handshake arbitration even if the requesting future is dropped.

Host.Reconcile still exposes the retained prefix of the Host evidence journal. Background ordered publication and durable per-destination acknowledgment now live in `publication::Publisher`; their separate-process test covers more than128 records and a lost acknowledgment. Full automatic production delivery, source observation streams, native cancellation and watchdog services remain incomplete. See `../../PUBLICATION.md` for the evidence scope and the pending platform control-journal cursor binding.

The reply's CallContext is transport correlation metadata. Platform evidence ingestion must authenticate the responding Host from the verified channel, not treat the echoed requester context as proof of producer identity.
