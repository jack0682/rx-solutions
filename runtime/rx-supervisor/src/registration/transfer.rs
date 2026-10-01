//! Source-side declaration custody fence. P has not accepted anything by this operation.
use super::*;
use rx_storage::SqliteRepository;
const FREEZE: &str = "components/registry-freeze";
const SCHEMA: &str = "rx.registration-source-freeze.v1";
const PREFIXES: [&str; 3] = [
    "components/registration/",
    "components/resident-selections",
    FREEZE,
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreezeRequest {
    pub id: Id,
    pub target_installation: Id,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreezeRecord {
    pub request: FreezeRequest,
    pub declarations_digest: Digest,
    pub declaration_count: Counter,
    pub history_head: Counter,
}
/// Constructed only after the real SQLite transaction installed and checked its fences.
/// Serialization is an export, not a remote proof or a P acceptance receipt.
#[derive(Debug, Serialize)]
pub struct FrozenRegistry {
    record: FreezeRecord,
    declarations: Vec<Record>,
    selection_index: Option<Record>,
    local_declaration_writer: &'static str,
    platform_acceptance: &'static str,
    process_ownership: &'static str,
}

impl FrozenRegistry {
    pub fn record(&self) -> &FreezeRecord {
        &self.record
    }
    pub fn declarations(&self) -> &[Record] {
        &self.declarations
    }
    pub fn selection_index(&self) -> Option<&Record> {
        self.selection_index.as_ref()
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DeclarationAuthority {
    Local,
    SourceFrozen { request: FreezeRequest },
}
pub(super) fn authority(tx: &mut dyn Transaction) -> Result<DeclarationAuthority> {
    match tx.get(&name(FREEZE))? {
        Some(row) => Ok(DeclarationAuthority::SourceFrozen {
            request: decode::<FreezeRecord>(&row, SCHEMA)?.request,
        }),
        None => Ok(DeclarationAuthority::Local),
    }
}

pub(super) fn require_local_authority(tx: &mut dyn Transaction) -> Result<()> {
    if tx.get(&name(FREEZE))?.is_some() {
        return Err(StoreError::Invalid("registration source frozen for transfer; local declaration/assignment authority removed".into()));
    }
    Ok(())
}
fn contents(tx: &mut dyn Transaction) -> Result<(Vec<Record>, Option<Record>, Digest)> {
    let declarations = tx.scan("components/registration/")?;
    if declarations.is_empty() {
        return Err(StoreError::Invalid(
            "empty registration source cannot be transferred".into(),
        ));
    }
    for row in &declarations {
        let registration: Registration = decode(row, REGISTRATION)?;
        if row.key != key(&registration.id) {
            return Err(StoreError::Integrity("declaration identity differs".into()));
        }
    }
    let index = tx.get(&name("components/resident-selections"))?;
    if let Some(row) = &index {
        let selections: Vec<resident::Selection> = decode(row, resident::INDEX)?;
        let mut names = std::collections::BTreeSet::new();
        let mut ids = std::collections::BTreeSet::new();
        for entry in selections {
            if !names.insert(entry.selection)
                || !ids.insert(entry.registration.clone())
                || !declarations
                    .iter()
                    .any(|row| row.key == key(&entry.registration))
            {
                return Err(StoreError::Integrity(
                    "resident selection source differs".into(),
                ));
            }
        }
    }
    let digest = canonical::digest("RX-REGISTRATION-SOURCE-CUT-v1", &(&declarations, &index))
        .map_err(|e| StoreError::Integrity(e.to_string()))?;
    Ok((declarations, index, digest))
}
fn view(tx: &mut dyn Transaction, record: FreezeRecord) -> Result<FrozenRegistry> {
    let (declarations, selection_index, digest) = contents(tx)?;
    if digest != record.declarations_digest
        || declarations.len() as u64 != record.declaration_count.0
    {
        return Err(StoreError::Integrity(
            "frozen declaration cut differs".into(),
        ));
    }
    Ok(FrozenRegistry {
        record,
        declarations,
        selection_index,
        local_declaration_writer: "PERMANENTLY_FROZEN",
        platform_acceptance: "NOT_ESTABLISHED",
        process_ownership: "NOT_TRANSFERRED",
    })
}
impl Registry<SqliteRepository> {
    /// Explicit source-side preparation. Do not run against an active older manager:
    /// opening this repository requires its exclusive writer lock first.
    /// Existing observation keys remain writable; new local assignments are refused.
    pub fn freeze_for_transfer(&mut self, request: FreezeRequest) -> Result<FrozenRegistry> {
        let prefixes = PREFIXES.map(name);
        let existing = self.repository.sealed_prefixes()?;
        if !existing.is_empty()
            && existing
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
                != prefixes.iter().cloned().collect()
        {
            return Err(StoreError::Integrity(
                "source has unrelated namespace fences".into(),
            ));
        }
        let was_frozen = !existing.is_empty();
        self.repository.seal_prefixes(&prefixes, |tx| {
            if let Some(row) = tx.get(&name(FREEZE))? {
                if !was_frozen {
                    return Err(StoreError::Integrity(
                        "freeze marker without storage fences".into(),
                    ));
                }
                let record: FreezeRecord = decode(&row, SCHEMA)?;
                if record.request != request {
                    return Err(StoreError::KeyConflict);
                }
                return view(tx, record);
            }
            if was_frozen {
                return Err(StoreError::Integrity(
                    "storage fences without freeze marker".into(),
                ));
            }
            let (declarations, _, digest) = contents(tx)?;
            let record = FreezeRecord {
                request,
                declarations_digest: digest,
                declaration_count: Counter(declarations.len() as u64),
                history_head: tx.control_head()?,
            };
            let row = tx.put(&name(FREEZE), None, &document(SCHEMA, &record)?)?;
            tx.append_control(&fresh_id(), &row, &document(SCHEMA, &record)?)?;
            view(tx, record)
        })
    }
    pub fn frozen_source(&mut self) -> Result<FrozenRegistry> {
        let expected = PREFIXES
            .map(name)
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        if self
            .repository
            .sealed_prefixes()?
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            != expected
        {
            return Err(StoreError::Integrity(
                "source declaration fences absent/different".into(),
            ));
        }
        self.repository.transact(|tx| {
            let row = tx
                .get(&name(FREEZE))?
                .ok_or_else(|| StoreError::Integrity("source freeze marker absent".into()))?;
            view(tx, decode(&row, SCHEMA)?)
        })
    }
    /// Pages the original immutable control history only through the recorded cut.
    pub fn frozen_history(
        &mut self,
        freeze: &Id,
        after: Counter,
        limit: usize,
    ) -> Result<Vec<StoredEvent>> {
        let source = self.frozen_source()?;
        if &source.record.request.id != freeze {
            return Err(StoreError::KeyConflict);
        }
        if !(1..=128).contains(&limit) {
            return Err(StoreError::Invalid("history page must be 1..128".into()));
        }
        if after >= source.record.history_head {
            return Ok(vec![]);
        }
        Ok(self
            .repository
            .control_events_after(after, limit)?
            .into_iter()
            .take_while(|e| e.seq <= source.record.history_head)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn declaration() -> Declaration {
        Declaration {
            label: name("status"),
            catalog: CatalogReference {
                program: name("status"),
                digest: Digest::from_bytes([1; 32]),
            },
        }
    }
    #[test]
    fn source_freeze_keeps_identity_history_and_observations_but_refuses_new_local_work() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registration.db");
        let mut registry = Registry::new(SqliteRepository::open(&path).unwrap());
        let registered = registry.register(declaration()).unwrap();
        let mut changed = declaration();
        changed.label = name("renamed");
        let updated = registry
            .update(&registered.registration.id, registered.revision, changed)
            .unwrap();
        let binding = Binding {
            registration: registered.registration.id.clone(),
            registration_revision: updated.revision,
            catalog: updated.registration.declaration.catalog.clone(),
            run: fresh_id(),
            selection: name("status"),
            instance: fresh_id(),
        };
        registry.assign(&binding).unwrap();
        let before = registry.history(&registered.registration.id).unwrap();
        let request = FreezeRequest {
            id: fresh_id(),
            target_installation: fresh_id(),
        };
        let frozen = registry.freeze_for_transfer(request.clone()).unwrap();
        let bytes = canonical::bytes(&frozen).unwrap();
        assert_eq!(frozen.declarations[0].revision, updated.revision);
        assert_eq!(
            decode::<Registration>(&frozen.declarations[0], REGISTRATION)
                .unwrap()
                .id,
            registered.registration.id
        );
        registry.into_repository().close().unwrap();
        let mut registry = Registry::new(SqliteRepository::open(&path).unwrap());
        assert_eq!(
            canonical::bytes(&registry.freeze_for_transfer(request.clone()).unwrap()).unwrap(),
            bytes
        );
        assert!(
            registry
                .freeze_for_transfer(FreezeRequest {
                    target_installation: fresh_id(),
                    ..request.clone()
                })
                .is_err()
        );
        assert!(registry.register(declaration()).is_err());
        assert!(
            registry
                .update(&registered.registration.id, updated.revision, declaration())
                .is_err()
        );
        assert!(
            registry
                .retire(&registered.registration.id, updated.revision)
                .is_err()
        );
        assert!(registry.check_start(&binding).is_err());
        assert!(registry.ensure_new_work(&fresh_id()).is_err());
        assert!(matches!(
            registry
                .query(&binding.registration)
                .unwrap()
                .declaration_authority,
            DeclarationAuthority::SourceFrozen { .. }
        ));
        let new_binding = Binding {
            instance: fresh_id(),
            ..binding.clone()
        };
        assert!(registry.assign(&new_binding).is_err());
        registry
            .observe(
                &binding,
                Observation {
                    state: ExecutionState::Unknown,
                    pid: None,
                    exit_code: None,
                    detail: "manager loss remains unknown".into(),
                    process_identity: None,
                },
            )
            .unwrap();
        assert_eq!(
            registry.query(&binding.registration).unwrap().executions[0]
                .last_observed
                .state,
            ExecutionState::Unknown
        );
        let mut collected = Vec::new();
        let mut after = Counter(0);
        loop {
            let page = registry.frozen_history(&request.id, after, 1).unwrap();
            if page.is_empty() {
                break;
            }
            after = page.last().unwrap().seq;
            collected.extend(page);
        }
        assert_eq!(collected, before);
        assert_eq!(
            canonical::bytes(&registry.frozen_source().unwrap()).unwrap(),
            bytes
        );
    }
    #[test]
    fn malformed_source_or_unbacked_marker_cannot_become_a_frozen_receipt() {
        for fake_marker in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut registry =
                Registry::new(SqliteRepository::open(dir.path().join("source.db")).unwrap());
            registry.register(declaration()).unwrap();
            let request = FreezeRequest {
                id: fresh_id(),
                target_installation: fresh_id(),
            };
            registry.repository.transact(|tx| {
                if fake_marker {
                    let (declarations,_,digest)=contents(tx)?;
                    let record=FreezeRecord{request:request.clone(),declarations_digest:digest,declaration_count:Counter(declarations.len() as u64),history_head:tx.control_head()?};
                    tx.put(&name(FREEZE),None,&document(SCHEMA,&record)?)?;
                } else {
                    tx.put(&name("components/resident-selections"),None,&document(resident::INDEX,&serde_json::json!([{"selection":"missing","registration":fresh_id()}]))?)?;
                }
                Ok(())
            }).unwrap();
            assert!(registry.freeze_for_transfer(request).is_err());
            assert!(registry.repository.sealed_prefixes().unwrap().is_empty());
        }
    }
}
