//! Durable software lifecycle and change preparation. No native reconfiguration or activation.
use super::{Result, config::Loaded};
use crate::{
    HostMeta,
    gate::StopSnapshot,
    journal::{decode, doc, key, name},
};
use rx_domain::{canonical, types::*};
use rx_ports::{Repository, StoreError, Transaction};
use rx_storage::SqliteRepository;
use serde::{Deserialize, Serialize};

const LIFECYCLE: &str = "rx.host.service-lifecycle.v1";
const PREPARATION: &str = "rx.host.binding-preparation.v1";
const ACTIVE: &str = "host-maintenance/active";
const STATE: &str = "host-service/lifecycle";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StopSeal {
    pub attempt: Id,
    pub installation_identity: Digest,
    pub startup_configuration_digest: Digest,
    pub delivery_journal: Id,
    pub evidence_journal: Id,
    pub delivery_sequence: Counter,
    pub journal_tail: Counter,
    pub state_digest: Digest,
    pub snapshot: StopSnapshot,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "phase",
    rename_all = "SCREAMING_SNAKE_CASE",
    deny_unknown_fields
)]
enum Lifecycle {
    Starting {
        attempt: Id,
        installation_identity: Digest,
        startup_configuration_digest: Digest,
    },
    Stopped {
        seal: Box<StopSeal>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum State {
    Prepared,
    Cancelled,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preparation {
    pub request: Id,
    pub request_digest: Digest,
    pub state: State,
    pub plan_digest: Digest,
    pub current_identity: Digest,
    pub proposed_identity: Digest,
    pub proposed_bindings_digest: Digest,
    pub proposed_configuration: super::config::Configuration,
    pub stop: StopSeal,
    pub installation_changed: bool,
    pub activation_authorized: bool,
}
pub(crate) fn startup_digest(config: &super::config::Configuration) -> rx_ports::Result<Digest> {
    canonical::digest("RX-HOST-SERVICE-STARTUP-v1", config)
        .map_err(|e| StoreError::Invalid(e.to_string()))
}
fn digest(v: &impl Serialize) -> rx_ports::Result<Digest> {
    canonical::digest("RX-HOST-MAINTENANCE-v1", v).map_err(|e| StoreError::Invalid(e.to_string()))
}
fn meta(tx: &mut dyn Transaction) -> rx_ports::Result<HostMeta> {
    decode(
        &tx.get(&name("host/meta"))?
            .ok_or(StoreError::Integrity("Host metadata missing".into()))?,
        "rx.host.meta.v1",
    )
}
fn pending(tx: &mut dyn Transaction) -> rx_ports::Result<Option<Preparation>> {
    tx.get(&name(ACTIVE))?
        .map(|v| decode(&v, PREPARATION))
        .transpose()
}
pub(super) fn begin_startup(
    tx: &mut dyn Transaction,
    identity: Digest,
    startup_configuration_digest: Digest,
    attempt: &Id,
) -> rx_ports::Result<()> {
    if pending(tx)?.is_some_and(|v| v.state == State::Prepared) {
        return Err(StoreError::Unavailable(
            "Host binding preparation must be resolved before startup".into(),
        ));
    }
    let old = tx.get(&name(STATE))?;
    let value = Lifecycle::Starting {
        attempt: attempt.clone(),
        installation_identity: identity,
        startup_configuration_digest,
    };
    tx.put(
        &name(STATE),
        old.map(|v| v.revision),
        &doc(LIFECYCLE, &value)?,
    )?;
    tx.put(
        &key("host-service/start", attempt),
        None,
        &doc(LIFECYCLE, &value)?,
    )?;
    Ok(())
}
fn operational_digest(store: &mut SqliteRepository) -> rx_ports::Result<Digest> {
    let (_, mut records) = store.snapshot()?;
    records.retain(|r| {
        !r.key.as_str().starts_with("host-service/")
            && !r.key.as_str().starts_with("host-maintenance/")
    });
    records.sort_by(|a, b| a.key.cmp(&b.key));
    digest(&records)
}
pub(crate) fn seal_stop(
    store: &mut SqliteRepository,
    identity: Digest,
    startup_configuration_digest: Digest,
    attempt: &Id,
    snapshot: StopSnapshot,
) -> rx_ports::Result<StopSeal> {
    if snapshot.admission_open || !snapshot.safe_to_drop {
        return Err(StoreError::Invalid(
            "current closed-admission native drop proof required".into(),
        ));
    }
    let state_digest = operational_digest(store)?;
    let journal_tail = store.journal_head()?;
    store.transact(|tx| {
        let row=tx.get(&name(STATE))?.ok_or(StoreError::Integrity("startup marker absent".into()))?;
        let state: Lifecycle=decode(&row,LIFECYCLE)?;
        if !matches!(state,Lifecycle::Starting {attempt:a,installation_identity:i,startup_configuration_digest:c} if &a==attempt && i==identity && c==startup_configuration_digest) {
            return Err(StoreError::Integrity("Host startup attempt changed".into()));
        }
        let m=meta(tx)?;
        let seal=StopSeal {attempt:attempt.clone(),installation_identity:identity,startup_configuration_digest,delivery_journal:m.delivery_journal,evidence_journal:m.evidence_journal,delivery_sequence:m.delivery_seq,journal_tail,state_digest,snapshot};
        let stopped=Lifecycle::Stopped {seal:Box::new(seal.clone())};
        tx.put(&name(STATE),Some(row.revision),&doc(LIFECYCLE,&stopped)?)?;
        tx.put(&key("host-service/stop",attempt),None,&doc("rx.host.stop-seal.v1",&seal)?)?;
        Ok(seal)
    })
}

/// Acquire the same runtime owner and existing journal as the service; no missing database is initialized.
fn stopped_store(loaded: &Loaded) -> Result<(std::fs::File, SqliteRepository)> {
    super::validate_state_files(&loaded.config.data_directory)?;
    if !loaded.config.data_directory.join("host.db").is_file() {
        return Err("Host journal missing".into());
    }
    let owner = super::runtime_owner(&loaded.config.runtime_directory)?;
    let bytes = rx_package::directory::read_relative_file(
        &loaded.config.data_directory,
        &rx_package::PackagePath::new("installation.json")?,
        1_048_576,
    )?;
    let descriptor: super::Installation = canonical::decode_json(&bytes)?;
    if descriptor.maintenance_protocol.as_ref().map(Name::as_str) != Some("rx.host-maintenance.v1")
        || descriptor.schema.as_str() != "rx.host-installation.v2"
        || descriptor.identity != loaded.identity
        || descriptor.installation != loaded.config.installation
        || descriptor.host != loaded.config.host
    {
        return Err("current Host installation differs".into());
    }
    let mut store = SqliteRepository::open(loaded.config.data_directory.join("host.db"))?;
    store.transact(|tx| {
        let m = meta(tx)?;
        if m.delivery_journal != descriptor.delivery_journal
            || m.evidence_journal != descriptor.evidence_journal
        {
            return Err(StoreError::Integrity(
                "Host journal generation differs".into(),
            ));
        }
        Ok(())
    })?;
    Ok((owner, store))
}
pub fn prepare(
    plan: &rx_process_contract::host_binding_plan::Plan,
    current: &Loaded,
    proposed: &Loaded,
    request: &Id,
) -> Result<Preparation> {
    prepare_inner(plan, current, proposed, request, || Ok(()), || {})
}
#[cfg(feature = "test-harness")]
pub fn prepare_with_boundary(
    plan: &rx_process_contract::host_binding_plan::Plan,
    current: &Loaded,
    proposed: &Loaded,
    request: &Id,
    before_commit: impl FnOnce() -> std::result::Result<(), String>,
    after_commit: impl FnOnce(),
) -> Result<Preparation> {
    prepare_inner(
        plan,
        current,
        proposed,
        request,
        before_commit,
        after_commit,
    )
}
fn prepare_inner(
    plan: &rx_process_contract::host_binding_plan::Plan,
    current: &Loaded,
    proposed: &Loaded,
    request: &Id,
    before_commit: impl FnOnce() -> std::result::Result<(), String>,
    after_commit: impl FnOnce(),
) -> Result<Preparation> {
    let inspection = super::binding_change::inspect(plan, current, proposed)?;
    if !inspection.software_matches {
        return Err("proposed Host configuration does not match the plan".into());
    }
    let request_digest = digest(&(plan, current.identity, proposed.identity, &proposed.config))?;
    let (_owner, mut store) = stopped_store(current)?;
    let state_digest = operational_digest(&mut store)?;
    let tail = store.journal_head()?;
    let prepared = store.transact(|tx| {
        let receipt = key("host-maintenance/request", request);
        if let Some(row) = tx.get(&receipt)? {
            let old: Preparation = decode(&row, PREPARATION)?;
            if old.request_digest != request_digest {
                return Err(StoreError::KeyConflict);
            }
            return Ok(old);
        }
        if pending(tx)?.is_some_and(|v| v.state == State::Prepared) {
            return Err(StoreError::Unavailable(
                "another Host binding preparation is pending".into(),
            ));
        }
        let lifecycle: Lifecycle = decode(
            &tx.get(&name(STATE))?
                .ok_or(StoreError::Invalid("no durable normal-stop proof".into()))?,
            LIFECYCLE,
        )?;
        let Lifecycle::Stopped { seal } = lifecycle else {
            return Err(StoreError::Invalid(
                "Host startup was not followed by a sealed stop".into(),
            ));
        };
        let m = meta(tx)?;
        if seal.startup_configuration_digest != startup_digest(&current.config)?
            || seal.installation_identity != current.identity
            || seal.delivery_journal != m.delivery_journal
            || seal.evidence_journal != m.evidence_journal
            || seal.delivery_sequence != m.delivery_seq
            || seal.journal_tail != tail
            || seal.state_digest != state_digest
            || !seal.snapshot.pending_operations.is_empty()
        {
            return Err(StoreError::Invalid(
                "Host stop proof/state is stale or reconciliation is required".into(),
            ));
        }
        let p = Preparation {
            request: request.clone(),
            request_digest,
            state: State::Prepared,
            plan_digest: plan.digest().map_err(StoreError::Invalid)?,
            current_identity: current.identity,
            proposed_identity: proposed.identity,
            proposed_bindings_digest: proposed.config.bindings.sha256,
            proposed_configuration: proposed.config.clone(),
            stop: *seal,
            installation_changed: false,
            activation_authorized: false,
        };
        let old = tx.get(&name(ACTIVE))?;
        tx.put(
            &name(ACTIVE),
            old.map(|r| r.revision),
            &doc(PREPARATION, &p)?,
        )?;
        tx.put(&receipt, None, &doc(PREPARATION, &p)?)?;
        tx.put(
            &key("host-maintenance/prepared", request),
            None,
            &doc(PREPARATION, &p)?,
        )?;
        before_commit().map_err(StoreError::Unavailable)?;
        Ok(p)
    })?;
    after_commit();
    Ok(prepared)
}
pub fn cancel(current: &Loaded, request: &Id) -> Result<Preparation> {
    let (_owner, mut store) = stopped_store(current)?;
    Ok(store.transact(|tx| {
        let row = tx
            .get(&key("host-maintenance/request", request))?
            .ok_or(StoreError::Invalid("Host preparation absent".into()))?;
        let mut p: Preparation = decode(&row, PREPARATION)?;
        if p.state == State::Cancelled {
            return Ok(p);
        }
        let active = tx
            .get(&name(ACTIVE))?
            .ok_or(StoreError::Integrity("active preparation missing".into()))?;
        let a: Preparation = decode(&active, PREPARATION)?;
        if a.request != *request || a.request_digest != p.request_digest {
            return Err(StoreError::Integrity(
                "Host preparation identity differs".into(),
            ));
        }
        p.state = State::Cancelled;
        tx.put(&row.key, Some(row.revision), &doc(PREPARATION, &p)?)?;
        tx.put(&name(ACTIVE), Some(active.revision), &doc(PREPARATION, &p)?)?;
        tx.put(
            &key("host-maintenance/cancelled", request),
            None,
            &doc(PREPARATION, &p)?,
        )?;
        Ok(p)
    })?)
}

/// Recover durable preparation status even when proposed input files are no longer available.
pub fn lookup(current: &Loaded, request: &Id) -> Result<Option<Preparation>> {
    let (_owner, mut store) = stopped_store(current)?;
    Ok(store.transact(|tx| {
        tx.get(&key("host-maintenance/request", request))?
            .map(|row| decode(&row, PREPARATION))
            .transpose()
    })?)
}
