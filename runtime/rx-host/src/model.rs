use rx_domain::{intent::Intent, types::*};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub trait Clock: Send + Sync {
    fn healthy(&self) -> bool {
        true
    }
    fn now(&self) -> TimePoint;
}
/// Created by an authenticated peer adapter, never from an RPC body's peer/role fields.
#[derive(Clone, Debug)]
pub struct Caller {
    pub peer: Name,
    pub session: Id,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub host: Name,
    pub platform: Name,
    pub cell: Name,
    pub definition: ArtifactRef,
    pub envelope: ArtifactRef,
    pub qualification: Id,
    pub qualification_revision: Counter,
    pub allowed_intents: Vec<Intent>,
    pub scope_ids: Vec<Name>,
    pub condition_ids: Vec<Name>,
    pub environment: Environment,
    pub purposes: BTreeSet<Purpose>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Environment {
    Simulation,
    Physical,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Purpose {
    Production,
    Setup,
    Recovery,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PermitParent {
    Mandate(Id),
    Recovery {
        case: Id,
        plan: Digest,
        step: Name,
        visit: Counter,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostMeta {
    pub delivery_journal: Id,
    pub evidence_journal: Id,
    pub delivery_seq: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellState {
    pub epoch: Counter,
    pub scopes: BTreeMap<Name, Counter>,
    pub blocked: BTreeSet<Id>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReceiptState {
    Prepared,
    SendEntered,
    NativeRejected,
    NativeAccepted,
    ResultCaptured,
    VoidedBeforeSend,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryRecord {
    pub operation: Id,
    pub digest: Digest,
    pub invocation: Option<Id>,
    pub state: ReceiptState,
    pub journal_seq: Counter,
    pub prepared_boot: Id,
    pub prepare_until: TimePoint,
    pub cell: Name,
    pub intent: Intent,
    pub permit: Id,
    pub device_session: Id,
    pub evidence_ids: Vec<Id>,
    pub permit_digest: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredGrant {
    pub id: Id,
    pub host_boot: Id,
    pub owner: Name,
    pub session: Id,
    pub resources: Vec<Name>,
    pub fence: Counter,
    pub expires_at: TimePoint,
    pub renew_seq: Counter,
    pub ttl_ns: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateReceipt {
    pub id: Id,
    pub host_boot: Id,
    pub journal: Id,
    pub sequence: Counter,
    pub cell: Name,
    pub state: CellState,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoidRecord {
    pub operation: Id,
    pub digest: Digest,
    pub sequence: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceFence {
    pub resource: Name,
    pub maximum: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permit {
    pub id: Id,
    pub operation: Id,
    pub digest: Digest,
    pub cell: Name,
    pub epoch: Counter,
    pub scopes: BTreeMap<Name, Counter>,
    pub envelope: Digest,
    pub qualification: Id,
    pub qualification_revision: Counter,
    pub grant: Id,
    pub host_boot: Id,
    pub conditions: BTreeSet<Name>,
    pub expires_at: TimePoint,
    pub purpose: Purpose,
    pub parent: PermitParent,
    pub source_digest: Option<Digest>,
}
pub struct Inspection {
    pub state: CellState,
    pub host_boot: Id,
    pub definition: Digest,
    pub pending_operations: Vec<Id>,
    pub pending_permits: Vec<Id>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub operation: Id,
    pub intent: Intent,
    pub digest: Digest,
    pub grant: Id,
    pub permit: Permit,
}
#[derive(Clone, Debug)]
pub struct Guard {
    pub device_session: Id,
    pub valid_until: TimePoint,
    pub satisfied: BTreeSet<Name>,
}
#[derive(Clone, Debug)]
pub struct LocalHandover {
    pub device_session: Id,
    pub observed_at: TimePoint,
    pub uncertainty_ns: Counter,
    pub no_pending_commands: bool,
    pub control_available: bool,
    pub support_stable: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCapture {
    pub status_schema: Name,
    pub status: i64,
    pub native_id: Option<String>,
    pub captured_at: TimePoint,
    pub device_session: Id,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRecord {
    pub evidence_id: Id,
    pub operation: Id,
    pub invocation: Id,
    pub profile_digest: Digest,
    pub capture: NativeCapture,
}
#[derive(Clone, Copy, Debug)]
pub enum ProtectionIncident {
    StoreFault,
    OwnerLost,
    Expired,
}

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("host storage: {0}")]
    Store(#[from] rx_ports::StoreError),
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("identity or scope conflict")]
    Conflict,
    #[error("caller is not the current platform")]
    Forbidden,
    #[error("stale epoch, grant or boot")]
    Stale,
    #[error("local guard or protection condition is not satisfied")]
    Guard,
    #[error("unresolved native work prevents handover")]
    Busy,
    #[error("receipt is not present in this journal")]
    NotFound,
    #[error("operation was permanently voided before native entry")]
    Voided,
    #[error("native result not known: {0}")]
    NativeUnknown(String),
}
pub type Result<T> = std::result::Result<T, HostError>;
