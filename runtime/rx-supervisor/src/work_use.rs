//! Bounded non-actuating support-gap work. Diagnostic access remains separate.
//! A committed report is inert history, never a current authorization capability.
use crate::{Error, Result, decision, use_assessment::*};
use rx_domain::{canonical, types::*};
use serde::{Deserialize, Serialize};

pub(crate) fn name(value: &str) -> Name {
    Name::new(value).expect("literal")
}
pub(crate) fn digest<T: Serialize>(domain: &str, value: &T) -> Result<Digest> {
    canonical::digest(domain, value).map_err(|e| Error::Invalid(e.to_string()))
}
pub(crate) fn invalid(reason: &str) -> Error {
    Error::Invalid(format!("work-use/{reason}"))
}
pub(crate) const ROLE: &str = "work/support-gap-report";
pub(crate) const READINESS_ROLE: &str = "diagnostics/support-summary";

/// Task data only. It cannot supply a policy, readiness declaration or permission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub operation: Id,
    pub selection: Name,
    pub operating_area: Name,
    pub required_native_packages: Counter,
    pub required_support_profiles: Counter,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Comparison {
    pub observed: Counter,
    pub required: Counter,
    pub shortfall: Counter,
}
impl Comparison {
    fn compute(observed: Counter, required: Counter) -> Self {
        Self {
            observed,
            required,
            shortfall: Counter(required.0.saturating_sub(observed.0)),
        }
    }
}
/// Unauthoritative stored output. No API accepts this as permission or a prepared use.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub task: Task,
    pub registration: Id,
    pub registration_revision: Counter,
    pub instance: Id,
    pub run: Id,
    pub observed_at: TimePoint,
    pub source_digest: Digest,
    pub input_digest: Digest,
    pub readiness_semantics: Digest,
    pub native_packages: Comparison,
    pub support_profiles: Comparison,
    pub decision: decision::Reference,
    pub signature_verification: String,
    pub operating_area_policy: String,
    pub current_permission: String,
    pub physical_qualification: String,
    pub observation_origin: serde_json::Value,
}
/// Ephemeral prepared work; neither deserializable nor constructible by callers.
/// A retained positive assessment or Report cannot be substituted for this object.
///
/// ```compile_fail
/// use rx_supervisor::work_use::Prepared;
/// let _: Prepared = serde_json::from_str("{}").unwrap();
/// ```
/// ```compile_fail
/// use rx_supervisor::work_use::{Prepared, Report};
/// fn restore(history: Report) -> Prepared { history.into() }
/// ```
/// ```compile_fail
/// use rx_supervisor::{work_use::Prepared, use_assessment::WorkUseAssessment};
/// fn reuse_display(past: WorkUseAssessment) -> Prepared { past.into() }
/// ```
/// ```compile_fail
/// use rx_supervisor::{work_use::{Prepared, Task}, decision::VerifiedDecision};
/// fn forge(task: Task, proof: VerifiedDecision, context: rx_domain::types::Digest) -> Prepared {
///     Prepared { task, proof, context }
/// }
/// ```
#[derive(Debug)]
pub struct Prepared {
    pub(crate) task: Task,
    pub(crate) context: Digest,
    pub(crate) proof: decision::VerifiedDecision,
}
impl Prepared {
    pub fn decision(&self) -> &decision::VerifiedDecision {
        &self.proof
    }
    pub fn task(&self) -> &Task {
        &self.task
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Input {
    pub subject: UseSubject,
    pub configuration: Digest,
    pub native_packages: Counter,
    pub support_profiles: Counter,
    pub observed_at: TimePoint,
    pub source_digest: Digest,
    pub readiness_semantics: Digest,
    pub origins: serde_json::Value,
}
impl Input {
    pub fn context(&self, task: &Task) -> Result<serde_json::Value> {
        // Fresh observer nonce/time is recorded in the artifact, not required to
        // equal an older observation. The payload and authored meaning must match.
        Ok(
            serde_json::json!({"operation":"support-gap-report.v1","task":task,
            "subject":self.subject,"configuration":self.configuration,
            "input_digest":self.input_digest(task)?,"readiness_semantics":self.readiness_semantics}),
        )
    }
    pub fn input_digest(&self, task: &Task) -> Result<Digest> {
        digest(
            "RX-SUPPORT-GAP-INPUT-v1",
            &(
                task,
                self.native_packages,
                self.support_profiles,
                self.source_digest,
            ),
        )
    }
    pub fn report(&self, task: &Task, decision: decision::Reference) -> Result<Report> {
        Ok(Report {
            task: task.clone(),
            registration: self.subject.registration.clone(),
            registration_revision: self.subject.registration_revision,
            instance: self
                .subject
                .instance
                .clone()
                .ok_or_else(|| invalid("instance-missing"))?,
            run: self.subject.run.clone(),
            observed_at: self.observed_at.clone(),
            source_digest: self.source_digest,
            input_digest: self.input_digest(task)?,
            readiness_semantics: self.readiness_semantics,
            native_packages: Comparison::compute(
                self.native_packages,
                task.required_native_packages,
            ),
            support_profiles: Comparison::compute(
                self.support_profiles,
                task.required_support_profiles,
            ),
            decision,
            signature_verification: "EXTERNAL_SIGNATURE_AND_CONTEXT_VERIFIED_AT_LOGICAL_CUT".into(),
            operating_area_policy: "NOT_EVALUATED_BY_HOST; PRODUCTION_PROVIDER_NOT_CONNECTED"
                .into(),
            current_permission: "NONE; HISTORICAL_WORK_RESULT_ONLY".into(),
            physical_qualification: "NOT_PERFORMED".into(),
            observation_origin: self.origins.clone(),
        })
    }
}
