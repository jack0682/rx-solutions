//! Persistent component identity. No supervisor, process plan, OS or F1 policy copy.
//! The caller owns this repository independently of any execution manager.
use rx_domain::{canonical, types::*};
use rx_ports::{Document, Record, Repository, StoreError, StoredEvent, Transaction};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub mod diagnostic;
mod recovery;
pub use recovery::*;

type Result<T> = rx_ports::Result<T>;
const REGISTRATION: &str = "rx.component-registration.v1";
const EXECUTION: &str = "rx.component-execution-observation.v1";
const EVENT: &str = "rx.component-history.v1";

fn name(value: &str) -> Name {
    Name::new(value).expect("internal key")
}
fn fresh_id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).expect("UUID")
}
fn key(id: &Id) -> Name {
    name(&format!("components/registration/{id}"))
}
fn execution_prefix(id: &Id) -> String {
    format!("components/execution/{id}/")
}
fn execution_key(component: &Id, instance: &Id) -> Name {
    name(&format!("{}{instance}", execution_prefix(component)))
}

/// A reference to accepted author content, never an editable policy snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogReference {
    pub program: Name,
    pub digest: Digest,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    pub label: Name,
    pub catalog: CatalogReference,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RegistrationState {
    Accepted,
    Retired,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    pub id: Id,
    pub declaration: Declaration,
    pub state: RegistrationState,
}
#[derive(Clone, Debug, Serialize)]
pub struct VersionedRegistration {
    pub revision: Counter,
    pub registration: Registration,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub registration: Id,
    pub registration_revision: Counter,
    pub catalog: CatalogReference,
    /// Opaque consumer run and selection identities, not part of the registration key.
    pub run: Id,
    pub selection: Name,
    pub instance: Id,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecutionState {
    Assigned,
    Running,
    Exited,
    NotStarted,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub state: ExecutionState,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub detail: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Execution {
    pub binding: Binding,
    /// Historical report. Persistence does not establish current process/resource ownership.
    pub last_observed: Observation,
}
#[derive(Clone, Debug, Serialize)]
pub struct View {
    pub registration: VersionedRegistration,
    pub executions: Vec<Execution>,
    pub recovery: RecoveryView,
    pub execution_ownership: &'static str,
    pub functional_readiness: crate::use_assessment::ReadinessAssessment,
    pub work_use_permission: crate::use_assessment::WorkUseAssessment,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    registration: Id,
    action: Name,
    entity: Record,
}
fn document<T: Serialize>(schema: &str, value: &T) -> Result<Document> {
    Ok(Document {
        schema: name(schema),
        value: serde_json::to_value(value).map_err(|e| StoreError::Invalid(e.to_string()))?,
    })
}
fn decode<T: DeserializeOwned>(record: &Record, schema: &str) -> Result<T> {
    if record.document.schema.as_str() != schema {
        return Err(StoreError::Integrity(
            "component document schema differs".into(),
        ));
    }
    canonical::decode_json(
        &canonical::bytes(&record.document.value)
            .map_err(|e| StoreError::Integrity(e.to_string()))?,
    )
    .map_err(|e| StoreError::Integrity(e.to_string()))
}
fn load(tx: &mut dyn Transaction, id: &Id) -> Result<VersionedRegistration> {
    let row = tx
        .get(&key(id))?
        .ok_or_else(|| StoreError::Invalid("component registration not found".into()))?;
    let registration: Registration = decode(&row, REGISTRATION)?;
    if registration.id != *id {
        return Err(StoreError::Integrity("component identity differs".into()));
    }
    Ok(VersionedRegistration {
        revision: row.revision,
        registration,
    })
}
fn executions(tx: &mut dyn Transaction, id: &Id) -> Result<Vec<Execution>> {
    tx.scan(&execution_prefix(id))?
        .iter()
        .map(|r| {
            let execution: Execution = decode(r, EXECUTION)?;
            if execution.binding.registration != *id
                || r.key != execution_key(id, &execution.binding.instance)
            {
                return Err(StoreError::Integrity("execution identity differs".into()));
            }
            Ok(execution)
        })
        .collect()
}
fn save(
    tx: &mut dyn Transaction,
    id: &Id,
    key: &Name,
    expected: Option<Counter>,
    doc: &Document,
    action: &str,
) -> Result<Record> {
    let entity = tx.put(key, expected, doc)?;
    tx.append_control(
        &fresh_id(),
        &entity,
        &document(
            EVENT,
            &Change {
                registration: id.clone(),
                action: name(action),
                entity: entity.clone(),
            },
        )?,
    )?;
    Ok(entity)
}
fn eligible(tx: &mut dyn Transaction, binding: &Binding) -> Result<()> {
    let current = load(tx, &binding.registration)?;
    if current.registration.state != RegistrationState::Accepted {
        return Err(StoreError::Invalid(
            "retired component: new execution assignment prohibited; history retained".into(),
        ));
    }
    if current.revision != binding.registration_revision {
        return Err(StoreError::RevisionConflict(
            "component declaration revision changed".into(),
        ));
    }
    if current.registration.declaration.catalog != binding.catalog {
        return Err(StoreError::Invalid(
            "accepted catalog differs; explicit declaration review required".into(),
        ));
    }
    Ok(())
}

pub struct Registry<R> {
    repository: R,
}
impl<R: Repository> Registry<R> {
    pub fn new(repository: R) -> Self {
        Self { repository }
    }
    pub fn into_repository(self) -> R {
        self.repository
    }
    pub fn register(&mut self, declaration: Declaration) -> Result<VersionedRegistration> {
        let registration = Registration {
            id: fresh_id(),
            declaration,
            state: RegistrationState::Accepted,
        };
        self.repository.transact(|tx| {
            let row = save(
                tx,
                &registration.id,
                &key(&registration.id),
                None,
                &document(REGISTRATION, &registration)?,
                "registered",
            )?;
            Ok(VersionedRegistration {
                revision: row.revision,
                registration,
            })
        })
    }
    pub fn update(
        &mut self,
        id: &Id,
        expected: Counter,
        declaration: Declaration,
    ) -> Result<VersionedRegistration> {
        self.modify(id, expected, Some(declaration))
    }
    pub fn retire(&mut self, id: &Id, expected: Counter) -> Result<VersionedRegistration> {
        self.modify(id, expected, None)
    }
    fn modify(
        &mut self,
        id: &Id,
        expected: Counter,
        declaration: Option<Declaration>,
    ) -> Result<VersionedRegistration> {
        self.repository.transact(|tx| {
            let mut current = load(tx, id)?;
            if current.revision != expected {
                return Err(StoreError::RevisionConflict(
                    "component revision changed".into(),
                ));
            }
            if current.registration.state == RegistrationState::Retired {
                return Err(StoreError::Invalid(
                    "retired registration cannot be reactivated or rewritten".into(),
                ));
            }
            let action = if let Some(declaration) = declaration {
                current.registration.declaration = declaration;
                "declaration-changed"
            } else {
                current.registration.state = RegistrationState::Retired;
                "retired-new-assignments-prohibited"
            };
            let row = save(
                tx,
                id,
                &key(id),
                Some(expected),
                &document(REGISTRATION, &current.registration)?,
                action,
            )?;
            current.revision = row.revision;
            Ok(current)
        })
    }
    /// Independent of any execution manager, plan or live process.
    pub fn query(&mut self, id: &Id) -> Result<View> {
        self.repository.transact(|tx| {
            let executions = executions(tx, id)?;
            let recovery = recovery::view(tx, id, &executions)?;
            Ok(View {
                registration: load(tx, id)?,
                executions,
                recovery,
                execution_ownership: "NOT_ESTABLISHED_BY_PERSISTENT_RECORDS",
                functional_readiness: crate::use_assessment::ReadinessAssessment::not_evaluated(),
                work_use_permission: crate::use_assessment::WorkUseAssessment::not_evaluated(),
            })
        })
    }
    pub fn list(&mut self) -> Result<Vec<VersionedRegistration>> {
        self.repository.transact(|tx| {
            tx.scan("components/registration/")?
                .iter()
                .map(|r| {
                    let registration: Registration = decode(r, REGISTRATION)?;
                    if r.key != key(&registration.id) {
                        return Err(StoreError::Integrity("registration key differs".into()));
                    }
                    Ok(VersionedRegistration {
                        revision: r.revision,
                        registration,
                    })
                })
                .collect()
        })
    }
    pub fn history(&mut self, id: &Id) -> Result<Vec<StoredEvent>> {
        // The shared port pages at 128. Never mistake the first page for complete history.
        let mut after = Counter(0);
        let mut history = vec![];
        loop {
            let page = self.repository.control_events_after(after, 128)?;
            if page.is_empty() {
                return Ok(history);
            }
            for event in page {
                after = event.seq;
                if event.document.schema.as_str() == EVENT {
                    let change: Change = serde_json::from_value(event.document.value.clone())
                        .map_err(|e| StoreError::Integrity(e.to_string()))?;
                    if change.registration == *id {
                        history.push(event);
                    }
                }
            }
        }
    }
    pub(crate) fn assign(&mut self, binding: &Binding) -> Result<()> {
        self.assign_with_resume(binding, None)
    }
    pub(crate) fn assign_with_resume(
        &mut self,
        binding: &Binding,
        permit: Option<&ResumePermit>,
    ) -> Result<()> {
        self.repository.transact(|tx| {
            eligible(tx, binding)?;
            if binding.registration == binding.instance { return Err(StoreError::Invalid("registration is not an execution instance".into())); }
            recovery::admit(tx,binding,permit)?;
            let execution = Execution { binding: binding.clone(), last_observed: Observation {
                state: ExecutionState::Assigned, pid: None, exit_code: None,
                detail: "assignment recorded before external effects; execution outcome not confirmed".into(),
            } };
            save(tx, &binding.registration, &execution_key(&binding.registration, &binding.instance), None, &document(EXECUTION, &execution)?, "execution-assigned")?;
            Ok(())
        })
    }
    pub(crate) fn check_start(&mut self, binding: &Binding) -> Result<()> {
        self.repository.transact(|tx| eligible(tx, binding))
    }
    pub(crate) fn observe(&mut self, binding: &Binding, observation: Observation) -> Result<()> {
        self.repository.transact(|tx| {
            let key = execution_key(&binding.registration, &binding.instance);
            let row = tx
                .get(&key)?
                .ok_or_else(|| StoreError::Integrity("execution assignment absent".into()))?;
            let mut execution: Execution = decode(&row, EXECUTION)?;
            if execution.binding != *binding {
                return Err(StoreError::Integrity(
                    "execution report binding differs".into(),
                ));
            }
            if execution.last_observed == observation {
                return Ok(());
            }
            if recovery::frozen(tx,&binding.registration,&binding.instance)? {
                return Err(StoreError::Invalid("original observation has a disposition; append separate evidence instead of rewriting history".into()));
            }
            execution.last_observed = observation;
            save(
                tx,
                &binding.registration,
                &key,
                Some(row.revision),
                &document(EXECUTION, &execution)?,
                "execution-observed",
            )?;
            Ok(())
        })
    }
}
