//! Durable execution-result delivery and remote stop observation. Never calls the local manager.
use super::*;
use rx_ports::{Document, Repository, StoreError};
use rx_storage::SqliteRepository;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::sync::{oneshot, watch};
#[derive(Clone, Default, Serialize)]
pub struct Status {
    pub stop_requested: bool,
    pub pending: bool,
    pub delivered_sequence: Counter,
    pub error: Option<String>,
    pub stopped: bool,
}
#[derive(Clone, Default)]
struct Input {
    observation: Option<data::Observation>,
    finish: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    key: Id,
    observation: data::Observation,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    peer: data::Peer,
    assignment: Id,
    grant: Id,
    latest: Option<data::Observation>,
    pending: Option<Pending>,
    accepted: Option<data::ObservationReceipt>,
}
const KEY: &str = "resident-execution/delivery";
const SCHEMA: &str = "rx.resident-execution-delivery.v1";
fn n(s: &str) -> Name {
    Name::new(s).expect("fixed key")
}
fn load(tx: &mut dyn rx_ports::Transaction) -> rx_ports::Result<(Counter, Row)> {
    let row = tx
        .get(&n(KEY))?
        .ok_or_else(|| StoreError::Integrity("execution outbox absent".into()))?;
    if row.document.schema != n(SCHEMA) {
        return Err(StoreError::Integrity("execution outbox schema".into()));
    }
    let value = canonical::decode_json(
        &canonical::bytes(&row.document.value).map_err(|e| StoreError::Integrity(e.to_string()))?,
    )
    .map_err(|e| StoreError::Integrity(e.to_string()))?;
    Ok((row.revision, value))
}
fn save(
    tx: &mut dyn rx_ports::Transaction,
    revision: Option<Counter>,
    row: &Row,
) -> rx_ports::Result<()> {
    tx.put(
        &n(KEY),
        revision,
        &Document {
            schema: n(SCHEMA),
            value: serde_json::to_value(row).map_err(|e| StoreError::Invalid(e.to_string()))?,
        },
    )?;
    Ok(())
}
pub struct Delivery {
    input: watch::Sender<Input>,
    status: watch::Receiver<Status>,
    done: Option<oneshot::Receiver<()>>,
}
impl Delivery {
    pub(crate) fn start(client: Client, lease: Arc<Lease>, path: PathBuf) -> Result<Self> {
        let (input, rx) = watch::channel(Input::default());
        let (status, view) = watch::channel(Status::default());
        let (done, finished) = oneshot::channel();
        std::thread::Builder::new()
            .name("rx-execution-delivery".into())
            .spawn(move || {
                let result = (|| -> Result<()> {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    rt.block_on(deliver(client, lease, path, rx, status.clone()))
                })();
                status.send_modify(|s| {
                    if let Err(e) = result {
                        s.error = Some(e.to_string());
                    }
                    s.stopped = true;
                });
                let _ = done.send(());
            })?;
        Ok(Self {
            input,
            status: view,
            done: Some(finished),
        })
    }
    pub fn observe(&self, observation: data::Observation) -> Result<()> {
        if self.input.is_closed() {
            return Err(invalid("execution delivery stopped; source state retained"));
        }
        if self.input.borrow().observation.as_ref() != Some(&observation) {
            self.input.send_replace(Input {
                observation: Some(observation),
                finish: false,
            });
        }
        Ok(())
    }
    pub fn status(&self) -> Status {
        self.status.borrow().clone()
    }
    pub async fn finish(mut self, observation: data::Observation) -> Status {
        self.input.send_replace(Input {
            observation: Some(observation),
            finish: true,
        });
        if let Some(done) = self.done.take()
            && !matches!(
                tokio::time::timeout(Duration::from_secs(9), done).await,
                Ok(Ok(()))
            )
        {
            let mut s = self.status();
            s.error = Some("execution delivery deadline; original records retained".into());
            return s;
        }
        self.status()
    }
}
async fn deliver(
    mut client: Client,
    lease: Arc<Lease>,
    path: PathBuf,
    mut input: watch::Receiver<Input>,
    status: watch::Sender<Status>,
) -> Result<()> {
    let mut store = SqliteRepository::open(path)?;
    store.transact(|tx|{
        tx.require_resident_execution_reader()?;
        if tx.get(&n(KEY))?.is_some(){let (_,old)=load(tx)?;if old.peer!=*client.peer()||old.assignment!=lease.intent.id||old.grant!=lease.grant.id{return Err(StoreError::Integrity("prior execution delivery belongs to another incarnation; explicit reconciliation required".into()));}}
        else{save(tx,None,&Row{peer:client.peer().clone(),assignment:lease.intent.id.clone(),grant:lease.grant.id.clone(),latest:None,pending:None,accepted:None})?;}Ok(())
    })?;
    let mut finish_at = None;
    loop {
        let value = input.borrow_and_update().clone();
        if value.finish {
            finish_at.get_or_insert_with(Instant::now);
        }
        let result = store.transact(|tx| {
            let (revision, mut row) = load(tx)?;
            let mut changed = false;
            if let Some(mut latest) = value.observation.clone() {
                if latest.assignment != row.assignment || latest.grant != row.grant {
                    return Err(StoreError::Integrity(
                        "execution delivery identity differs".into(),
                    ));
                }
                latest.sequence = Counter(0);
                if row.latest.as_ref() != Some(&latest) {
                    row.latest = Some(latest);
                    changed = true;
                }
            }
            if row.pending.is_none()
                && let Some(latest) = &row.latest
            {
                let mut accepted = row.accepted.as_ref().map(|r| r.observation.clone());
                if let Some(a) = &mut accepted {
                    a.sequence = Counter(0);
                }
                if accepted.as_ref() != Some(latest) {
                    let mut observation = latest.clone();
                    observation.sequence = Counter(
                        row.accepted
                            .as_ref()
                            .map_or(1, |v| v.observation.sequence.0 + 1),
                    );
                    row.pending = Some(Pending {
                        key: Id::new(uuid::Uuid::new_v4().to_string()).expect("UUID"),
                        observation,
                    });
                    changed = true;
                }
            }
            if changed {
                save(tx, Some(revision), &row)?;
            }
            Ok(row)
        });
        let mut state = status.borrow().clone();
        state.error = None;
        match result {
            Err(error) => state.error = Some(error.to_string()),
            Ok(row) => {
                state.pending = row.pending.is_some();
                state.delivered_sequence = row
                    .accepted
                    .as_ref()
                    .map_or(Counter(0), |r| r.observation.sequence);
                if let Some(pending) = row.pending {
                    match client.observe(&pending.key, &pending.observation).await {
                        Ok(receipt) => {
                            let stored = store.transact(|tx| {
                                let (revision, mut row) = load(tx)?;
                                if row.pending.as_ref().is_none_or(|p| {
                                    p.key != pending.key || p.observation != receipt.observation
                                }) {
                                    return Err(StoreError::Integrity(
                                        "execution receipt context differs".into(),
                                    ));
                                }
                                row.accepted = Some(receipt.clone());
                                row.pending = None;
                                save(tx, Some(revision), &row)
                            });
                            match stored {
                                Ok(()) => {
                                    state.delivered_sequence = receipt.observation.sequence;
                                    state.pending = false;
                                }
                                Err(e) => state.error = Some(e.to_string()),
                            }
                        }
                        Err(e) => state.error = Some(e.to_string()),
                    }
                }
                match client.inspect(&lease.intent.id).await {
                    Ok(view) => {
                        if view.assignment.intent != lease.intent
                            || view.assignment.grant.as_ref() != Some(&lease.grant)
                        {
                            lease.block();
                            state.stop_requested = true;
                            state.error = Some("assignment changed; local stop required".into());
                        }
                        if view.assignment.stop_requested || !view.peer_current {
                            lease.block();
                            state.stop_requested = true;
                        }
                    }
                    Err(e) => {
                        if matches!(&e,Error::Rpc(s) if matches!(s.code(),tonic::Code::Unauthenticated|tonic::Code::PermissionDenied|tonic::Code::FailedPrecondition))
                        {
                            lease.block();
                            state.stop_requested = true;
                        }
                        state.error = Some(e.to_string());
                    }
                }
            }
        }
        status.send_replace(state);
        if finish_at.is_some() {
            let row = store.transact(|tx| Ok(load(tx)?.1))?;
            let delivered = match (&row.latest, &row.accepted) {
                (Some(latest), Some(accepted)) => {
                    let mut a = accepted.observation.clone();
                    a.sequence = Counter(0);
                    a == *latest && row.pending.is_none()
                }
                _ => false,
            };
            if delivered || finish_at.is_some_and(|t| t.elapsed() >= Duration::from_secs(7)) {
                break;
            }
        }
        if input.has_changed().is_err() {
            break;
        }
        tokio::select! {_=tokio::time::sleep(Duration::from_millis(200))=>{},_=input.changed()=>{}}
    }
    store.close()?;
    Ok(())
}
