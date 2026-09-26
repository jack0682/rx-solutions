#![cfg(unix)]
#[path = "support/external_signer.rs"]
mod external_signer;
#[path = "support/decision_fixture.rs"]
mod fixture;
use external_signer::ExternalSigner;
use fixture::*;
use rx_domain::types::*;
use rx_storage::SqliteRepository;
use rx_supervisor::{
    decision::*,
    registration::{Declaration, Registry, diagnostic::*},
    use_assessment::*,
};
use std::{cell::RefCell, time::Duration};
#[derive(Default)]
struct Capture {
    request: RefCell<Option<Request>>,
}
impl WorkUsePort for Capture {
    fn assess(&self, r: &WorkUseRequest) -> WorkUseReply {
        *self.request.borrow_mut() = r.decision_request().cloned();
        WorkUseReply::Unsupported {
            conditions: [(n("test/capture"), "capture unsigned request only".into())].into(),
        }
    }
}
impl BindingJudgment for Capture {
    fn assess(&self, r: &AcceptanceRequest) -> AcceptanceReply {
        *self.request.borrow_mut() = r.decision_request().cloned();
        AcceptanceReply::Unsupported {
            condition: n("test/capture"),
            reason: "capture unsigned request only".into(),
        }
    }
}
struct Cached(VerifiedDecision);
impl WorkUsePort for Cached {
    fn assess(&self, _: &WorkUseRequest) -> WorkUseReply {
        WorkUseReply::Verified(Box::new(self.0.clone()))
    }
}
impl BindingJudgment for Cached {
    fn assess(&self, _: &AcceptanceRequest) -> AcceptanceReply {
        AcceptanceReply::Verified(Box::new(self.0.clone()))
    }
}
fn scope(role: &str) -> UseScope {
    UseScope {
        operating_area: n("test/diagnostics"),
        role: n(role),
    }
}
fn work(f: &mut Fixture, role: &str) -> Request {
    let c = Capture::default();
    f.manager().assess_use(scope(role), &c).unwrap();
    c.request.into_inner().unwrap()
}
fn issue(
    s: &ExternalSigner,
    r: &Request,
    ttl: u64,
) -> (Challenge, SignedDecision, VerifiedDecision) {
    let c = r.challenge(&s.key_id()).unwrap();
    let signed = s.decision(c.claim(uid(), Counter(ttl)));
    let p = c.verify(&signed).unwrap();
    (c, signed, p)
}
fn consumer(f: &Fixture, s: &ExternalSigner) -> (Consumer<SqliteRepository>, Id, TrackedBinding) {
    let mut authored = catalog();
    authored.decision_policy = Some(s.policy());
    let mut registry =
        Registry::new(SqliteRepository::open(f.dir.path().join("consumer.db")).unwrap());
    let id = registry
        .register(Declaration {
            label: n("consumer"),
            catalog: authored.reference().unwrap(),
        })
        .unwrap()
        .registration
        .id;
    let mut c = Consumer::open(registry, id.clone(), authored).unwrap();
    let b = c
        .track(
            n("snapshot-after-preparation"),
            n("test/diagnostics"),
            Some(f.reference.clone()),
        )
        .unwrap();
    (c, id, b)
}
fn bind_request(
    c: &mut Consumer<SqliteRepository>,
    id: &Id,
    kind: AcceptanceKind,
    provider: Option<RegistrationRef>,
    generation: Option<Generation>,
) -> Request {
    let capture = Capture::default();
    c.assess_binding_with_generation(id, kind, provider, generation, &capture)
        .unwrap();
    capture.request.into_inner().unwrap()
}
static CASES: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn external_work_decision_does_not_promote_readiness_execution_or_binding() {
    let _g = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let issuer = ExternalSigner::new();
    let mut f = Fixture::new(Some(issuer.policy()));
    f.ready();
    let before = f.manager().state().unwrap();
    let history = f.manager().history().unwrap();
    let request = work(&mut f, "diagnostics/operator-connected");
    let (_, _, proof) = issue(&issuer, &request, 5000);
    let view = f
        .manager()
        .assess_use(
            scope("diagnostics/operator-connected"),
            &Cached(proof.clone()),
        )
        .unwrap();
    assert_eq!(view.work_use_permission.state(), WorkUseState::Verified);
    assert_eq!(view.functional_readiness.state(), ConditionState::NotMet);
    assert_eq!(
        serde_json::to_value(f.manager().state().unwrap()).unwrap(),
        serde_json::to_value(before).unwrap()
    );
    assert_eq!(f.manager().history().unwrap(), history);
    let (mut c, _, b) = consumer(&f, &issuer);
    c.assign(&b.id, f.manager()).unwrap();
    let denied = c
        .assess_binding(&b.id, AcceptanceKind::Initial, None, &Cached(proof))
        .unwrap();
    assert_eq!(denied.state(), WorkUseState::Denied);
    assert!(denied.reason().contains("decision/kind"));
}

#[test]
fn initial_and_replacement_acceptance_are_distinct_and_do_not_grant_work_or_apply_binding() {
    let _g = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let issuer = ExternalSigner::new();
    let mut f = Fixture::new(Some(issuer.policy()));
    let mut replacement = Fixture::new(Some(issuer.policy()));
    f.ready();
    replacement.ready();
    let (mut c, _, b) = consumer(&f, &issuer);
    let run = c.assign(&b.id, f.manager()).unwrap().run.unwrap();
    c.begin(&b.id, &run.id, f.manager()).unwrap();
    let old_result = c
        .finish(&b.id, &run.id, f.manager())
        .unwrap()
        .result
        .unwrap();
    let before = c
        .inspect(&b.id, None, Some(&old_result.id), f.manager())
        .unwrap()
        .binding;
    let initial = bind_request(&mut c, &b.id, AcceptanceKind::Initial, None, None);
    let (_, _, proof) = issue(&issuer, &initial, 5000);
    assert_eq!(
        c.assess_binding(&b.id, AcceptanceKind::Initial, None, &Cached(proof.clone()))
            .unwrap()
            .state(),
        WorkUseState::Verified
    );
    let denied = f
        .manager()
        .assess_use(scope("diagnostics/support-summary"), &Cached(proof.clone()))
        .unwrap();
    assert_eq!(denied.work_use_permission.state(), WorkUseState::Denied);
    assert!(
        denied.work_use_permission.conditions()[0]
            .reason
            .contains("decision/kind")
    );
    let candidate = replacement.reference.clone();
    let generation = replacement.generation();
    let request = bind_request(
        &mut c,
        &b.id,
        AcceptanceKind::Replacement,
        Some(candidate.clone()),
        Some(generation.clone()),
    );
    let (_, _, replacement_proof) = issue(&issuer, &request, 5000);
    assert_eq!(
        c.assess_binding_with_generation(
            &b.id,
            AcceptanceKind::Replacement,
            Some(candidate.clone()),
            Some(generation.clone()),
            &Cached(proof)
        )
        .unwrap()
        .state(),
        WorkUseState::Denied
    );
    assert_eq!(
        c.assess_binding_with_generation(
            &b.id,
            AcceptanceKind::Replacement,
            Some(candidate.clone()),
            Some(generation),
            &Cached(replacement_proof.clone())
        )
        .unwrap()
        .state(),
        WorkUseState::Verified
    );
    assert_eq!(
        c.assess_binding(
            &b.id,
            AcceptanceKind::Replacement,
            Some(candidate),
            &Cached(replacement_proof.clone())
        )
        .unwrap()
        .state(),
        WorkUseState::Denied
    );
    assert_eq!(
        c.assess_binding(
            &b.id,
            AcceptanceKind::Initial,
            None,
            &Cached(replacement_proof)
        )
        .unwrap()
        .state(),
        WorkUseState::Denied
    );
    assert_eq!(
        c.inspect(&b.id, None, Some(&old_result.id), f.manager())
            .unwrap()
            .binding,
        before
    );
    assert_eq!(c.recorded_result(&old_result.id).unwrap(), old_result);
    assert_eq!(
        serde_json::to_value(old_result).unwrap()["work_use"],
        "UNSUPPORTED"
    );
}

#[test]
fn signature_key_context_and_wire_shape_attacks_are_rejected() {
    let _g = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let issuer = ExternalSigner::new();
    let attacker = ExternalSigner::new();
    let mut f = Fixture::new(Some(issuer.policy()));
    f.ready();
    let r = work(&mut f, "diagnostics/support-summary");
    let c = r.challenge(&issuer.key_id()).unwrap();
    let signed = issuer.decision(c.claim(uid(), Counter(5000)));
    let mut unsigned = signed.clone();
    unsigned.signature.signature = "00".repeat(64);
    assert_eq!(c.verify(&unsigned).unwrap_err(), Failure::Signature);
    let wrong = attacker.decision(signed.claim.clone());
    assert_eq!(c.verify(&wrong).unwrap_err(), Failure::Signature);
    let mut alias = signed.clone();
    alias.signature.key = n("test/alias");
    assert_eq!(c.verify(&alias).unwrap_err(), Failure::IssuerScope);
    let mut mutated = signed.clone();
    mutated.claim.challenge.owner.registration = uid();
    assert_eq!(c.verify(&mutated).unwrap_err(), Failure::Context);
    let other = r.challenge(&issuer.key_id()).unwrap();
    assert_eq!(other.verify(&signed).unwrap_err(), Failure::Context);
    let mut wire = serde_json::to_value(&signed).unwrap();
    wire["claim"]["remote_expiry"] = "2099-01-01".into();
    assert!(SignedDecision::decode(&serde_json::to_vec(&wire).unwrap()).is_err());
    wire["claim"]
        .as_object_mut()
        .unwrap()
        .remove("remote_expiry");
    wire["claim"]["verdict"] = "ACKNOWLEDGED".into();
    assert!(SignedDecision::decode(&serde_json::to_vec(&wire).unwrap()).is_err());
    assert!(SignedDecision::decode(&vec![b' '; 131073]).is_err());
    assert!(c.verify(&signed).is_ok());
}

#[test]
fn exact_registration_scope_generation_and_binding_are_required() {
    let _g = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let issuer = ExternalSigner::new();
    let mut f = Fixture::new(Some(issuer.policy()));
    let mut other = Fixture::new(Some(issuer.policy()));
    f.ready();
    other.ready();
    let r = work(&mut f, "diagnostics/support-summary");
    let (_, _, proof) = issue(&issuer, &r, 5000);
    assert_eq!(
        other
            .manager()
            .assess_use(scope("diagnostics/support-summary"), &Cached(proof.clone()))
            .unwrap()
            .work_use_permission
            .state(),
        WorkUseState::Denied
    );
    assert_eq!(
        f.manager()
            .assess_use(
                scope("diagnostics/operator-connected"),
                &Cached(proof.clone())
            )
            .unwrap()
            .work_use_permission
            .state(),
        WorkUseState::Denied
    );
    let wrong = UseScope {
        operating_area: n("test/another-area"),
        role: n("diagnostics/support-summary"),
    };
    assert_eq!(
        f.manager()
            .assess_use(wrong, &Cached(proof.clone()))
            .unwrap()
            .work_use_permission
            .state(),
        WorkUseState::Denied
    );
    f.restart();
    assert_eq!(
        f.manager()
            .assess_use(scope("diagnostics/support-summary"), &Cached(proof))
            .unwrap()
            .work_use_permission
            .state(),
        WorkUseState::Denied
    );
    let (mut c, _, b) = consumer(&f, &issuer);
    c.assign(&b.id, f.manager()).unwrap();
    let r = bind_request(&mut c, &b.id, AcceptanceKind::Initial, None, None);
    let (_, _, proof) = issue(&issuer, &r, 5000);
    let another = c
        .track(
            b.profile.clone(),
            b.operating_area.clone(),
            b.provider.clone(),
        )
        .unwrap();
    c.assign(&another.id, f.manager()).unwrap();
    assert_eq!(
        c.assess_binding(&another.id, AcceptanceKind::Initial, None, &Cached(proof))
            .unwrap()
            .state(),
        WorkUseState::Denied
    );
}

#[test]
fn expiry_is_anchored_to_challenge_and_ttl_cannot_exceed_author_policy() {
    let _g = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let issuer = ExternalSigner::new();
    let mut f = Fixture::new(Some(issuer.policy()));
    f.ready();
    let r = work(&mut f, "diagnostics/support-summary");
    for ttl in [0, 30001] {
        let c = r.challenge(&issuer.key_id()).unwrap();
        let s = issuer.decision(c.claim(uid(), Counter(ttl)));
        assert_eq!(c.verify(&s).unwrap_err(), Failure::Ttl);
    }
    let c = r.challenge(&issuer.key_id()).unwrap();
    let s = issuer.decision(c.claim(uid(), Counter(30)));
    std::thread::sleep(Duration::from_millis(60));
    assert_eq!(c.verify(&s).unwrap_err(), Failure::Expired);
    let (c, s, proof) = issue(&issuer, &r, 200);
    let clone = proof.clone();
    let reimported = c.verify(&s).unwrap();
    let record = f.manager().record_verified_decision(&proof).unwrap();
    let history = f.manager().history().unwrap();
    std::thread::sleep(Duration::from_millis(250));
    for p in [&proof, &clone, &reimported] {
        assert_eq!(p.check_live().unwrap_err(), Failure::Expired);
    }
    assert_eq!(c.verify(&s).unwrap_err(), Failure::Expired);
    assert_eq!(
        f.manager()
            .assess_use(scope("diagnostics/support-summary"), &Cached(proof))
            .unwrap()
            .work_use_permission
            .state(),
        WorkUseState::Denied
    );
    assert_eq!(
        f.manager()
            .recorded_decision(&record.reference.decision)
            .unwrap(),
        record
    );
    assert_eq!(f.manager().history().unwrap(), history);
}

#[test]
fn signed_revocation_blocks_clones_and_reimport_without_rewriting_history() {
    let _g = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let issuer = ExternalSigner::new();
    let attacker = ExternalSigner::new();
    let mut f = Fixture::new(Some(issuer.policy()));
    f.ready();
    let r = work(&mut f, "diagnostics/support-summary");
    let (c, s, proof) = issue(&issuer, &r, 5000);
    let clone = proof.clone();
    let record = f.manager().record_verified_decision(&proof).unwrap();
    let old = f.manager().history().unwrap();
    let revocation = issuer.revocation(proof.revocation_claim(n("operator/revoked")));
    let forged = attacker.revocation(revocation.claim.clone());
    assert_eq!(proof.revoke(&forged).unwrap_err(), Failure::Signature);
    assert!(proof.check_live().is_ok());
    let receipt = proof.revoke(&revocation).unwrap();
    assert_eq!(clone.check_live().unwrap_err(), Failure::Revoked);
    assert_eq!(c.verify(&s).unwrap_err(), Failure::Revoked);
    f.manager().record_verified_revocation(&receipt).unwrap();
    assert_eq!(
        f.manager()
            .recorded_decision(&record.reference.decision)
            .unwrap(),
        record
    );
    assert_eq!(&f.manager().history().unwrap()[..old.len()], &old);
    assert_eq!(
        f.manager()
            .assess_use(scope("diagnostics/support-summary"), &Cached(clone))
            .unwrap()
            .work_use_permission
            .state(),
        WorkUseState::Denied
    );
}

#[test]
fn receiver_loss_and_unconfigured_catalogs_cannot_restore_permission() {
    let _g = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let issuer = ExternalSigner::new();
    let mut f = Fixture::new(Some(issuer.policy()));
    let mut unconfigured = Fixture::new(None);
    f.ready();
    unconfigured.ready();
    let r = work(&mut f, "diagnostics/support-summary");
    let (_, _, proof) = issue(&issuer, &r, 5000);
    let record = f.manager().record_verified_decision(&proof).unwrap();
    let capture = Capture::default();
    unconfigured
        .manager()
        .assess_use(scope("diagnostics/support-summary"), &capture)
        .unwrap();
    assert!(capture.request.borrow().is_none());
    assert_eq!(
        unconfigured
            .manager()
            .assess_use(scope("diagnostics/support-summary"), &Cached(proof.clone()))
            .unwrap()
            .work_use_permission
            .state(),
        WorkUseState::Denied
    );
    let manager = f.managed.take().unwrap();
    let (_, _, _, mut registry) = manager.into_parts();
    assert_eq!(proof.check_live().unwrap_err(), Failure::Epoch);
    assert!(matches!(r.challenge(&issuer.key_id()), Err(Failure::Epoch)));
    assert_eq!(
        registry
            .recorded_decision(
                &record.reference.owner.registration,
                &record.reference.decision
            )
            .unwrap(),
        record
    );
}

#[test]
fn a_signed_decision_id_cannot_be_redefined_or_extended() {
    let _g = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let issuer = ExternalSigner::new();
    let mut f = Fixture::new(Some(issuer.policy()));
    f.ready();
    let r = work(&mut f, "diagnostics/support-summary");
    let (c, s, proof) = issue(&issuer, &r, 5000);
    let mut extended = s.claim.clone();
    extended.ttl_ms = Counter(6000);
    let extended = issuer.decision(extended);
    assert_eq!(c.verify(&extended).unwrap_err(), Failure::Identity);
    let next = r.challenge(&issuer.key_id()).unwrap();
    let reused = issuer.decision(next.claim(proof.reference().decision.clone(), Counter(5000)));
    assert_eq!(next.verify(&reused).unwrap_err(), Failure::Identity);
    assert!(proof.check_live().is_ok());
    assert!(c.verify(&s).is_ok());
}

#[test]
fn changed_program_anchor_cannot_reopen_an_existing_registration() {
    let _g = CASES.lock().unwrap_or_else(|e| e.into_inner());
    let issuer = ExternalSigner::new();
    let attacker = ExternalSigner::new();
    let mut f = Fixture::new(Some(issuer.policy()));
    let original = f.reference.clone();
    let mut program = f.program.clone();
    program.decision_policy = Some(attacker.policy());
    let manager = f.managed.take().unwrap();
    let (store, backend, authority, registry) = manager.into_parts();
    let support = rx_solution_catalog::DeviceCatalog::decode(include_bytes!(
        "../../../catalogs/device-support.v1.json"
    ))
    .unwrap();
    let error = match Managed::open(
        store,
        backend,
        authority,
        f.plan.clone(),
        [(program.id.clone(), program)].into(),
        &support,
        registry,
        f.reference.registration.clone(),
    ) {
        Ok(_) => panic!("changed author anchor must not reuse the registration"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("program/catalog declaration differs")
    );
    let mut registry =
        Registry::new(SqliteRepository::open(f.dir.path().join("registration.db")).unwrap());
    assert_eq!(
        RegistrationRef::from_registration(
            &registry.query(&original.registration).unwrap().registration
        ),
        original
    );
}
