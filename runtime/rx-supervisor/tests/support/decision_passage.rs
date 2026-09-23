//! Same Linux passage, explicitly authored test policies and external ephemeral
//! issuer. Default release catalogs have no decision anchors.
use super::external_signer::ExternalSigner;
use super::*;
use rx_supervisor::{decision::*, registration::diagnostic as d};
#[derive(Default)]
struct Capture {
    request: RefCell<Option<Request>>,
}
impl rx_supervisor::use_assessment::WorkUsePort for Capture {
    fn assess(
        &self,
        r: &rx_supervisor::use_assessment::WorkUseRequest,
    ) -> rx_supervisor::use_assessment::WorkUseReply {
        *self.request.borrow_mut() = r.decision_request().cloned();
        rx_supervisor::use_assessment::WorkUseReply::Unsupported {
            conditions: [(n("test/request"), "unsigned context only".into())].into(),
        }
    }
}
impl d::BindingJudgment for Capture {
    fn assess(&self, r: &d::AcceptanceRequest) -> d::AcceptanceReply {
        *self.request.borrow_mut() = r.decision_request().cloned();
        d::AcceptanceReply::Unsupported {
            condition: n("test/request"),
            reason: "unsigned context only".into(),
        }
    }
}
struct Cached(VerifiedDecision);
impl rx_supervisor::use_assessment::WorkUsePort for Cached {
    fn assess(
        &self,
        _: &rx_supervisor::use_assessment::WorkUseRequest,
    ) -> rx_supervisor::use_assessment::WorkUseReply {
        rx_supervisor::use_assessment::WorkUseReply::Verified(Box::new(self.0.clone()))
    }
}
impl d::BindingJudgment for Cached {
    fn assess(&self, _: &d::AcceptanceRequest) -> d::AcceptanceReply {
        d::AcceptanceReply::Verified(Box::new(self.0.clone()))
    }
}
fn scope() -> UseScope {
    UseScope {
        operating_area: n("test/diagnostics"),
        role: n("diagnostics/support-summary"),
    }
}
fn request(s: &mut Managed) -> Request {
    let c = Capture::default();
    s.assess_use(scope(), &c).unwrap();
    c.request.into_inner().unwrap()
}
fn signed(
    issuer: &ExternalSigner,
    r: &Request,
    ttl: u64,
) -> (Challenge, SignedDecision, VerifiedDecision) {
    let c = r.challenge(&issuer.key_id()).unwrap();
    let s = issuer.decision(c.claim(uid(), Counter(ttl)));
    let p = c.verify(&s).unwrap();
    (c, s, p)
}
fn open_provider(data: &Path, issuer: &ExternalSigner) -> (Managed, Owned) {
    std::fs::create_dir_all(data).unwrap();
    let root = Path::new("/opt/rx");
    let mut programs = release_programs(root).unwrap();
    assert!(programs.values().all(|p| p.decision_policy.is_none()));
    // Trusted author/test arrangement, not a site policy file or production default.
    programs
        .get_mut(&n("rx/status-http"))
        .unwrap()
        .decision_policy = Some(issuer.policy());
    let mut registry = Registry::new(SqliteRepository::open(data.join("registration.db")).unwrap());
    let component = registry
        .register(Declaration {
            label: n("explicit-test-issuer-provider"),
            catalog: catalog_reference(&programs[&n("rx/status-http")]).unwrap(),
        })
        .unwrap()
        .registration
        .id;
    let support = DeviceCatalog::decode(
        &std::fs::read(root.join("catalogs/device-support.v1.json")).unwrap(),
    )
    .unwrap();
    let backend = Owned::new(data.join("logs"));
    let managed = RegisteredSupervisor::open(
        SqliteRepository::open(data.join("execution.db")).unwrap(),
        backend.clone(),
        SoftwareOnly,
        plan(),
        programs,
        &support,
        registry,
        component,
    )
    .unwrap();
    (managed, backend)
}
fn generation(s: &mut Managed) -> d::Generation {
    let state = s.state().unwrap();
    let r = &state.records[&n("status")];
    d::Generation {
        run: state.plan,
        instance: r.instance.clone().unwrap(),
        pid: r.pid.unwrap(),
        configuration: state.plan_digest,
    }
}
pub(super) fn run(data: &Path) {
    let root = data.join("decisions");
    let issuer = ExternalSigner::new();
    let (mut provider, owner) = open_provider(&root.join("provider"), &issuer);
    ready(&mut provider);
    let r = request(&mut provider);
    let (c, statement, proof) = signed(&issuer, &r, 20_000);
    let mut unsigned = statement.clone();
    unsigned.signature.signature = "00".repeat(64);
    let error = c.verify(&unsigned).unwrap_err();
    assert_eq!(error, Failure::Signature);
    emit(
        "unverified-decision-rejected",
        "matching target metadata without a verified Ed25519 signature cannot construct positive evidence",
        json!({"error":error.to_string()}),
    );
    let history = provider.history().unwrap();
    let view = provider
        .assess_use(scope(), &Cached(proof.clone()))
        .unwrap();
    assert_eq!(view.work_use_permission.state(), WorkUseState::Verified);
    assert_eq!(view.functional_readiness.state(), ConditionState::Satisfied);
    assert_eq!(provider.history().unwrap(), history);
    let work_record = provider.record_verified_decision(&proof).unwrap();
    emit(
        "external-work-decision-verified",
        "external ephemeral test issuer key possession and exact target verified; no actual operating-area approval, policy correctness or physical qualification; production defaults have no anchors",
        json!({"author_test_policy":issuer.policy(),"signed_statement":statement,"assessment":view,"record":work_record}),
    );
    let registry_path = root.join("consumer.db");
    let mut authored = d::catalog();
    assert!(authored.decision_policy.is_none());
    authored.decision_policy = Some(issuer.policy());
    let mut registry = Registry::new(SqliteRepository::open(&registry_path).unwrap());
    let consumer_id = registry
        .register(Declaration {
            label: n("explicit-test-issuer-consumer"),
            catalog: authored.reference().unwrap(),
        })
        .unwrap()
        .registration
        .id;
    let mut consumer = d::Consumer::open(registry, consumer_id.clone(), authored.clone()).unwrap();
    let source_ref = d::RegistrationRef::from_registration(&provider.query().unwrap().registration);
    let binding = consumer
        .track(
            n("snapshot-after-preparation"),
            n("test/diagnostics"),
            Some(source_ref),
        )
        .unwrap();
    let run = consumer
        .assign(&binding.id, &mut provider)
        .unwrap()
        .run
        .unwrap();
    consumer.begin(&binding.id, &run.id, &mut provider).unwrap();
    let result = consumer
        .finish(&binding.id, &run.id, &mut provider)
        .unwrap()
        .result
        .unwrap();
    let capture = Capture::default();
    consumer
        .assess_binding(&binding.id, d::AcceptanceKind::Initial, None, &capture)
        .unwrap();
    let binding_request = capture.request.into_inner().unwrap();
    let (_, initial_statement, initial_proof) = signed(&issuer, &binding_request, 20_000);
    let initial = consumer
        .assess_binding(
            &binding.id,
            d::AcceptanceKind::Initial,
            None,
            &Cached(initial_proof.clone()),
        )
        .unwrap();
    assert_eq!(initial.state(), WorkUseState::Verified);
    let initial_record = consumer.record_verified_decision(&initial_proof).unwrap();
    let work_as_binding = consumer
        .assess_binding(
            &binding.id,
            d::AcceptanceKind::Initial,
            None,
            &Cached(proof.clone()),
        )
        .unwrap();
    assert_eq!(work_as_binding.state(), WorkUseState::Denied);
    assert!(work_as_binding.reason().contains("decision/kind"));
    let binding_as_work = provider
        .assess_use(scope(), &Cached(initial_proof.clone()))
        .unwrap();
    assert_eq!(
        binding_as_work.work_use_permission.state(),
        WorkUseState::Denied
    );
    assert!(
        binding_as_work.work_use_permission.conditions()[0]
            .reason
            .contains("decision/kind")
    );
    emit(
        "initial-binding-verified-without-work-promotion",
        "separate initial-binding decision verifies; work and binding proofs are rejected in both opposite directions; old diagnostic result retains UNSUPPORTED work use",
        json!({"signed_statement":initial_statement,"assessment":initial,"record":initial_record,"work_as_binding":work_as_binding,"binding_as_work":binding_as_work,"original_result":result}),
    );
    let (mut candidate, candidate_owner) = open_provider(&root.join("candidate"), &issuer);
    ready(&mut candidate);
    let candidate_ref =
        d::RegistrationRef::from_registration(&candidate.query().unwrap().registration);
    let candidate_generation = generation(&mut candidate);
    let before = consumer
        .inspect(&binding.id, None, Some(&result.id), &mut provider)
        .unwrap()
        .binding;
    let capture = Capture::default();
    consumer
        .assess_binding_with_generation(
            &binding.id,
            d::AcceptanceKind::Replacement,
            Some(candidate_ref.clone()),
            Some(candidate_generation.clone()),
            &capture,
        )
        .unwrap();
    let replacement_request = capture.request.into_inner().unwrap();
    let (_, replacement_statement, replacement_proof) =
        signed(&issuer, &replacement_request, 20_000);
    let replacement = consumer
        .assess_binding_with_generation(
            &binding.id,
            d::AcceptanceKind::Replacement,
            Some(candidate_ref.clone()),
            Some(candidate_generation.clone()),
            &Cached(replacement_proof),
        )
        .unwrap();
    assert_eq!(replacement.state(), WorkUseState::Verified);
    assert_eq!(
        consumer
            .inspect(&binding.id, None, Some(&result.id), &mut provider)
            .unwrap()
            .binding,
        before
    );
    assert_eq!(consumer.recorded_result(&result.id).unwrap(), result);
    emit(
        "replacement-decision-verified-without-automatic-rebind",
        "separate signature covers the old binding and proposed registration/generation; verification does not apply replacement, change original results or grant work use",
        json!({"signed_statement":replacement_statement,"assessment":replacement,"binding_unchanged":before}),
    );
    let another_registration = candidate
        .assess_use(scope(), &Cached(proof.clone()))
        .unwrap();
    assert_eq!(
        another_registration.work_use_permission.state(),
        WorkUseState::Denied
    );
    let different_scope = provider
        .assess_use(
            UseScope {
                operating_area: n("test/diagnostics"),
                role: n("diagnostics/operator-connected"),
            },
            &Cached(proof.clone()),
        )
        .unwrap();
    assert_eq!(
        different_scope.work_use_permission.state(),
        WorkUseState::Denied
    );
    provider.request_stop().unwrap();
    stopped(&mut provider);
    provider.rearm_software().unwrap();
    ready(&mut provider);
    let different_generation = provider
        .assess_use(scope(), &Cached(proof.clone()))
        .unwrap();
    assert_eq!(
        different_generation.work_use_permission.state(),
        WorkUseState::Denied
    );
    emit(
        "decision-reuse-rejected",
        "original verified proof cannot authorize a different registration, scope/role or provider execution generation",
        json!({"registration":another_registration,"scope":different_scope,"generation":different_generation}),
    );
    let r = request(&mut provider);
    let (c, statement, short) = signed(&issuer, &r, 200);
    let short_record = provider.record_verified_decision(&short).unwrap();
    let history = provider.history().unwrap();
    std::thread::sleep(Duration::from_millis(250));
    let expired = provider.assess_use(scope(), &Cached(short)).unwrap();
    assert_eq!(expired.work_use_permission.state(), WorkUseState::Denied);
    assert!(
        expired.work_use_permission.conditions()[0]
            .reason
            .contains("decision/expired")
    );
    assert_eq!(c.verify(&statement).unwrap_err(), Failure::Expired);
    assert_eq!(
        provider
            .recorded_decision(&short_record.reference.decision)
            .unwrap(),
        short_record
    );
    assert_eq!(provider.history().unwrap(), history);
    emit(
        "expired-decision-rejected",
        "local monotonic lifetime starts at challenge creation; old signed bytes cannot renew it and historical decision remains unchanged",
        json!({"assessment":expired,"original_record":short_record}),
    );
    let r = request(&mut provider);
    let (c, statement, live) = signed(&issuer, &r, 5000);
    let record = provider.record_verified_decision(&live).unwrap();
    let revocation = issuer.revocation(live.revocation_claim(n("test/operator-revoked")));
    let verified_revocation = live.revoke(&revocation).unwrap();
    provider
        .record_verified_revocation(&verified_revocation)
        .unwrap();
    assert_eq!(c.verify(&statement).unwrap_err(), Failure::Revoked);
    let revoked = provider.assess_use(scope(), &Cached(live)).unwrap();
    assert_eq!(revoked.work_use_permission.state(), WorkUseState::Denied);
    assert!(
        revoked.work_use_permission.conditions()[0]
            .reason
            .contains("decision/revoked")
    );
    assert_eq!(
        provider
            .recorded_decision(&record.reference.decision)
            .unwrap(),
        record
    );
    emit(
        "revoked-decision-rejected-with-history-preserved",
        "signed exact-decision revocation invalidates subsequent checks and reimports; original decision and diagnostic result are not rewritten",
        json!({"revocation":revocation,"assessment":revoked,"original_record":record,"original_result":consumer.recorded_result(&result.id).unwrap()}),
    );
    drop(consumer.into_registry());
    assert_eq!(initial_proof.check_live().unwrap_err(), Failure::Epoch);
    let mut reopened = d::Consumer::open(
        Registry::new(SqliteRepository::open(&registry_path).unwrap()),
        consumer_id,
        authored,
    )
    .unwrap();
    let after_restart = reopened
        .assess_binding(
            &binding.id,
            d::AcceptanceKind::Initial,
            None,
            &Cached(initial_proof),
        )
        .unwrap();
    assert_eq!(after_restart.state(), WorkUseState::Denied);
    assert_eq!(
        reopened
            .recorded_decision(&initial_record.reference.decision)
            .unwrap(),
        initial_record
    );
    emit(
        "decision-record-does-not-restore-live-proof",
        "reopened consumer has a new verifier epoch; old references remain readable while retained old proof is rejected",
        json!({"assessment":after_restart,"historical_record":initial_record}),
    );
    provider.request_stop().unwrap();
    stopped(&mut provider);
    candidate.request_stop().unwrap();
    stopped(&mut candidate);
    assert!(owner.0.borrow().owned_instances().is_empty());
    assert!(candidate_owner.0.borrow().owned_instances().is_empty());
}
