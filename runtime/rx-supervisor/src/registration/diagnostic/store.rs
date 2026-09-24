use super::*;
pub(super) const BINDING: &str = "rx.diagnostic-binding.v1";
const RUN: &str = "rx.diagnostic-run.v1";
const RESULT: &str = "rx.diagnostic-result.v1";
const MAX_SAMPLES: usize = 16;
pub(super) fn entity_key(kind: &str, component: &Id, id: &Id) -> Name {
    name(&format!("components/diagnostic/{kind}/{component}/{id}"))
}
pub(super) fn binding(
    tx: &mut dyn Transaction,
    component: &Id,
    id: &Id,
) -> Result<(Record, TrackedBinding)> {
    let row = tx
        .get(&entity_key("binding", component, id))?
        .ok_or_else(|| invalid("diagnostic binding not found"))?;
    let value: TrackedBinding = decode(&row, BINDING)?;
    if value.id != *id || value.consumer.registration != *component {
        return Err(invalid("diagnostic binding identity differs"));
    }
    replacement::check_version(tx, component, &value)?;
    Ok((row, value))
}
fn run(
    tx: &mut dyn Transaction,
    component: &Id,
    binding_id: &Id,
    id: &Id,
) -> Result<(Record, Run)> {
    let row = tx
        .get(&entity_key("run", component, id))?
        .ok_or_else(|| invalid("diagnostic run not found"))?;
    let value: Run = decode(&row, RUN)?;
    if value.id != *id
        || value.binding != *binding_id
        || value.samples.len() > MAX_SAMPLES
        || ((value.phase == RunPhase::Completed) != value.result.is_some())
    {
        return Err(invalid("diagnostic run identity or lifecycle differs"));
    }
    Ok((row, value))
}
fn result(tx: &mut dyn Transaction, component: &Id, id: &Id) -> Result<DiagnosticResult> {
    let row = tx
        .get(&entity_key("result", component, id))?
        .ok_or_else(|| invalid("diagnostic result not found"))?;
    let value: DiagnosticResult = decode(&row, RESULT)?;
    if value.id != *id
        || value.consumer.registration != *component
        || value.digest != digest("RX-DIAGNOSTIC-RESULT-v1", &value.body)?
    {
        return Err(invalid("diagnostic result identity or digest differs"));
    }
    let (_, source) = run(tx, component, &value.binding, &value.run)?;
    if source.phase != RunPhase::Completed || source.result.as_ref() != Some(id) {
        return Err(invalid("diagnostic result has no matching completed run"));
    }
    Ok(value)
}
pub(super) fn eligible_consumer(
    tx: &mut dyn Transaction,
    expected: &RegistrationRef,
) -> Result<Assessment> {
    let current = load(tx, &expected.registration)?;
    if current.registration.state != RegistrationState::Accepted {
        return Ok(not_met(
            "consumer/registration",
            "consumer registration is retired; historical records are retained",
        ));
    }
    if RegistrationRef::from_registration(&current) != *expected {
        return Ok(not_met(
            "consumer/catalog-revision",
            "consumer declaration/interpretation changed; explicit review required",
        ));
    }
    Ok(met(
        "consumer/registration",
        "current consumer registration matches its authored diagnostic catalog",
    ))
}
fn assignment(dependency: &Dependency, current: &Observed) -> Assessment {
    match dependency {
        Dependency::Required {
            window: Window::ResultGeneration,
            ..
        } => met(
            "dependency/generation-window",
            "provider input is required at result generation, not at diagnostic assignment",
        ),
        _ => current.assessment.clone(),
    }
}
fn ongoing(dependency: &Dependency, current: &Observed, active: Option<&Run>) -> Assessment {
    let Some(active) = active else {
        return unassessed("run/ongoing", "no diagnostic run selected");
    };
    if active.phase != RunPhase::Running {
        return unassessed("run/ongoing", "selected run is not in progress");
    }
    match dependency {
        Dependency::Required {
            window: Window::PreparationOnly,
            ..
        } if !active.samples.is_empty() => met(
            "dependency/preparation-captured",
            "this run consumes its captured preparation input; provider loss alone does not stop it",
        ),
        _ => current.assessment.clone(),
    }
}
fn result_use(
    dependency: &Dependency,
    current: &Observed,
    tracked: &TrackedBinding,
    value: Option<&DiagnosticResult>,
) -> Assessment {
    let Some(value) = value else {
        return unassessed("result/consumption", "no recorded result selected");
    };
    if value.consumer != tracked.consumer
        || value.profile != tracked.profile
        || value.binding != tracked.id
    {
        return not_met(
            "result/interpretation",
            "result belongs to another binding/catalog interpretation",
        );
    }
    match dependency {
        Dependency::Required {
            window: Window::Continuous,
            ..
        } => {
            if current.assessment.state != ConditionState::Satisfied {
                return current.assessment.clone();
            }
            let Some(sample) = &current.sample else {
                return unassessed("result/current-source", "no current provider evidence");
            };
            let prior = value
                .body
                .get("samples")
                .and_then(|v| v.as_array())
                .and_then(|a| a.last());
            if prior.and_then(|v| v.get("report_digest"))
                != Some(&serde_json::json!(sample.report_digest))
            {
                return not_met(
                    "result/current-report",
                    "current report differs from the completed continuous collection; history is retained",
                );
            }
            met(
                "result/current-report",
                "current same-generation report matches the recorded diagnostic result; no work permission",
            )
        }
        Dependency::Required { .. } | Dependency::Independent => met(
            "result/historical-diagnostic",
            "intact authored historical diagnostic is usable for diagnostic reading; provider lifetime does not invalidate it",
        ),
        _ => current.assessment.clone(),
    }
}

/// The caller drives explicit diagnostic checkpoints; this is not an autonomous
/// watcher or a claim that availability held between observations. History has no
/// deletion API; use a separately reviewed archival policy if retention is needed.
pub struct Consumer<R> {
    pub(super) registry: Registry<R>,
    pub(super) consumer: RegistrationRef,
    pub(super) catalog: Catalog,
    pub(super) decision_gate: crate::decision::Gate,
}
impl<R: Repository> Consumer<R> {
    pub fn open(mut registry: Registry<R>, component: Id, catalog: Catalog) -> Result<Self> {
        let current = registry.query(&component)?.registration;
        if current.registration.declaration.catalog != catalog.reference()? {
            return Err(invalid(
                "consumer/catalog: authored declaration differs from accepted registration",
            ));
        }
        Ok(Self {
            registry,
            consumer: RegistrationRef::from_registration(&current),
            catalog,
            decision_gate: crate::decision::Gate::new(),
        })
    }
    pub fn into_registry(self) -> Registry<R> {
        self.registry
    }
    pub fn record_verified_decision(
        &mut self,
        proof: &crate::decision::VerifiedDecision,
    ) -> Result<DecisionRecord> {
        self.registry.record_verified_decision(proof)
    }
    pub fn record_verified_revocation(
        &mut self,
        proof: &crate::decision::VerifiedRevocation,
    ) -> Result<DecisionRevocationRecord> {
        self.registry.record_verified_revocation(proof)
    }
    pub fn recorded_decision(&mut self, decision: &Id) -> Result<DecisionRecord> {
        self.registry
            .recorded_decision(&self.consumer.registration, decision)
    }
    pub fn history(&mut self) -> Result<Vec<StoredEvent>> {
        self.registry.history(&self.consumer.registration)
    }
    pub(super) fn load_binding(
        &mut self,
        id: &Id,
    ) -> Result<(Record, TrackedBinding, Dependency, Assessment)> {
        let component = self.consumer.registration.clone();
        let (row, value, eligibility) = self.registry.repository.transact(|tx| {
            let (row, value) = binding(tx, &component, id)?;
            let eligibility = eligible_consumer(tx, &value.consumer)?;
            Ok((row, value, eligibility))
        })?;
        if value.consumer.catalog != self.consumer.catalog {
            return Err(invalid(
                "binding/consumer-catalog: tracked declaration differs",
            ));
        }
        let dependency = self
            .catalog
            .profiles
            .get(&value.profile)
            .cloned()
            .ok_or_else(|| invalid("binding/profile: catalog profile absent"))?;
        Ok((row, value, dependency, eligibility))
    }
    /// Record a diagnostic relationship even when the provider has no execution.
    /// This neither accepts an initial work binding nor accepts a replacement.
    pub fn track(
        &mut self,
        profile: Name,
        operating_area: Name,
        provider: Option<RegistrationRef>,
    ) -> Result<TrackedBinding> {
        let declaration = self
            .catalog
            .profiles
            .get(&profile)
            .ok_or_else(|| invalid("catalog/profile: unknown diagnostic operation"))?;
        match (declaration, &provider) {
            (Dependency::Independent, Some(_)) | (Dependency::Required { .. }, None) => {
                return Err(invalid(
                    "binding/provider: selection disagrees with authored dependency",
                ));
            }
            _ => {}
        }
        if provider
            .as_ref()
            .is_some_and(|p| p.registration == self.consumer.registration)
        {
            return Err(invalid(
                "binding/provider: diagnostic self-dependency is unsupported",
            ));
        }
        let value = TrackedBinding {
            id: fresh_id(),
            consumer: self.consumer.clone(),
            profile,
            operating_area,
            provider,
            generation: None,
            purpose: Purpose::DiagnosticOnly,
        };
        self.registry.repository.transact(|tx| {
            let a = eligible_consumer(tx, &self.consumer)?;
            if a.state != ConditionState::Satisfied {
                return Err(invalid(&a.conditions[0].reason));
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
            save(
                tx,
                &self.consumer.registration,
                &entity_key("binding", &self.consumer.registration, &value.id),
                None,
                &document(BINDING, &value)?,
                "diagnostic-relationship-tracked",
            )?;
            Ok(value)
        })
    }
    pub fn assign(&mut self, id: &Id, source: &mut impl Source) -> Result<Progress> {
        let (row, mut tracked, declaration, eligibility) = self.load_binding(id)?;
        self.registry
            .repository
            .transact(|tx| replacement::admit_assignment(tx, &self.consumer.registration, id))?;
        if eligibility.state != ConditionState::Satisfied {
            return Ok(Progress {
                assessment: eligibility,
                run: None,
                result: None,
            });
        }
        let next = fresh_id();
        let current = if matches!(
            declaration,
            Dependency::Required {
                window: Window::ResultGeneration,
                ..
            }
        ) {
            Observed {
                assessment: met(
                    "dependency/generation-window",
                    "no provider input consumed at assignment",
                ),
                sample: None,
            }
        } else {
            observe(
                &tracked,
                Some(&next),
                &declaration,
                ProbePurpose::Assign,
                source,
            )
        };
        let decision = assignment(&declaration, &current);
        if decision.state != ConditionState::Satisfied {
            return Ok(Progress {
                assessment: decision,
                run: None,
                result: None,
            });
        }
        let value = Run {
            id: next,
            binding: id.clone(),
            phase: RunPhase::Assigned,
            purpose: Purpose::DiagnosticOnly,
            samples: current.sample.into_iter().collect(),
            result: None,
        };
        self.commit_run(
            &row,
            &mut tracked,
            None,
            &value,
            None,
            "diagnostic-assigned",
        )?;
        Ok(Progress {
            assessment: decision,
            run: Some(value),
            result: None,
        })
    }
    fn commit_run(
        &mut self,
        binding_row: &Record,
        tracked: &mut TrackedBinding,
        old_run: Option<&Record>,
        value: &Run,
        produced: Option<&DiagnosticResult>,
        action: &str,
    ) -> Result<()> {
        let observed_generation = value.samples.first().map(|s| s.generation.clone());
        if tracked.generation.is_none() {
            tracked.generation = observed_generation;
        } else if observed_generation
            .as_ref()
            .is_some_and(|g| Some(g) != tracked.generation.as_ref())
        {
            return Err(invalid(
                "binding/provider-generation: no automatic succession",
            ));
        }
        self.registry.repository.transact(|tx| {
            let eligibility = eligible_consumer(tx, &self.consumer)?;
            if eligibility.state != ConditionState::Satisfied {
                return Err(invalid(&eligibility.conditions[0].reason));
            }
            let (current, _) = binding(tx, &self.consumer.registration, &tracked.id)?;
            if current.revision != binding_row.revision {
                return Err(StoreError::RevisionConflict(
                    "diagnostic binding changed during observation".into(),
                ));
            }
            if old_run.is_none() {
                replacement::admit_assignment(tx, &self.consumer.registration, &tracked.id)?;
            }
            if old_run.is_none()
                && tx
                    .scan(&format!(
                        "components/diagnostic/run/{}/",
                        self.consumer.registration
                    ))?
                    .len()
                    >= 512
            {
                return Err(invalid(
                    "diagnostic run capacity reached; explicit archival policy required",
                ));
            }
            if current.document.value
                != serde_json::to_value(&*tracked).map_err(|e| invalid(&e.to_string()))?
            {
                save(
                    tx,
                    &self.consumer.registration,
                    &current.key,
                    Some(current.revision),
                    &document(BINDING, tracked)?,
                    "diagnostic-generation-pinned",
                )?;
            }
            if let Some(produced) = produced {
                save(
                    tx,
                    &self.consumer.registration,
                    &entity_key("result", &self.consumer.registration, &produced.id),
                    None,
                    &document(RESULT, produced)?,
                    "diagnostic-result-recorded",
                )?;
            }
            save(
                tx,
                &self.consumer.registration,
                &entity_key("run", &self.consumer.registration, &value.id),
                old_run.map(|r| r.revision),
                &document(RUN, value)?,
                action,
            )?;
            Ok(())
        })
    }
    pub fn begin(
        &mut self,
        binding_id: &Id,
        run_id: &Id,
        source: &mut impl Source,
    ) -> Result<Progress> {
        self.advance(binding_id, run_id, source, ProbePurpose::Begin)
    }
    pub fn poll(
        &mut self,
        binding_id: &Id,
        run_id: &Id,
        source: &mut impl Source,
    ) -> Result<Progress> {
        self.advance(binding_id, run_id, source, ProbePurpose::Poll)
    }
    pub fn finish(
        &mut self,
        binding_id: &Id,
        run_id: &Id,
        source: &mut impl Source,
    ) -> Result<Progress> {
        self.advance(binding_id, run_id, source, ProbePurpose::Finish)
    }
    fn advance(
        &mut self,
        binding_id: &Id,
        run_id: &Id,
        source: &mut impl Source,
        purpose: ProbePurpose,
    ) -> Result<Progress> {
        let (binding_row, mut tracked, declaration, eligibility) = self.load_binding(binding_id)?;
        let (row, mut value) = self
            .registry
            .repository
            .transact(|tx| run(tx, &self.consumer.registration, binding_id, run_id))?;
        if eligibility.state != ConditionState::Satisfied {
            return Ok(Progress {
                assessment: eligibility,
                run: Some(value),
                result: None,
            });
        }
        let expected = if purpose == ProbePurpose::Begin {
            RunPhase::Assigned
        } else {
            RunPhase::Running
        };
        if value.phase != expected {
            return Ok(Progress {
                assessment: not_met(
                    "run/phase",
                    "requested transition does not match durable phase; no implicit restart or result overwrite",
                ),
                run: Some(value),
                result: None,
            });
        }
        let needs_source = matches!(
            declaration,
            Dependency::Required {
                window: Window::Continuous,
                ..
            }
        ) || (purpose == ProbePurpose::Finish
            && matches!(
                declaration,
                Dependency::Required {
                    window: Window::ResultGeneration,
                    ..
                }
            ));
        let current = if needs_source {
            observe(&tracked, Some(run_id), &declaration, purpose, source)
        } else {
            Observed {
                assessment: match &declaration {
                    Dependency::Required {
                        window: Window::PreparationOnly,
                        ..
                    } if !value.samples.is_empty() => met(
                        "dependency/preparation-captured",
                        "consume this run's captured diagnostic input without requiring a live provider",
                    ),
                    Dependency::Required {
                        window: Window::ResultGeneration,
                        ..
                    } => met(
                        "dependency/generation-window",
                        "diagnostic operation is preparing its result envelope; provider input is required at finish",
                    ),
                    Dependency::Independent => met(
                        "dependency/independent",
                        "consume the authored local catalog description",
                    ),
                    _ => unsupported(
                        "dependency/unsupported",
                        "no usable authored input declaration",
                    ),
                },
                sample: None,
            }
        };
        if current.assessment.state != ConditionState::Satisfied {
            return Ok(Progress {
                assessment: current.assessment,
                run: Some(value),
                result: None,
            });
        }
        if let Some(sample) = current.sample {
            let limit = if purpose == ProbePurpose::Finish {
                MAX_SAMPLES
            } else {
                MAX_SAMPLES - 1
            };
            if value.samples.len() >= limit {
                return Ok(Progress {
                    assessment: not_met(
                        "run/sample-capacity",
                        "bounded collection is full; finish before collecting further samples",
                    ),
                    run: Some(value),
                    result: None,
                });
            }
            value.samples.push(sample);
        }
        let produced = if purpose == ProbePurpose::Finish {
            let body = serde_json::json!({"meaning":"diagnostic report consumption only; not work permission or physical qualification", "profile":tracked.profile,
                "declaration":declaration, "samples":value.samples, "sample_count":value.samples.len(),
                "catalog":self.catalog.reference()?, "local_catalog_summary": if matches!(declaration, Dependency::Independent) { Some(&self.catalog) } else { None }});
            let result = DiagnosticResult {
                id: fresh_id(),
                run: value.id.clone(),
                binding: binding_id.clone(),
                consumer: self.consumer.clone(),
                profile: tracked.profile.clone(),
                purpose: Purpose::DiagnosticOnly,
                digest: digest("RX-DIAGNOSTIC-RESULT-v1", &body)?,
                body,
                work_use: WorkUsePermission::Unsupported,
            };
            value.result = Some(result.id.clone());
            value.phase = RunPhase::Completed;
            Some(result)
        } else {
            value.phase = RunPhase::Running;
            None
        };
        self.commit_run(
            &binding_row,
            &mut tracked,
            Some(&row),
            &value,
            produced.as_ref(),
            match purpose {
                ProbePurpose::Begin => "diagnostic-begun",
                ProbePurpose::Poll => "diagnostic-polled",
                _ => "diagnostic-completed",
            },
        )?;
        Ok(Progress {
            assessment: current.assessment,
            run: Some(value),
            result: produced,
        })
    }
    /// Read original evidence without claiming it is currently consumable.
    pub fn recorded_result(&mut self, id: &Id) -> Result<DiagnosticResult> {
        self.registry
            .repository
            .transact(|tx| result(tx, &self.consumer.registration, id))
    }
    pub fn inspect(
        &mut self,
        binding_id: &Id,
        run_id: Option<&Id>,
        result_id: Option<&Id>,
        source: &mut impl Source,
    ) -> Result<Inspection> {
        let (_, tracked, declaration, eligibility) = self.load_binding(binding_id)?;
        let (active, produced) = self.registry.repository.transact(|tx| {
            let active = run_id
                .map(|id| run(tx, &self.consumer.registration, binding_id, id).map(|(_, r)| r))
                .transpose()?;
            let produced = result_id
                .map(|id| result(tx, &self.consumer.registration, id))
                .transpose()?;
            if produced.as_ref().is_some_and(|p| p.binding != *binding_id) {
                return Err(invalid(
                    "result/binding: result belongs to another relationship",
                ));
            }
            Ok((active, produced))
        })?;
        let current = observe(
            &tracked,
            run_id,
            &declaration,
            ProbePurpose::Inspect,
            source,
        );
        let (mut new_assignment, ongoing, result_consumption) =
            if eligibility.state == ConditionState::Satisfied {
                (
                    assignment(&declaration, &current),
                    ongoing(&declaration, &current, active.as_ref()),
                    result_use(&declaration, &current, &tracked, produced.as_ref()),
                )
            } else {
                (eligibility.clone(), eligibility.clone(), eligibility)
            };
        if !self.registry.repository.transact(|tx| {
            replacement::assignment_is_current(tx, &self.consumer.registration, binding_id)
        })? {
            new_assignment = not_met(
                "binding/superseded",
                "new assignment must resolve the active route; ongoing runs and results keep their immutable binding version",
            );
        }
        Ok(Inspection {
            binding: tracked,
            run: active,
            result: produced,
            new_assignment,
            ongoing,
            result_consumption,
            work_use: WorkUsePermission::Unsupported,
            checkpoints: CheckpointPolicy::current(),
            limitations: vec![
                "diagnostic tracking is not initial or replacement binding acceptance",
                "positive consumer-side binding and work-use providers unsupported",
                "explicit checkpoints do not prove continuous availability between samples",
                "actual device operations, collaborative resource binding, Linux resource enforcement and multi-host unsupported",
                "external recovery investigator and control-effect group-target signal race remain unsupported/unresolved",
            ],
        })
    }
    /// A current diagnostic-consumption gate, distinct from historical record access.
    pub fn consume_result(&mut self, id: &Id, source: &mut impl Source) -> Result<Progress> {
        let historical = self.recorded_result(id)?;
        let view = self.inspect(&historical.binding, None, Some(id), source)?;
        Ok(Progress {
            result: (view.result_consumption.state == ConditionState::Satisfied)
                .then_some(historical),
            run: None,
            assessment: view.result_consumption,
        })
    }
    pub fn assess_binding(
        &mut self,
        id: &Id,
        kind: AcceptanceKind,
        proposed_provider: Option<RegistrationRef>,
        port: &impl BindingJudgment,
    ) -> Result<AcceptanceAssessment> {
        self.assess_binding_with_generation(id, kind, proposed_provider, None, port)
    }
    /// Replacement positives require an explicitly identified proposed generation.
    /// These target coordinates do not independently prove physical availability.
    pub fn assess_binding_with_generation(
        &mut self,
        id: &Id,
        kind: AcceptanceKind,
        proposed_provider: Option<RegistrationRef>,
        proposed_generation: Option<Generation>,
        port: &impl BindingJudgment,
    ) -> Result<AcceptanceAssessment> {
        let (_, tracked, _, eligibility) = self.load_binding(id)?;
        if (kind == AcceptanceKind::Initial
            && (proposed_provider.is_some() || proposed_generation.is_some()))
            || (kind == AcceptanceKind::Replacement && proposed_provider.is_none())
        {
            return Err(invalid(
                "binding/decision-kind: initial and replacement judgments are distinct requests",
            ));
        }
        let decision = if eligibility.state == ConditionState::Satisfied
            && tracked.generation.is_some()
            && (kind == AcceptanceKind::Initial || proposed_generation.is_some())
        {
            self.catalog.decision_policy.as_ref().and_then(|policy| self.decision_gate.request(
                match kind { AcceptanceKind::Initial => crate::decision::Kind::InitialBinding, AcceptanceKind::Replacement => crate::decision::Kind::ReplacementBinding },
                crate::decision::Owner { registration: self.consumer.registration.clone(), revision: self.consumer.revision,
                    program: self.consumer.catalog.program.clone(), catalog: self.consumer.catalog.digest },
                &tracked.operating_area, &tracked.profile,
                serde_json::json!({"binding":tracked,"proposed_provider":proposed_provider,"proposed_generation":proposed_generation}),policy).ok())
        } else {
            None
        };
        let request = AcceptanceRequest {
            kind,
            binding: tracked,
            proposed_provider,
            proposed_generation,
            decision,
        };
        let reply = port.assess(&request);
        if let AcceptanceReply::Verified(proof) = reply {
            let checked = request
                .decision
                .as_ref()
                .ok_or(crate::decision::Failure::Unconfigured)
                .and_then(|r| r.check(&proof));
            let (state, reason, reference) = match checked {
                Ok(reference) => (WorkUseState::Verified, "external issuer key possession and exact binding target verified; not work-use permission, physical truth or automatic replacement".into(), Some(reference)),
                Err(error) => (WorkUseState::Denied, error.to_string(), None),
            };
            return Ok(AcceptanceAssessment {
                request,
                state,
                condition: name("binding/external-decision"),
                reason,
                decision_reference: reference
                    .as_ref()
                    .map(|r| name(&format!("decision/{}", r.decision))),
                verified_decision: reference,
            });
        }
        let (state, condition, reason, reference) = match reply {
            AcceptanceReply::Verified(_) => unreachable!("handled above"),
            AcceptanceReply::NotEvaluated { condition, reason } => {
                (WorkUseState::NotEvaluated, condition, reason, None)
            }
            AcceptanceReply::Denied {
                condition,
                reason,
                decision_reference,
            } => (
                WorkUseState::Denied,
                condition,
                reason,
                Some(decision_reference),
            ),
            AcceptanceReply::Unsupported { condition, reason } => {
                (WorkUseState::Unsupported, condition, reason, None)
            }
        };
        if reason.trim().is_empty() || reason.len() > 1024 {
            return Ok(AcceptanceAssessment {
                request,
                state: WorkUseState::Unsupported,
                condition: name("binding/provider-response"),
                reason: "binding judgment lacks a bounded named explanation".into(),
                decision_reference: None,
                verified_decision: None,
            });
        }
        Ok(AcceptanceAssessment {
            request,
            state,
            condition,
            reason,
            decision_reference: reference,
            verified_decision: None,
        })
    }
}
