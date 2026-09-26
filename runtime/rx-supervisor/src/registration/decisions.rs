//! Immutable decision/revocation observations. Stored references cannot restore a
//! live monotonic lease or change existing diagnostic/execution results.
use super::*;
use crate::decision::{Reference, RevocationReference, VerifiedDecision, VerifiedRevocation};
const DECISION: &str = "rx.external-decision-observation.v1";
const REVOCATION: &str = "rx.external-decision-revocation-observation.v1";
fn decision_key(component: &Id, decision: &Id) -> Name {
    name(&format!("components/decision/{component}/{decision}"))
}
fn revocation_key(component: &Id, decision: &Id) -> Name {
    name(&format!(
        "components/decision-revocation/{component}/{decision}"
    ))
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionRecord {
    pub reference: Reference,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionRevocationRecord {
    pub reference: RevocationReference,
}
impl<R: Repository> Registry<R> {
    /// Explicit audit recording only. It neither activates a binding nor grants
    /// execution, and does not silently make ordinary assessment mutate history.
    pub fn record_verified_decision(&mut self, proof: &VerifiedDecision) -> Result<DecisionRecord> {
        proof
            .check_live()
            .map_err(|e| StoreError::Invalid(e.to_string()))?;
        let reference = proof.reference().clone();
        self.repository.transact(|tx|{
            let current=load(tx,&reference.owner.registration)?;
            if current.registration.state!=RegistrationState::Accepted || current.revision!=reference.owner.revision
                || current.registration.declaration.catalog.program!=reference.owner.program
                || current.registration.declaration.catalog.digest!=reference.owner.catalog {
                return Err(StoreError::Invalid("decision/current-registration: reference no longer matches accepted author content".into()));
            }
            proof.check_live().map_err(|e|StoreError::Invalid(e.to_string()))?;
            let key=decision_key(&reference.owner.registration,&reference.decision);
            let value=DecisionRecord{reference:reference.clone()};
            if let Some(row)=tx.get(&key)?{
                let old:DecisionRecord=decode(&row,DECISION)?;
                if old!=value{return Err(StoreError::Invalid("decision/identity: refusing to replace a historical decision".into()));}
                return Ok(old);
            }
            save(tx,&reference.owner.registration,&key,None,&document(DECISION,&value)?,"external-decision-observed")?;
            Ok(value)
        })
    }
    pub fn recorded_decision(&mut self, component: &Id, decision: &Id) -> Result<DecisionRecord> {
        self.repository.transact(|tx| {
            let row = tx
                .get(&decision_key(component, decision))?
                .ok_or_else(|| StoreError::Invalid("decision record not found".into()))?;
            let value: DecisionRecord = decode(&row, DECISION)?;
            if value.reference.owner.registration != *component
                || value.reference.decision != *decision
            {
                return Err(StoreError::Integrity(
                    "decision record identity differs".into(),
                ));
            }
            Ok(value)
        })
    }
    pub fn record_verified_revocation(
        &mut self,
        proof: &VerifiedRevocation,
    ) -> Result<DecisionRevocationRecord> {
        let reference = proof.reference().clone();
        let target = &reference.target;
        self.repository.transact(|tx| {
            let original = tx
                .get(&decision_key(&target.owner.registration, &target.decision))?
                .ok_or_else(|| {
                    StoreError::Invalid("revocation requires the original recorded decision".into())
                })?;
            let original: DecisionRecord = decode(&original, DECISION)?;
            if original.reference != *target {
                return Err(StoreError::Invalid(
                    "revocation target differs from original decision".into(),
                ));
            }
            let value = DecisionRevocationRecord {
                reference: reference.clone(),
            };
            let key = revocation_key(&target.owner.registration, &target.decision);
            if let Some(row) = tx.get(&key)? {
                let old: DecisionRevocationRecord = decode(&row, REVOCATION)?;
                if old != value {
                    return Err(StoreError::Invalid(
                        "revocation history already records another signed statement".into(),
                    ));
                }
                return Ok(old);
            }
            save(
                tx,
                &target.owner.registration,
                &key,
                None,
                &document(REVOCATION, &value)?,
                "external-decision-revoked",
            )?;
            Ok(value)
        })
    }
}
