//! Explicit application changes one future-assignment route, never a run's
//! binding identity. Frozen versions and all prior results remain readable.
use super::*;
use crate::decision;
use store::{BINDING, binding, eligible_consumer, entity_key};
const ROUTE: &str = "rx.diagnostic-route.v1";
const VERSION: &str = "rx.diagnostic-binding-version.v1";
const APPLICATION: &str = "rx.diagnostic-replacement-application.v1";
const CONSUMPTION: &str = "rx.diagnostic-replacement-consumption.v1";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointPolicy {
    pub cadence: String,
    pub maximum_detection_delay_ms: Option<Counter>,
    pub observations: String,
    pub failure_detection: String,
    pub follow_up: String,
}
impl CheckpointPolicy {
    pub fn current() -> Self {
        Self {
            cadence:"EXPLICIT_CALLER_DRIVEN_CHECKPOINTS; NO_TIMER_OR_BACKGROUND_MONITOR".into(),
            maximum_detection_delay_ms:None,
            observations:"PREPARATION_ONLY captures at ASSIGN then reuses captured input; RESULT_GENERATION captures at FINISH; CONTINUOUS observes at ASSIGN/BEGIN/POLL/FINISH; INSPECT explicitly observes dependent providers; replacement APPLY probes its candidate".into(),
            failure_detection:"at the next required observation: known loss/changed identity/report is NOT_MET; unavailable evidence is NOT_EVALUATED".into(),
            follow_up:"withhold the affected transition/result consumption; preserve runs/results; no automatic stop, replay, replacement or physical handover".into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Route {
    root: Id,
    active: Id,
    generation: Counter,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Version {
    root: Id,
    binding: Id,
    digest: Digest,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
struct Snapshot {
    route: Route,
    route_revision: Option<Counter>,
    binding_revision: Counter,
    active: TrackedBinding,
}
#[derive(Clone, Debug, Serialize)]
pub struct Routing {
    pub root: Id,
    pub revision: Option<Counter>,
    pub generation: Counter,
    pub active: TrackedBinding,
    pub meaning: &'static str,
    pub checkpoints: CheckpointPolicy,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplacementIntent {
    pub application: Id,
    /// Root or a known version of this same relationship, never a new provider ID.
    pub binding: Id,
    pub proposed_provider: RegistrationRef,
    pub proposed_generation: Generation,
}
/// Live proof holder. Assessment/receipt DTOs and signed historical references
/// cannot construct it or restore its receiver after restart.
///
/// ```compile_fail
/// use rx_supervisor::registration::diagnostic::PreparedReplacement;
/// let _: PreparedReplacement = serde_json::from_str("{}").unwrap();
/// ```
/// ```compile_fail
/// use rx_supervisor::registration::diagnostic::{AcceptanceAssessment,PreparedReplacement};
/// fn restore(v: AcceptanceAssessment) -> PreparedReplacement { v.into() }
/// ```
/// ```compile_fail
/// use rx_supervisor::registration::diagnostic::{ReplacementReceipt,PreparedReplacement};
/// fn restore(v: ReplacementReceipt) -> PreparedReplacement { v.into() }
/// ```
/// ```compile_fail
/// use rx_supervisor::registration::diagnostic::PreparedReplacement;
/// fn forge(p: PreparedReplacement) { let _ = p.proof; }
/// ```
#[derive(Debug, Serialize)]
pub struct PreparedReplacement {
    intent: ReplacementIntent,
    snapshot: Snapshot,
    next: TrackedBinding,
    proof: decision::VerifiedDecision,
}
impl PreparedReplacement {
    pub fn decision(&self) -> &decision::VerifiedDecision {
        &self.proof
    }
    pub fn application(&self) -> &Id {
        &self.intent.application
    }
}
/// Inert history of one explicit application; not current approval, current
/// source availability, process ownership or physical handover authority.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplacementReceipt {
    pub application: Id,
    pub root: Id,
    pub generation: Counter,
    pub previous: TrackedBinding,
    pub next: TrackedBinding,
    pub decision: decision::Reference,
    pub candidate_observation: Sample,
    pub signature_verification: String,
    pub operating_area_policy: String,
    pub application_effect: String,
    pub current_authority: String,
    pub process_ownership: String,
    pub physical_handover: String,
    pub work_use: WorkUsePermission,
    pub checkpoints: CheckpointPolicy,
}
fn version_key(component: &Id, id: &Id) -> Name {
    entity_key("version", component, id)
}
fn route_key(component: &Id, id: &Id) -> Name {
    entity_key("route", component, id)
}
fn application_key(component: &Id, id: &Id) -> Name {
    entity_key("application", component, id)
}
fn consumption_key(component: &Id, id: &Id) -> Name {
    entity_key("replacement-consumed", component, id)
}
fn version(tx: &mut dyn Transaction, component: &Id, id: &Id) -> Result<Option<Version>> {
    tx.get(&version_key(component, id))?
        .map(|r| {
            let v: Version = decode(&r, VERSION)?;
            if v.binding != *id {
                return Err(invalid("replacement/version-identity"));
            }
            Ok(v)
        })
        .transpose()
}
pub(super) fn check_version(
    tx: &mut dyn Transaction,
    component: &Id,
    b: &TrackedBinding,
) -> Result<()> {
    if let Some(v) = version(tx, component, &b.id)?
        && v.digest != digest("RX-DIAGNOSTIC-BINDING-VERSION-v1", b)?
    {
        return Err(StoreError::Integrity(
            "immutable diagnostic binding version differs".into(),
        ));
    }
    Ok(())
}
fn snapshot(tx: &mut dyn Transaction, component: &Id, id: &Id) -> Result<Snapshot> {
    let root = version(tx, component, id)?
        .map(|v| v.root)
        .unwrap_or_else(|| id.clone());
    let (route, revision) = if let Some(row) = tx.get(&route_key(component, &root))? {
        let route: Route = decode(&row, ROUTE)?;
        if route.root != root || route.generation.0 == 0 {
            return Err(invalid("replacement/route-identity"));
        }
        for id in [&root, &route.active] {
            if version(tx, component, id)?.is_none_or(|v| v.root != root) {
                return Err(invalid("replacement/incomplete-version-membership"));
            }
        }
        (route, Some(row.revision))
    } else {
        if version(tx, component, &root)?.is_some() {
            return Err(invalid("replacement/version-without-route"));
        }
        (
            Route {
                root: root.clone(),
                active: root,
                generation: Counter(0),
            },
            None,
        )
    };
    let (row, active) = binding(tx, component, &route.active)?;
    Ok(Snapshot {
        route,
        route_revision: revision,
        binding_revision: row.revision,
        active,
    })
}
pub(super) fn assignment_is_current(
    tx: &mut dyn Transaction,
    component: &Id,
    id: &Id,
) -> Result<bool> {
    Ok(snapshot(tx, component, id)?.route.active == *id)
}
pub(super) fn admit_assignment(tx: &mut dyn Transaction, component: &Id, id: &Id) -> Result<()> {
    if !assignment_is_current(tx, component, id)? {
        return Err(StoreError::RevisionConflict(
            "replacement/superseded-binding; use assign_current(root) for a new run".into(),
        ));
    }
    Ok(())
}
fn seal(tx: &mut dyn Transaction, component: &Id, root: &Id, b: &TrackedBinding) -> Result<()> {
    if b.generation.is_none() {
        return Err(invalid("replacement/unpinned-generation"));
    }
    let v = Version {
        root: root.clone(),
        binding: b.id.clone(),
        digest: digest("RX-DIAGNOSTIC-BINDING-VERSION-v1", b)?,
    };
    if let Some(old) = version(tx, component, &b.id)? {
        if old != v {
            return Err(invalid("replacement/immutable-version-conflict"));
        }
    } else {
        save(
            tx,
            component,
            &version_key(component, &b.id),
            None,
            &document(VERSION, &v)?,
            "diagnostic-version-sealed",
        )?;
    }
    Ok(())
}
impl<R: Repository> Consumer<R> {
    /// Resolves configuration only. Each assignment still needs its authored
    /// dependency observations; a stored route is not current permission.
    pub fn routing(&mut self, id: &Id) -> Result<Routing> {
        let s = self
            .registry
            .repository
            .transact(|tx| snapshot(tx, &self.consumer.registration, id))?;
        Ok(Routing {
            root: s.route.root,
            revision: s.route_revision,
            generation: s.route.generation,
            active: s.active,
            meaning: "ONE_FUTURE_ASSIGNMENT_ROUTE; NOT_CURRENT_SOURCE_AVAILABILITY_OR_WORK_PERMISSION",
            checkpoints: CheckpointPolicy::current(),
        })
    }
    pub fn assign_current(&mut self, root: &Id, source: &mut impl Source) -> Result<Progress> {
        let current = self.routing(root)?;
        // assign and commit_run both recheck the route; no stale observed target
        // can publish a new run after another explicit application won the CAS.
        self.assign(&current.active.id, source)
    }
    fn replacement_request(
        &self,
        s: &Snapshot,
        intent: &ReplacementIntent,
        next: &TrackedBinding,
    ) -> Result<AcceptanceRequest> {
        let policy = self
            .catalog
            .decision_policy
            .as_ref()
            .ok_or_else(|| invalid("replacement/author-policy-absent"))?;
        let decision=self.decision_gate.request(decision::Kind::ReplacementBinding,
            decision::Owner{registration:self.consumer.registration.clone(),revision:self.consumer.revision,
                program:self.consumer.catalog.program.clone(),catalog:self.consumer.catalog.digest},
            &s.active.operating_area,&s.active.profile,
            serde_json::json!({"binding":s.active,"proposed_provider":intent.proposed_provider,"proposed_generation":intent.proposed_generation,
                "application":{"id":intent.application,"route":s.route,"route_revision":s.route_revision,"binding_revision":s.binding_revision,"next_binding":next}}),policy)
            .map_err(|e|invalid(&e.to_string()))?;
        Ok(AcceptanceRequest {
            kind: AcceptanceKind::Replacement,
            binding: s.active.clone(),
            proposed_provider: Some(intent.proposed_provider.clone()),
            proposed_generation: Some(intent.proposed_generation.clone()),
            decision: Some(decision),
        })
    }
    /// Verifies an application-bound approval without changing a route or spending
    /// it. The separate apply_replacement call is always required.
    pub fn prepare_replacement(
        &mut self,
        intent: ReplacementIntent,
        port: &impl BindingJudgment,
    ) -> Result<PreparedReplacement> {
        let s = self.registry.repository.transact(|tx| {
            let eligible = eligible_consumer(tx, &self.consumer)?;
            if eligible.state != ConditionState::Satisfied {
                return Err(invalid(&eligible.conditions[0].reason));
            }
            if tx
                .get(&application_key(
                    &self.consumer.registration,
                    &intent.application,
                ))?
                .is_some()
            {
                return Err(invalid(
                    "replacement/application-already-recorded; query history",
                ));
            }
            snapshot(tx, &self.consumer.registration, &intent.binding)
        })?;
        if s.active.consumer != self.consumer {
            return Err(invalid(
                "replacement/consumer-binding-revision; old interpretation cannot be migrated by replacement",
            ));
        }
        if !matches!(
            self.catalog.profiles.get(&s.active.profile),
            Some(Dependency::Required { .. })
        ) || s.active.generation.is_none()
        {
            return Err(invalid("replacement/requires-pinned-dependent-binding"));
        }
        if intent.proposed_provider.registration == self.consumer.registration
            || (s.active.provider.as_ref() == Some(&intent.proposed_provider)
                && s.active.generation.as_ref() == Some(&intent.proposed_generation))
        {
            return Err(invalid("replacement/self-or-unchanged-target"));
        }
        let mut next = s.active.clone();
        next.id = fresh_id();
        next.provider = Some(intent.proposed_provider.clone());
        next.generation = Some(intent.proposed_generation.clone());
        let request = self.replacement_request(&s, &intent, &next)?;
        let AcceptanceReply::Verified(proof) = port.assess(&request) else {
            return Err(invalid(
                "replacement/no-live-external-approval; assessment alone never applies",
            ));
        };
        request
            .decision_request()
            .expect("configured application request")
            .check(&proof)
            .map_err(|e| invalid(&e.to_string()))?;
        Ok(PreparedReplacement {
            intent,
            snapshot: s,
            next,
            proof: *proof,
        })
    }
    /// Atomic explicit switch for future assignments only. Existing runs/results
    /// retain their immutable IDs; no drain or provider/process ownership transfer.
    pub fn apply_replacement(
        &mut self,
        p: &PreparedReplacement,
        source: &mut impl Source,
    ) -> Result<ReplacementReceipt> {
        let current = self
            .registry
            .repository
            .transact(|tx| snapshot(tx, &self.consumer.registration, &p.snapshot.route.root))?;
        if current != p.snapshot {
            return Err(StoreError::RevisionConflict(
                "replacement/route-changed-since-approval".into(),
            ));
        }
        let request = self.replacement_request(&current, &p.intent, &p.next)?;
        let declaration = self
            .catalog
            .profiles
            .get(&p.next.profile)
            .ok_or_else(|| invalid("replacement/profile-missing"))?;
        let observed = observe(&p.next, None, declaration, ProbePurpose::Inspect, source);
        if observed.assessment.state != ConditionState::Satisfied {
            return Err(invalid(&format!(
                "replacement/candidate-unavailable: {}",
                observed
                    .assessment
                    .conditions
                    .iter()
                    .map(|c| c.reason.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            )));
        }
        let sample = observed
            .sample
            .ok_or_else(|| invalid("replacement/candidate-observation-missing"))?;
        let guard = request
            .decision_request()
            .expect("configured request")
            .commit_guard(&p.proof)
            .map_err(|e| invalid(&e.to_string()))?;
        self.registry.repository.transact(|tx| {
            let reference = guard.check().map_err(|e| invalid(&e.to_string()))?;
            let eligible = eligible_consumer(tx, &self.consumer)?;
            if eligible.state != ConditionState::Satisfied {
                return Err(invalid(&eligible.conditions[0].reason));
            }
            if snapshot(tx, &self.consumer.registration, &p.snapshot.route.root)? != p.snapshot {
                return Err(StoreError::RevisionConflict(
                    "replacement/route-CAS-conflict".into(),
                ));
            }
            if tx
                .scan(&format!(
                    "components/diagnostic/binding/{}/",
                    self.consumer.registration
                ))?
                .len()
                >= 64
            {
                return Err(invalid(
                    "diagnostic binding capacity reached; explicit archival policy required",
                ));
            }
            let generation = Counter(
                p.snapshot
                    .route
                    .generation
                    .0
                    .checked_add(1)
                    .ok_or_else(|| invalid("replacement/route-generation-overflow"))?,
            );
            seal(
                tx,
                &self.consumer.registration,
                &p.snapshot.route.root,
                &p.snapshot.active,
            )?;
            save(
                tx,
                &self.consumer.registration,
                &entity_key("binding", &self.consumer.registration, &p.next.id),
                None,
                &document(BINDING, &p.next)?,
                "replacement-version-created",
            )?;
            seal(
                tx,
                &self.consumer.registration,
                &p.snapshot.route.root,
                &p.next,
            )?;
            let route = Route {
                root: p.snapshot.route.root.clone(),
                active: p.next.id.clone(),
                generation,
            };
            save(
                tx,
                &self.consumer.registration,
                &route_key(&self.consumer.registration, &route.root),
                p.snapshot.route_revision,
                &document(ROUTE, &route)?,
                "replacement-route-applied",
            )?;
            let receipt = ReplacementReceipt {
                application: p.intent.application.clone(),
                root: route.root.clone(),
                generation,
                previous: p.snapshot.active.clone(),
                next: p.next.clone(),
                decision: reference.clone(),
                candidate_observation: sample,
                signature_verification: "EXTERNAL_SIGNATURE_AND_CONTEXT_VERIFIED_AT_LOGICAL_CUT"
                    .into(),
                operating_area_policy: "NOT_EVALUATED_BY_HOST; PRODUCTION_PROVIDER_NOT_CONNECTED"
                    .into(),
                application_effect: "EXPLICIT_SWITCH_FOR_FUTURE_DIAGNOSTIC_ASSIGNMENTS".into(),
                current_authority: "NONE; HISTORICAL_APPLICATION_ONLY".into(),
                process_ownership: "UNCHANGED; NO_PID_ADOPTION".into(),
                physical_handover: "NOT_ASSESSED".into(),
                work_use: WorkUsePermission::Unsupported,
                checkpoints: CheckpointPolicy::current(),
            };
            save(
                tx,
                &self.consumer.registration,
                &application_key(&self.consumer.registration, &p.intent.application),
                None,
                &document(APPLICATION, &receipt)?,
                "replacement-application-recorded",
            )?;
            save(
                tx,
                &self.consumer.registration,
                &consumption_key(&self.consumer.registration, &reference.decision),
                None,
                &document(
                    CONSUMPTION,
                    &serde_json::json!({"application":p.intent.application,"decision":reference}),
                )?,
                "replacement-decision-consumed",
            )?;
            guard.check().map_err(|e| invalid(&e.to_string()))?;
            Ok(receipt)
        })
    }
    pub fn recorded_replacement(&mut self, id: &Id) -> Result<ReplacementReceipt> {
        self.registry.repository.transact(|tx| {
            let row = tx
                .get(&application_key(&self.consumer.registration, id))?
                .ok_or_else(|| invalid("replacement/application-not-found"))?;
            let value: ReplacementReceipt = decode(&row, APPLICATION)?;
            if value.application != *id
                || value.previous.consumer.registration != self.consumer.registration
                || value.next.consumer != value.previous.consumer
            {
                return Err(invalid("replacement/receipt-identity"));
            }
            let (_, previous) = binding(tx, &self.consumer.registration, &value.previous.id)?;
            let (_, next) = binding(tx, &self.consumer.registration, &value.next.id)?;
            if previous != value.previous
                || next != value.next
                || value.decision.kind != decision::Kind::ReplacementBinding
            {
                return Err(StoreError::Integrity(
                    "replacement receipt differs from immutable versions".into(),
                ));
            }
            Ok(value)
        })
    }
}
