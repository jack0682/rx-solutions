//! Process-kill fixture, not a product entry point.
use rx_domain::{intent::*, types::*};
use rx_host::{native::*, simulation::*, *};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{Arc, atomic::AtomicU64},
};

fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn intent() -> Intent {
    Intent {
        kind: Kind::EnsureState,
        target: name("sim/chuck"),
        profile_digest: Digest::from_bytes([1; 32]),
        site_config_digest: Digest::from_bytes([2; 32]),
        calibration_digests: vec![],
        resource_set: vec![name("sim/controller")],
        execution_timeout_ms: Counter(5000),
        prepare_validity_ms: Counter(1000),
        completion_rule: name("sim/closed"),
        cancel_rule: name("sim/stop"),
        body: Body::Predicate(PredicateGoal {
            predicate_id: name("chuck.closed"),
            target: TypedValue::Boolean(true),
            settle_ms: Counter(0),
        }),
    }
}
fn binding() -> Binding {
    Binding {
        host: name("host/sim"),
        platform: name("platform"),
        cell: name("cell/sim"),
        environment: Environment::Simulation,
        purposes: [Purpose::Production, Purpose::Setup].into_iter().collect(),
        definition: ArtifactRef {
            sha256: Digest::from_bytes([3; 32]),
            schema_id: name("rx.cell-definition.v1"),
            size_bytes: Counter(1),
        },
        envelope: ArtifactRef {
            sha256: Digest::from_bytes([4; 32]),
            schema_id: name("rx.operating-envelope.v1"),
            size_bytes: Counter(1),
        },
        qualification: Id::new("55555555-5555-4555-8555-555555555555").unwrap(),
        qualification_revision: Counter(1),
        allowed_intents: vec![intent()],
        scope_ids: vec![name("scope/main")],
        condition_ids: vec![name("sim/ready")],
    }
}
struct KillBoundary {
    directory: PathBuf,
    point: String,
}
impl KillBoundary {
    fn wait_for_kill(&self, point: &str) {
        if self.point != point {
            return;
        }
        std::fs::write(self.directory.join("boundary-ready"), point).unwrap();
        loop {
            std::thread::park();
        }
    }
}
impl BoundaryHook for KillBoundary {
    fn after_qualification_commit(&self) {
        self.wait_for_kill("qualification-commit");
    }
    fn after_configuration_commit(&self) {
        self.wait_for_kill("configuration-commit");
    }
    fn after_send_commit(&self) {
        self.wait_for_kill("before-native");
    }
    fn after_native_entry(&self) {
        self.wait_for_kill("after-native");
    }
}
#[derive(Serialize, Deserialize)]
struct Saved {
    request: Request,
    invocation: Id,
    delivery_journal: Id,
    evidence_journal: Id,
}
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let directory = PathBuf::from(args.get(1).ok_or("directory required")?);
    let point = args.get(2).ok_or("point required")?.clone();
    std::fs::create_dir_all(&directory)?;
    let c = ManualClock {
        clock_id: "simulation/boottime".into(),
        ticks: Arc::new(AtomicU64::new(1000)),
    };
    let native = FileDevice::open(directory.join("device"), c.clone())?;
    let host = Host::with_hooks(
        directory.join("host.db"),
        native,
        c,
        vec![binding()],
        KillBoundary {
            directory: directory.clone(),
            point: point.clone(),
        },
    )?;
    let caller = Caller {
        peer: name("platform"),
        session: id(),
    };
    host.bind_platform(caller.clone())?;
    if point == "configuration-commit" || point == "qualification-commit" {
        use rx_domain::host_configuration as cfg;
        let snapshot = host.inspect_process_configuration(&caller)?.snapshot;
        let scopes = [(name("scope/main"), Counter(2))].into_iter().collect();
        let fence = id();
        let receipt = host.fence(
            &caller,
            fence.clone(),
            &name("cell/sim"),
            Counter(2),
            scopes,
            std::collections::BTreeSet::from([id()]),
        )?;
        let request = cfg::Request {
            schema: name("rx.host-process-configuration-request.v1"),
            id: id(),
            change: id(),
            preparation: Counter(1),
            plan_digest: Digest::from_bytes([9; 32]),
            host: snapshot.host,
            expected_host_boot: snapshot.host_boot,
            expected_delivery_journal: snapshot.delivery_journal,
            binding_digest: snapshot.binding_digest,
            cells: vec![cfg::CellTarget {
                cell: name("cell/sim"),
                expected_context: None,
                before_configuration: Digest::from_bytes([6; 32]),
                after_configuration: Digest::from_bytes([7; 32]),
                recipe: binding().definition,
                definition: binding().definition.sha256,
                envelope: binding().envelope.sha256,
                environment: name("SIMULATION"),
                required_intents: vec![intent().digest()?],
                required_conditions: binding().condition_ids,
                epoch: Counter(2),
                scopes: receipt.state.scopes,
                fence_request: fence,
            }],
        };
        std::fs::write(
            directory.join("configuration-request.json"),
            rx_domain::canonical::bytes(&request)?,
        )?;
        let applied = host.accept_process_configuration(&caller, request)?;
        if point == "qualification-commit" {
            use rx_domain::host_qualification as q;
            let snapshot = applied.snapshot;
            let current = &snapshot.cells[0];
            let context = current.applied.as_ref().unwrap();
            let gate = host.fence(
                &caller,
                id(),
                &current.cell,
                Counter(current.epoch.0 + 1),
                current
                    .scopes
                    .iter()
                    .map(|(k, v)| (k.clone(), Counter(v.0 + 1)))
                    .collect(),
                BTreeSet::from([id()]),
            )?;
            let request = q::Request {
                schema: name("rx.host-qualification-request.v1"),
                id: id(),
                host: snapshot.host.clone(),
                expected_host_boot: snapshot.host_boot.clone(),
                delivery_journal: snapshot.delivery_journal.clone(),
                binding_digest: snapshot.binding_digest,
                change: context.change.clone(),
                review: id(),
                review_revision: Counter(1),
                review_digest: Digest::from_bytes([81; 32]),
                decision_revision: Counter(1),
                policy_digest: Digest::from_bytes([82; 32]),
                application_digest: Digest::from_bytes([83; 32]),
                cells: vec![q::CellTarget {
                    cell: current.cell.clone(),
                    configuration: context.configuration,
                    context_request: context.request.clone(),
                    context_sequence: context.receipt_sequence,
                    definition: current.definition,
                    envelope: current.envelope,
                    environment: current.environment.clone(),
                    qualification: id(),
                    qualification_revision: Counter(1),
                    dependencies: vec![context.configuration, current.definition, current.envelope],
                    limitations: binding().envelope,
                    allowed_intents: vec![intent().digest()?],
                    purposes: vec![name("PRODUCTION")],
                    epoch: gate.state.epoch,
                    scopes: gate.state.scopes,
                    fence_request: gate.id,
                    required_blocks: gate.state.blocked.into_iter().collect(),
                }],
            };
            std::fs::write(
                directory.join("qualification-request.json"),
                rx_domain::canonical::bytes(&request)?,
            )?;
            host.accept_qualification(&caller, request)?;
        }
        return Err("configuration/qualification hook was not reached".into());
    }
    if point == "qualification-recover" {
        let request: rx_domain::host_qualification::Request = rx_domain::canonical::decode_json(
            &std::fs::read(directory.join("qualification-request.json"))?,
        )?;
        let observed = host.lookup_qualification(&caller, &request.id)?;
        let state = host.inspect_cell(&caller, &binding().cell)?.state;
        if observed.receipt.as_ref().map(|r| r.status)
            != Some(rx_domain::host_qualification::Status::Accepted)
            || observed.receipt_matches_current_host
            || host
                .arm(
                    &caller,
                    id(),
                    &binding().cell,
                    state.epoch,
                    &state.scopes,
                    &state.blocked,
                )
                .is_ok()
            || !FileDevice::effects(directory.join("device"))?.is_empty()
        {
            return Err("qualification recovery differs".into());
        }
        std::fs::write(
            directory.join("qualification-result.json"),
            rx_domain::canonical::bytes(&observed)?,
        )?;
        return Ok(());
    }
    if point == "configuration-recover" {
        let request: rx_domain::host_configuration::Request = rx_domain::canonical::decode_json(
            &std::fs::read(directory.join("configuration-request.json"))?,
        )?;
        let observed = host.lookup_process_configuration(&caller, &request.id)?;
        let cell = host.inspect_cell(&caller, &name("cell/sim"))?.state;
        if host
            .arm(
                &caller,
                id(),
                &name("cell/sim"),
                cell.epoch,
                &cell.scopes,
                &cell.blocked,
            )
            .is_ok()
        {
            return Err("unqualified configuration armed".into());
        }
        if observed.receipt.as_ref().map(|r| r.status)
            != Some(rx_domain::host_configuration::Status::AppliedUnqualified)
            || observed.context_matches_current_host
            || !FileDevice::effects(directory.join("device"))?.is_empty()
        {
            return Err("configuration recovery differs".into());
        }
        std::fs::write(
            directory.join("configuration-result.json"),
            rx_domain::canonical::bytes(&observed)?,
        )?;
        return Ok(());
    }
    if point == "recover" {
        let saved: Saved =
            rx_domain::canonical::decode_json(&std::fs::read(directory.join("request.json"))?)?;
        let before = host.receipt(&caller, &saved.request.operation)?;
        // Replay cannot turn SEND_ENTERED back into a new invocation, even with a new Host boot.
        host.authorize(&caller, saved.request.clone(), &saved.invocation)?;
        let recovered = host.reconcile(&caller, &saved.request.operation)?;
        let meta = host.journals()?;
        if meta.delivery_journal != saved.delivery_journal
            || meta.evidence_journal != saved.evidence_journal
        {
            return Err("journal identity changed".into());
        }
        let report = serde_json::json!({"before":before.state,"after":recovered.state,
            "effects":FileDevice::effects(directory.join("device"))?.len(),
            "evidence":host.evidence_after(&caller,Counter(0),128)?.len()});
        std::fs::write(directory.join("result.json"), serde_json::to_vec(&report)?)?;
        println!("{report}");
        return Ok(());
    }
    let scopes = [(name("scope/main"), Counter(1))].into_iter().collect();
    host.arm(
        &caller,
        id(),
        &name("cell/sim"),
        Counter(1),
        &scopes,
        &BTreeSet::new(),
    )?;
    let grant = host.acquire_grant(
        &caller,
        id(),
        vec![name("sim/controller")],
        Counter(1),
        Counter(10_000_000_000),
    )?;
    let intent = intent();
    let digest = intent.digest()?;
    let operation = id();
    let request = Request {
        operation: operation.clone(),
        intent,
        digest,
        grant: grant.id.clone(),
        permit: Permit {
            id: id(),
            operation,
            digest,
            cell: name("cell/sim"),
            epoch: Counter(1),
            scopes,
            envelope: binding().envelope.sha256,
            qualification: binding().qualification,
            qualification_revision: Counter(1),
            grant: grant.id,
            host_boot: host.boot_id()?,
            conditions: [name("sim/ready")].into_iter().collect(),
            expires_at: TimePoint {
                clock_id: "simulation/boottime".into(),
                ticks_ns: Counter(1_000_000_000),
            },
            source_digest: None,
            purpose: Purpose::Production,
            parent: PermitParent::Mandate(id()),
        },
    };
    let record = host.prepare(&caller, request.clone())?;
    let meta = host.journals()?;
    let saved = Saved {
        request: request.clone(),
        invocation: record.invocation.ok_or("invocation missing")?,
        delivery_journal: meta.delivery_journal,
        evidence_journal: meta.evidence_journal,
    };
    std::fs::write(directory.join("request.json"), serde_json::to_vec(&saved)?)?;
    host.authorize(&caller, request, &saved.invocation)?;
    Err("kill hook unexpectedly returned".into())
}
