//! Verification of externally signed decisions. Author catalogs are trusted code;
//! site/decision input is not. No signing, site key loader or positive default key.
use rx_domain::{canonical, types::*};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Kind {
    WorkUse,
    InitialBinding,
    ReplacementBinding,
}
/// Authored policy, not a site configuration DTO. No Deserialize or key loader.
#[derive(Clone, Debug, Serialize)]
pub struct Authority {
    pub issuer: Name,
    pub public_key: [u8; 32],
    pub operating_area: Name,
    pub roles: BTreeSet<Name>,
    pub kinds: BTreeSet<Kind>,
    pub max_ttl_ms: Counter,
}
#[derive(Clone, Debug, Serialize)]
pub struct Policy {
    pub authorities: BTreeMap<Name, Authority>,
}
impl Policy {
    pub(crate) fn validate(&self) -> std::result::Result<(), String> {
        if self.authorities.is_empty() || self.authorities.len() > 8 {
            return Err("decision/anchors: author policy requires 1..8 authorities".into());
        }
        for authority in self.authorities.values() {
            if authority.public_key == [0; 32]
                || authority.roles.is_empty()
                || authority.roles.len() > 16
                || authority.kinds.is_empty()
                || !(1..=60_000).contains(&authority.max_ttl_ms.0)
            {
                return Err("decision/anchor-policy: nonzero key, bounded roles/kinds and 1..60000ms maximum TTL required".into());
            }
        }
        Ok(())
    }
    pub(crate) fn fingerprint(&self) -> std::result::Result<Digest, String> {
        self.validate()?;
        canonical::digest("RX-EXTERNAL-DECISION-POLICY-v1", self).map_err(|e| e.to_string())
    }
}

use rx_package::SignatureEnvelope;
use serde::Deserialize;
use serde_json::Value;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, Failure>;
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Failure {
    #[error("decision/unconfigured: no author-owned issuer policy for this current target")]
    Unconfigured,
    #[error("decision/policy: invalid author policy")]
    Policy,
    #[error("decision/issuer-scope: key, issuer, role or operating area is not authorized")]
    IssuerScope,
    #[error("decision/context: exact current subject/catalog/generation/binding differs")]
    Context,
    #[error(
        "decision/kind: work use, initial binding and replacement binding are separate decisions"
    )]
    Kind,
    #[error("decision/epoch: receiver was closed/restarted or belongs to another owner")]
    Epoch,
    #[error("decision/ttl: lifetime must be positive and within the authored maximum")]
    Ttl,
    #[error("decision/expired: local monotonic deadline measured from challenge creation elapsed")]
    Expired,
    #[error("decision/revoked: the external decision has been invalidated")]
    Revoked,
    #[error("decision/signature: detached Ed25519 verification failed")]
    Signature,
    #[error("decision/encoding: malformed or oversized decision input")]
    Encoding,
    #[error("decision/identity: decision id already names different signed content")]
    Identity,
    #[error("decision/capacity: receiver decision history is full")]
    Capacity,
    #[error("decision/receiver: receiver ledger unavailable")]
    Receiver,
}
fn fresh() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).expect("UUID")
}
fn named(s: &str) -> Name {
    Name::new(s).expect("literal")
}
fn hash<T: Serialize>(domain: &str, value: &T) -> Result<Digest> {
    canonical::digest(domain, value).map_err(|_| Failure::Encoding)
}
fn message<T: Serialize>(domain: &str, key: &Name, value: &T) -> Result<Vec<u8>> {
    let payload = canonical::bytes(&(key, value)).map_err(|_| Failure::Encoding)?;
    if payload.len() > 65_536 {
        return Err(Failure::Encoding);
    }
    let mut bytes = domain.as_bytes().to_vec();
    bytes.push(0);
    bytes.extend(payload);
    Ok(bytes)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Owner {
    pub registration: Id,
    pub revision: Counter,
    pub program: Name,
    pub catalog: Digest,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChallengeInfo {
    pub id: Id,
    pub epoch: Id,
    pub kind: Kind,
    pub owner: Owner,
    pub operating_area: Name,
    pub role: Name,
    pub subject: Value,
    pub context_digest: Digest,
    pub policy_digest: Digest,
    pub key: Name,
    pub issuer: Name,
    pub max_ttl_ms: Counter,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    Approve,
}

/// Untrusted transport data. Signing bytes are supplied for the external issuer;
/// constructing a claim is not signing it or creating a verified decision.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    pub schema: Name,
    pub verdict: Verdict,
    pub challenge: ChallengeInfo,
    pub decision: Id,
    pub ttl_ms: Counter,
}
impl Claim {
    pub fn signing_message(&self, key: &Name) -> Result<Vec<u8>> {
        message("RX-EXTERNAL-DECISION-v1", key, self)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedDecision {
    pub claim: Claim,
    pub signature: SignatureEnvelope,
}
impl SignedDecision {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 131_072 {
            return Err(Failure::Encoding);
        }
        canonical::decode_json(bytes).map_err(|_| Failure::Encoding)
    }
}
/// Inert historical reference. No local deadline, live receiver or key capability.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub decision: Id,
    pub kind: Kind,
    pub owner: Owner,
    pub issuer: Name,
    pub key: Name,
    pub epoch: Id,
    pub challenge: Id,
    pub context_digest: Digest,
    pub policy_digest: Digest,
    pub signed_digest: Digest,
    pub ttl_ms: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevocationClaim {
    pub schema: Name,
    pub target: Reference,
    pub reason: Name,
}
impl RevocationClaim {
    pub fn signing_message(&self, key: &Name) -> Result<Vec<u8>> {
        message("RX-EXTERNAL-DECISION-REVOCATION-v1", key, self)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedRevocation {
    pub claim: RevocationClaim,
    pub signature: SignatureEnvelope,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevocationReference {
    pub target: Reference,
    pub signed_digest: Digest,
    pub reason: Name,
}
#[derive(Debug)]
struct Entry {
    reference: Reference,
    revoked: bool,
}
#[derive(Debug)]
struct Runtime {
    epoch: Id,
    active: AtomicBool,
    entries: Mutex<BTreeMap<Id, Entry>>,
}
impl Runtime {
    fn active(&self) -> Result<()> {
        if self.active.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(Failure::Epoch)
        }
    }
}
/// Only the registered owner creates this receiver. Dropping it invalidates every
/// retained challenge/proof even if an adapter still holds clones.
#[derive(Debug)]
pub(crate) struct Gate {
    runtime: Arc<Runtime>,
}
impl Gate {
    pub(crate) fn new() -> Self {
        Self {
            runtime: Arc::new(Runtime {
                epoch: fresh(),
                active: AtomicBool::new(true),
                entries: Mutex::new(BTreeMap::new()),
            }),
        }
    }
    pub(crate) fn request(
        &self,
        kind: Kind,
        owner: Owner,
        scope: &Name,
        role: &Name,
        subject: Value,
        policy: &Policy,
    ) -> Result<Request> {
        self.runtime.active()?;
        let policy_digest = policy.fingerprint().map_err(|_| Failure::Policy)?;
        let context_digest = hash(
            "RX-EXTERNAL-DECISION-CONTEXT-v1",
            &(kind, &owner, scope, role, &subject, policy_digest),
        )?;
        Ok(Request {
            kind,
            owner,
            operating_area: scope.clone(),
            role: role.clone(),
            subject,
            context_digest,
            policy_digest,
            policy: policy.clone(),
            runtime: self.runtime.clone(),
        })
    }
}
impl Drop for Gate {
    fn drop(&mut self) {
        self.runtime.active.store(false, Ordering::SeqCst);
    }
}
/// Sealed current-context request. No public constructor, key input or Deserialize.
#[derive(Clone, Debug, Serialize)]
pub struct Request {
    kind: Kind,
    owner: Owner,
    operating_area: Name,
    role: Name,
    subject: Value,
    context_digest: Digest,
    policy_digest: Digest,
    #[serde(skip)]
    policy: Policy,
    #[serde(skip)]
    runtime: Arc<Runtime>,
}
impl Request {
    pub fn challenge(&self, key: &Name) -> Result<Challenge> {
        self.runtime.active()?;
        let authority = self
            .policy
            .authorities
            .get(key)
            .ok_or(Failure::IssuerScope)?;
        if authority.operating_area != self.operating_area
            || !authority.roles.contains(&self.role)
            || !authority.kinds.contains(&self.kind)
        {
            return Err(Failure::IssuerScope);
        }
        Ok(Challenge {
            info: ChallengeInfo {
                id: fresh(),
                epoch: self.runtime.epoch.clone(),
                kind: self.kind,
                owner: self.owner.clone(),
                operating_area: self.operating_area.clone(),
                role: self.role.clone(),
                subject: self.subject.clone(),
                context_digest: self.context_digest,
                policy_digest: self.policy_digest,
                key: key.clone(),
                issuer: authority.issuer.clone(),
                max_ttl_ms: authority.max_ttl_ms,
            },
            created: Instant::now(),
            authority: authority.clone(),
            runtime: self.runtime.clone(),
        })
    }
    /// Recheck at each receiving boundary. A prior positive display or serialized
    /// reference is not sufficient to call this method.
    pub(crate) fn check(&self, verified: &VerifiedDecision) -> Result<Reference> {
        if verified.reference.kind != self.kind {
            return Err(Failure::Kind);
        }
        if verified.reference.context_digest != self.context_digest
            || verified.reference.owner != self.owner
            || verified.reference.policy_digest != self.policy_digest
        {
            return Err(Failure::Context);
        }
        if verified.reference.epoch != self.runtime.epoch
            || !Arc::ptr_eq(&verified.runtime, &self.runtime)
        {
            return Err(Failure::Epoch);
        }
        verified.check_live()?;
        Ok(verified.reference.clone())
    }
}
/// A locally timed challenge. Serialization exports only the issuer's request
/// data; it cannot restore the local Instant or receiver after restart.
#[derive(Debug, Serialize)]
pub struct Challenge {
    info: ChallengeInfo,
    #[serde(skip)]
    created: Instant,
    #[serde(skip)]
    authority: Authority,
    #[serde(skip)]
    runtime: Arc<Runtime>,
}
impl Challenge {
    pub fn info(&self) -> &ChallengeInfo {
        &self.info
    }
    pub fn claim(&self, decision: Id, ttl_ms: Counter) -> Claim {
        Claim {
            schema: named("rx.external-decision.v1"),
            verdict: Verdict::Approve,
            challenge: self.info.clone(),
            decision,
            ttl_ms,
        }
    }
    pub fn verify(&self, signed: &SignedDecision) -> Result<VerifiedDecision> {
        self.runtime.active()?;
        let claim = &signed.claim;
        if claim.schema.as_str() != "rx.external-decision.v1" || claim.challenge != self.info {
            return Err(Failure::Context);
        }
        if signed.signature.key != self.info.key {
            return Err(Failure::IssuerScope);
        }
        if claim.ttl_ms.0 == 0 || claim.ttl_ms.0 > self.authority.max_ttl_ms.0 {
            return Err(Failure::Ttl);
        }
        let expires = self
            .created
            .checked_add(Duration::from_millis(claim.ttl_ms.0))
            .ok_or(Failure::Ttl)?;
        if Instant::now() >= expires {
            return Err(Failure::Expired);
        }
        if signed.signature.signature.len() != 128 {
            return Err(Failure::Signature);
        }
        rx_package::verify_detached_message(
            &claim.signing_message(&signed.signature.key)?,
            &signed.signature.signature,
            &self.authority.public_key,
        )
        .map_err(|_| Failure::Signature)?;
        self.runtime.active()?;
        if Instant::now() >= expires {
            return Err(Failure::Expired);
        }
        let reference = Reference {
            decision: claim.decision.clone(),
            kind: self.info.kind,
            owner: self.info.owner.clone(),
            issuer: self.info.issuer.clone(),
            key: self.info.key.clone(),
            epoch: self.info.epoch.clone(),
            challenge: self.info.id.clone(),
            context_digest: self.info.context_digest,
            policy_digest: self.info.policy_digest,
            signed_digest: hash("RX-EXTERNAL-DECISION-SIGNED-v1", signed)?,
            ttl_ms: claim.ttl_ms,
        };
        let mut ledger = self.runtime.entries.lock().map_err(|_| Failure::Receiver)?;
        if let Some(previous) = ledger.get(&reference.decision) {
            if previous.reference != reference {
                return Err(Failure::Identity);
            }
            if previous.revoked {
                return Err(Failure::Revoked);
            }
        } else {
            if ledger.len() >= 512 {
                return Err(Failure::Capacity);
            }
            ledger.insert(
                reference.decision.clone(),
                Entry {
                    reference: reference.clone(),
                    revoked: false,
                },
            );
        }
        drop(ledger);
        Ok(VerifiedDecision {
            reference,
            expires,
            authority: self.authority.clone(),
            runtime: self.runtime.clone(),
        })
    }
}
/// Positive evidence from strict signature verification only. Clones share the
/// original deadline/receiver/revocation ledger. No Deserialize or public ctor.
#[derive(Clone, Debug)]
pub struct VerifiedDecision {
    reference: Reference,
    expires: Instant,
    authority: Authority,
    runtime: Arc<Runtime>,
}
impl Serialize for VerifiedDecision {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        self.reference.serialize(s)
    }
}
impl VerifiedDecision {
    pub fn reference(&self) -> &Reference {
        &self.reference
    }
    pub fn check_live(&self) -> Result<()> {
        self.runtime.active()?;
        if Instant::now() >= self.expires {
            return Err(Failure::Expired);
        }
        let ledger = self.runtime.entries.lock().map_err(|_| Failure::Receiver)?;
        let entry = ledger
            .get(&self.reference.decision)
            .ok_or(Failure::Receiver)?;
        if entry.reference != self.reference {
            return Err(Failure::Identity);
        }
        if entry.revoked {
            return Err(Failure::Revoked);
        }
        self.runtime.active()?;
        if Instant::now() >= self.expires {
            return Err(Failure::Expired);
        }
        Ok(())
    }
    pub fn revocation_claim(&self, reason: Name) -> RevocationClaim {
        RevocationClaim {
            schema: named("rx.external-decision-revocation.v1"),
            target: self.reference.clone(),
            reason,
        }
    }
    pub fn revoke(&self, signed: &SignedRevocation) -> Result<VerifiedRevocation> {
        self.runtime.active()?;
        if signed.claim.schema.as_str() != "rx.external-decision-revocation.v1"
            || signed.claim.target != self.reference
        {
            return Err(Failure::Context);
        }
        if signed.signature.key != self.reference.key {
            return Err(Failure::IssuerScope);
        }
        if signed.signature.signature.len() != 128 {
            return Err(Failure::Signature);
        }
        rx_package::verify_detached_message(
            &signed.claim.signing_message(&signed.signature.key)?,
            &signed.signature.signature,
            &self.authority.public_key,
        )
        .map_err(|_| Failure::Signature)?;
        let mut ledger = self.runtime.entries.lock().map_err(|_| Failure::Receiver)?;
        let entry = ledger
            .get_mut(&self.reference.decision)
            .ok_or(Failure::Receiver)?;
        if entry.reference != self.reference {
            return Err(Failure::Identity);
        }
        entry.revoked = true;
        Ok(VerifiedRevocation {
            reference: RevocationReference {
                target: self.reference.clone(),
                signed_digest: hash("RX-EXTERNAL-REVOCATION-SIGNED-v1", signed)?,
                reason: signed.claim.reason.clone(),
            },
        })
    }
}
#[derive(Clone, Debug)]
pub struct VerifiedRevocation {
    reference: RevocationReference,
}
impl Serialize for VerifiedRevocation {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        self.reference.serialize(s)
    }
}
impl VerifiedRevocation {
    pub fn reference(&self) -> &RevocationReference {
        &self.reference
    }
}
