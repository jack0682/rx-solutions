//! A segment of the same real Linux passage, using the installed provider and
//! repository-backed framework consumer. No second driver or substitute service.
use super::*;
use rx_supervisor::registration::diagnostic as d;
fn provider(data: &Path) -> (Managed, Owned) {
    std::fs::create_dir_all(data).unwrap();
    let programs = release_programs(Path::new("/opt/rx")).unwrap();
    let mut registry = Registry::new(SqliteRepository::open(data.join("registration.db")).unwrap());
    let component = registry
        .register(Declaration {
            label: n("dependency-provider"),
            catalog: catalog_reference(&programs[&n("rx/status-http")]).unwrap(),
        })
        .unwrap()
        .registration
        .id;
    drop(registry);
    let config = json!({"component":component,"plan":plan()});
    open(data, &config, "provider")
}
fn assign(
    c: &mut d::Consumer<SqliteRepository>,
    b: &d::TrackedBinding,
    source: &mut impl d::Source,
) -> d::Run {
    let r = c.assign(&b.id, source).unwrap();
    assert_eq!(r.assessment.state, ConditionState::Satisfied);
    let r = r.run.unwrap();
    assert_eq!(r.phase, d::RunPhase::Assigned);
    r
}
fn begin(
    c: &mut d::Consumer<SqliteRepository>,
    b: &d::TrackedBinding,
    r: &d::Run,
    source: &mut impl d::Source,
) {
    let p = c.begin(&b.id, &r.id, source).unwrap();
    assert_eq!(p.assessment.state, ConditionState::Satisfied);
    assert_eq!(p.run.unwrap().phase, d::RunPhase::Running);
}
fn finish(
    c: &mut d::Consumer<SqliteRepository>,
    b: &d::TrackedBinding,
    r: &d::Run,
    source: &mut impl d::Source,
) -> d::DiagnosticResult {
    let p = c.finish(&b.id, &r.id, source).unwrap();
    assert_eq!(p.assessment.state, ConditionState::Satisfied);
    assert_eq!(p.run.unwrap().phase, d::RunPhase::Completed);
    p.result.unwrap()
}
pub(super) fn run(data: &Path) {
    let root = data.join("dependencies");
    let (mut source, mut owner) = provider(&root.join("provider"));
    let provider_ref = d::RegistrationRef::from_registration(&source.query().unwrap().registration);
    let consumer_path = root.join("consumer.db");
    let mut registry = Registry::new(SqliteRepository::open(&consumer_path).unwrap());
    let consumer_id = registry
        .register(Declaration {
            label: n("framework-diagnostic-consumer"),
            catalog: d::catalog().reference().unwrap(),
        })
        .unwrap()
        .registration
        .id;
    let mut consumer = d::Consumer::open(registry, consumer_id.clone(), d::catalog()).unwrap();
    let prep = consumer
        .track(
            n("snapshot-after-preparation"),
            n("passage/diagnostics"),
            Some(provider_ref.clone()),
        )
        .unwrap();
    let continuous = consumer
        .track(
            n("current-report-collection"),
            n("passage/diagnostics"),
            Some(provider_ref.clone()),
        )
        .unwrap();
    let generation = consumer
        .track(
            n("snapshot-at-generation"),
            n("passage/diagnostics"),
            Some(provider_ref.clone()),
        )
        .unwrap();
    let initial = consumer
        .assess_binding(
            &prep.id,
            d::AcceptanceKind::Initial,
            None,
            &d::NoBindingJudgment,
        )
        .unwrap();
    assert_eq!(initial.state(), WorkUseState::Unsupported);
    let unstarted = consumer.inspect(&prep.id, None, None, &mut source).unwrap();
    assert_eq!(unstarted.new_assignment.state, ConditionState::NotEvaluated);
    emit(
        "dependency-registered-without-execution",
        "provider and framework consumer have separate registration identities; tracking exists with zero provider executions and is not initial binding acceptance",
        json!({"provider":source.query().unwrap(),"consumer":consumer_id,"tracking":unstarted,"initial_acceptance":initial}),
    );
    ready(&mut source);
    let first = assign(&mut consumer, &prep, &mut source);
    emit(
        "diagnostic-assigned",
        "actual current HTTP report captured as this preparation-only run's input; no positive work permission",
        json!(&first),
    );
    begin(&mut consumer, &prep, &first, &mut source);
    let running = consumer
        .inspect(&prep.id, Some(&first.id), None, &mut source)
        .unwrap();
    assert_eq!(running.run.as_ref().unwrap().phase, d::RunPhase::Running);
    emit(
        "diagnostic-in-progress",
        "assigned run has begun and has no result yet; this is framework diagnostic work, not the publisher process lifetime",
        json!(running),
    );
    let historical = finish(&mut consumer, &prep, &first, &mut source);
    assert_eq!(historical.body["sample_count"], 1);
    assert!(
        historical.body["samples"][0]["report"]["conditions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "report/native-package-count" && c["observed"] == 675)
    );
    emit(
        "diagnostic-result-recorded",
        "actual provider self-report consumed into an immutable diagnostic result and completed run in one transaction; startup audit count is metadata, not physical qualification",
        json!(&historical),
    );
    let pending_prep = assign(&mut consumer, &prep, &mut source);
    begin(&mut consumer, &prep, &pending_prep, &mut source);
    let continuous_done = assign(&mut consumer, &continuous, &mut source);
    begin(&mut consumer, &continuous, &continuous_done, &mut source);
    let current_result = finish(&mut consumer, &continuous, &continuous_done, &mut source);
    let pending_continuous = assign(&mut consumer, &continuous, &mut source);
    begin(&mut consumer, &continuous, &pending_continuous, &mut source);
    assert_eq!(
        consumer
            .poll(&continuous.id, &pending_continuous.id, &mut source)
            .unwrap()
            .run
            .unwrap()
            .samples
            .len(),
        3
    );
    let pending_generation = assign(&mut consumer, &generation, &mut d::NoSource);
    begin(
        &mut consumer,
        &generation,
        &pending_generation,
        &mut d::NoSource,
    );
    let independent = consumer
        .track(n("catalog-summary"), n("passage/independent"), None)
        .unwrap();
    let pending_independent = assign(&mut consumer, &independent, &mut d::NoSource);
    begin(
        &mut consumer,
        &independent,
        &pending_independent,
        &mut d::NoSource,
    );
    let instance = source.state().unwrap().records[&n("status")]
        .instance
        .clone()
        .unwrap();
    owner.terminate(&instance, true).unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let s = source.tick().unwrap();
        if s.state.records[&n("status")].phase == Phase::Exited {
            break;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(owner.0.borrow().owned_instances().is_empty());
    let before = consumer.history().unwrap();
    let prep_view = consumer
        .inspect(
            &prep.id,
            Some(&pending_prep.id),
            Some(&historical.id),
            &mut source,
        )
        .unwrap();
    assert_eq!(prep_view.new_assignment.state, ConditionState::NotMet);
    assert_eq!(prep_view.ongoing.state, ConditionState::Satisfied);
    assert_eq!(
        prep_view.result_consumption.state,
        ConditionState::Satisfied
    );
    assert_eq!(consumer.history().unwrap(), before);
    let completed_after_loss = finish(&mut consumer, &prep, &pending_prep, &mut source);
    assert_eq!(
        consumer
            .consume_result(&historical.id, &mut source)
            .unwrap()
            .result,
        Some(historical.clone())
    );
    emit(
        "preparation-only-provider-loss",
        "new capture is withheld; ongoing work consumes captured input and completes; prior diagnostic result remains usable without requiring a live provider",
        json!({"three_targets":prep_view,"completed_after_loss":completed_after_loss}),
    );
    let continuous_view = consumer
        .inspect(
            &continuous.id,
            Some(&pending_continuous.id),
            Some(&current_result.id),
            &mut source,
        )
        .unwrap();
    assert_eq!(continuous_view.new_assignment.state, ConditionState::NotMet);
    assert_eq!(continuous_view.ongoing.state, ConditionState::NotMet);
    assert_eq!(
        continuous_view.result_consumption.state,
        ConditionState::NotMet
    );
    assert!(
        consumer
            .finish(&continuous.id, &pending_continuous.id, &mut source)
            .unwrap()
            .result
            .is_none()
    );
    assert_eq!(
        consumer.recorded_result(&current_result.id).unwrap(),
        current_result
    );
    let generation_view = consumer
        .inspect(
            &generation.id,
            Some(&pending_generation.id),
            None,
            &mut source,
        )
        .unwrap();
    assert_eq!(
        generation_view.new_assignment.state,
        ConditionState::Satisfied
    );
    assert_eq!(generation_view.ongoing.state, ConditionState::NotMet);
    assert!(
        consumer
            .finish(&generation.id, &pending_generation.id, &mut source)
            .unwrap()
            .result
            .is_none()
    );
    emit(
        "continuous-and-generation-provider-loss",
        "continuous progress/current result use and pending generation are withheld with named reasons; original results are preserved; no process-stop policy is inferred",
        json!({"continuous":continuous_view,"at_generation":generation_view}),
    );
    let unaffected = finish(
        &mut consumer,
        &independent,
        &pending_independent,
        &mut d::NoSource,
    );
    assert!(unaffected.body["local_catalog_summary"].is_object());
    emit(
        "unrelated-diagnostic-completed",
        "independent authored catalog-summary work was already running and produces its real local result after provider loss; no global diagnostic stop",
        json!(unaffected),
    );
    let (mut replacement, replacement_owner) = provider(&root.join("replacement"));
    ready(&mut replacement);
    let same_format = replacement
        .assess_use(
            UseScope {
                operating_area: n("passage/diagnostics"),
                role: n("diagnostics/support-summary"),
            },
            &NoWorkUseProvider,
        )
        .unwrap();
    assert_eq!(
        same_format.functional_readiness.state(),
        ConditionState::Satisfied
    );
    let candidate = d::RegistrationRef::from_registration(&same_format.registration);
    let before = consumer
        .inspect(
            &continuous.id,
            Some(&pending_continuous.id),
            None,
            &mut source,
        )
        .unwrap()
        .binding;
    let rejected = consumer
        .finish(&continuous.id, &pending_continuous.id, &mut replacement)
        .unwrap();
    assert_eq!(rejected.assessment.state, ConditionState::NotMet);
    assert!(rejected.result.is_none());
    let judgment = consumer
        .assess_binding(
            &continuous.id,
            d::AcceptanceKind::Replacement,
            Some(candidate),
            &d::NoBindingJudgment,
        )
        .unwrap();
    assert_eq!(judgment.state(), WorkUseState::Unsupported);
    assert_eq!(
        consumer
            .inspect(
                &continuous.id,
                Some(&pending_continuous.id),
                None,
                &mut source
            )
            .unwrap()
            .binding,
        before
    );
    emit(
        "same-format-replacement-not-inherited",
        "new actual publisher passes the same authored report profile but cannot replace the pinned source; replacement acceptance is separate and its positive provider is unsupported",
        json!({"candidate":same_format,"rejected":rejected,"replacement_acceptance":judgment}),
    );
    replacement.request_stop().unwrap();
    stopped(&mut replacement);
    assert!(replacement_owner.0.borrow().owned_instances().is_empty());
    drop(consumer.into_registry());
    let mut reopened = d::Consumer::open(
        Registry::new(SqliteRepository::open(consumer_path).unwrap()),
        consumer_id,
        d::catalog(),
    )
    .unwrap();
    assert_eq!(
        reopened.recorded_result(&historical.id).unwrap(),
        historical
    );
    emit(
        "diagnostic-history-reopened",
        "result and completed run survive consumer reconstruction; historical access is distinct from current consumption and never restores work permission",
        json!(
            reopened
                .consume_result(&historical.id, &mut d::NoSource)
                .unwrap()
        ),
    );
}
