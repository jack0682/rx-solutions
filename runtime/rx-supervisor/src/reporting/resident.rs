//! Optional delivery worker. It never owns or calls the local process supervisor.
use super::{Client, Connection, Error, outbox::Outbox};
use crate::registration::Execution;
use rx_domain::{canonical, resident_reporting::Peer, types::*};
use rx_storage::SqliteRepository;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::sync::{oneshot, watch};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub connection: PathBuf,
    pub scopes: PathBuf,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectionFile {
    schema: String,
    endpoint: String,
    server_name: String,
    ca: PathBuf,
    certificate: PathBuf,
    private_key: PathBuf,
    principal: Name,
    installation: Id,
    store_generation: Id,
    shared_clock_id: String,
    release_digest: Digest,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    component: Id,
    registration: Id,
    scope: Id,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scopes {
    schema: String,
    selections: BTreeMap<Name, Selection>,
    #[serde(default)]
    instances: BTreeMap<Id, Selection>,
}
fn read(path: &Path) -> Result<Vec<u8>> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 1_048_576 {
        return Err("reporting file type/size".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(1_048_577)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1_048_576 {
        return Err("reporting file too large".into());
    }
    Ok(bytes)
}
fn connection(path: &Path) -> Result<Connection> {
    let value: ConnectionFile = canonical::decode_json(&read(path)?)?;
    if value.schema != "rx.resident-report-connection.v1" {
        return Err("reporting connection schema".into());
    }
    Ok(Connection {
        endpoint: value.endpoint,
        server_name: value.server_name,
        ca_pem: read(&value.ca)?,
        certificate_pem: read(&value.certificate)?,
        private_key_pem: read(&value.private_key)?,
        principal: value.principal,
        installation: value.installation,
        store_generation: value.store_generation,
        shared_clock_id: value.shared_clock_id,
        release_digest: value.release_digest,
    })
}
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Status {
    pub peer: Option<Peer>,
    pub retained_snapshots: usize,
    pub pending_requests: usize,
    pub held_snapshots: usize,
    pub delivered_snapshots: usize,
    pub unresolved_deliveries: u64,
    pub error: Option<String>,
    pub stopped: bool,
}
#[derive(Clone, Default)]
struct Input {
    snapshots: Vec<Execution>,
    finish: bool,
}

pub struct Resident {
    input: watch::Sender<Input>,
    status: watch::Receiver<Status>,
    done: Option<oneshot::Receiver<()>>,
}
impl Resident {
    pub fn start(config: Configuration, journal: PathBuf) -> Result<Self> {
        let (input, receiver) = watch::channel(Input::default());
        let (status, view) = watch::channel(Status::default());
        let (done, finished) = oneshot::channel();
        std::thread::Builder::new()
            .name("rx-resident-reporting".into())
            .spawn(move || {
                let result = (|| -> Result<()> {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    runtime.block_on(deliver(config, journal, receiver, status.clone()))
                })();
                status.send_modify(|s| {
                    if let Err(e) = result {
                        s.error = Some(format!("reporter stopped: {e}"));
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
    /// Bounded latest-value handoff. The source registry retains the authoritative snapshots.
    pub fn observe(&self, snapshots: Vec<Execution>) -> Result<()> {
        if snapshots.len() > 128 {
            return Err("reporting selection capacity exceeded".into());
        }
        if self.input.is_closed() {
            return Err("reporting worker stopped; registry snapshots retained".into());
        }
        if self.input.borrow().snapshots != snapshots {
            self.input.send_replace(Input {
                snapshots,
                finish: false,
            });
        }
        Ok(())
    }
    pub fn status(&self) -> Status {
        self.status.borrow().clone()
    }
    /// Called only after the local children have stopped. Delivery has a finite tail.
    pub async fn finish(mut self, snapshots: Vec<Execution>) -> Status {
        if snapshots.len() > 128 {
            let mut status = self.status();
            status.error =
                Some("reporting selection capacity exceeded; source registry retained".into());
            return status;
        }
        self.input.send_replace(Input {
            snapshots,
            finish: true,
        });
        if let Some(done) = self.done.take() {
            let error = match tokio::time::timeout(Duration::from_secs(9), done).await {
                Ok(Ok(())) => None,
                Ok(Err(_)) => Some("reporting worker ended without a completion signal"),
                Err(_) => {
                    Some("reporting drain deadline; pending requests and source registry retained")
                }
            };
            if let Some(error) = error {
                let mut status = self.status();
                status.error = Some(error.into());
                return status;
            }
        }
        self.status()
    }
}
async fn attempt(
    client: &mut Client,
    outbox: &mut Outbox<SqliteRepository>,
    selected: &Selection,
    execution: &Execution,
) -> Result<()> {
    let scope = client
        .inspect_scope(&selected.scope, &selected.component, &selected.registration)
        .await?;
    let head = client.head(&scope, &execution.binding.instance).await?;
    if let Some(pending) = outbox.prepare(client.peer(), &scope, &head)? {
        let receipt = client
            .publish(&scope, &pending.key, pending.report.clone())
            .await?;
        outbox.acknowledge(&pending, receipt)?;
    }
    Ok(())
}
async fn deliver(
    config: Configuration,
    journal: PathBuf,
    mut input: watch::Receiver<Input>,
    status: watch::Sender<Status>,
) -> Result<()> {
    let repository = SqliteRepository::open(journal)?;
    let mut outbox = Outbox::new(repository);
    let boot = Id::new(uuid::Uuid::new_v4().to_string())?;
    let mut client: Option<Client> = None;
    let mut finish_at = None;
    let mut cursor = 0usize;
    loop {
        let input_value = input.borrow_and_update().clone();
        if input_value.finish {
            finish_at.get_or_insert_with(Instant::now);
        }
        // Persist snapshots before network I/O; failed writes leave the source registry intact.
        let persistence = input_value
            .snapshots
            .iter()
            .try_for_each(|execution| outbox.observe(execution.clone()));
        if let Err(error) = persistence {
            status.send_modify(|s| {
                s.error = Some(format!(
                    "report persistence unavailable; source registry retained: {error}"
                ))
            });
            if finish_at.is_some_and(|at| at.elapsed() >= Duration::from_secs(7)) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
            if input.has_changed().is_err() {
                return Ok(());
            }
            continue;
        }
        let (retained, total_retained) = outbox.retained(128)?;
        let unlisted = total_retained.saturating_sub(retained.len());
        let mut sources: BTreeMap<Id, Execution> = retained
            .into_iter()
            .map(|e| (e.binding.instance.clone(), e))
            .collect();
        sources.extend(
            input_value
                .snapshots
                .iter()
                .cloned()
                .map(|e| (e.binding.instance.clone(), e)),
        );
        let snapshots: Vec<_> = sources.into_values().collect();
        let mut current = status.borrow().clone();
        current.retained_snapshots = snapshots.len();
        current.held_snapshots = unlisted;
        current.error = None;
        if client.is_none() {
            match connection(&config.connection) {
                Ok(connection) => match Client::connect_incarnation(connection, boot.clone()).await
                {
                    Ok(connected) => {
                        current.peer = Some(connected.peer().clone());
                        client = Some(connected);
                    }
                    Err(e) => current.error = Some(e.to_string()),
                },
                Err(e) => current.error = Some(e.to_string()),
            }
        }
        if let Some(connected) = client.as_mut() {
            let scopes = read(&config.scopes)
                .and_then(|bytes| Ok(canonical::decode_json::<Scopes>(&bytes)?));
            match scopes {
                Ok(scopes)
                    if scopes.schema == "rx.resident-report-scopes.v1"
                        && scopes.selections.len() <= 128
                        && scopes.instances.len() <= 128 =>
                {
                    if let Some(execution) = snapshots.get(cursor % snapshots.len().max(1)) {
                        cursor = cursor.wrapping_add(1);
                        if let Some(selection) = scopes
                            .instances
                            .get(&execution.binding.instance)
                            .or_else(|| scopes.selections.get(&execution.binding.selection))
                        {
                            if selection.registration != execution.binding.registration {
                                current.error =
                                    Some("configured reporting registration differs".into());
                            } else if let Err(e) =
                                attempt(connected, &mut outbox, selection, execution).await
                            {
                                let stale = e.downcast_ref::<Error>().is_some_and(|e| matches!(e, Error::Rpc(s) if s.code() == tonic::Code::Unauthenticated));
                                current.error = Some(e.to_string());
                                if stale {
                                    client = None;
                                    current.peer = None;
                                }
                            }
                        } else {
                            current.error =
                                Some("owner reporting scope required for selection".into());
                        }
                    }
                }
                Ok(_) => current.error = Some("reporting scopes schema/capacity".into()),
                Err(e) => current.error = Some(format!("owner reporting scopes unavailable: {e}")),
            }
        }
        current.pending_requests = 0;
        current.delivered_snapshots = 0;
        current.unresolved_deliveries = 0;
        let mut all_delivered = current.error.is_none() && unlisted == 0;
        for execution in &snapshots {
            let row = outbox
                .inspect(&execution.binding.instance)?
                .ok_or("outbox snapshot absent")?;
            current.pending_requests += usize::from(row.pending.is_some());
            current.unresolved_deliveries = current
                .unresolved_deliveries
                .saturating_add(row.unresolved_deliveries.0);
            let delivered = row.pending.is_none()
                && row.accepted.as_ref().is_some_and(|r| {
                    let o = &execution.last_observed;
                    current.peer.as_ref() == Some(&r.reporter)
                        && r.report.state == o.state
                        && r.report.pid == o.pid
                        && r.report.exit_code == o.exit_code
                        && r.report.detail == o.detail
                });
            all_delivered &= delivered;
            current.delivered_snapshots += usize::from(delivered);
        }
        status.send_replace(current);
        if finish_at.is_some() && all_delivered {
            return Ok(());
        }
        if finish_at.is_some_and(|at| at.elapsed() >= Duration::from_secs(7)) {
            status.send_modify(|s| {
                s.error = Some(
                    "reporting drain incomplete; requests and source snapshots retained".into(),
                )
            });
            return Ok(());
        }
        tokio::select! {
            changed = input.changed() => { if changed.is_err() { return Ok(()); } },
            _ = tokio::time::sleep(Duration::from_millis(500)) => {},
        }
    }
}
