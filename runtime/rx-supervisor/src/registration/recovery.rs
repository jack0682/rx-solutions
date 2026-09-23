//! Explicit local software recovery. Historical execution reports are never rewritten.
use super::*;

const DISPOSITION: &str = "rx.component-recovery-disposition.v1";
const RESUME: &str = "rx.component-recovery-resume.v1";
fn disposition_key(component: &Id, instance: &Id) -> Name {
    name(&format!("components/disposition/{component}/{instance}"))
}
fn resume_key(component: &Id, disposition: &Id) -> Name {
    name(&format!("components/resume/{component}/{disposition}"))
}
fn invalid(message: &str) -> StoreError {
    StoreError::Invalid(message.into())
}

/// Ephemeral evidence minted only after the real OS owner observed and retired
/// its non-actuating Child handle. There is deliberately no Deserialize or public
/// constructor. A persisted reference/digest is not this capability.
#[derive(Debug, Serialize)]
pub struct OwnedExit {
    reference: Id,
    instance: Id,
    pid: u32,
    exit_code: Option<i32>,
    observed_at: TimePoint,
}
impl OwnedExit {
    pub(crate) fn observed(
        instance: Id,
        pid: u32,
        exit_code: Option<i32>,
        observed_at: TimePoint,
    ) -> Self {
        Self {
            reference: fresh_id(),
            instance,
            pid,
            exit_code,
            observed_at,
        }
    }
    pub fn instance(&self) -> &Id {
        &self.instance
    }
    pub fn observed_at(&self) -> &TimePoint {
        &self.observed_at
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationRef {
    pub registration: Id,
    pub instance: Id,
    pub revision: Counter,
    pub digest: Digest,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryReport {
    pub actor: Name,
    pub scope: Name,
    pub observed_at: TimePoint,
    pub procedure: Name,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DispositionKind {
    ConfirmedClosure,
    UnableToResolve,
}
#[derive(Debug, Serialize)]
pub enum WeakObservation {
    Timeout,
    CurrentIdle,
    ProcessAbsent,
    ElapsedTime,
}
#[derive(Debug, Serialize)]
pub enum RecoveryEvidence {
    None,
    ObservationOnly {
        report: RecoveryReport,
        observation: WeakObservation,
    },
    OwnedChildExit {
        report: RecoveryReport,
        witness: OwnedExit,
    },
    Investigation {
        report: RecoveryReport,
        finding: String,
    },
}
#[derive(Debug, Serialize)]
pub struct DispositionRequest {
    pub target: ObservationRef,
    pub kind: DispositionKind,
    pub evidence: RecoveryEvidence,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "basis",
    rename_all = "SCREAMING_SNAKE_CASE",
    deny_unknown_fields
)]
pub enum EvidenceReference {
    OwnedChildExit { reference: Id, digest: Digest },
    Investigation { finding: String },
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PastOutcome {
    Unresolved,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ResourceRecovery {
    NotClaimed,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkUsePermission {
    Unsupported,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Disposition {
    pub id: Id,
    pub target: ObservationRef,
    pub kind: DispositionKind,
    pub report: RecoveryReport,
    pub evidence: EvidenceReference,
    pub past_outcome: PastOutcome,
    pub resource_recovery: ResourceRecovery,
    pub work_use_permission: WorkUsePermission,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResumeRequest {
    pub id: Id,
    pub disposition: Id,
    pub actor: Name,
    pub requested_at: TimePoint,
    pub next_run: Id,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResumeRecord {
    pub request: ResumeRequest,
    pub registration_revision: Counter,
    pub catalog: CatalogReference,
    pub consumed_by: Option<Id>,
}
/// Not deserializable. A lost, unconsumed permit can only be reissued through
/// the same explicit request and a fresh authority check; consumption is durable.
#[derive(Clone, Debug, Serialize)]
pub struct ResumePermit {
    registration: Id,
    request: ResumeRequest,
}
/// Bind these decisions to authenticated local policy. Report strings alone do
/// not authenticate an actor. Implementations must be pure (no OS/network calls
/// in repository transactions). This port cannot certify arbitrary report truth.
/// Functional work permission and resource recovery are outside this authority.
pub trait RecoveryAuthority {
    fn may_dispose(&self, _request: &DispositionRequest) -> bool {
        false
    }
    fn may_resume(&self, _disposition: &Disposition, _request: &ResumeRequest) -> bool {
        false
    }
}
#[derive(Default)]
pub struct DenyRecovery;
impl RecoveryAuthority for DenyRecovery {}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RecoveryGate {
    NoRecoveryBlock,
    BlockedUnresolved,
    ExplicitResumeRequired,
    ResumeAuthorizedForNamedRun,
}
#[derive(Clone, Debug, Serialize)]
pub struct RecoveryView {
    pub dispositions: Vec<Disposition>,
    pub resume_requests: Vec<ResumeRecord>,
    /// Only the recovery gate, never lifecycle/F1/functional work permission.
    pub new_execution: RecoveryGate,
    pub limitations: Vec<&'static str>,
}
fn valid_time(time: &TimePoint) -> bool {
    !time.clock_id.trim().is_empty() && time.clock_id.len() <= 128 && time.ticks_ns.0 > 0
}
fn reference(row: &Record, execution: &Execution) -> Result<ObservationRef> {
    Ok(ObservationRef {
        registration: execution.binding.registration.clone(),
        instance: execution.binding.instance.clone(),
        revision: row.revision,
        digest: canonical::digest("RX-RECOVERY-OBSERVATION-v1", execution)
            .map_err(|e| StoreError::Integrity(e.to_string()))?,
    })
}
fn target(tx: &mut dyn Transaction, expected: &ObservationRef) -> Result<Execution> {
    let row = tx
        .get(&execution_key(&expected.registration, &expected.instance))?
        .ok_or_else(|| invalid("recovery target missing"))?;
    let execution: Execution = decode(&row, EXECUTION)?;
    if reference(&row, &execution)? != *expected {
        return Err(StoreError::RevisionConflict(
            "original observation revision/digest differs".into(),
        ));
    }
    Ok(execution)
}
fn all_dispositions(tx: &mut dyn Transaction, component: &Id) -> Result<Vec<Disposition>> {
    tx.scan(&format!("components/disposition/{component}/"))?
        .iter()
        .map(|r| {
            let d: Disposition = decode(r, DISPOSITION)?;
            if d.target.registration != *component
                || r.key != disposition_key(component, &d.target.instance)
                || !matches!(
                    (&d.kind, &d.evidence),
                    (
                        DispositionKind::ConfirmedClosure,
                        EvidenceReference::OwnedChildExit { .. }
                    ) | (
                        DispositionKind::UnableToResolve,
                        EvidenceReference::Investigation { .. }
                    )
                )
            {
                return Err(StoreError::Integrity(
                    "disposition identity/evidence differs".into(),
                ));
            }
            target(tx, &d.target)?;
            Ok(d)
        })
        .collect()
}
fn resumes(tx: &mut dyn Transaction, component: &Id) -> Result<Vec<ResumeRecord>> {
    tx.scan(&format!("components/resume/{component}/"))?
        .iter()
        .map(|r| {
            let value: ResumeRecord = decode(r, RESUME)?;
            if r.key != resume_key(component, &value.request.disposition) {
                return Err(StoreError::Integrity("resume identity differs".into()));
            }
            Ok(value)
        })
        .collect()
}
fn unresolved(e: &Execution) -> bool {
    !matches!(
        e.last_observed.state,
        ExecutionState::Exited | ExecutionState::NotStarted
    )
}
fn consumed(tx: &mut dyn Transaction, d: &Disposition, r: &ResumeRecord) -> Result<bool> {
    if d.kind != DispositionKind::ConfirmedClosure || r.request.disposition != d.id {
        return Ok(false);
    }
    let Some(instance) = &r.consumed_by else {
        return Ok(false);
    };
    let row = tx
        .get(&execution_key(&d.target.registration, instance))?
        .ok_or_else(|| StoreError::Integrity("resume consumption has no execution".into()))?;
    let execution: Execution = decode(&row, EXECUTION)?;
    if execution.binding.instance != *instance
        || *instance == d.target.instance
        || execution.binding.run != r.request.next_run
        || execution.binding.catalog != r.catalog
        || execution.binding.registration_revision != r.registration_revision
    {
        return Err(StoreError::Integrity(
            "consumed resume does not match assigned execution".into(),
        ));
    }
    Ok(true)
}
pub(super) fn view(
    tx: &mut dyn Transaction,
    component: &Id,
    executions: &[Execution],
) -> Result<RecoveryView> {
    let ds = all_dispositions(tx, component)?;
    let rs = resumes(tx, component)?;
    let mut gate = RecoveryGate::NoRecoveryBlock;
    for e in executions.iter().filter(|e| unresolved(e)) {
        let Some(d) = ds.iter().find(|d| {
            d.target.instance == e.binding.instance && d.kind == DispositionKind::ConfirmedClosure
        }) else {
            gate = RecoveryGate::BlockedUnresolved;
            break;
        };
        let r = rs.iter().find(|r| r.request.disposition == d.id);
        if let Some(r) = r {
            if !consumed(tx, d, r)? {
                gate = RecoveryGate::ResumeAuthorizedForNamedRun;
            }
        } else {
            gate = RecoveryGate::ExplicitResumeRequired;
        }
    }
    Ok(RecoveryView {
        dispositions: ds,
        resume_requests: rs,
        new_execution: gate,
        limitations: vec![
            "external investigation provider after total manager-process/Child-handle loss is unsupported",
            "confirmed closure without the original saved PID binding is unsupported",
            "functional readiness and work-use permission unsupported",
            "resource recovery, F1 receipt restoration and physical outcome confirmation not claimed",
        ],
    })
}
pub(super) fn frozen(tx: &mut dyn Transaction, component: &Id, instance: &Id) -> Result<bool> {
    Ok(tx.get(&disposition_key(component, instance))?.is_some())
}
pub(super) fn admit(
    tx: &mut dyn Transaction,
    binding: &Binding,
    permit: Option<&ResumePermit>,
) -> Result<()> {
    let ds = all_dispositions(tx, &binding.registration)?;
    let rs = resumes(tx, &binding.registration)?;
    let mut selected = None;
    if let Some(permit) = permit {
        if permit.registration != binding.registration || permit.request.next_run != binding.run {
            return Err(invalid("resume permit targets another component/run"));
        }
        let d = ds
            .iter()
            .find(|d| d.id == permit.request.disposition)
            .ok_or_else(|| invalid("resume disposition absent"))?;
        let r = rs
            .iter()
            .find(|r| r.request == permit.request)
            .ok_or_else(|| invalid("explicit resume request absent or changed"))?;
        if d.kind != DispositionKind::ConfirmedClosure || r.consumed_by.is_some() {
            return Err(invalid(
                "confirmed disposition resume already consumed or unavailable",
            ));
        }
        if r.registration_revision != binding.registration_revision || r.catalog != binding.catalog
        {
            return Err(invalid("resume declaration/catalog changed"));
        }
        if binding.instance == d.target.instance
            || binding.run == target(tx, &d.target)?.binding.run
        {
            return Err(invalid("recovery requires a new run and instance"));
        }
        selected = Some((d, r));
    }
    for e in executions(tx, &binding.registration)?
        .iter()
        .filter(|e| unresolved(e))
    {
        let d = ds.iter().find(|d| {
            d.target.instance == e.binding.instance && d.kind == DispositionKind::ConfirmedClosure
        });
        let Some(d) = d else {
            return Err(invalid(
                "previous assignment unresolved; explicit confirmed disposition required",
            ));
        };
        if selected.is_some_and(|(chosen, _)| chosen.id == d.id) {
            continue;
        }
        let Some(r) = rs.iter().find(|r| r.request.disposition == d.id) else {
            return Err(invalid(
                "confirmed disposition alone does not permit assignment; explicit resume request required",
            ));
        };
        if !consumed(tx, d, r)? {
            return Err(invalid(
                "explicit resume permit required for this assignment",
            ));
        }
    }
    if let Some((d, r)) = selected {
        let key = resume_key(&binding.registration, &d.id);
        let row = tx
            .get(&key)?
            .ok_or_else(|| invalid("resume record missing"))?;
        let mut r = r.clone();
        r.consumed_by = Some(binding.instance.clone());
        save(
            tx,
            &binding.registration,
            &key,
            Some(row.revision),
            &document(RESUME, &r)?,
            "resume-consumed",
        )?;
    }
    Ok(())
}
impl<R: Repository> Registry<R> {
    pub fn recovery_target(&mut self, component: &Id, instance: &Id) -> Result<ObservationRef> {
        self.repository.transact(|tx| {
            load(tx, component)?;
            let row = tx
                .get(&execution_key(component, instance))?
                .ok_or_else(|| invalid("execution absent"))?;
            let e: Execution = decode(&row, EXECUTION)?;
            reference(&row, &e)
        })
    }
    pub fn dispose(
        &mut self,
        request: &DispositionRequest,
        authority: &impl RecoveryAuthority,
    ) -> Result<Disposition> {
        let (report, evidence) = match (&request.kind, &request.evidence) {
            (
                DispositionKind::ConfirmedClosure,
                RecoveryEvidence::OwnedChildExit { report, witness },
            ) => {
                if report.scope.as_str() != "host/direct-child-exit"
                    || report.observed_at != witness.observed_at
                    || witness.instance != request.target.instance
                {
                    return Err(invalid("owned exit witness target/scope/time differs"));
                }
                let digest = canonical::digest("RX-OWNED-EXIT-v1", witness)
                    .map_err(|e| invalid(&e.to_string()))?;
                (
                    report.clone(),
                    EvidenceReference::OwnedChildExit {
                        reference: witness.reference.clone(),
                        digest,
                    },
                )
            }
            (
                DispositionKind::UnableToResolve,
                RecoveryEvidence::Investigation { report, finding },
            ) if report.scope.as_str() == "host/execution-investigation"
                && !finding.trim().is_empty()
                && finding.len() <= 2048 =>
            {
                (
                    report.clone(),
                    EvidenceReference::Investigation {
                        finding: finding.clone(),
                    },
                )
            }
            _ => {
                return Err(invalid(
                    "disposition requires supported positive evidence or an explicit unresolved investigation; timeout/idle/absence/time alone is insufficient",
                ));
            }
        };
        if !valid_time(&report.observed_at) {
            return Err(invalid("evidence observation time required"));
        }
        if !authority.may_dispose(request) {
            return Err(invalid("recovery disposition authority denied"));
        }
        self.repository.transact(|tx| {
            load(tx, &request.target.registration)?;
            let e = target(tx, &request.target)?;
            if !matches!(
                e.last_observed.state,
                ExecutionState::Unknown | ExecutionState::Assigned
            ) {
                return Err(invalid("recovery target is not an unconfirmed assignment"));
            }
            if let RecoveryEvidence::OwnedChildExit { witness, .. } = &request.evidence
                && e.last_observed.pid != Some(witness.pid)
            {
                return Err(invalid(
                    "owned exit witness requires matching original instance/PID binding; absence is not closure evidence",
                ));
            }
            if !authority.may_dispose(request) {
                return Err(invalid("recovery disposition authority changed"));
            }
            let d = Disposition {
                id: fresh_id(),
                target: request.target.clone(),
                kind: request.kind,
                report,
                evidence,
                past_outcome: PastOutcome::Unresolved,
                resource_recovery: ResourceRecovery::NotClaimed,
                work_use_permission: WorkUsePermission::Unsupported,
            };
            save(
                tx,
                &request.target.registration,
                &disposition_key(&request.target.registration, &request.target.instance),
                None,
                &document(DISPOSITION, &d)?,
                "recovery-disposition-recorded",
            )?;
            Ok(d)
        })
    }
    pub fn request_resume(
        &mut self,
        component: &Id,
        request: ResumeRequest,
        authority: &impl RecoveryAuthority,
    ) -> Result<ResumePermit> {
        if !valid_time(&request.requested_at) {
            return Err(invalid("explicit resume request time required"));
        }
        self.repository.transact(|tx| {
            let current = load(tx, component)?;
            if current.registration.state != RegistrationState::Accepted {
                return Err(invalid("retired registration cannot resume"));
            }
            let ds = all_dispositions(tx, component)?;
            let d = ds
                .iter()
                .find(|d| d.id == request.disposition)
                .ok_or_else(|| invalid("disposition not found"))?;
            if d.kind != DispositionKind::ConfirmedClosure {
                return Err(invalid(
                    "unable-to-resolve disposition does not enable new execution",
                ));
            }
            let old = target(tx, &d.target)?;
            if old.binding.run == request.next_run
                || old.binding.catalog != current.registration.declaration.catalog
            {
                return Err(invalid(
                    "resume requires new run and unchanged reviewed catalog",
                ));
            }
            if !authority.may_resume(d, &request) {
                return Err(invalid("explicit resume authority denied"));
            }
            let key = resume_key(component, &d.id);
            if let Some(row) = tx.get(&key)? {
                let previous: ResumeRecord = decode(&row, RESUME)?;
                if previous.consumed_by.is_some() || previous.request != request {
                    return Err(invalid(
                        "disposition already claimed/consumed by another explicit resume",
                    ));
                }
                if previous.registration_revision != current.revision
                    || previous.catalog != current.registration.declaration.catalog
                {
                    return Err(invalid("pending resume declaration changed"));
                }
            } else {
                let r = ResumeRecord {
                    request: request.clone(),
                    registration_revision: current.revision,
                    catalog: current.registration.declaration.catalog,
                    consumed_by: None,
                };
                save(
                    tx,
                    component,
                    &key,
                    None,
                    &document(RESUME, &r)?,
                    "resume-explicitly-requested",
                )?;
            }
            Ok(ResumePermit {
                registration: component.clone(),
                request,
            })
        })
    }
}
