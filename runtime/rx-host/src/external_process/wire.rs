//! Closed private-process messages. They carry native facts, never authority or P outcomes.
use super::profile::PROTOCOL;
use crate::NativeCapture;
use rx_domain::{intent::Intent, types::*};
use rx_process_contract::execution_v2::host_inputs::BoundInput;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dispatch {
    pub operation: Id,
    pub invocation: Id,
    pub intent: Intent,
    pub input: BoundInput,
    pub device_session: Id,
    pub admitted_at: TimePoint,
    pub expires_at: TimePoint,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema: Name,
    pub challenge: Id,
    pub profile_digest: Digest,
    pub device_session: Id,
    pub now: TimePoint,
    pub dispatch: Option<Dispatch>,
    pub sources: Vec<Name>,
}
impl Request {
    pub fn new(profile_digest: Digest, device_session: Id, now: TimePoint) -> Self {
        Self {
            schema: Name::new(PROTOCOL).expect("constant"),
            challenge: crate::journal::id(),
            profile_digest,
            device_session,
            now,
            dispatch: None,
            sources: vec![],
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub schema: Name,
    pub challenge: Id,
    pub request_sha256: Digest,
    pub operation: Id,
    pub invocation: Id,
    pub intent_digest: Digest,
    pub profile_digest: Digest,
    pub device_session: Id,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sample {
    pub value: TypedValue,
    pub acquired_at: TimePoint,
    pub uncertainty_ns: Counter,
    pub quality_good: bool,
    pub origin_age_bounded: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema: Name,
    pub challenge: Id,
    pub profile_digest: Digest,
    pub device_session: Id,
    pub observed_at: TimePoint,
    pub uncertainty_ns: Counter,
    pub no_pending_commands: bool,
    pub control_available: bool,
    pub support_stable: bool,
    pub safe_to_drop: bool,
    pub samples: BTreeMap<Name, Sample>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Completion {
    pub schema: Name,
    pub challenge: Id,
    pub dispatch_digest: Digest,
    pub operation: Id,
    pub invocation: Id,
    pub intent_digest: Digest,
    pub profile_digest: Digest,
    pub device_session: Id,
    pub capture: Option<NativeCapture>,
    pub current: Snapshot,
}
