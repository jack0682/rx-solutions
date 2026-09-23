//! Catalog-owned host requirements. An admission receipt covers one whole launch,
//! never a partially applied subset. This module implements no OS enforcement.
use crate::{Error, Result};
use rx_domain::types::*;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Capacity {
    CpuMillicores,
    MemoryBytes,
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
