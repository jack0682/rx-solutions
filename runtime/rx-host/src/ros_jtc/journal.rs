use super::{Profile, protocol::Reply};
use crate::{
    HostError, NativeCapture, Result,
    journal::{decode, doc, id, key, name},
};
use rx_domain::{intent::Intent, types::*};
use rx_ports::{Repository, StoreError};
use rx_storage::SqliteRepository;
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub journal: Id,
    pub profile: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Meta {
    identity: Identity,
    pending: Option<Id>,
    count: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub operation: Id,
    pub invocation: Id,
    pub intent: Intent,
    pub intent_digest: Digest,
    pub profile_digest: Digest,
    pub controller_session: Id,
    pub bridge_instance: Id,
    pub entered_at: TimePoint,
    pub goal_digest: Digest,
    pub send: Option<Reply>,
    pub result: Option<Reply>,
    pub capture: Option<NativeCapture>,
    pub issue: Option<String>,
    pub disputed: bool,
}
pub(super) fn initialize(directory: &Path, profile: &Profile) -> Result<Identity> {
    let identity = Identity {
        journal: id(),
        profile: profile.digest()?,
    };
    std::fs::create_dir(directory).map_err(|e| HostError::Invalid(e.to_string()))?;
    let mut store = SqliteRepository::open(directory.join("native.sqlite3"))?;
    store.transact(|tx| {
        tx.put(
            &name("jtc/meta"),
            None,
            &doc(
                "rx.jtc.meta.v1",
                &Meta {
                    identity: identity.clone(),
                    pending: None,
                    count: Counter(0),
                },
            )?,
        )?;
        Ok(())
    })?;
    Ok(identity)
}
pub(super) fn open(
    directory: &Path,
    identity: &Identity,
    profile: &Profile,
) -> Result<SqliteRepository> {
    let path = directory.join("native.sqlite3");
    let m = std::fs::symlink_metadata(&path).map_err(|e| HostError::Invalid(e.to_string()))?;
    if !m.is_file() || m.file_type().is_symlink() || m.len() == 0 {
        return Err(HostError::Invalid(
            "existing native journal required".into(),
        ));
    }
    let mut store = SqliteRepository::open(path)?;
    store.check_integrity()?;
    let profile_hash = profile.digest()?;
    store.transact(|tx| {
        let row = tx
            .get(&name("jtc/meta"))?
            .ok_or(StoreError::Integrity("JTC metadata absent".into()))?;
        let meta: Meta = decode(&row, "rx.jtc.meta.v1")?;
        if meta.identity.journal != identity.journal
            || meta.identity.profile != identity.profile
            || identity.profile != profile_hash
        {
            return Err(StoreError::Integrity("JTC journal/profile identity".into()));
        }
        let rows = tx.scan("jtc-operation/")?;
        if rows.len() as u64 != meta.count.0 {
            return Err(StoreError::Integrity("JTC journal count differs".into()));
        }
        let mut pending = vec![];
        for row in rows {
            let e: Entry = decode(&row, "rx.jtc.entry.v1")?;
            super::adapter::validate_record(&e)
                .map_err(|e| StoreError::Integrity(e.to_string()))?;
            let trajectory = profile
                .trajectory(&e.intent)
                .map_err(|e| StoreError::Integrity(e.to_string()))?;
            if row.key != key("jtc-operation", &e.operation)
                || e.intent
                    .digest()
                    .map_err(|e| StoreError::Integrity(e.to_string()))?
                    != e.intent_digest
                || e.profile_digest != profile_hash
                || e.goal_digest != trajectory.reference.sha256
                || e.capture.as_ref().is_some_and(|c| {
                    c.device_session != e.controller_session
                        || c.native_id.as_deref() != Some(e.invocation.as_str())
                })
            {
                return Err(StoreError::Integrity(
                    "JTC stored operation identity".into(),
                ));
            }
            let index = tx
                .get(&key("jtc-invocation", &e.invocation))?
                .ok_or(StoreError::Integrity("JTC invocation index absent".into()))?;
            if decode::<Id>(&index, "rx.jtc.invocation.v1")? != e.operation {
                return Err(StoreError::Integrity("JTC invocation index differs".into()));
            }
            if e.capture.is_none() {
                pending.push(e.operation);
            }
        }
        if pending != meta.pending.into_iter().collect::<Vec<_>>() {
            return Err(StoreError::Integrity("JTC pending index differs".into()));
        }
        Ok(())
    })?;
    Ok(store)
}
pub(super) fn read(store: &mut SqliteRepository, operation: &Id) -> Result<Option<Entry>> {
    store
        .transact(|tx| {
            tx.get(&key("jtc-operation", operation))?
                .map(|r| decode(&r, "rx.jtc.entry.v1"))
                .transpose()
        })
        .map_err(Into::into)
}
pub(super) fn pending(store: &mut SqliteRepository) -> Result<Option<Id>> {
    store
        .transact(|tx| {
            let row = tx
                .get(&name("jtc/meta"))?
                .ok_or(StoreError::Integrity("JTC metadata absent".into()))?;
            Ok(decode::<Meta>(&row, "rx.jtc.meta.v1")?.pending)
        })
        .map_err(Into::into)
}
pub(super) fn enter(store: &mut SqliteRepository, e: &Entry) -> Result<()> {
    store
        .transact(|tx| {
            let row = tx
                .get(&name("jtc/meta"))?
                .ok_or(StoreError::Integrity("JTC metadata absent".into()))?;
            let mut m: Meta = decode(&row, "rx.jtc.meta.v1")?;
            if m.pending.is_some() || m.count.0 >= 10_000 {
                return Err(StoreError::Invalid("JTC pending work/capacity".into()));
            }
            tx.put(
                &key("jtc-invocation", &e.invocation),
                None,
                &doc("rx.jtc.invocation.v1", &e.operation)?,
            )?;
            tx.put(
                &key("jtc-operation", &e.operation),
                None,
                &doc("rx.jtc.entry.v1", e)?,
            )?;
            tx.append(&id(), &doc("rx.jtc.entry.v1", e)?)?;
            m.pending = Some(e.operation.clone());
            m.count = Counter(m.count.0 + 1);
            tx.put(
                &name("jtc/meta"),
                Some(row.revision),
                &doc("rx.jtc.meta.v1", &m)?,
            )?;
            Ok(())
        })
        .map_err(Into::into)
}
pub(super) fn update(store: &mut SqliteRepository, e: &Entry) -> Result<()> {
    store
        .transact(|tx| {
            let k = key("jtc-operation", &e.operation);
            let row = tx
                .get(&k)?
                .ok_or(StoreError::Integrity("JTC entry absent".into()))?;
            let old: Entry = decode(&row, "rx.jtc.entry.v1")?;
            if old.capture.is_some()
                || old.invocation != e.invocation
                || old.intent_digest != e.intent_digest
            {
                return Err(StoreError::KeyConflict);
            }
            tx.put(&k, Some(row.revision), &doc("rx.jtc.entry.v1", e)?)?;
            tx.append(&id(), &doc("rx.jtc.entry.v1", e)?)?;
            if e.capture.is_some() {
                let row = tx
                    .get(&name("jtc/meta"))?
                    .ok_or(StoreError::Integrity("JTC meta absent".into()))?;
                let mut m: Meta = decode(&row, "rx.jtc.meta.v1")?;
                if m.pending.as_ref() != Some(&e.operation) {
                    return Err(StoreError::KeyConflict);
                }
                m.pending = None;
                tx.put(
                    &name("jtc/meta"),
                    Some(row.revision),
                    &doc("rx.jtc.meta.v1", &m)?,
                )?;
            }
            Ok(())
        })
        .map_err(Into::into)
}
