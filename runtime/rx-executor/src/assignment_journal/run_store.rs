use super::*;
const BINDING: &str = "rx.executor-run-journal-binding.v1";
const RUN_HEADER: &str = "rx.executor-journal.v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Binding {
    service: Identity,
    pub(super) preparation: Preparation,
}

/// Verified repository, intentionally not a Worker/Journal before attach commits.
pub struct RunStore<R> {
    pub(super) repository: R,
    pub(super) binding: Binding,
}
impl<R: Repository> RunStore<R> {
    pub fn into_repository(self) -> R {
        self.repository
    }
    pub(super) fn verify(&mut self) -> Result<()> {
        self.repository.transact(|tx| verify(tx, &self.binding))
    }
}
impl<R: Repository> AssignmentJournal<R> {
    /// A new empty run repository may be initialized only while its exact reservation is Preparing.
    /// Existing request journals require explicit migration; this never adds an identity to one.
    pub fn initialize_run<Q: Repository>(&mut self, id: &Id, repository: Q) -> Result<RunStore<Q>> {
        let binding = self.enter_run_creation(id)?;
        initialize_entered(binding, repository)
    }

    /// Required-open works in all phases for recovery/audit. It never initializes missing headers.
    pub fn open_run_required<Q: Repository>(
        &mut self,
        id: &Id,
        mut repository: Q,
    ) -> Result<RunStore<Q>> {
        let (binding, _) = self.run_binding(id)?;
        if !self.creation_entered(id)? {
            return integrity("run journal has no durable creation entry");
        }
        repository.transact(|tx| verify(tx, &binding))?;
        Ok(RunStore {
            repository,
            binding,
        })
    }

    /// Commit before create_new or run-header writes. A repeated/unknown entry is never replayed.
    pub(super) fn enter_run_creation(&mut self, id: &Id) -> Result<Binding> {
        self.repository.transact(|tx| {
            let state = audit(tx, &self.identity)?;
            let record = read(tx, id)?;
            if record.value.phase != Phase::Preparing || state.current.as_ref() != Some(id) {
                return invalid("only current Preparing may enter run initialization");
            }
            let preparation = record.value.preparation;
            if creation_entered(tx, &preparation)? {
                return invalid("run creation already entered; required-open recovery only");
            }
            tx.put(&creation_key(id), None, &doc(CREATION, &preparation)?)?;
            let event_id = Id::new(uuid::Uuid::new_v4().to_string()).expect("UUID");
            tx.append(&event_id, &doc(CREATION, &preparation)?)?;
            Ok(Binding {
                service: self.identity.clone(),
                preparation,
            })
        })
    }

    pub(super) fn run_binding(&mut self, id: &Id) -> Result<(Binding, Phase)> {
        let record = self
            .get(id)?
            .ok_or_else(|| StoreError::Integrity("run reservation missing".into()))?;
        Ok((
            Binding {
                service: self.identity.clone(),
                preparation: record.value.preparation,
            },
            record.value.phase,
        ))
    }
}

/// Only the current call that successfully committed enter_run_creation reaches this function.
pub(super) fn initialize_entered<Q: Repository>(
    binding: Binding,
    mut repository: Q,
) -> Result<RunStore<Q>> {
    let (head, records) = repository.snapshot()?;
    if head.0 != 0 || !records.is_empty() {
        return invalid("run initialization requires an empty store/history");
    }
    repository.transact(|tx| {
        if tx.get(&name("executor/header"))?.is_some() || tx.control_head()?.0 != 0 {
            return invalid("new run journal must be empty");
        }
        tx.put(
            &name("executor/header"),
            None,
            &doc(RUN_HEADER, &binding.preparation.scope)?,
        )?;
        tx.put(
            &name("executor/attachment-binding"),
            None,
            &doc(BINDING, &binding)?,
        )?;
        Ok(())
    })?;
    Ok(RunStore {
        repository,
        binding,
    })
}
fn verify(tx: &mut dyn Transaction, binding: &Binding) -> Result<()> {
    let header = required(tx, &name("executor/header"))?;
    if header.revision != Counter(1)
        || decode::<Scope>(&header, RUN_HEADER)? != binding.preparation.scope
    {
        return integrity("required run journal Scope differs");
    }
    let row = required(tx, &name("executor/attachment-binding"))?;
    if row.revision != Counter(1) || !same(&decode::<Binding>(&row, BINDING)?, binding)? {
        return integrity("required run journal identity differs");
    }
    Ok(())
}
pub(super) fn verify_recovery(
    tx: &mut dyn Transaction,
    service: &Identity,
    preparation: &Preparation,
) -> Result<()> {
    verify(
        tx,
        &Binding {
            service: service.clone(),
            preparation: preparation.clone(),
        },
    )
}
pub(super) fn matches(
    service: &Identity,
    preparation: &Preparation,
    binding: &Binding,
) -> Result<()> {
    if binding.service != *service || !same(&binding.preparation, preparation)? {
        return integrity("run repository belongs to another service or reservation");
    }
    Ok(())
}
