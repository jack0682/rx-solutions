use super::*;
use crate::client::{CompletedVisit, ProductionBasis, ValidatedProduction};
use rx_process_contract::{
    execution::{PartDisposition, RunState},
    production,
};
pub enum Coordination {
    Waiting,
    Visit(Counter),
    Retire(CompletedVisit),
    Finished,
    Deferred(Outcome),
    Attention,
}
impl<R: Repository> Worker<R> {
    pub async fn production_view(&mut self) -> Result<ValidatedProduction, Error> {
        let view = self
            .client
            .production_view(&self.journal.scope().run)
            .await?;
        if view.data().resolved.sha256 != self.journal.scope().resolved_digest {
            return Err(Error::Invalid("production recipe changed".into()));
        }
        Ok(view)
    }
    pub async fn coordinate(
        &mut self,
        view: &ValidatedProduction,
        current: Option<Counter>,
        complete_hint: bool,
    ) -> Result<Coordination, Error> {
        let data = view.data();
        let run = &data.run.run;
        if run.id != self.journal.scope().run || run.cell != self.journal.scope().cell {
            return Err(Error::Invalid("coordinator scope differs".into()));
        }
        for part in &data.parts {
            let source = ProductionBasis {
                view,
                visit: part.ordinal,
            };
            self.journal.observe(Observation {
                logical: logical(part.ordinal, Stage::BeginPart),
                target: ObservedTarget::Part {
                    value: part.clone(),
                },
                basis: source.journal_basis(),
            })?;
            if part.disposition == PartDisposition::ConfirmedCompleted {
                self.journal.observe(Observation {
                    logical: logical(part.ordinal, Stage::CompletePart),
                    target: ObservedTarget::Part {
                        value: part.clone(),
                    },
                    basis: source.journal_basis(),
                })?;
            }
        }
        if let Some(ordinal) = current {
            let part = data
                .parts
                .iter()
                .find(|p| p.ordinal == ordinal)
                .ok_or_else(|| Error::Invalid("active coordinator part disappeared".into()))?;
            if part.disposition == PartDisposition::ConfirmedCompleted {
                return Ok(view
                    .completed_visit(ordinal)
                    .map(Coordination::Retire)
                    .unwrap_or(Coordination::Waiting));
            }
        }
        if run.state == RunState::Completed {
            return Ok(Coordination::Finished);
        }
        if !view.is_current() || !data.admission_allowed {
            return Ok(Coordination::Waiting);
        }
        if run.purpose != Some(Purpose::Production) {
            return Ok(Coordination::Attention);
        }
        if data.parts.iter().any(|p| {
            !matches!(
                p.disposition,
                PartDisposition::InProgress | PartDisposition::ConfirmedCompleted
            )
        }) {
            return Ok(Coordination::Attention);
        }
        // This worker coordinates serial production only. P may legitimately expose multiple
        // active parts; selecting the first would silently choose which material to operate on.
        // Preserve the completed-current-visit retirement and observation handling above.
        let active = match serial_in_progress(&data.parts) {
            Ok(part) => part,
            Err(MultipleActiveParts) => return Ok(Coordination::Attention),
        };
        if let Some(part) = active {
            if current.is_some_and(|c| c != part.ordinal) {
                return Ok(Coordination::Attention);
            }
            if !complete_hint {
                return Ok(Coordination::Visit(part.ordinal));
            }
            let snapshot = self.snapshot(part.ordinal).await?;
            if snapshot.context_identity() != view.identity(part.ordinal) {
                return Ok(Coordination::Deferred(Outcome::ContextChanged));
            }
            if snapshot.frontier().state != rx_process_contract::frontier::State::Completed {
                return Ok(Coordination::Visit(part.ordinal));
            }
            let body = Body::CompletePart {
                ordinal: part.ordinal,
                command: Box::new(production::CompletePart {
                    cell: run.cell.clone(),
                    run: run.id.clone(),
                    part: part.id.clone(),
                    expected_run: snapshot.data().run.revision,
                    expected_part: part.revision,
                }),
            };
            // Use the fresh execution cut for completion, preserving the same logical visit.
            return match self
                .execute(logical(part.ordinal, Stage::CompletePart), body, &snapshot)
                .await?
            {
                Execution::Reply(Response::Part(_)) => Ok(Coordination::Waiting),
                Execution::Reply(_) => Err(Error::Invalid("part completion reply differs".into())),
                Execution::Deferred(value) => Ok(Coordination::Deferred(value)),
            };
        }
        let budget = run
            .budget
            .as_ref()
            .ok_or_else(|| Error::Invalid("production budget missing".into()))?;
        if budget.remaining().0 == 0 {
            return Ok(Coordination::Attention);
        }
        let ordinal = Counter(data.parts.len() as u64 + 1);
        let source = ProductionBasis {
            view,
            visit: ordinal,
        };
        let body = Body::BeginPart {
            cell: run.cell.clone(),
            run: run.id.clone(),
            ordinal,
            mandate: run
                .mandate
                .clone()
                .ok_or_else(|| Error::Invalid("production mandate missing".into()))?,
            expected_budget: budget.revision(),
            expected_cell: data.cell_revision,
        };
        match self
            .execute(logical(ordinal, Stage::BeginPart), body, &source)
            .await?
        {
            Execution::Reply(Response::Part(part)) => Ok(Coordination::Visit(part.ordinal)),
            Execution::Reply(_) => Err(Error::Invalid("part begin reply differs".into())),
            Execution::Deferred(value) => Ok(Coordination::Deferred(value)),
        }
    }
}
struct MultipleActiveParts;
fn serial_in_progress(
    parts: &[production::Part],
) -> std::result::Result<Option<&production::Part>, MultipleActiveParts> {
    let mut active = parts
        .iter()
        .filter(|p| p.disposition == PartDisposition::InProgress);
    let first = active.next();
    if active.next().is_some() {
        return Err(MultipleActiveParts);
    }
    Ok(first)
}
fn logical(visit: Counter, stage: Stage) -> Logical {
    Logical {
        visit,
        node: name("production/part"),
        stage,
        control: None,
    }
}

#[cfg(test)]
mod tests;
