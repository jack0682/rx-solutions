//! Offline signed work decisions. The host never loads a private key or launches an issuer.
//! External callers cannot access or create receiving capabilities.
//!
//! ```compile_fail
//! fn leak(r: &rx_supervisor::decision::Request) { let _ = &r.policy; }
//! ```
//! ```compile_fail
//! let _ = rx_supervisor::decision::Request::new();
//! ```
//! ```compile_fail
//! fn leak(p: &rx_supervisor::work_use::Prepared) { let _ = &p.proof; }
//! ```
//! ```compile_fail
//! let _ = rx_supervisor::work_use::Prepared::new();
//! ```
//! ```compile_fail
//! fn leak(p: &rx_supervisor::decision::VerifiedDecision) { let _ = &p.runtime; }
//! ```
//! ```compile_fail
//! let _ = rx_supervisor::decision::VerifiedDecision::new();
//! ```
use crate::{Error, Result, decision::*, use_assessment::*, work_use::Prepared};
use rx_domain::{canonical, types::*};
use rx_package::operating_area as authored;
use rx_storage::mailbox::{Mailbox, MailboxGuard};
use serde::Serialize;
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    time::Instant,
};
fn n(s: &str) -> Name {
    Name::new(s).expect("literal")
}
#[derive(Clone, Debug, Serialize)]
pub struct ConnectionObservation {
    pub state: &'static str,
    pub challenge: Option<Id>,
    pub condition: Option<String>,
    pub meaning: &'static str,
}
struct Session {
    context: Digest,
    challenge: Challenge,
    started: Instant,
    proof: Option<VerifiedDecision>,
    published: bool,
}
pub struct OfflineWorkUseProvider {
    directory: PathBuf,
    session: RefCell<Option<Session>>,
    observation: RefCell<ConnectionObservation>,
}
impl OfflineWorkUseProvider {
    pub fn new(directory: impl AsRef<Path>) -> Self {
        Self {
            directory: directory.as_ref().into(),
            session: RefCell::new(None),
            observation: RefCell::new(ConnectionObservation {
                state: "CONFIGURED_NOT_CONNECTED",
                challenge: None,
                condition: None,
                meaning: "OFFLINE_CHECKPOINT_OBSERVATION; NOT_CURRENT_PERMISSION_OR_PHYSICAL_AUTHORITY",
            }),
        }
    }
    pub fn observation(&self) -> ConnectionObservation {
        self.observation.borrow().clone()
    }
    pub fn begin_attempt(&self) {
        self.observe("NOT_ASSESSED_AT_CURRENT_BOUNDARY", None);
    }
    pub fn pending(&self) -> bool {
        self.observation.borrow().state == "AWAITING_EXTERNAL_DECISION"
    }
    fn observe(&self, state: &'static str, condition: Option<&str>) {
        let mut o = self.observation.borrow_mut();
        o.state = state;
        o.condition = condition.map(str::to_owned);
    }
    fn denial(&self, condition: &str, reason: impl ToString) -> WorkUseReply {
        self.observe("REFUSED", Some(condition));
        WorkUseReply::Denied {
            decision_reference: n("offline/work-decision-refused"),
            conditions: [(n(condition), reason.to_string())].into(),
        }
    }
    fn failed(&self, failure: Failure) -> WorkUseReply {
        let condition = match failure {
            Failure::IssuerScope => "work-use/issuer-scope",
            Failure::Expired => "work-use/ttl-expired",
            Failure::Signature => "work-use/signature-mismatch",
            Failure::Revoked => "work-use/revoked",
            Failure::Context => "work-use/context-mismatch",
            Failure::Kind => "work-use/role-kind-mismatch",
            Failure::Epoch => "work-use/receiver-epoch",
            Failure::Encoding => "work-use/encoding",
            Failure::Ttl => "work-use/ttl-invalid",
            _ => "work-use/receiver-failure",
        };
        self.denial(condition, failure)
    }
    fn refresh(
        guard: &MailboxGuard,
        challenge: &Id,
        proof: &VerifiedDecision,
    ) -> std::result::Result<(), Failure> {
        if let Some(bytes) = guard
            .read(&format!("revocation-{challenge}.json"), 131_072)
            .map_err(|_| Failure::Receiver)?
        {
            let signed: SignedRevocation =
                canonical::decode_json(&bytes).map_err(|_| Failure::Encoding)?;
            proof.revoke(&signed)?;
        }
        proof.check_live()
    }
    /// The cooperating publication lock spans final revocation acquisition and
    /// the entire caller's F10 transaction/physical commit. No retry on refusal.
    pub fn commit_checkpoint<T>(
        &self,
        prepared: &Prepared,
        commit: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let session = self.session.borrow();
        let session = session
            .as_ref()
            .ok_or_else(|| Error::Invalid("work-use/unconnected".into()))?;
        let proof = session
            .proof
            .as_ref()
            .ok_or_else(|| Error::Invalid("work-use/no-verified-decision".into()))?;
        if proof.reference() != prepared.decision().reference() {
            return Err(Error::Invalid("work-use/context-mismatch".into()));
        }
        let mailbox = Mailbox::open(&self.directory)
            .map_err(|e| Error::Invalid(format!("work-use/mailbox-unavailable: {e}")))?;
        let _guard = mailbox
            .lock()
            .map_err(|e| Error::Invalid(format!("work-use/mailbox-unavailable: {e}")))?;
        if let Err(e) = Self::refresh(&_guard, &session.challenge.info().id, proof) {
            let _ = self.failed(e.clone());
            return Err(Error::Invalid(format!("work-use/{e}")));
        }
        let result = commit();
        self.observe(
            if result.is_ok() {
                "COMMIT_CALLBACK_COMPLETED"
            } else {
                "COMMIT_REFUSED"
            },
            None,
        );
        result
    }
}
/// Catalog programs cannot enter the legacy single-area fallback. This is
/// selected from the owner's sealed subject, never from requested area or reply.
pub(crate) fn receiver_binding(program: &str) -> crate::Result<&'static authored::Area> {
    Ok(authored::for_program(program)
        .map_err(|e| Error::Invalid(e.into()))?
        .unwrap_or(&authored::SUPPORT))
}

impl WorkUsePort for OfflineWorkUseProvider {
    fn assess(&self, request: &WorkUseRequest) -> WorkUseReply {
        let Some(decision) = request.decision_request() else {
            return self.denial(
                "work-use/unconnected",
                "no authored receiving request; host is not an issuer",
            );
        };
        let context = match canonical::digest("RX-OFFLINE-WORK-REQUEST-v1", decision) {
            Ok(v) => v,
            Err(_) => return self.failed(Failure::Encoding),
        };
        let mut slot = self.session.borrow_mut();
        if slot.is_none() {
            let started = Instant::now();
            let binding = match receiver_binding(request.subject().program.as_str()) {
                Ok(value) => value,
                Err(e) => return self.denial("work-use/area-catalog-invalid", e),
            };
            let challenge = match decision.challenge(&n(binding.key_id)) {
                Ok(v) => v,
                Err(e) => return self.failed(e),
            };
            self.observation.borrow_mut().challenge = Some(challenge.info().id.clone());
            *slot = Some(Session {
                context,
                challenge,
                started,
                proof: None,
                published: false,
            });
        }
        let session = slot.as_mut().expect("created session");
        if session.context != context {
            return self.failed(Failure::Context);
        }
        let id = session.challenge.info().id.clone();
        let mailbox = match Mailbox::open(&self.directory) {
            Ok(v) => v,
            Err(e) => return self.denial("work-use/unconnected", e),
        };
        let guard = match mailbox.lock() {
            Ok(v) => v,
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    && session.published
                    && session.proof.is_none()
                    && session.started.elapsed().as_millis()
                        < u128::from(session.challenge.info().max_ttl_ms.0) =>
            {
                // An already published request is still awaiting a complete reply.
                // This does not retry a failed use or create/renew a challenge.
                self.observe(
                    "AWAITING_EXTERNAL_DECISION",
                    Some("work-use/pending-publication"),
                );
                return WorkUseReply::NotEvaluated { conditions: [(n("work-use/pending-publication"), "cooperating issuer publication in progress; original deadline retained; no use attempted".into())].into() };
            }
            Err(e) => return self.denial("work-use/mailbox-unavailable", e),
        };
        let bytes = match canonical::bytes(session.challenge.info()) {
            Ok(v) => v,
            Err(_) => return self.failed(Failure::Encoding),
        };
        if let Err(e) = guard.publish_once(&format!("request-{id}.json"), &bytes) {
            return self.denial("work-use/mailbox-unavailable", e);
        }
        session.published = true;
        let read = |prefix: &str| guard.read(&format!("{prefix}-{id}.json"), 131_072);
        let denial = match read("denial") {
            Ok(v) => v,
            Err(e) => return self.denial("work-use/mailbox-unavailable", e),
        };
        let response = match read("decision") {
            Ok(v) => v,
            Err(e) => return self.denial("work-use/mailbox-unavailable", e),
        };
        if denial.is_some() && response.is_some() {
            return self.denial(
                "work-use/mailbox-conflict",
                "both approval and denial published",
            );
        }
        if let Some(bytes) = denial {
            let value: authored::Denial = match canonical::decode_json(&bytes) {
                Ok(v) => v,
                Err(_) => return self.failed(Failure::Encoding),
            };
            if value.challenge != id {
                return self.failed(Failure::Context);
            }
            return self.denial(
                "work-use/judge-policy-denied",
                format!(
                    "unverified negative issuer report; {}: {}",
                    value.rule, value.reason
                ),
            );
        }
        if let Some(proof) = &session.proof {
            if let Err(e) = Self::refresh(&guard, &id, proof) {
                return self.failed(e);
            }
            self.observe("EXTERNAL_DECISION_VERIFIED", None);
            return WorkUseReply::Verified(Box::new(proof.clone()));
        }
        let Some(response) = response else {
            if session.started.elapsed().as_millis()
                >= u128::from(session.challenge.info().max_ttl_ms.0)
            {
                return self.denial(
                    "work-use/unconnected",
                    "no external decision within original challenge lifetime; no automatic renewal",
                );
            }
            self.observe("AWAITING_EXTERNAL_DECISION", None);
            return WorkUseReply::NotEvaluated {
                conditions: [(
                    n("work-use/pending-external-decision"),
                    "original nonce and deadline retained; no work use attempted".into(),
                )]
                .into(),
            };
        };
        let signed = match SignedDecision::decode(&response) {
            Ok(v) => v,
            Err(e) => return self.failed(e),
        };
        let info = session.challenge.info();
        if signed.signature.key != info.key {
            return self.denial(
                "work-use/issuer-scope",
                "untrusted response key is outside authored issuer scope",
            );
        }
        if signed.claim.challenge.operating_area != info.operating_area {
            return self.denial(
                "work-use/operating-area-mismatch",
                "untrusted claimed area differs; issuer identity not verified",
            );
        }
        if signed.claim.challenge.role != info.role || signed.claim.challenge.kind != info.kind {
            return self.denial(
                "work-use/role-kind-mismatch",
                "untrusted claimed role/kind differs; issuer identity not verified",
            );
        }
        let proof = match session.challenge.verify(&signed) {
            Ok(v) => v,
            Err(e) => return self.failed(e),
        };
        let reference = match canonical::bytes(proof.reference()) {
            Ok(v) => v,
            Err(_) => return self.failed(Failure::Encoding),
        };
        if let Err(e) = guard.publish_once(&format!("reference-{id}.json"), &reference) {
            return self.denial("work-use/mailbox-unavailable", e);
        }
        if let Err(e) = Self::refresh(&guard, &id, &proof) {
            return self.failed(e);
        }
        session.proof = Some(proof.clone());
        self.observe("EXTERNAL_DECISION_VERIFIED", None);
        WorkUseReply::Verified(Box::new(proof))
    }
}
/// Configuration is only an offline directory. No key, policy, issuer selector
/// or signer command is accepted from site data.
pub enum ResidentWorkProvider {
    Unconfigured,
    Offline(Box<OfflineWorkUseProvider>),
}
impl ResidentWorkProvider {
    pub fn begin_attempt(&self) {
        if let Self::Offline(provider) = self {
            provider.begin_attempt();
        }
    }

    pub fn pending(&self) -> bool {
        matches!(self,Self::Offline(p) if p.pending())
    }
    pub fn observation(&self) -> serde_json::Value {
        match self {
            Self::Unconfigured => serde_json::json!("NOT_CONNECTED"),
            Self::Offline(p) => {
                serde_json::to_value(p.observation()).expect("serializable observation")
            }
        }
    }
    pub fn commit<T>(&self, p: &Prepared, f: impl FnOnce() -> Result<T>) -> Result<T> {
        match self {
            Self::Unconfigured => Err(Error::Invalid("work-use/unconnected".into())),
            Self::Offline(provider) => provider.commit_checkpoint(p, f),
        }
    }
}
impl WorkUsePort for ResidentWorkProvider {
    fn assess(&self, r: &WorkUseRequest) -> WorkUseReply {
        match self {
            Self::Unconfigured => NoWorkUseProvider.assess(r),
            Self::Offline(p) => p.assess(r),
        }
    }
}

#[cfg(test)]
mod catalog_selection_tests {
    use super::*;
    #[test]
    fn every_catalog_program_selects_its_record_without_legacy_fallback() {
        for area in authored::catalog().unwrap() {
            let binding = receiver_binding(area.program).unwrap();
            assert!(std::ptr::eq(binding, area));
            assert_eq!(binding.role, area.role);
            assert_eq!(binding.key_id, area.key_id);
        }
        let legacy = receiver_binding("test/unrelated-authored-library-program").unwrap();
        assert_eq!(legacy.key_id, authored::KEY_ID);
        assert_eq!(legacy.role, authored::ROLE);
        assert_ne!(
            receiver_binding(authored::COMPACT.program).unwrap().role,
            legacy.role
        );
    }
}
