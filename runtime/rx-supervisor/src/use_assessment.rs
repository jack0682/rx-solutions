//! Catalog predicates over component self-reports, separate from operating-area
//! work-use judgment. This module has no supervisor/plan/store or grant issuer.
use rx_domain::{canonical, types::*};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
fn n(s: &str) -> Name {
    Name::new(s).expect("internal name")
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StatusEndpoint {
    Health,
    SupportSummary,
}
impl StatusEndpoint {
    pub fn path(self) -> &'static str {
        match self {
            Self::Health => "/health",
            Self::SupportSummary => "/api/v1/solution/support",
        }
    }
}
/// Provenance declared by the author, not an independent validation of the field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReportOrigin {
    NotObserved,
    ReleaseDeclaration,
    StartupAuditDerived,
    StartupCatalogDerived,
    InstanceEnvironment,
    OperatingAreaReport,
    VerifiedExternalDecision,
    ProviderUnavailable,
}
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExpectedValue {
    Text(String),
    Unsigned(u64),
}
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "condition", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReadinessCondition {
    InstanceMatches,
    Equals {
        field: Name,
        expected: ExpectedValue,
        origin: ReportOrigin,
    },
    Unsigned {
        field: Name,
        origin: ReportOrigin,
    },
    Unsupported {
        reason: String,
    },
}
#[derive(Clone, Debug, Serialize)]
pub struct ReadinessProfile {
    pub endpoint: StatusEndpoint,
    pub conditions: BTreeMap<Name, ReadinessCondition>,
}
/// Profile keys are the intended-use/role names; a caller cannot substitute a
/// weaker profile for the requested role. No site deserializer is provided.
#[derive(Clone, Debug, Serialize)]
pub struct ReadinessContract(pub BTreeMap<Name, ReadinessProfile>);
impl ReadinessContract {
    pub(crate) fn validate(&self) -> std::result::Result<(), String> {
        if self.0.is_empty() || self.0.len() > 8 {
            return Err("readiness contract requires 1..8 named use profiles".into());
        }
        for profile in self.0.values() {
            if profile.conditions.is_empty()
                || profile.conditions.len() > 32
                || profile
                    .conditions
                    .values()
                    .filter(|v| matches!(v, ReadinessCondition::InstanceMatches))
                    .count()
                    != 1
            {
                return Err("readiness profile requires 1..32 named conditions and exactly one instance correlation".into());
            }
            for condition in profile.conditions.values() {
                match condition {
                    ReadinessCondition::Equals { origin, .. }
                    | ReadinessCondition::Unsigned { origin, .. }
                        if !matches!(
                            origin,
                            ReportOrigin::ReleaseDeclaration
                                | ReportOrigin::StartupAuditDerived
                                | ReportOrigin::StartupCatalogDerived
                        ) =>
                    {
                        return Err("component field provenance cannot be operating-area authority or missing evidence".into());
                    }
                    ReadinessCondition::Equals {
                        expected: ExpectedValue::Text(v),
                        ..
                    } if v.is_empty() || v.len() > 256 => {
                        return Err("bounded nonempty readiness value required".into());
                    }
                    ReadinessCondition::Unsupported { reason }
                        if reason.trim().is_empty() || reason.len() > 1024 =>
                    {
                        return Err("named unsupported condition requires a reason".into());
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConditionState {
    NotEvaluated,
    NotMet,
    Unsupported,
    Satisfied,
}
#[derive(Clone, Debug, Serialize)]
pub struct ConditionAssessment {
    pub name: Name,
    pub state: ConditionState,
    pub reason: String,
    pub origin: ReportOrigin,
    pub observed: Option<Value>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UseScope {
    pub operating_area: Name,
    pub role: Name,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UseSubject {
    pub(crate) registration: Id,
    pub(crate) registration_revision: Counter,
    pub(crate) program: Name,
    pub(crate) catalog_digest: Digest,
    pub(crate) run: Id,
    pub(crate) selection: Name,
    pub(crate) instance: Option<Id>,
    pub(crate) pid: Option<u32>,
}
impl UseSubject {
    pub fn instance(&self) -> Option<&Id> {
        self.instance.as_ref()
    }
    pub fn registration(&self) -> &Id {
        &self.registration
    }
}
/// Per-assessment nonce and complete current subject. Not site-constructible or
/// deserializable; an old response cannot be rebound to a new assessment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StatusObservationRequest {
    id: Id,
    subject: UseSubject,
    scope: UseScope,
    endpoint: StatusEndpoint,
}
impl StatusObservationRequest {
    pub(crate) fn new(subject: UseSubject, scope: UseScope, endpoint: StatusEndpoint) -> Self {
        Self {
            id: Id::new(uuid::Uuid::new_v4().to_string()).expect("UUID"),
            subject,
            scope,
            endpoint,
        }
    }
    pub fn instance(&self) -> Option<&Id> {
        self.subject.instance.as_ref()
    }
    pub fn pid(&self) -> Option<u32> {
        self.subject.pid
    }
    pub fn endpoint(&self) -> StatusEndpoint {
        self.endpoint
    }
    pub(crate) fn subject(&self) -> &UseSubject {
        &self.subject
    }
}
/// A response read in an owned-child context, not proof of arbitrary physical
/// truth. Private fields and no Deserialize prevent metadata/receipt promotion.
#[derive(Debug, Serialize)]
pub struct StatusObservation {
    request: StatusObservationRequest,
    reported: Value,
    observed_at: TimePoint,
    digest: Digest,
}
impl StatusObservation {
    pub(crate) fn captured(
        request: &StatusObservationRequest,
        reported: Value,
    ) -> std::result::Result<Self, String> {
        let ticks = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos(),
        )
        .map_err(|e| e.to_string())?;
        let digest =
            canonical::digest("RX-FUNCTION-REPORT-v1", &reported).map_err(|e| e.to_string())?;
        Ok(Self {
            request: request.clone(),
            reported,
            observed_at: TimePoint {
                clock_id: "unix-utc-ns".into(),
                ticks_ns: Counter(ticks),
            },
            digest,
        })
    }
    pub fn request(&self) -> &StatusObservationRequest {
        &self.request
    }
}
#[derive(Debug)]
pub enum StatusObservationResult {
    Observed(Box<StatusObservation>),
    NotEvaluated { condition: Name, reason: String },
    NotMet { condition: Name, reason: String },
    Unsupported { condition: Name, reason: String },
}
#[derive(Clone, Debug, Serialize)]
struct ReportEvidence {
    request_id: Id,
    endpoint: StatusEndpoint,
    observed_at: TimePoint,
    payload_digest: Digest,
    basis: &'static str,
}
#[derive(Clone, Debug, Serialize)]
pub struct ReadinessAssessment {
    state: ConditionState,
    scope: Option<UseScope>,
    subject: Option<UseSubject>,
    conditions: Vec<ConditionAssessment>,
    evidence: Option<ReportEvidence>,
    meaning: &'static str,
}
impl ReadinessAssessment {
    pub(crate) fn support_gap_input(
        &self,
        profile: &ReadinessProfile,
        configuration: Digest,
    ) -> crate::Result<crate::work_use::Input> {
        use crate::work_use::{digest, invalid};
        if self.state != ConditionState::Satisfied {
            return Err(invalid("readiness-not-satisfied"));
        }
        let evidence = self
            .evidence
            .as_ref()
            .ok_or_else(|| invalid("fresh-report-missing"))?;
        let count = |field: &str| -> crate::Result<(Counter, ReportOrigin)> {
            let candidates: Vec<_> = profile
                .conditions
                .iter()
                .filter_map(|(name, condition)| {
                    if let ReadinessCondition::Unsigned {
                        field: authored,
                        origin,
                    } = condition
                        && authored.as_str() == field
                    {
                        Some((name, *origin))
                    } else {
                        None
                    }
                })
                .collect();
            if candidates.len() != 1 {
                return Err(invalid("report-count-not-authored"));
            }
            let (name, origin) = candidates[0];
            let value = self
                .conditions
                .iter()
                .find(|c| c.name == *name && c.state == ConditionState::Satisfied)
                .and_then(|c| c.observed.as_ref())
                .and_then(Value::as_u64)
                .ok_or_else(|| invalid("report-count-not-observed"))?;
            Ok((Counter(value), origin))
        };
        let (native_packages, native_origin) = count("native_packages")?;
        let (support_profiles, support_origin) = count("support_profiles")?;
        Ok(crate::work_use::Input {
            subject: self
                .subject
                .clone()
                .ok_or_else(|| invalid("current-subject-missing"))?,
            configuration,
            native_packages,
            support_profiles,
            observed_at: evidence.observed_at.clone(),
            source_digest: evidence.payload_digest,
            readiness_semantics: digest(
                "RX-WORK-READINESS-v1",
                &(profile, &self.scope, self.state, &self.conditions),
            )?,
            origins: serde_json::json!({"basis":"COMPONENT_SELF_REPORT", "native_packages":native_origin,"support_profiles":support_origin}),
        })
    }
    pub(crate) fn diagnostic_basis(&self) -> Option<(Id, Digest, TimePoint)> {
        self.evidence.as_ref().map(|e| {
            (
                e.request_id.clone(),
                e.payload_digest,
                e.observed_at.clone(),
            )
        })
    }
    pub fn state(&self) -> ConditionState {
        self.state
    }
    pub fn conditions(&self) -> &[ConditionAssessment] {
        &self.conditions
    }
    pub(crate) fn not_evaluated() -> Self {
        Self::unobserved(
            None,
            None,
            ConditionState::NotEvaluated,
            n("readiness/assessment"),
            "no current-owner assessment has been requested".into(),
        )
    }
    pub(crate) fn unobserved(
        scope: Option<UseScope>,
        subject: Option<UseSubject>,
        state: ConditionState,
        name: Name,
        reason: String,
    ) -> Self {
        Self {
            state,
            scope,
            subject,
            conditions: vec![ConditionAssessment {
                name,
                state,
                reason,
                origin: ReportOrigin::NotObserved,
                observed: None,
            }],
            evidence: None,
            meaning: "catalog conditions over a component self-report; not independent functional or physical qualification",
        }
    }
    pub(crate) fn evaluate(
        request: &StatusObservationRequest,
        profile: &ReadinessProfile,
        result: StatusObservationResult,
    ) -> Self {
        let observation = match result {
            StatusObservationResult::Observed(o) => o,
            other => {
                let (state, condition, reason) = match other {
                    StatusObservationResult::NotEvaluated { condition, reason } => {
                        (ConditionState::NotEvaluated, condition, reason)
                    }
                    StatusObservationResult::NotMet { condition, reason } => {
                        (ConditionState::NotMet, condition, reason)
                    }
                    StatusObservationResult::Unsupported { condition, reason } => {
                        (ConditionState::Unsupported, condition, reason)
                    }
                    _ => unreachable!(),
                };
                let mut report = Self::unobserved(
                    Some(request.scope.clone()),
                    Some(request.subject.clone()),
                    state,
                    condition,
                    reason,
                );
                report
                    .conditions
                    .extend(profile.conditions.keys().map(|name| ConditionAssessment {
                        name: name.clone(),
                        state: ConditionState::NotEvaluated,
                        reason: "no applicable response; authored condition not evaluated".into(),
                        origin: ReportOrigin::NotObserved,
                        observed: None,
                    }));
                return report;
            }
        };
        if observation.request != *request {
            return Self::unobserved(
                Some(request.scope.clone()),
                Some(request.subject.clone()),
                ConditionState::NotMet,
                n("report/assessment-binding"),
                "response belongs to another assessment request/instance/scope".into(),
            );
        }
        let instance_ok = request
            .instance()
            .is_some_and(|i| observation.reported["supervisor_instance"] == i.as_str());
        let mut conditions = vec![];
        for (name, condition) in &profile.conditions {
            let (state, reason, origin, observed) = if matches!(
                condition,
                ReadinessCondition::InstanceMatches
            ) {
                (
                    if instance_ok {
                        ConditionState::Satisfied
                    } else {
                        ConditionState::NotMet
                    },
                    "compare reported RX_PROCESS_INSTANCE_ID with the current execution instance"
                        .into(),
                    ReportOrigin::InstanceEnvironment,
                    observation.reported.get("supervisor_instance").cloned(),
                )
            } else if !instance_ok {
                (
                    ConditionState::NotEvaluated,
                    "response instance differs; do not assess its other fields".into(),
                    ReportOrigin::NotObserved,
                    None,
                )
            } else {
                match condition {
                    ReadinessCondition::Equals {
                        field,
                        expected,
                        origin,
                    } => {
                        let actual = observation.reported.get(field.as_str()).cloned();
                        let matches = match expected {
                            ExpectedValue::Text(v) => {
                                actual.as_ref().and_then(Value::as_str) == Some(v)
                            }
                            ExpectedValue::Unsigned(v) => {
                                actual.as_ref().and_then(Value::as_u64) == Some(*v)
                            }
                        };
                        (
                            if matches {
                                ConditionState::Satisfied
                            } else {
                                ConditionState::NotMet
                            },
                            format!(
                                "component-reported {field} compared with authored {expected:?}; provenance {origin:?}"
                            ),
                            *origin,
                            actual,
                        )
                    }
                    ReadinessCondition::Unsigned { field, origin } => {
                        let actual = observation.reported.get(field.as_str()).cloned();
                        let matches = actual.as_ref().and_then(Value::as_u64).is_some();
                        (
                            if matches {
                                ConditionState::Satisfied
                            } else {
                                ConditionState::NotMet
                            },
                            format!(
                                "component-reported {field} must be a nonnegative count; not physical validation"
                            ),
                            *origin,
                            actual,
                        )
                    }
                    ReadinessCondition::Unsupported { reason } => (
                        ConditionState::Unsupported,
                        reason.clone(),
                        ReportOrigin::NotObserved,
                        None,
                    ),
                    _ => unreachable!(),
                }
            };
            conditions.push(ConditionAssessment {
                name: name.clone(),
                state,
                reason,
                origin,
                observed,
            });
        }
        let state = [
            ConditionState::NotMet,
            ConditionState::Unsupported,
            ConditionState::NotEvaluated,
        ]
        .into_iter()
        .find(|s| conditions.iter().any(|c| c.state == *s))
        .unwrap_or(ConditionState::Satisfied);
        Self {
            state,
            scope: Some(request.scope.clone()),
            subject: Some(request.subject.clone()),
            conditions,
            evidence: Some(ReportEvidence {
                request_id: request.id.clone(),
                endpoint: request.endpoint,
                observed_at: observation.observed_at,
                payload_digest: observation.digest,
                basis: "COMPONENT_SELF_REPORT",
            }),
            meaning: "assessment compares authored self-report conditions; even SATISFIED is not proof of functional operation, calibration, physical readiness or work-use permission",
        }
    }
}
/// Display of a past assessment, not a credential. Only a proof-bearing reply
/// can produce Verified in the receiving API. No Deserialize or grant issuer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkUseState {
    Verified,
    NotEvaluated,
    Denied,
    Unsupported,
}
#[derive(Clone, Debug, Serialize)]
pub struct WorkUseRequest {
    scope: UseScope,
    subject: UseSubject,
    readiness: ReadinessAssessment,
    #[serde(skip)]
    decision: Option<crate::decision::Request>,
}
impl WorkUseRequest {
    pub fn decision_request(&self) -> Option<&crate::decision::Request> {
        self.decision.as_ref()
    }
    pub(crate) fn new(
        scope: UseScope,
        subject: UseSubject,
        readiness: ReadinessAssessment,
        decision: Option<crate::decision::Request>,
    ) -> Self {
        Self {
            scope,
            subject,
            readiness,
            decision,
        }
    }
    pub fn scope(&self) -> &UseScope {
        &self.scope
    }
    pub fn subject(&self) -> &UseSubject {
        &self.subject
    }
    pub fn readiness(&self) -> &ReadinessAssessment {
        &self.readiness
    }
}
/// A reported judgment or externally verified proof, never a host-issued grant.
/// The integration is responsible for the truth/provenance of a reported denial.
pub enum WorkUseReply {
    Verified(Box<crate::decision::VerifiedDecision>),
    NotEvaluated {
        conditions: BTreeMap<Name, String>,
    },
    Unsupported {
        conditions: BTreeMap<Name, String>,
    },
    Denied {
        decision_reference: Name,
        conditions: BTreeMap<Name, String>,
    },
}
/// Work-use authority belongs to operating-area task judgment. Positive provider
/// connection is absent by default. Verification uses author-pinned policy;
/// this port cannot construct a verified decision without external signed bytes.
pub trait WorkUsePort {
    fn assess(&self, _request: &WorkUseRequest) -> WorkUseReply {
        WorkUseReply::Unsupported{conditions:[(n("work-use/operating-area-provider"),"operating-area positive work-use provider is not connected; the host is not an issuer".into())].into()}
    }
}
#[derive(Default)]
pub struct NoWorkUseProvider;
impl WorkUsePort for NoWorkUseProvider {}
#[derive(Clone, Debug, Serialize)]
pub struct WorkUseAssessment {
    state: WorkUseState,
    scope: Option<UseScope>,
    subject: Option<UseSubject>,
    conditions: Vec<ConditionAssessment>,
    decision_reference: Option<Name>,
    positive_provider: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    verified_decision: Option<crate::decision::Reference>,
}
impl WorkUseAssessment {
    pub fn verified_decision(&self) -> Option<&crate::decision::Reference> {
        self.verified_decision.as_ref()
    }
    pub fn state(&self) -> WorkUseState {
        self.state
    }
    pub fn conditions(&self) -> &[ConditionAssessment] {
        &self.conditions
    }
    pub(crate) fn not_evaluated() -> Self {
        Self {
            state: WorkUseState::NotEvaluated,
            scope: None,
            subject: None,
            conditions: vec![ConditionAssessment {
                name: n("work-use/assessment"),
                state: ConditionState::NotEvaluated,
                reason: "no scoped operating-area work-use assessment requested".into(),
                origin: ReportOrigin::NotObserved,
                observed: None,
            }],
            decision_reference: None,
            verified_decision: None,
            positive_provider: "UNSUPPORTED: operating-area positive permission provider connection",
        }
    }
    pub(crate) fn assessed(request: &WorkUseRequest, reply: WorkUseReply) -> Self {
        if let WorkUseReply::Verified(proof) = reply {
            let checked = request
                .decision
                .as_ref()
                .ok_or(crate::decision::Failure::Unconfigured)
                .and_then(|r| r.check(&proof));
            let (state, condition_state, reason, reference) = match checked {
                Ok(reference) => (WorkUseState::Verified, ConditionState::Satisfied,
                    "external issuer key possession and exact current context verified; not correctness, readiness, binding acceptance or physical qualification".into(), Some(reference)),
                Err(error) => (WorkUseState::Denied, ConditionState::NotMet, error.to_string(), None),
            };
            return Self {
                state,
                scope: Some(request.scope.clone()),
                subject: Some(request.subject.clone()),
                conditions: vec![ConditionAssessment {
                    name: n("work-use/external-decision"),
                    state: condition_state,
                    reason,
                    origin: ReportOrigin::VerifiedExternalDecision,
                    observed: None,
                }],
                decision_reference: reference
                    .as_ref()
                    .map(|r| n(&format!("decision/{}", r.decision))),
                positive_provider: "external signed-decision verification only; operating-area service connection unsupported",
                verified_decision: reference,
            };
        }
        let (state, decision, conditions, origin) = match reply {
            WorkUseReply::Verified(_) => unreachable!("handled above"),
            WorkUseReply::NotEvaluated { conditions } => (
                WorkUseState::NotEvaluated,
                None,
                conditions,
                ReportOrigin::NotObserved,
            ),
            WorkUseReply::Unsupported { conditions } => (
                WorkUseState::Unsupported,
                None,
                conditions,
                ReportOrigin::ProviderUnavailable,
            ),
            WorkUseReply::Denied {
                decision_reference,
                conditions,
            } => (
                WorkUseState::Denied,
                Some(decision_reference),
                conditions,
                ReportOrigin::OperatingAreaReport,
            ),
        };
        let (mut state, mut decision, mut origin, mut conditions) =
            (state, decision, origin, conditions);
        if conditions.is_empty()
            || conditions.len() > 32
            || conditions
                .values()
                .any(|v| v.trim().is_empty() || v.len() > 1024)
        {
            state = WorkUseState::Unsupported;
            decision = None;
            origin = ReportOrigin::ProviderUnavailable;
            conditions = [(
                n("work-use/provider-response"),
                "provider did not supply bounded named reasons; no permission is inferred".into(),
            )]
            .into();
        }
        let condition_state = match state {
            WorkUseState::Verified => ConditionState::Satisfied,
            WorkUseState::NotEvaluated => ConditionState::NotEvaluated,
            WorkUseState::Denied => ConditionState::NotMet,
            WorkUseState::Unsupported => ConditionState::Unsupported,
        };
        Self {
            state,
            scope: Some(request.scope.clone()),
            subject: Some(request.subject.clone()),
            conditions: conditions
                .into_iter()
                .map(|(name, reason)| ConditionAssessment {
                    name,
                    state: condition_state,
                    reason,
                    origin,
                    observed: None,
                })
                .collect(),
            decision_reference: decision,
            verified_decision: None,
            positive_provider: "UNSUPPORTED: operating-area positive permission provider connection",
        }
    }
}
