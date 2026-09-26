//! Result and decision consumption share one registration repository transaction.
use super::*;
use crate::{
    decision,
    work_use::{Input, Report, Task},
};
const RESULT: &str = "rx.work-support-gap-result.v1";
const CONSUMPTION: &str = "rx.work-decision-consumption.v1";
fn result_key(operation: &Id) -> Name {
    name(&format!("work/result/{operation}"))
}
fn consumption_key(component: &Id, decision: &Id) -> Name {
    name(&format!("work/consumed/{component}/{decision}"))
}
fn invalid(reason: &str) -> StoreError {
    StoreError::Invalid(format!("work-use/{reason}"))
}
#[derive(Serialize)]
struct Consumption<'a> {
    operation: &'a Id,
    reference: &'a decision::Reference,
}
impl<R: Repository> Registry<R> {
    pub(crate) fn ensure_new_work(&mut self, operation: &Id) -> Result<()> {
        self.repository.transact(|tx| {
            if tx.get(&result_key(operation))?.is_some() {
                return Err(StoreError::RevisionConflict(
                    "work-use/operation-already-committed; query history, never replay".into(),
                ));
            }
            Ok(())
        })
    }
    pub(crate) fn commit_work(
        &mut self,
        task: &Task,
        input: &Input,
        guard: &decision::CommitGuard<'_>,
    ) -> Result<Report> {
        // The receiving caller retains guard across the entire transact, including
        // SDK commit IO. Revocation cannot interleave with a successful commit.
        self.repository.transact(|tx| {
            let reference = guard.check().map_err(|e| invalid(&e.to_string()))?;
            let current = load(tx, &input.subject.registration)?;
            if current.registration.state != RegistrationState::Accepted
                || current.revision != input.subject.registration_revision
                || current.registration.declaration.catalog.program != input.subject.program
                || current.registration.declaration.catalog.digest != input.subject.catalog_digest
            {
                return Err(invalid("current-registration-changed"));
            }
            let instance = input
                .subject
                .instance
                .as_ref()
                .ok_or_else(|| invalid("instance-missing"))?;
            let row = tx
                .get(&execution_key(&input.subject.registration, instance))?
                .ok_or_else(|| invalid("execution-missing"))?;
            let execution: Execution = decode(&row, EXECUTION)?;
            if execution.binding.run != input.subject.run
                || execution.binding.selection != task.selection
                || execution.binding.registration_revision != input.subject.registration_revision
                || execution.binding.catalog != current.registration.declaration.catalog
                || execution.last_observed.state != ExecutionState::Running
                || execution.last_observed.pid != input.subject.pid
            {
                return Err(invalid("current-execution-changed"));
            }
            // CAS None makes operation and grant consumption unique. Either both
            // writes/control events commit or neither does; no in-memory spending.
            let report = input
                .report(task, reference.clone())
                .map_err(|e| invalid(&e.to_string()))?;
            save(
                tx,
                &report.registration,
                &result_key(&task.operation),
                None,
                &document(RESULT, &report)?,
                "work-result-committed",
            )?;
            save(
                tx,
                &report.registration,
                &consumption_key(&report.registration, &reference.decision),
                None,
                &document(
                    CONSUMPTION,
                    &Consumption {
                        operation: &task.operation,
                        reference: &reference,
                    },
                )?,
                "work-decision-consumed",
            )?;
            // Logical use/completion cut: last check before returning to commit.
            // TTL may expire during subsequent commit IO; the self-report remains
            // an observation at report.observed_at, not atomic physical truth.
            guard.check().map_err(|e| invalid(&e.to_string()))?;
            Ok(report)
        })
    }
    /// Historical recovery by operation ID after response loss. Never permission.
    pub fn recorded_work(&mut self, component: &Id, operation: &Id) -> Result<Report> {
        self.repository.transact(|tx| {
            let row = tx
                .get(&result_key(operation))?
                .ok_or_else(|| invalid("result-not-found"))?;
            let value: Report = decode(&row, RESULT)?;
            if value.registration != *component || value.task.operation != *operation {
                return Err(StoreError::Integrity("work result identity differs".into()));
            }
            Ok(value)
        })
    }
}
