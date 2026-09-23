//! Repository-backed, diagnostic-only consumption of component reports. No process
//! orchestration or work-permission issuer. Source I/O stays behind a separate port.
use super::*;
use crate::use_assessment::{
    ConditionAssessment, ConditionState, ReadinessAssessment, ReportOrigin, UseScope, WorkUseState,
};
use std::collections::BTreeMap;
mod store;
pub use store::Consumer;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Window {
    PreparationOnly,
    ResultGeneration,
    Continuous,
}
/// Undeclared and unknown never mean independent. Only the catalog author can
/// change these values; site requests carry a profile name, not a replacement.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "declaration", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Dependency {
    Undeclared,
    Unknown {
        reason: String,
    },
    Independent,
    Required {
        window: Window,
        provider_role: Name,
        interpretation: Name,
    },
}
#[derive(Clone, Debug, Serialize)]
pub struct Catalog {
    pub program: Name,
    pub version: Counter,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_policy: Option<crate::decision::Policy>,
    pub profiles: BTreeMap<Name, Dependency>,
}
impl Catalog {
    pub fn reference(&self) -> Result<CatalogReference> {
        if let Some(policy) = &self.decision_policy {
            policy.fingerprint().map_err(|e| invalid(&e))?;
        }
        if self.version.0 == 0 || self.profiles.is_empty() || self.profiles.len() > 16 {
            return Err(invalid(
                "diagnostic catalog requires version and 1..16 profiles",
            ));
        }
        for declaration in self.profiles.values() {
            if let Dependency::Unknown { reason } = declaration
                && (reason.trim().is_empty() || reason.len() > 1024)
            {
                return Err(invalid("unknown dependency requires a bounded reason"));
            }
        }
        Ok(CatalogReference {
            program: self.program.clone(),
            digest: digest("RX-DIAGNOSTIC-CATALOG-v1", self)?,
        })
    }
}
/// The framework's bounded read-only reference operation, not an executable
/// program entry and not a replacement for the existing process catalog.
pub fn catalog() -> Catalog {
    let mut profiles: BTreeMap<_, _> = [
        ("snapshot-after-preparation", Window::PreparationOnly),
        ("snapshot-at-generation", Window::ResultGeneration),
        ("current-report-collection", Window::Continuous),
    ]
    .into_iter()
    .map(|(profile, window)| {
        (
            name(profile),
            Dependency::Required {
                window,
                provider_role: name("diagnostics/support-summary"),
                interpretation: name("reported-support-summary.v1"),
            },
        )
    })
    .collect();
    profiles.insert(name("catalog-summary"), Dependency::Independent);
    Catalog {
        program: name("rx/diagnostic-consumer"),
        version: Counter(1),
        decision_policy: None,
        profiles,
    }
}
fn invalid(message: &str) -> StoreError {
    StoreError::Invalid(message.into())
}
fn digest<T: Serialize>(domain: &str, value: &T) -> Result<Digest> {
    canonical::digest(domain, value).map_err(|e| invalid(&e.to_string()))
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistrationRef {
    pub registration: Id,
    pub revision: Counter,
    pub catalog: CatalogReference,
}
impl RegistrationRef {
    pub fn from_registration(value: &VersionedRegistration) -> Self {
        Self {
            registration: value.registration.id.clone(),
            revision: value.revision,
            catalog: value.registration.declaration.catalog.clone(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Generation {
    pub run: Id,
    pub instance: Id,
    pub pid: u32,
    pub configuration: Digest,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Purpose {
    DiagnosticOnly,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackedBinding {
    pub id: Id,
    pub consumer: RegistrationRef,
    pub profile: Name,
    pub operating_area: Name,
    pub provider: Option<RegistrationRef>,
    pub generation: Option<Generation>,
    pub purpose: Purpose,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RunPhase {
    Assigned,
    Running,
    Completed,
}
/// Persisted history. It cannot be passed to a source/action API as live evidence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sample {
    pub reference: Id,
    pub provider: RegistrationRef,
    pub generation: Generation,
    pub request: Id,
    pub report_digest: Digest,
    pub observed_at: TimePoint,
    pub report: serde_json::Value,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub id: Id,
    pub binding: Id,
    pub phase: RunPhase,
    pub purpose: Purpose,
    pub samples: Vec<Sample>,
    pub result: Option<Id>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticResult {
    pub id: Id,
    pub run: Id,
    pub binding: Id,
    pub consumer: RegistrationRef,
    pub profile: Name,
    pub purpose: Purpose,
    pub body: serde_json::Value,
    pub digest: Digest,
    pub work_use: WorkUsePermission,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProbePurpose {
    Assign,
    Begin,
    Poll,
    Finish,
    Inspect,
}
/// Fresh call context. No public constructor or Deserialize. Stored samples never
/// become a fresh request or a binding-acceptance token.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Probe {
    id: Id,
    binding: Id,
    run: Option<Id>,
    purpose: ProbePurpose,
    provider: RegistrationRef,
    scope: UseScope,
}
impl Probe {
    pub fn provider(&self) -> &RegistrationRef {
        &self.provider
    }
    pub fn scope(&self) -> &UseScope {
        &self.scope
    }
}
#[derive(Debug, Serialize)]
pub struct ProviderObservation {
    request: Probe,
    provider: RegistrationRef,
    generation: Option<Generation>,
    assessment: ReadinessAssessment,
}
impl ProviderObservation {
    pub(crate) fn captured(
        request: &Probe,
        provider: RegistrationRef,
        generation: Option<Generation>,
        assessment: ReadinessAssessment,
    ) -> Self {
        Self {
            request: request.clone(),
            provider,
            generation,
            assessment,
        }
    }
}
pub enum SourceReply {
    Observed(Box<ProviderObservation>),
    Missing { known_lost: bool, reason: String },
    Unsupported { reason: String },
}
pub trait Source {
    fn observe(&mut self, _: &Probe) -> SourceReply {
        SourceReply::Unsupported {
            reason: "current provider observation source is not connected".into(),
        }
    }
}
pub struct NoSource;
impl Source for NoSource {}

#[derive(Clone, Debug, Serialize)]
pub struct Assessment {
    pub state: ConditionState,
    pub conditions: Vec<ConditionAssessment>,
}
fn assessment(state: ConditionState, condition: &str, reason: &str) -> Assessment {
    Assessment {
        state,
        conditions: vec![ConditionAssessment {
            name: name(condition),
            state,
            reason: reason.into(),
            origin: ReportOrigin::NotObserved,
            observed: None,
        }],
    }
}
fn met(condition: &str, reason: &str) -> Assessment {
    assessment(ConditionState::Satisfied, condition, reason)
}
fn not_met(condition: &str, reason: &str) -> Assessment {
    assessment(ConditionState::NotMet, condition, reason)
}
fn unassessed(condition: &str, reason: &str) -> Assessment {
    assessment(ConditionState::NotEvaluated, condition, reason)
}
fn unsupported(condition: &str, reason: &str) -> Assessment {
    assessment(ConditionState::Unsupported, condition, reason)
}
#[derive(Clone, Debug, Serialize)]
pub struct Inspection {
    pub binding: TrackedBinding,
    pub run: Option<Run>,
    pub result: Option<DiagnosticResult>,
    pub new_assignment: Assessment,
    pub ongoing: Assessment,
    pub result_consumption: Assessment,
    pub work_use: WorkUsePermission,
    pub limitations: Vec<&'static str>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Progress {
    pub assessment: Assessment,
    pub run: Option<Run>,
    pub result: Option<DiagnosticResult>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AcceptanceKind {
    Initial,
    Replacement,
}
#[derive(Clone, Debug, Serialize)]
pub struct AcceptanceRequest {
    pub kind: AcceptanceKind,
    pub binding: TrackedBinding,
    pub proposed_provider: Option<RegistrationRef>,
    pub proposed_generation: Option<Generation>,
    #[serde(skip)]
    decision: Option<crate::decision::Request>,
}
impl AcceptanceRequest {
    pub fn decision_request(&self) -> Option<&crate::decision::Request> {
        self.decision.as_ref()
    }
}
/// Positive acceptance carries a verified external decision, never a host grant.
/// Diagnostic tracking/execution alone is not an accepted work binding.
#[derive(Clone, Debug, Serialize)]
pub enum AcceptanceReply {
    Verified(Box<crate::decision::VerifiedDecision>),
    NotEvaluated {
        condition: Name,
        reason: String,
    },
    Denied {
        condition: Name,
        reason: String,
        decision_reference: Name,
    },
    Unsupported {
        condition: Name,
        reason: String,
    },
}
pub trait BindingJudgment {
    fn assess(&self, _: &AcceptanceRequest) -> AcceptanceReply {
        AcceptanceReply::Unsupported { condition: name("binding/consumer-judgment-provider"),
            reason: "positive consumer-side binding acceptance provider is not connected; host is not an issuer".into() }
    }
}
pub struct NoBindingJudgment;
impl BindingJudgment for NoBindingJudgment {}
#[derive(Clone, Debug, Serialize)]
pub struct AcceptanceAssessment {
    request: AcceptanceRequest,
    state: WorkUseState,
    condition: Name,
    reason: String,
    decision_reference: Option<Name>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verified_decision: Option<crate::decision::Reference>,
}
impl AcceptanceAssessment {
    pub fn request(&self) -> &AcceptanceRequest {
        &self.request
    }
    pub fn state(&self) -> WorkUseState {
        self.state
    }
    pub fn condition(&self) -> &Name {
        &self.condition
    }
    pub fn reason(&self) -> &str {
        &self.reason
    }
    pub fn decision_reference(&self) -> Option<&Name> {
        self.decision_reference.as_ref()
    }
    pub fn verified_decision(&self) -> Option<&crate::decision::Reference> {
        self.verified_decision.as_ref()
    }
}

struct Observed {
    assessment: Assessment,
    sample: Option<Sample>,
}
fn observe(
    binding: &TrackedBinding,
    run: Option<&Id>,
    declaration: &Dependency,
    purpose: ProbePurpose,
    source: &mut impl Source,
) -> Observed {
    let Dependency::Required { provider_role, .. } = declaration else {
        let a = match declaration {
            Dependency::Independent => met(
                "dependency/independent",
                "author explicitly declares this catalog-summary operation independent",
            ),
            Dependency::Undeclared => unsupported(
                "dependency/undeclared",
                "author has not declared the dependency scope",
            ),
            Dependency::Unknown { reason } => unassessed("dependency/unknown", reason),
            _ => unreachable!(),
        };
        return Observed {
            assessment: a,
            sample: None,
        };
    };
    let Some(provider) = &binding.provider else {
        return Observed {
            assessment: unassessed("dependency/provider", "no provider registration selected"),
            sample: None,
        };
    };
    let request = Probe {
        id: fresh_id(),
        binding: binding.id.clone(),
        run: run.cloned(),
        purpose,
        provider: provider.clone(),
        scope: UseScope {
            operating_area: binding.operating_area.clone(),
            role: provider_role.clone(),
        },
    };
    let rejection = match source.observe(&request) {
        SourceReply::Missing { known_lost, reason } => assessment(
            if known_lost {
                ConditionState::NotMet
            } else {
                ConditionState::NotEvaluated
            },
            "dependency/provider-observation",
            &reason,
        ),
        SourceReply::Unsupported { reason } => unsupported("dependency/provider-source", &reason),
        SourceReply::Observed(value) => {
            if value.request != request || value.provider != *provider {
                not_met(
                    "dependency/observation-context",
                    "old request or another registration/revision/catalog cannot supply this dependency",
                )
            } else if binding
                .generation
                .as_ref()
                .zip(value.generation.as_ref())
                .is_some_and(|(expected, actual)| actual != expected)
            {
                not_met(
                    "dependency/provider-generation",
                    "provider execution/configuration changed; same format cannot inherit the tracked binding",
                )
            } else if value.assessment.state() != ConditionState::Satisfied {
                Assessment {
                    state: value.assessment.state(),
                    conditions: value.assessment.conditions().to_vec(),
                }
            } else if let (Some(generation), Some((reference, report_digest, observed_at))) =
                (value.generation, value.assessment.diagnostic_basis())
            {
                let sample = Sample {
                    reference,
                    provider: provider.clone(),
                    generation,
                    request: request.id,
                    report_digest,
                    observed_at,
                    report: serde_json::to_value(&value.assessment)
                        .expect("serializable assessment"),
                };
                return Observed {
                    assessment: met(
                        "dependency/provider-report",
                        "fresh request-bound provider report satisfies the authored self-report conditions; no work permission",
                    ),
                    sample: Some(sample),
                };
            } else {
                not_met(
                    "dependency/report-evidence",
                    "satisfied metadata without current execution and captured report evidence is insufficient",
                )
            }
        }
    };
    let rejection = if rejection
        .conditions
        .iter()
        .any(|c| c.reason.trim().is_empty() || c.reason.len() > 1024)
    {
        unsupported(
            "dependency/provider-response",
            "source response lacks a bounded named explanation",
        )
    } else {
        rejection
    };
    Observed {
        assessment: rejection,
        sample: None,
    }
}
