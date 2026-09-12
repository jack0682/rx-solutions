use super::*;
use crate::journal::{decode, doc, id, key, name};
use rx_ports::{Repository, StoreError};
use rx_storage::SqliteRepository;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub journal: Id,
    pub profile: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Meta {
    pub identity: Identity,
    pub pending: Option<Id>,
    pub entries: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub operation: Id,
    pub invocation: Id,
    pub intent: Intent,
    pub intent_digest: Digest,
    pub profile_digest: Digest,
    pub device_session: Id,
    pub plc_epoch: u64,
    pub command_m: u32,
    pub target: bool,
    pub request_frame: Vec<u8>,
    pub request_sha256: Digest,
    pub snapshot: Snapshot,
    pub entered_at: TimePoint,
    /// Atomic capture for already-satisfied state, or after an acknowledged send and observation.
    pub capture: Option<NativeCapture>,
    pub completion_snapshot: Option<Snapshot>,
    pub acknowledgement: Option<TimePoint>,
    pub write_error: Option<String>,
}
pub(super) fn initialize(directory: &Path, profile: &Profile) -> Result<Identity> {
    let identity = Identity {
        journal: id(),
        profile: profile.digest()?,
    };
    fs::create_dir(directory).map_err(|e| HostError::Invalid(e.to_string()))?;
    let mut store = SqliteRepository::open(directory.join("native.sqlite3"))?;
    store.transact(|tx| {
        tx.put(
            &name("melsec/meta"),
            None,
            &doc(
                "rx.melsec.meta.v1",
                &Meta {
                    identity: identity.clone(),
                    pending: None,
                    entries: Counter(0),
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
    let metadata = fs::symlink_metadata(&path).map_err(|e| HostError::Invalid(e.to_string()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() == 0 {
        return Err(HostError::Invalid("native journal missing/replaced".into()));
    }
    let mut store = SqliteRepository::open(path)?;
    store.check_integrity()?;
    let profile_digest = profile.digest()?;
    store.transact(|tx| {
        let row = tx
            .get(&name("melsec/meta"))?
            .ok_or(StoreError::Integrity("native meta absent".into()))?;
        let meta: Meta = decode(&row, "rx.melsec.meta.v1")?;
        if meta.identity.journal != identity.journal
            || meta.identity.profile != identity.profile
            || meta.identity.profile != profile_digest
        {
            return Err(StoreError::Integrity(
                "native installation/journal/profile mismatch".into(),
            ));
        }
        let rows = tx.scan("melsec-operation/")?;
        let mut pending = Vec::new();
        if rows.len() as u64 != meta.entries.0 {
            return Err(StoreError::Integrity("native entry count mismatch".into()));
        }
        for row in rows {
            let entry: Entry = decode(&row, "rx.melsec.entry.v1")?;
            validate_entry(&entry, profile)?;
            if row.key != key("melsec-operation", &entry.operation)
                || entry
                    .intent
                    .digest()
                    .map_err(|e| StoreError::Integrity(e.to_string()))?
                    != entry.intent_digest
                || entry.profile_digest != profile_digest
                || entry.intent.profile_digest != profile_digest
            {
                return Err(StoreError::Integrity(
                    "native entry identity mismatch".into(),
                ));
            }
            let index = tx
                .get(&key("melsec-invocation", &entry.invocation))?
                .ok_or(StoreError::Integrity(
                    "native invocation index absent".into(),
                ))?;
            if decode::<Id>(&index, "rx.melsec.invocation.v1")? != entry.operation {
                return Err(StoreError::Integrity(
                    "native invocation index mismatch".into(),
                ));
            }
            if entry.capture.is_none() {
                pending.push(entry.operation);
            }
        }
        if pending != meta.pending.clone().into_iter().collect::<Vec<_>>() {
            return Err(StoreError::Integrity(
                "native pending index mismatch".into(),
            ));
        }
        Ok(())
    })?;
    Ok(store)
}
fn validate_entry(entry: &Entry, profile: &Profile) -> rx_ports::Result<()> {
    let (mapping, target) = profile
        .mapping(&entry.intent)
        .map_err(|e| StoreError::Integrity(e.to_string()))?;
    let initial = &entry.snapshot;
    let valid_initial = initial.plc_epoch == entry.plc_epoch
        && initial.plc_epoch != 0
        && initial.sequence != 0
        && initial.bit(profile.status.valid_bit)
        && initial.acquired_at.clock_id == entry.entered_at.clock_id
        && initial.acquired_at.ticks_ns <= entry.entered_at.ticks_ns;
    let expected_frame = profile
        .transport
        .write_m_frame(mapping.command_m, target)
        .map_err(|e| StoreError::Integrity(e.to_string()))?;
    if entry.request_frame != expected_frame
        || entry.request_sha256 != rx_package::content_digest(&expected_frame)
        || entry.command_m != mapping.command_m
        || target != entry.target
        || !valid_initial
        || entry.acknowledgement.as_ref().is_some_and(|a| {
            a.clock_id != entry.entered_at.clock_id || a.ticks_ns < entry.entered_at.ticks_ns
        })
        || (entry.write_error.is_some()
            && (entry.acknowledgement.is_some() || entry.capture.is_some()))
    {
        return Err(StoreError::Integrity(
            "native request/snapshot metadata mismatch".into(),
        ));
    }
    match (&entry.capture, &entry.completion_snapshot) {
        (None, None) => (),
        (Some(c), Some(s)) => {
            let after_ack = if let Some(ack) = &entry.acknowledgement {
                s.sequence > initial.sequence
                    && s.acquired_at.clock_id == ack.clock_id
                    && s.acquired_at.ticks_ns >= ack.ticks_ns
            } else {
                s.sequence == initial.sequence && s.flags == initial.flags
            };
            if !after_ack
                || s.plc_epoch != entry.plc_epoch
                || !s.bit(profile.status.valid_bit)
                || !s.bit(profile.status.queue_empty_bit)
                || s.bit(mapping.completion_bit) != target
                || c.device_session != entry.device_session
                || c.status_schema.as_str() != "rx.melsec.predicate-satisfied.v1"
                || c.status != 0
                || c.native_id != Some(format!("{}:{}", s.plc_epoch, s.sequence))
                || c.captured_at != s.acquired_at
            {
                return Err(StoreError::Integrity(
                    "native completion evidence mismatch".into(),
                ));
            }
        }
        _ => {
            return Err(StoreError::Integrity(
                "native completion snapshot absent".into(),
            ));
        }
    }
    Ok(())
}
pub(super) fn read(store: &mut SqliteRepository, operation: &Id) -> Result<Option<Entry>> {
    store
        .transact(|tx| {
            tx.get(&key("melsec-operation", operation))?
                .map(|r| decode(&r, "rx.melsec.entry.v1"))
                .transpose()
        })
        .map_err(Into::into)
}
pub(super) fn pending(store: &mut SqliteRepository) -> Result<Option<Id>> {
    store
        .transact(|tx| {
            let row = tx
                .get(&name("melsec/meta"))?
                .ok_or(StoreError::Integrity("native meta absent".into()))?;
            Ok(decode::<Meta>(&row, "rx.melsec.meta.v1")?.pending)
        })
        .map_err(Into::into)
}
pub(super) fn enter(store: &mut SqliteRepository, entry: &Entry) -> Result<()> {
    store
        .transact(|tx| {
            let row = tx
                .get(&name("melsec/meta"))?
                .ok_or(StoreError::Integrity("native meta absent".into()))?;
            let mut meta: Meta = decode(&row, "rx.melsec.meta.v1")?;
            if meta.pending.is_some() || meta.entries.0 >= 10_000 {
                return Err(StoreError::Invalid(
                    "native pending work or journal capacity".into(),
                ));
            }
            tx.put(
                &key("melsec-invocation", &entry.invocation),
                None,
                &doc("rx.melsec.invocation.v1", &entry.operation)?,
            )?;
            tx.put(
                &key("melsec-operation", &entry.operation),
                None,
                &doc("rx.melsec.entry.v1", entry)?,
            )?;
            tx.append(&id(), &doc("rx.melsec.entry.v1", entry)?)?;
            meta.pending = entry.capture.is_none().then(|| entry.operation.clone());
            meta.entries = Counter(meta.entries.0 + 1);
            tx.put(
                &name("melsec/meta"),
                Some(row.revision),
                &doc("rx.melsec.meta.v1", &meta)?,
            )?;
            Ok(())
        })
        .map_err(Into::into)
}
pub(super) fn update(store: &mut SqliteRepository, entry: &Entry) -> Result<()> {
    store
        .transact(|tx| {
            let key = key("melsec-operation", &entry.operation);
            let row = tx
                .get(&key)?
                .ok_or(StoreError::Integrity("native entry absent".into()))?;
            let old: Entry = decode(&row, "rx.melsec.entry.v1")?;
            if old.capture.is_some()
                || old.invocation != entry.invocation
                || old.intent_digest != entry.intent_digest
            {
                return Err(StoreError::KeyConflict);
            }
            tx.put(&key, Some(row.revision), &doc("rx.melsec.entry.v1", entry)?)?;
            tx.append(&id(), &doc("rx.melsec.entry.v1", entry)?)?;
            if entry.capture.is_some() {
                let m = tx
                    .get(&name("melsec/meta"))?
                    .ok_or(StoreError::Integrity("native meta absent".into()))?;
                let mut meta: Meta = decode(&m, "rx.melsec.meta.v1")?;
                if meta.pending.as_ref() != Some(&entry.operation) {
                    return Err(StoreError::KeyConflict);
                }
                meta.pending = None;
                tx.put(
                    &name("melsec/meta"),
                    Some(m.revision),
                    &doc("rx.melsec.meta.v1", &meta)?,
                )?;
            }
            Ok(())
        })
        .map_err(Into::into)
}
