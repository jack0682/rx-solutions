//! P-attributed snapshots and one-shot local consumption, separate from legacy author rows.
use super::*;
use crate::resident_execution::Lease;
use rx_domain::resident_execution as data;
use rx_storage::SqliteRepository;
const OWNER: &str = "components/platform-authority";
const OWNER_SCHEMA: &str = "rx.platform-component-authority.v1";
const SNAPSHOT: &str = "rx.platform-component-snapshot.v1";
const CLAIM: &str = "rx.platform-execution-consumption.v1";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    installation: Id,
    assignment: Id,
    node: data::Node,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Consumed {
    owner: Id,
    grant: Id,
    peer: data::Peer,
    binding: Binding,
}
fn snapshot_key(id: &Id) -> Name {
    name(&format!("components/platform-snapshot/{id}"))
}
fn claim_key(instance: &Id) -> Name {
    name(&format!("components/platform-consumption/{instance}"))
}
pub(super) fn owner(tx: &mut dyn Transaction) -> Result<Option<Id>> {
    tx.get(&name(OWNER))?
        .map(|r| decode(&r, OWNER_SCHEMA))
        .transpose()
}
pub(super) fn snapshot(tx: &mut dyn Transaction, id: &Id) -> Result<Option<VersionedRegistration>> {
    let Some(row) = tx.get(&snapshot_key(id))? else {
        return Ok(None);
    };
    let value: Snapshot = decode(&row, SNAPSHOT)?;
    if value.node.registration.id != *id || owner(tx)?.as_ref() != Some(&value.installation) {
        return Err(StoreError::Integrity("P snapshot identity differs".into()));
    }
    Ok(Some(VersionedRegistration {
        revision: value.node.revision,
        registration: value.node.registration,
    }))
}
pub(super) fn ids(tx: &mut dyn Transaction) -> Result<Vec<Id>> {
    tx.scan("components/platform-snapshot/")?
        .into_iter()
        .map(|row| {
            let s: Snapshot = decode(&row, SNAPSHOT)?;
            if row.key != snapshot_key(&s.node.registration.id) {
                return Err(StoreError::Integrity("P snapshot key differs".into()));
            }
            Ok(s.node.registration.id)
        })
        .collect()
}
impl Registry<SqliteRepository> {
    /// Bound to the actual canonical path; parent aliases and other registry files cannot substitute.
    pub fn platform_binding(&self) -> Result<Digest> {
        canonical::digest(
            "RX-REGISTRY-SOURCE-PATH-v1",
            &self.repository.canonical_path()?,
        )
        .map_err(|e| StoreError::Invalid(e.to_string()))
    }
    pub(crate) fn platform_preflight(
        &mut self,
        intent: &data::Intent,
        peer: &data::Peer,
    ) -> crate::Result<Option<FreezeRecord>> {
        if self.platform_binding()? != peer.registry || intent.supervisor != peer.principal {
            return Err(crate::Error::Invalid(
                "execution registry or Supervisor differs".into(),
            ));
        }
        let has_legacy = self
            .repository
            .transact(|tx| Ok(!tx.scan("components/registration/")?.is_empty()))?;
        let legacy = if has_legacy {
            let frozen = self.frozen_source()?;
            if frozen.record().request.target_installation != peer.installation
                || !frozen.platform_acknowledged()
            {
                return Err(crate::Error::Invalid(
                    "legacy registry requires completed source/target reconciliation".into(),
                ));
            }
            Some(frozen.record().clone())
        } else {
            None
        };
        self.repository.transact(|tx| {
            if owner(tx)?.is_some_and(|id| id != peer.installation) {
                return Err(StoreError::Integrity(
                    "registry belongs to another Platform".into(),
                ));
            }
            for (selection, node) in &intent.nodes {
                if node.origin.as_ref().is_some_and(|o| {
                    o.registry != peer.registry || legacy.as_ref() != Some(&o.freeze)
                }) {
                    return Err(StoreError::Integrity(
                        "actual legacy source differs from assignment".into(),
                    ));
                }
                if let Some(previous) = snapshot(tx, &node.registration.id)?
                    && (previous.revision > node.revision
                        || (previous.revision == node.revision
                            && previous.registration != node.registration))
                {
                    return Err(StoreError::Integrity(
                        "P snapshot regressed or changed at one revision".into(),
                    ));
                }
                let binding = Binding {
                    registration: node.registration.id.clone(),
                    registration_revision: node.revision,
                    catalog: node.registration.declaration.catalog.clone(),
                    run: intent.run.clone(),
                    selection: selection.clone(),
                    instance: node.instance.clone(),
                };
                recovery::admit(tx, &binding, None)?;
            }
            Ok(())
        })?;
        Ok(legacy)
    }
    pub(crate) fn enroll_platform(&mut self, lease: &Lease) -> crate::Result<()> {
        self.platform_preflight(&lease.intent, &lease.grant.peer)?;
        if !lease.start_window() {
            return Err(crate::Error::Invalid(
                "start grant is no longer live".into(),
            ));
        }
        self.repository.seal_prefixes(
            &rx_domain::component_transfer::PREFIXES.map(name),
            |tx| {
                tx.require_resident_execution_reader()?;
                match owner(tx)? {
                    Some(old) if old != lease.grant.peer.installation => {
                        return Err(StoreError::Integrity(
                            "platform registry owner differs".into(),
                        ));
                    }
                    Some(_) => {}
                    None => {
                        tx.put(
                            &name(OWNER),
                            None,
                            &document(OWNER_SCHEMA, &lease.grant.peer.installation)?,
                        )?;
                    }
                }
                for node in lease.intent.nodes.values() {
                    let key = snapshot_key(&node.registration.id);
                    let old = tx.get(&key)?;
                    if let Some(row) = &old {
                        let prior: Snapshot = decode(row, SNAPSHOT)?;
                        if prior.node.revision > node.revision
                            || (prior.node.revision == node.revision
                                && prior.node.registration != node.registration)
                        {
                            return Err(StoreError::Integrity("P snapshot cannot regress".into()));
                        }
                    }
                    let value = Snapshot {
                        installation: lease.grant.peer.installation.clone(),
                        assignment: lease.intent.id.clone(),
                        node: node.clone(),
                    };
                    let row =
                        tx.put(&key, old.map(|r| r.revision), &document(SNAPSHOT, &value)?)?;
                    tx.append_control(&fresh_id(), &row, &row.document)?;
                }
                Ok(())
            },
        )?;
        Ok(())
    }
}
impl<R: Repository> Registry<R> {
    pub(crate) fn assign_platform(&mut self, binding: &Binding, lease: &Lease) -> Result<()> {
        self.repository.transact(|tx| {
            if owner(tx)?.as_ref() != Some(&lease.grant.peer.installation)
                || !lease.check_entered(binding)
            {
                return Err(StoreError::Invalid(
                    "live platform execution context differs".into(),
                ));
            }
            let current = load(tx, &binding.registration)?;
            if current.revision != binding.registration_revision
                || current.registration.declaration.catalog != binding.catalog
                || current.registration.state != RegistrationState::Accepted
            {
                return Err(StoreError::Integrity(
                    "platform binding differs from accepted snapshot".into(),
                ));
            }
            recovery::admit(tx, binding, None)?;
            let consumed = Consumed {
                owner: lease.owner.clone(),
                grant: lease.grant.id.clone(),
                peer: lease.grant.peer.clone(),
                binding: binding.clone(),
            };
            tx.put(
                &claim_key(&binding.instance),
                None,
                &document(CLAIM, &consumed)?,
            )?;
            let execution = Execution {
                binding: binding.clone(),
                last_observed: Observation {
                    process_identity: None,
                    state: ExecutionState::Assigned,
                    pid: None,
                    exit_code: None,
                    detail:
                        "P-assigned identity persisted before OS effects; outcome not yet known"
                            .into(),
                },
            };
            save(
                tx,
                &binding.registration,
                &execution_key(&binding.registration, &binding.instance),
                None,
                &document(EXECUTION, &execution)?,
                "platform-execution-assigned",
            )?;
            Ok(())
        })
    }
    pub(crate) fn platform_observation_owned(
        &mut self,
        binding: &Binding,
        lease: &Lease,
    ) -> Result<bool> {
        self.repository.transact(|tx| {
            if let Some(row) = tx.get(&claim_key(&binding.instance))? {
                let consumed: Consumed = decode(&row, CLAIM)?;
                return Ok(consumed.owner == lease.owner
                    && consumed.grant == lease.grant.id
                    && consumed.peer == lease.grant.peer
                    && consumed.binding == *binding);
            }
            Ok(tx
                .get(&execution_key(&binding.registration, &binding.instance))?
                .is_none())
        })
    }
    pub(crate) fn check_platform_start(&mut self, binding: &Binding, lease: &Lease) -> Result<()> {
        self.repository.transact(|tx| {
            if !lease.check_entered(binding)
                || owner(tx)?.as_ref() != Some(&lease.grant.peer.installation)
            {
                return Err(StoreError::Invalid("start grant expired or revoked".into()));
            }
            let row = tx
                .get(&claim_key(&binding.instance))?
                .ok_or_else(|| StoreError::Integrity("P consumption record absent".into()))?;
            let consumed: Consumed = decode(&row, CLAIM)?;
            if consumed.owner != lease.owner
                || consumed.grant != lease.grant.id
                || consumed.peer != lease.grant.peer
                || consumed.binding != *binding
            {
                return Err(StoreError::Integrity(
                    "P consumption context differs".into(),
                ));
            }
            Ok(())
        })
    }
}
