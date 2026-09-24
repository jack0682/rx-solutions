//! Catalog-owned host requirements. An admission receipt covers one whole launch,
//! never a partially applied subset. The OS backend owns actual enforcement.
use crate::{Error, Result};
use rx_domain::types::*;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Capacity {
    CpuMillicores,
    MemoryBytes,
    /// Per-process virtual address-space usage ceiling, not RSS, physical RAM,
    /// reserved capacity, or an availability guarantee.
    AddressSpaceBytes,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "meaning", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Requirement {
    NotRequired,
    Unknown,
    UpperBound { resource: Capacity, amount: Counter },
    ReservedCapacity { resource: Capacity, amount: Counter },
    ExclusiveAccess { resource: Name },
    SharedAccess { resource: Name },
}

/// Keys identify requirements in rejection and observation output, not site parameters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Requirements(pub BTreeMap<Name, Requirement>);
impl Requirements {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.0.len() > 32 {
            return Err(Error::Invalid("at most 32 execution requirements".into()));
        }
        for (name, requirement) in &self.0 {
            if matches!(requirement, Requirement::UpperBound { amount, .. }
                | Requirement::ReservedCapacity { amount, .. } if amount.0 == 0)
            {
                return Err(Error::Invalid(format!(
                    "execution requirement {name}: zero capacity is not NotRequired"
                )));
            }
        }
        Ok(())
    }
    pub fn needs_enforcement(&self) -> bool {
        self.0
            .values()
            .any(|r| !matches!(r, Requirement::NotRequired))
    }
}

/// Created only from the selected catalog and the current saved plan/instance.
/// A backend may inspect or clone a request but cannot weaken its fields.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Request {
    program: Name,
    process: Name,
    instance: Id,
    plan_digest: Digest,
    requirements: Requirements,
}
impl Request {
    pub(crate) fn new(
        program: Name,
        process: Name,
        instance: Id,
        plan_digest: Digest,
        requirements: Requirements,
    ) -> Self {
        Self {
            program,
            process,
            instance,
            plan_digest,
            requirements,
        }
    }
    pub fn requirements(&self) -> &Requirements {
        &self.requirements
    }
    pub fn instance(&self) -> &Id {
        &self.instance
    }
    pub fn program(&self) -> &Name {
        &self.program
    }
    pub fn process(&self) -> &Name {
        &self.process
    }
    pub fn reject(&self, reason: &str) -> Vec<Unmet> {
        self.requirements
            .0
            .iter()
            .filter(|(_, r)| !matches!(r, Requirement::NotRequired))
            .map(|(name, _)| Unmet {
                requirement: name.clone(),
                reason: reason.into(),
            })
            .collect()
    }
    pub(crate) fn unknown(&self) -> Vec<Unmet> {
        self.requirements
            .0
            .iter()
            .filter(|(_, r)| matches!(r, Requirement::Unknown))
            .map(|(name, _)| Unmet {
                requirement: name.clone(),
                reason: "requirement is unknown, not NotRequired".into(),
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Unmet {
    pub requirement: Name,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "basis", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Evidence {
    /// A test double's result, never evidence of OS policy application.
    Simulation {
        provider: Name,
        reference: String,
    },
    /// A trusted future backend's report; does not prove timing or physical safety.
    HostReport {
        provider: Name,
        reference: String,
    },
    NoRequirements,
    /// Observed by the owning backend through Linux procfs after target exec.
    /// Historical evidence only; not policy correctness or physical handover.
    LinuxRlimit {
        observation: Box<KernelObservation>,
    },
}

/// Only the OS implementation can construct this evidence. Stored JSON is not
/// deserializable back into a receipt or a current enforcement capability.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct KernelObservation {
    request: Request,
    pid: u32,
    instance: Id,
    requirement: Name,
    resource: Capacity,
    soft_bytes: Counter,
    hard_bytes: Counter,
    previous_soft_bytes: Option<Counter>,
    previous_hard_bytes: Option<Counter>,
    observer: &'static str,
    scope: &'static str,
    capacity_reservation: &'static str,
    physical_handover: &'static str,
}
impl KernelObservation {
    #[cfg(target_os = "linux")]
    pub(crate) fn observed(
        request: &Request,
        pid: u32,
        requirement: Name,
        amount: u64,
        previous: (Option<u64>, Option<u64>),
    ) -> Self {
        Self {
            request: request.clone(),
            pid,
            instance: request.instance.clone(),
            requirement,
            resource: Capacity::AddressSpaceBytes,
            soft_bytes: Counter(amount),
            hard_bytes: Counter(amount),
            previous_soft_bytes: previous.0.map(Counter),
            previous_hard_bytes: previous.1.map(Counter),
            observer: "OWNING_PARENT_PROCFS_AFTER_EXEC",
            scope: "PER_PROCESS_VIRTUAL_ADDRESS_SPACE_UPPER_BOUND",
            capacity_reservation: "NONE_CREATED",
            physical_handover: "NOT_ASSESSED",
        }
    }
    pub(crate) fn stored(&self) -> StoredLimit {
        StoredLimit {
            pid: self.pid,
            instance: self.instance.clone(),
            requirement: self.requirement.clone(),
            soft_bytes: self.soft_bytes,
            hard_bytes: self.hard_bytes,
            scope: self.scope.into(),
            capacity_reservation: self.capacity_reservation.into(),
            physical_handover: self.physical_handover.into(),
        }
    }
}

/// Inert history loaded from the execution store, deliberately distinct from
/// KernelObservation/Receipt. It cannot be submitted as application evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredLimit {
    pub pid: u32,
    pub instance: Id,
    pub requirement: Name,
    pub soft_bytes: Counter,
    pub hard_bytes: Counter,
    pub scope: String,
    pub capacity_reservation: String,
    pub physical_handover: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ResourceLifetime {
    /// Application may have begun. No current policy or absence is inferred.
    Unconfirmed,
    ObservedAtStart,
    NoPolicyRemainingAfterRejectedStart,
    DirectChildExitedDescendantsUnassessed,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceHistory {
    pub lifetime: ResourceLifetime,
    pub last_observed: Option<StoredLimit>,
    pub current_enforcement: String,
    pub capacity_reservation: String,
    pub physical_handover: String,
}
impl ResourceHistory {
    pub(crate) fn unconfirmed() -> Self {
        Self {
            lifetime: ResourceLifetime::Unconfirmed,
            last_observed: None,
            current_enforcement: "NOT_ESTABLISHED_BY_HISTORY".into(),
            capacity_reservation: "NOT_ESTABLISHED".into(),
            physical_handover: "NOT_ASSESSED".into(),
        }
    }
}

/// No public field, deserializer or subset constructor can manufacture a partial receipt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Receipt {
    request: Request,
    evidence: Evidence,
}
impl Receipt {
    /// Backends attest the entire request only after enforcing it. The constructor
    /// binds identity and meaning; it cannot verify the truth of an external report.
    pub fn reported(request: &Request, evidence: Evidence) -> Result<Self> {
        request.requirements.validate()?;
        if !request.unknown().is_empty() {
            return Err(Error::Invalid(
                "unknown requirements cannot be admitted".into(),
            ));
        }
        match &evidence {
            Evidence::LinuxRlimit { observation }
                if observation.request != *request || observation.pid == 0 =>
            {
                return Err(Error::Invalid(
                    "kernel observation belongs to another complete request".into(),
                ));
            }
            Evidence::NoRequirements if request.requirements.needs_enforcement() => {
                return Err(Error::Invalid(
                    "required policy cannot be reported as no requirements".into(),
                ));
            }
            Evidence::Simulation { reference, .. } | Evidence::HostReport { reference, .. }
                if reference.trim().is_empty() || reference.len() > 1024 =>
            {
                return Err(Error::Invalid(
                    "bounded application evidence reference required".into(),
                ));
            }
            _ => {}
        }
        Ok(Self {
            request: request.clone(),
            evidence,
        })
    }
    pub(crate) fn matches(&self, request: &Request) -> bool {
        self.request == *request
    }
    pub(crate) fn stored_limit(&self) -> Option<StoredLimit> {
        if let Evidence::LinuxRlimit { observation } = &self.evidence {
            Some(observation.stored())
        } else {
            None
        }
    }
}

/// The resource/exec boundary has only all-admitted or none-applied rejection.
/// Transport/exec uncertainty remains SpawnFailure::Uncertain, never Rejected.
pub enum Decision {
    Admitted { pid: u32, receipt: Receipt },
    Rejected { unmet: Vec<Unmet> },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Application {
    LegacyNotDeclared,
    NotApplied,
    /// Historical observation bound to one launch, not a current enforcement monitor.
    ReportedAtStart {
        receipt: Receipt,
    },
    Unconfirmed {
        reason: String,
    },
}
#[derive(Clone, Debug, Serialize)]
pub struct Status {
    /// None is legacy non-participation; Some(empty) explicitly declares no requirements.
    pub requested: Option<Requirements>,
    pub application: Application,
    pub not_applied_reasons: Vec<Unmet>,
}
