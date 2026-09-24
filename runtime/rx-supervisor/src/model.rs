use rx_domain::types::*;
pub use rx_service_status::{GuardedObservation, GuardedState, GuardedStatusBinding};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub schema: Name,
    pub id: Id,
    pub environment: Environment,
    pub profiles: Vec<Name>,
    pub processes: Vec<Process>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Environment {
    Simulation,
    Physical,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Process {
    pub id: Name,
    pub program: Name,
    pub parameters: BTreeMap<Name, String>,
    pub depends_on: Vec<Name>,
    pub startup_timeout_ms: Counter,
    pub shutdown_timeout_ms: Counter,
    pub restart_limit: Counter,
    pub restart_backoff_ms: Counter,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Effect {
    NonActuating,
    RequiresPlatformAuthority,
    ProtocolGuardedService,
}
/// Supplied by the release/program catalog, never by the site process selection.
#[derive(Clone, Debug, Serialize)]
pub struct Program {
    pub id: Name,
    pub effect: Effect,
    pub executable: PathBuf,
    pub executable_sha256: Digest,
    pub files: BTreeMap<PathBuf, Digest>,
    pub fixed_arguments: Vec<String>,
    pub arguments: BTreeMap<Name, Argument>,
    pub ready: ReadyProbe,
    /// Authored by the catalog. No field in the site Process can replace this.
    /// Omission preserves the legacy program digest and startup path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_requirements: Option<crate::execution::Requirements>,
    /// Author-owned intended-use conditions over a supported observation source.
    /// Omission preserves legacy catalog/plan digest inputs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub functional_readiness: Option<crate::use_assessment::ReadinessContract>,
    /// Optional author-owned external decision anchors; never loaded from site input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_policy: Option<crate::decision::Policy>,
}
#[derive(Clone, Debug, Serialize)]
pub enum Argument {
    Choice { flag: String, values: Vec<String> },
    Port { flag: String },
}
#[derive(Clone, Debug, Serialize)]
pub enum ReadyProbe {
    AliveOnly,
    HttpStatus { port_parameter: Name },
    GuardedStatus(GuardedStatusBinding),
}
#[derive(Clone, Debug)]
pub struct Launch {
    /// Current plan selection, never a persistent registration or OS identity.
    pub selection: Name,
    pub instance: Id,
    pub effect: Effect,
    pub executable: PathBuf,
    pub executable_sha256: Digest,
    pub files: BTreeMap<PathBuf, Digest>,
    pub arguments: Vec<String>,
    pub ready: ReadyProbe,
    pub port: Option<u16>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Phase {
    Pending,
    Prepared,
    SpawnEntered,
    Starting,
    ProcessReady,
    Unready,
    StopRequested,
    Exited,
    Skipped,
    StartFailed,
    Unknown,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub id: Name,
    pub instance: Option<Id>,
    pub phase: Phase,
    pub attempts: Counter,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guarded_exit: Option<GuardedExit>,
    /// Historical resource observations never restore a current receipt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<crate::execution::ResourceHistory>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "SCREAMING_SNAKE_CASE",
    deny_unknown_fields
)]
pub enum GuardedExit {
    Confirmed {
        reconciliation_required: bool,
        sequence: Counter,
        observed_at: TimePoint,
        payload_digest: Digest,
    },
    Unconfirmed {
        reason: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub schema: Name,
    pub plan: Id,
    pub plan_digest: Digest,
    pub stop_requested: bool,
    pub records: BTreeMap<Name, Record>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub schema: &'static str,
    pub state: State,
    pub blocked: Vec<String>,
    pub all_exited: bool,
    pub control_prepared: bool,
    pub physical_shutdown_assessed: bool,
    pub guarded_shutdown_confirmed: bool,
    pub reconciliation_required: bool,
    pub execution_admission: BTreeMap<Name, crate::execution::Status>,
}
/// This port must validate existing platform lifecycle authority for control owners.
/// No public CLI, process liveness or Boolean from a site file can implement that authority.
pub trait LifecycleAuthority {
    fn may_start(&self, plan: &Plan, process: &Process, launch: &Launch) -> bool;
    fn may_stop(&self, plan: &Plan, process: &Process, launch: &Launch) -> bool;
}
pub struct SoftwareOnly;
impl LifecycleAuthority for SoftwareOnly {
    fn may_start(&self, _: &Plan, _: &Process, l: &Launch) -> bool {
        l.effect == Effect::NonActuating
    }
    fn may_stop(&self, _: &Plan, _: &Process, l: &Launch) -> bool {
        l.effect == Effect::NonActuating
    }
}

/// Release-catalog software boot/guarded-shutdown permission. Never authorizes a physical lifecycle.
pub struct GuardedServices;
impl LifecycleAuthority for GuardedServices {
    fn may_start(&self, _: &Plan, _: &Process, launch: &Launch) -> bool {
        launch.effect == Effect::NonActuating
            || (launch.effect == Effect::ProtocolGuardedService
                && matches!(launch.ready, ReadyProbe::GuardedStatus(_)))
    }
    fn may_stop(&self, _: &Plan, _: &Process, launch: &Launch) -> bool {
        // Permission to request the daemon's cooperative stop, not permission to drop its adapter.
        launch.effect == Effect::NonActuating
            || (launch.effect == Effect::ProtocolGuardedService
                && matches!(launch.ready, ReadyProbe::GuardedStatus(_)))
    }
}
