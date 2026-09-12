//! Filesystem convenience boundaries; callers retain ServiceOwner for the deployment root.
use super::*;
use rx_storage::SqliteRepository;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// A missing run file may be initialized only when creation has never been entered.
pub enum RecoveredFile {
    NeedsInitialization(VersionedAttachment),
    Existing {
        attachment: VersionedAttachment,
        run: Box<RunStore<SqliteRepository>>,
    },
}

impl AssignmentJournal<SqliteRepository> {
    pub fn initialize_file(path: &Path, identity: Identity) -> Result<Self> {
        reserve_new(path)?;
        Self::initialize(SqliteRepository::open(path)?, identity)
    }
    pub fn open_file_required(path: &Path, identity: Identity) -> Result<Self> {
        let repository = existing(path)?;
        Self::open_required(repository, identity)
    }
}
impl<R: Repository> AssignmentJournal<R> {
    pub fn recover_current_file(&mut self, root: &Path) -> Result<Option<RecoveredFile>> {
        let Some(attachment) = self.current()? else {
            return Ok(None);
        };
        let id = &attachment.value.preparation.id;
        let path = self.run_file(root, id)?;
        let entered = self.creation_entered(id)?;
        match fs::symlink_metadata(&path) {
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && attachment.value.phase == Phase::Preparing
                    && !entered =>
            {
                Ok(Some(RecoveredFile::NeedsInitialization(attachment)))
            }
            Err(error) => Err(StoreError::Integrity(error.to_string())),
            Ok(_) => {
                let run = self.open_run_file_required(root, id)?;
                Ok(Some(RecoveredFile::Existing {
                    attachment,
                    run: Box::new(run),
                }))
            }
        }
    }
    /// Local paths are derived from the reserved UUID, not supplied by a P response.
    pub fn run_file(&mut self, root: &Path, id: &Id) -> Result<PathBuf> {
        let (binding, _) = self.run_binding(id)?;
        Ok(root.join(format!("run-{}.sqlite3", binding.preparation.scope.run)))
    }
    pub fn initialize_run_file(
        &mut self,
        root: &Path,
        id: &Id,
    ) -> Result<RunStore<SqliteRepository>> {
        // A lost marker commit response must fail before any attempt to reserve the run file.
        let binding = self.enter_run_creation(id)?;
        let path = root.join(format!("run-{}.sqlite3", binding.preparation.scope.run));
        reserve_new(&path)?;
        run_store::initialize_entered(binding, SqliteRepository::open(path)?)
    }
    pub fn open_run_file_required(
        &mut self,
        root: &Path,
        id: &Id,
    ) -> Result<RunStore<SqliteRepository>> {
        let path = self.run_file(root, id)?;
        self.open_run_required(id, existing(&path)?)
    }
}
fn reserve_new(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| StoreError::Invalid("journal parent required".into()))?;
    if !parent.is_dir() {
        return invalid("journal parent must already exist");
    }
    // Leave partial/unknown initialization in place; never overwrite or silently reinitialize it.
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| StoreError::Unavailable(e.to_string()))?;
    Ok(())
}
fn existing(path: &Path) -> Result<SqliteRepository> {
    let metadata = fs::symlink_metadata(path).map_err(|e| StoreError::Integrity(e.to_string()))?;
    if !metadata.file_type().is_file() || metadata.len() == 0 {
        return integrity("existing nonempty regular journal file required");
    }
    let repository = SqliteRepository::open(path)?;
    repository.check_integrity()?;
    Ok(repository)
}
