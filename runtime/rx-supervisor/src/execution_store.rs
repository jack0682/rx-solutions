//! One immutable execution journal per explicitly resumed run. Registration stays
//! in the resident root; ordinary run selection never invents a new run journal.
use crate::{Error, Result, model::State};
use rx_domain::{canonical, types::Id};
use rx_ports::Repository;
use rx_storage::SqliteRepository;
use std::{
    fs,
    path::{Path, PathBuf},
};
fn regular(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_file() && !m.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(Error::Invalid("execution-store-file-type".into())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}
fn directory(path: &Path, create: bool) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(Error::Invalid("execution-store-directory-type".into())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && create => Ok(fs::create_dir(path)?),
        Err(e) => Err(e.into()),
    }
}
fn store_plan(path: &Path) -> Result<Option<Id>> {
    if !regular(path)? {
        return Ok(None);
    }
    regular(&path.with_extension("writer.lock"))?;
    let mut store = SqliteRepository::open(path)?;
    let (_, rows) = store.snapshot()?;
    let row = rows
        .iter()
        .find(|r| r.key.as_str() == "supervisor/state")
        .ok_or_else(|| Error::Reconciliation("existing-execution-store-has-no-plan".into()))?;
    if row.document.schema.as_str() != "rx.supervisor-state.v1" {
        return Err(Error::Invalid("execution-store-schema".into()));
    }
    let state: State = canonical::decode_json(
        &canonical::bytes(&row.document.value).map_err(|e| Error::Invalid(e.to_string()))?,
    )
    .map_err(|e| Error::Invalid(e.to_string()))?;
    Ok(Some(state.plan))
}
fn run_path(root: &Path, run: &Id, create: bool) -> Result<PathBuf> {
    // Id is UUID-validated, nevertheless reject any future path-like extension.
    if Path::new(run.as_str()).components().count() != 1 {
        return Err(Error::Invalid("execution-run-id-path".into()));
    }
    let runs = root.join("runs");
    directory(&runs, create)?;
    let path = runs.join(run.as_str());
    directory(&path, create)?;
    Ok(path)
}
pub fn resolve(root: &Path, run: &Id, allow_initial: bool) -> Result<PathBuf> {
    directory(root, false)?;
    let original = root.join("supervisor.db");
    let original_plan = store_plan(&original)?;
    if original_plan.as_ref() == Some(run) {
        return Ok(root.to_path_buf());
    }
    if original_plan.is_none() && allow_initial && !root.join("runs").exists() {
        return Ok(root.to_path_buf());
    }
    let path = run_path(root, run, false)
        .map_err(|_| Error::Reconciliation("unknown-run-id-no-store-created".into()))?;
    if store_plan(&path.join("supervisor.db"))?.as_ref() != Some(run) {
        return Err(Error::Reconciliation(
            "unknown-run-id-no-store-created".into(),
        ));
    }
    Ok(path)
}
/// Invoke only after explicit resume authorization; never called by ordinary run.
pub fn create_resume(root: &Path, run: &Id) -> Result<PathBuf> {
    directory(root, false)?;
    if store_plan(&root.join("supervisor.db"))?.is_none() {
        return Err(Error::Reconciliation(
            "original-execution-store-required".into(),
        ));
    }
    let path = run_path(root, run, true)?;
    if regular(&path.join("supervisor.db"))? {
        return Err(Error::Reconciliation(
            "resume-requires-fresh-execution-store".into(),
        ));
    }
    regular(&path.join("supervisor.writer.lock"))?;
    Ok(path)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_run_does_not_create_nested_execution_store() {
        let root = tempfile::tempdir().unwrap();
        let run = Id::new(uuid::Uuid::new_v4().to_string()).unwrap();
        let error = resolve(root.path(), &run, false).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unknown-run-id-no-store-created")
        );
        assert!(!root.path().join("runs").exists());
        assert!(!root.path().join("supervisor.db").exists());
    }
}
