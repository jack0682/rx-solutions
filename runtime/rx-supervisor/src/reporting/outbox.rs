//! Durable report requests. Network failure never overwrites an entered request.
use super::{VerifiedScope, from_execution};
use crate::registration::Execution;
use rx_domain::{canonical, resident_reporting::*, types::*};
use rx_ports::{Document, Repository, StoreError, Transaction};
use serde::{Deserialize, Serialize};

const SCHEMA: &str = "rx.resident-report-outbox.v1";
const MAX_PENDING: usize = 128;
type Result<T> = rx_ports::Result<T>;
fn name(s: &str) -> Name {
    Name::new(s).expect("internal key")
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).expect("UUID")
}
fn key(instance: &Id) -> Name {
    name(&format!("resident-report/outbox/{instance}"))
}
fn doc<T: Serialize>(schema: &str, v: &T) -> Result<Document> {
    Ok(Document {
        schema: name(schema),
        value: serde_json::to_value(v).map_err(|e| StoreError::Invalid(e.to_string()))?,
    })
}
fn invalid(s: &str) -> StoreError {
    StoreError::Invalid(s.into())
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pending {
    pub key: Id,
    pub peer: Peer,
    pub scope: Scope,
    pub report: Report,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub latest: Execution,
    pub pending: Option<Pending>,
    pub accepted: Option<Receipt>,
    pub unresolved_deliveries: Counter,
}
fn load(tx: &mut dyn Transaction, instance: &Id) -> Result<Option<(Counter, Row)>> {
    tx.get(&key(instance))?
        .map(|r| {
            if r.document.schema != name(SCHEMA) {
                return Err(StoreError::Integrity("report outbox schema".into()));
            }
            let row: Row = canonical::decode_json(
                &canonical::bytes(&r.document.value).map_err(|e| invalid(&e.to_string()))?,
            )
            .map_err(|e| invalid(&e.to_string()))?;
            if row.latest.binding.instance != *instance
                || row.pending.as_ref().is_some_and(|p| {
                    p.report.source != row.latest.binding
                        || p.scope.id != p.report.scope
                        || p.scope.reporter_session != p.peer.id
                })
                || row
                    .accepted
                    .as_ref()
                    .is_some_and(|p| p.report.source != row.latest.binding)
            {
                return Err(StoreError::Integrity("report outbox identity".into()));
            }
            Ok((r.revision, row))
        })
        .transpose()
}
fn save(
    tx: &mut dyn Transaction,
    instance: &Id,
    revision: Option<Counter>,
    row: &Row,
) -> Result<()> {
    tx.put(&key(instance), revision, &doc(SCHEMA, row)?)?;
    Ok(())
}
fn event(tx: &mut dyn Transaction, row: &Row, value: &impl Serialize) -> Result<()> {
    let record = tx
        .get(&key(&row.latest.binding.instance))?
        .ok_or_else(|| invalid("outbox absent"))?;
    tx.append_control(
        &id(),
        &record,
        &doc("rx.resident-report-delivery-history.v1", value)?,
    )?;
    Ok(())
}

pub struct Outbox<R> {
    repository: R,
}
impl<R: Repository> Outbox<R> {
    pub fn new(repository: R) -> Self {
        Self { repository }
    }
    pub fn into_repository(self) -> R {
        self.repository
    }
    pub fn inspect(&mut self, instance: &Id) -> Result<Option<Row>> {
        self.repository
            .transact(|tx| Ok(load(tx, instance)?.map(|(_, row)| row)))
    }
    /// Include retained unsent history from earlier runs; source snapshots remain historical.
    pub fn retained(&mut self, limit: usize) -> Result<(Vec<Execution>, usize)> {
        self.repository.transact(|tx| {
            let mut values = Vec::new();
            let mut total = 0;
            for record in tx.scan("resident-report/outbox/")? {
                let row: Row = canonical::decode_json(
                    &canonical::bytes(&record.document.value)
                        .map_err(|e| invalid(&e.to_string()))?,
                )
                .map_err(|e| invalid(&e.to_string()))?;
                let instance = &row.latest.binding.instance;
                if record.key != key(instance) {
                    return Err(StoreError::Integrity("retained report key differs".into()));
                }
                let (_, row) = load(tx, instance)?.ok_or_else(|| invalid("retained row absent"))?;
                let observed = &row.latest.last_observed;
                let delivered = row.pending.is_none()
                    && row.accepted.as_ref().is_some_and(|r| {
                        r.report.state == observed.state
                            && r.report.pid == observed.pid
                            && r.report.exit_code == observed.exit_code
                            && r.report.detail == observed.detail
                    });
                if !delivered {
                    total += 1;
                    if values.len() < limit {
                        values.push(row.latest);
                    }
                }
            }
            Ok((values, total))
        })
    }

    /// Latest registry snapshots may coalesce before becoming requests; requests never coalesce.
    pub fn observe(&mut self, execution: Execution) -> Result<()> {
        if execution.last_observed.detail.len() > 2048 || execution.last_observed.pid == Some(0) {
            return Err(invalid("report observation bounds"));
        }
        self.repository.transact(|tx| {
            let instance = &execution.binding.instance;
            let existing = load(tx, instance)?;
            let (revision, mut row) = if let Some((revision, row)) = existing {
                if row.latest.binding != execution.binding {
                    return Err(invalid("source identity changed"));
                }
                if row.latest == execution {
                    return Ok(());
                }
                (Some(revision), row)
            } else {
                (
                    None,
                    Row {
                        latest: execution.clone(),
                        pending: None,
                        accepted: None,
                        unresolved_deliveries: Counter(0),
                    },
                )
            };
            row.latest = execution;
            let instance = row.latest.binding.instance.clone();
            save(tx, &instance, revision, &row)
        })
    }
    /// Validated server head plus owner-approved scope determines sequence continuity.
    /// An old uncertain request is retained in history, never labelled a success by succession.
    pub fn prepare(
        &mut self,
        peer: &Peer,
        scope: &VerifiedScope,
        head: &Head,
    ) -> Result<Option<Pending>> {
        let allowed = scope.view();
        if allowed.reporter_session != peer.id || head.scope != allowed.id {
            return Err(invalid("scope/head peer differs"));
        }
        self.repository.transact(|tx| {
            let (revision, mut row) = load(tx, &head.instance)?.ok_or_else(|| invalid("snapshot absent"))?;
            let latest = from_execution(scope, &row.latest, Counter(1)).map_err(|e| invalid(&e.to_string()))?;
            if head.receipt.as_ref().is_some_and(|r| r.component != allowed.component
                || r.component_revision != allowed.component_revision || r.report.source != latest.source) {
                return Err(invalid("head source differs"));
            }
            let mut history = None;
            if let Some(pending) = &row.pending {
                if pending.peer == *peer && pending.scope.id == allowed.id {
                    return Ok(Some(pending.clone()));
                }
                if allowed.root_scope() != pending.scope.root_scope() || allowed.continuation.is_none() {
                    return Err(invalid("owner-approved report continuation required"));
                }
                let recovered = head.receipt.as_ref().filter(|r| r.report == pending.report && r.reporter == pending.peer);
                // Preserve the exact original request and the limited finding before advancing.
                history = Some(serde_json::json!({"pending":pending,"finding":if recovered.is_some(){"ACCEPTED_RECEIPT_RECOVERED"}else{"PRIOR_DELIVERY_UNRESOLVED"},"head":head}));
                if recovered.is_none() { row.unresolved_deliveries = row.unresolved_deliveries.increment().map_err(|e| invalid(&e.to_string()))?; }
                row.pending = None;
            }
            if let Some(previous) = &row.accepted
                && head.receipt.as_ref().is_none_or(|r| r.report.sequence < previous.report.sequence) {
                return Err(invalid("server history regressed; explicit store reconciliation required"));
            }
            let accepted_changed = row.accepted != head.receipt;
            row.accepted = head.receipt.clone();
            let already_reported = head.receipt.as_ref().is_some_and(|r| {
                r.report.scope == allowed.id && r.reporter == *peer && r.report.state == latest.state && r.report.pid == latest.pid
                    && r.report.exit_code == latest.exit_code && r.report.detail == latest.detail
            });
            if !already_reported {
                let pending_count = tx.scan("resident-report/outbox/")?.iter().filter(|r| r.key != key(&head.instance) && !r.document.value["pending"].is_null()).count();
                if pending_count >= MAX_PENDING { return Err(invalid("report pending capacity exceeded")); }
                let sequence = head.receipt.as_ref().map_or(Ok(Counter(1)), |r| r.report.sequence.increment().map_err(|e| invalid(&e.to_string())))?;
                let report = Report { sequence, ..latest };
                row.pending = Some(Pending { key: id(), peer: peer.clone(), scope: allowed.clone(), report });
            }
            if already_reported && !accepted_changed && history.is_none() { return Ok(None); }
            save(tx, &head.instance, Some(revision), &row)?;
            if let Some(history) = history { event(tx, &row, &history)?; }
            if let Some(pending) = &row.pending {
                event(tx, &row, &serde_json::json!({"finding":"REQUEST_PREPARED","pending":pending}))?;
            }
            Ok(row.pending)
        })
    }
    pub fn acknowledge(&mut self, pending: &Pending, receipt: Receipt) -> Result<()> {
        if receipt.report != pending.report
            || receipt.reporter != pending.peer
            || receipt.component != pending.scope.component
            || receipt.component_revision != pending.scope.component_revision
        {
            return Err(invalid("acknowledgement differs"));
        }
        self.repository.transact(|tx| {
            let instance = &pending.report.source.instance;
            let (revision, mut row) =
                load(tx, instance)?.ok_or_else(|| invalid("pending absent"))?;
            if row.pending.as_ref() != Some(pending) {
                if row.pending.is_none() && row.accepted.as_ref() == Some(&receipt) {
                    return Ok(());
                }
                return Err(invalid("pending request changed"));
            }
            row.pending = None;
            row.accepted = Some(receipt.clone());
            save(tx, instance, Some(revision), &row)?;
            event(
                tx,
                &row,
                &serde_json::json!({"pending":pending,"finding":"ACCEPTED","receipt":receipt}),
            )
        })
    }
}

#[cfg(test)]
mod tests;
