//! Recoverable offline binding publication. This never opens an adapter or restores Arm.
use super::*;
use crate::service::{self, AdapterFactory, Builtin, Installation, NativeInstallation};
use std::{fs, io::Write, path::Path};
const COMMIT: &str = "rx.host.binding-commit.v1";
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Phase {
    Reserved,
    NativeReady,
    Committed,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitRecord {
    pub schema: Name,
    pub request: Id,
    pub request_digest: Digest,
    pub plan_digest: Digest,
    pub before_identity: Digest,
    pub after_identity: Digest,
    pub phase: Phase,
    pub native_generation: rx_package::PackagePath,
    pub delivery_journal: Id,
    pub evidence_journal: Id,
    pub activation_authorized: bool,
    generation_digest: Option<Digest>,
    before_descriptor: Installation,
    after_descriptor: Option<Installation>,
}
#[derive(Serialize)]
pub struct CommitView {
    pub record: CommitRecord,
    pub currently_installed: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Generation {
    request: Id,
    proposed_identity: Digest,
    native: Option<NativeInstallation>,
}
fn same(a: &impl Serialize, b: &impl Serialize) -> Result<bool> {
    Ok(canonical::bytes(a)? == canonical::bytes(b)?)
}
fn existing_store(
    loaded: &Loaded,
) -> Result<(
    rx_storage::ExclusiveFileLock,
    SqliteRepository,
    Installation,
)> {
    service::validate_state_files(&loaded.config.data_directory)?;
    if !loaded.config.data_directory.join("host.db").is_file() {
        return Err("Host journal missing".into());
    }
    let owner = service::runtime_owner(&loaded.config.runtime_directory)?;
    let descriptor = service::read_installation(&loaded.config.data_directory)?;
    if descriptor.schema.as_str() != "rx.host-installation.v2"
        || descriptor.maintenance_protocol.as_ref().map(Name::as_str)
            != Some("rx.host-maintenance.v1")
        || descriptor.installation != loaded.config.installation
        || descriptor.host != loaded.config.host
    {
        return Err("Host maintenance identity differs".into());
    }
    service::native_storage_root(&loaded.config.data_directory, &descriptor)?;
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
    Ok((owner, store, descriptor))
}
fn read_commit(tx: &mut dyn Transaction, request: &Id) -> rx_ports::Result<Option<CommitRecord>> {
    tx.get(&key("host-maintenance/commit", request))?
        .map(|r| decode(&r, COMMIT))
        .transpose()
}
pub fn lookup_commit(loaded: &Loaded, request: &Id) -> Result<Option<CommitView>> {
    let (_owner, mut store, descriptor) = existing_store(loaded)?;
    let value = store.transact(|tx| read_commit(tx, request))?;
    value
        .map(|record| {
            if loaded.identity != descriptor.identity
                && !(loaded.identity == record.before_identity
                    && descriptor.identity == record.after_identity)
            {
                return Err(
                    "configuration does not identify the current or recovering installation".into(),
                );
            }
            let currently_installed = record
                .after_descriptor
                .as_ref()
                .map(|after| same(after, &descriptor))
                .transpose()?
                .unwrap_or(false);
            Ok(CommitView {
                record,
                currently_installed,
            })
        })
        .transpose()
}
pub fn commit(
    plan: &rx_process_contract::host_binding_plan::Plan,
    current: &Loaded,
    proposed: &Loaded,
    request: &Id,
) -> Result<CommitRecord> {
    commit_inner(plan, current, proposed, request, |_| Ok(()))
}
#[cfg(feature = "test-harness")]
pub fn commit_with_boundary(
    plan: &rx_process_contract::host_binding_plan::Plan,
    current: &Loaded,
    proposed: &Loaded,
    request: &Id,
    hook: impl FnMut(&str) -> std::result::Result<(), String>,
) -> Result<CommitRecord> {
    commit_inner(plan, current, proposed, request, hook)
}
fn sync_tree(path: &Path) -> Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err("native staging symlink refused".into());
        }
        if kind.is_dir() {
            sync_tree(&entry.path())?;
        } else if kind.is_file() {
            fs::File::open(entry.path())?.sync_all()?;
        } else {
            return Err("native staging special file refused".into());
        }
    }
    fs::File::open(path)?.sync_all()?;
    Ok(())
}
fn commit_inner(
    plan: &rx_process_contract::host_binding_plan::Plan,
    current: &Loaded,
    proposed: &Loaded,
    request: &Id,
    mut hook: impl FnMut(&str) -> std::result::Result<(), String>,
) -> Result<CommitRecord> {
    if current
        .bindings
        .iter()
        .chain(&proposed.bindings)
        .any(|b| b.environment != crate::Environment::Simulation)
    {
        return Err("binding commit currently requires simulation qualification scope".into());
    }
    let inspection = service::binding_change::inspect(plan, current, proposed)?;
    if !inspection.software_matches {
        return Err("proposed binding does not match approved plan".into());
    }
    for old in &current.bindings {
        let next = proposed
            .bindings
            .iter()
            .find(|b| b.cell == old.cell)
            .ok_or("cell cohort changed")?;
        if old
            .scope_ids
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            != next.scope_ids.iter().collect()
        {
            return Err("scope topology change needs a separate migration".into());
        }
    }
    let fingerprint = digest(&(plan, current.identity, proposed.identity, &proposed.config))?;
    let (_owner, mut store, descriptor) = existing_store(current)?;
    let mut record = if let Some(old) = store.transact(|tx| read_commit(tx, request))? {
        if old.request_digest != fingerprint {
            return Err("original binding commit request differs".into());
        }
        if old.phase == Phase::Committed {
            if !old
                .after_descriptor
                .as_ref()
                .is_some_and(|after| same(after, &descriptor).unwrap_or(false))
            {
                return Err(
                    "committed binding is no longer installed; no rollback performed".into(),
                );
            }
            return Ok(old);
        }
        old
    } else {
        if descriptor.identity != current.identity {
            return Err("current Host descriptor differs".into());
        }
        let state_digest = operational_digest(&mut store)?;
        let tail = store.journal_head()?;
        store.transact(|tx| {
            let row = tx
                .get(&key("host-maintenance/request", request))?
                .ok_or(StoreError::Invalid("binding preparation missing".into()))?;
            let mut prepared: Preparation = decode(&row, PREPARATION)?;
            let active =
                pending(tx)?.ok_or(StoreError::Invalid("active preparation missing".into()))?;
            if prepared.state != State::Prepared
                || active.request != *request
                || prepared.request_digest != fingerprint
                || prepared.stop.state_digest != state_digest
                || prepared.stop.journal_tail != tail
            {
                return Err(StoreError::Invalid(
                    "binding preparation or normal-stop state changed".into(),
                ));
            }
            let value = CommitRecord {
                schema: name(COMMIT),
                request: request.clone(),
                request_digest: fingerprint,
                plan_digest: prepared.plan_digest,
                before_identity: current.identity,
                after_identity: proposed.identity,
                phase: Phase::Reserved,
                native_generation: rx_package::PackagePath::new(format!(
                    "native-generations/{request}"
                ))
                .map_err(StoreError::Invalid)?,
                delivery_journal: descriptor.delivery_journal.clone(),
                evidence_journal: descriptor.evidence_journal.clone(),
                activation_authorized: false,
                generation_digest: None,
                before_descriptor: descriptor.clone(),
                after_descriptor: None,
            };
            prepared.state = State::Committing;
            tx.put(&row.key, Some(row.revision), &doc(PREPARATION, &prepared)?)?;
            let active_row = tx.get(&name(ACTIVE))?.ok_or(StoreError::Integrity(
                "active preparation disappeared".into(),
            ))?;
            tx.put(
                &name(ACTIVE),
                Some(active_row.revision),
                &doc(PREPARATION, &prepared)?,
            )?;
            tx.put(
                &key("host-maintenance/commit", request),
                None,
                &doc(COMMIT, &value)?,
            )?;
            Ok(value)
        })?
    };
    let state_digest = operational_digest(&mut store)?;
    let tail = store.journal_head()?;
    store.transact(|tx| {
        let row = tx
            .get(&key("host-maintenance/request", request))?
            .ok_or(StoreError::Integrity("original preparation missing".into()))?;
        let p: Preparation = decode(&row, PREPARATION)?;
        let active = pending(tx)?.ok_or(StoreError::Integrity("active commit missing".into()))?;
        let lifecycle: Lifecycle = decode(
            &tx.get(&name(STATE))?
                .ok_or(StoreError::Integrity("stop state missing".into()))?,
            LIFECYCLE,
        )?;
        if p.state != State::Committing
            || active.state != State::Committing
            || active.request != *request
            || p.request_digest != fingerprint
            || p.stop.state_digest != state_digest
            || p.stop.journal_tail != tail
            || !matches!(lifecycle,Lifecycle::Stopped{seal} if seal.attempt==p.stop.attempt)
        {
            return Err(StoreError::Invalid(
                "original stopped state changed during binding commit".into(),
            ));
        }
        Ok(())
    })?;
    hook("INTENT_RECORDED")?;
    let data = &current.config.data_directory;
    if record.phase == Phase::Reserved {
        let parent = data.join("native-generations");
        if !parent.exists() {
            fs::create_dir(&parent)?;
        }
        service::real_directory(&parent)?;
        let destination = data.join(record.native_generation.as_str());
        if !destination.exists() {
            let stage = tempfile::Builder::new()
                .prefix(".binding-stage-")
                .tempdir_in(&parent)?;
            let native=<Builtin as AdapterFactory<crate::service_clock::SystemClock>>::initialize_metadata(&Builtin,&proposed.config.backend,stage.path())?;
            let generation = Generation {
                request: request.clone(),
                proposed_identity: proposed.identity,
                native,
            };
            service::private_write(
                &stage.path().join("generation.json"),
                &canonical::bytes(&generation)?,
            )?;
            sync_tree(stage.path())?;
            service::publish_installation(&parent, stage.path(), &destination)?;
            fs::File::open(&parent)?.sync_all()?;
        }
        service::real_directory(&destination)?;
        let generation_bytes = rx_package::directory::read_relative_file(
            &destination,
            &rx_package::PackagePath::new("generation.json")?,
            1_048_576,
        )?;
        let generation: Generation = canonical::decode_json(&generation_bytes)?;
        record.generation_digest = Some(rx_package::content_digest(&generation_bytes));
        if generation.request != *request || generation.proposed_identity != proposed.identity {
            return Err("native generation belongs to another transition".into());
        }
        let mut after = record.before_descriptor.clone();
        after.identity = proposed.identity;
        after.native = generation.native;
        after.native_directory = Some(record.native_generation.clone());
        record.after_descriptor = Some(after);
        record.phase = Phase::NativeReady;
        store.transact(|tx| {
            let row = tx
                .get(&key("host-maintenance/commit", request))?
                .ok_or(StoreError::Integrity("commit record missing".into()))?;
            tx.put(&row.key, Some(row.revision), &doc(COMMIT, &record)?)?;
            Ok(())
        })?;
    }
    let generation_bytes = rx_package::directory::read_relative_file(
        &data.join(record.native_generation.as_str()),
        &rx_package::PackagePath::new("generation.json")?,
        1_048_576,
    )?;
    if record.generation_digest != Some(rx_package::content_digest(&generation_bytes)) {
        return Err("staged native generation changed".into());
    }
    hook("NATIVE_READY")?;
    let installed = service::read_installation(data)?;
    let after = record
        .after_descriptor
        .as_ref()
        .ok_or("proposed descriptor missing")?;
    if !same(&installed, after)? {
        if !same(&installed, &record.before_descriptor)? {
            return Err("installation changed outside the original binding commit".into());
        }
        let mut temporary = tempfile::NamedTempFile::new_in(data)?;
        temporary.write_all(&canonical::bytes(after)?)?;
        temporary.as_file().sync_all()?;
        temporary.persist(data.join("installation.json"))?;
        fs::File::open(data)?.sync_all()?;
    }
    hook("DESCRIPTOR_PUBLISHED")?;
    record.phase = Phase::Committed;
    store.transact(|tx| {
        let row = tx
            .get(&key("host-maintenance/request", request))?
            .ok_or(StoreError::Integrity("preparation missing".into()))?;
        let mut prepared: Preparation = decode(&row, PREPARATION)?;
        if prepared.state != State::Committing || prepared.request_digest != record.request_digest {
            return Err(StoreError::Integrity("commit preparation changed".into()));
        }
        prepared.state = State::Committed;
        prepared.installation_changed = true;
        tx.put(&row.key, Some(row.revision), &doc(PREPARATION, &prepared)?)?;
        let active = tx
            .get(&name(ACTIVE))?
            .ok_or(StoreError::Integrity("active preparation missing".into()))?;
        tx.put(
            &active.key,
            Some(active.revision),
            &doc(PREPARATION, &prepared)?,
        )?;
        let committed = tx
            .get(&key("host-maintenance/commit", request))?
            .ok_or(StoreError::Integrity("commit record missing".into()))?;
        tx.put(
            &committed.key,
            Some(committed.revision),
            &doc(COMMIT, &record)?,
        )?;
        let k = key("host-binding-required", &plan.cell);
        let old = tx.get(&k)?;
        tx.put(
            &k,
            old.map(|r| r.revision),
            &doc(
                REQUIRED,
                &BindingRequirement {
                    configuration: plan.after_configuration.sha256,
                    pending: true,
                    acknowledged_by: None,
                },
            )?,
        )?;
        Ok(())
    })?;
    Ok(record)
}
