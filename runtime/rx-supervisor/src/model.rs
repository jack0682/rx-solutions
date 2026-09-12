use rx_domain::types::*;
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
}
#[derive(Clone, Debug)]
pub struct Launch {
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
