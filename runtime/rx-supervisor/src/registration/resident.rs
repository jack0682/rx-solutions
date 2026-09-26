//! Durable selection lookup, not execution ownership or permission.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

const INDEX: &str = "rx.resident-selections.v1";
const INDEX_KEY: &str = "components/resident-selections";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    selection: Name,
    registration: Id,
}

impl<R: Repository> Registry<R> {
    /// Bootstrap only a new, empty pair of stores. Thereafter every selection
    /// must resolve through the saved index; labels/PIDs never repair a gap.
    pub(crate) fn resident_registrations(
        &mut self,
        declarations: &BTreeMap<Name, Declaration>,
        fresh_execution_store: bool,
    ) -> Result<BTreeMap<Name, VersionedRegistration>> {
        let empty_registry =
            self.repository.snapshot()?.1.is_empty() && self.repository.journal_head()?.0 == 0;
        self.repository.transact(|tx| {
            let index_key = name(INDEX_KEY);
            let index = if let Some(row) = tx.get(&index_key)? {
                decode::<Vec<Selection>>(&row, INDEX)?
            } else {
                if !fresh_execution_store || tx.control_head()?.0 != 0 || !empty_registry {
                    return Err(StoreError::Integrity("resident selection mapping missing; no automatic registration or adoption".into()));
                }
                let mut index = Vec::new();
                for (selection, declaration) in declarations {
                    let registration = Registration {
                        id: fresh_id(),
                        declaration: declaration.clone(),
                        state: RegistrationState::Accepted,
                    };
                    save(tx, &registration.id, &key(&registration.id), None,
                        &document(REGISTRATION, &registration)?, "registered")?;
                    index.push(Selection { selection: selection.clone(), registration: registration.id });
                }
                let doc = document(INDEX, &index)?;
                let row = tx.put(&index_key, None, &doc)?;
                tx.append_control(&fresh_id(), &row, &doc)?;
                index
            };
            let mut selections = BTreeSet::new();
            let mut registrations = BTreeSet::new();
            for entry in &index {
                if !selections.insert(entry.selection.clone()) {
                    return Err(StoreError::Integrity("duplicate resident selection mapping".into()));
                }
                if !registrations.insert(entry.registration.clone()) {
                    return Err(StoreError::Integrity("duplicate resident registration mapping".into()));
                }
            }
            if selections != declarations.keys().cloned().collect() {
                return Err(StoreError::Integrity("resident selection mapping missing or selection set changed; explicit review required".into()));
            }
            let mut result = BTreeMap::new();
            for entry in index {
                let accepted = load(tx, &entry.registration)?;
                if accepted.registration.state == RegistrationState::Retired {
                    return Err(StoreError::Invalid("resident registration retired; no replacement or reactivation".into()));
                }
                if accepted.registration.declaration.catalog != declarations[&entry.selection].catalog {
                    return Err(StoreError::Integrity("resident catalog digest changed; explicit declaration review required".into()));
                }
                result.insert(entry.selection, accepted);
            }
            Ok(result)
        })
    }
}
