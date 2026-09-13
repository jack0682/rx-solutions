//! Local ownership of an executor service root, acquired before registering a P peer.
#[cfg(unix)]
use std::fs::{self, OpenOptions};
use std::{fs::File, io, path::Path};

#[cfg(unix)]
const LOCK_FILE: &str = ".rx-executor-service.lock";

/// Hold for the entire service lifetime, including connection and shutdown failures.
/// This coordinates processes using one journal directory, not all executors for a cell.
#[derive(Debug)]
pub struct ServiceOwner {
    lock: File,
}

impl ServiceOwner {
    #[cfg(unix)]
    pub fn acquire(journal: &Path) -> io::Result<Self> {
        Self::acquire_inner(journal, true)
    }

    /// Offline inspection requires the existing owner file and never creates a root or lock.
    #[cfg(unix)]
    pub fn acquire_existing(journal: &Path) -> io::Result<Self> {
        Self::acquire_inner(journal, false)
    }

    #[cfg(unix)]
    fn acquire_inner(journal: &Path, create: bool) -> io::Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        let filename = journal.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "executor journal file required",
            )
        })?;
        if filename == LOCK_FILE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "executor journal cannot be the service ownership file",
            ));
        }
        let parent = journal
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        if create {
            fs::create_dir_all(parent)?;
        }
        // Resolve directory aliases before opening the one stable ownership file.
        let root = fs::canonicalize(parent)?;
        let path = root.join(LOCK_FILE);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if !metadata.file_type().is_file() => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "executor service ownership path must be a regular file",
                ));
            }
            Ok(_) => {}
            Err(error) if create && error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let lock = OpenOptions::new()
            .create(create)
            .truncate(false)
            .read(true)
            .write(true)
            // NOFOLLOW also rejects a symlink installed after the metadata check.
            // NONBLOCK prevents a replaced special file from hanging startup.
            .custom_flags(
                (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32,
            )
            .mode(0o600)
            .open(path)?;
        if !lock.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "executor service ownership handle must be a regular file",
            ));
        }
        lock.try_lock().map_err(|error| {
            io::Error::other(format!(
                "EXECUTOR_SERVICE_ROOT_UNAVAILABLE: {}: {error}",
                root.display()
            ))
        })?;
        // Never unlink the lock file: another process may already have opened its inode.
        Ok(Self { lock })
    }

    #[cfg(not(unix))]
    pub fn acquire(_journal: &Path) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "executor service ownership requires Unix file semantics",
        ))
    }

    #[cfg(not(unix))]
    pub fn acquire_existing(journal: &Path) -> io::Result<Self> {
        Self::acquire(journal)
    }
}

impl Drop for ServiceOwner {
    fn drop(&mut self) {
        // A concurrent spawn may retain a fork-inherited descriptor until exec despite CLOEXEC.
        // Normal owner release must unlock the shared open-file description before closing it.
        let _ = self.lock.unlock();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn explicit_release_does_not_wait_for_an_inherited_open_file_description() {
        let root = tempfile::tempdir().unwrap();
        let journal = root.path().join("run.sqlite3");
        let owner = ServiceOwner::acquire(&journal).unwrap();
        // A duplicate shares the open-file description, just like a descriptor inherited by fork.
        let inherited = owner.lock.try_clone().unwrap();
        assert!(ServiceOwner::acquire(&journal).is_err());
        drop(owner);

        let next = ServiceOwner::acquire(&journal).unwrap();
        drop(inherited);
        assert!(ServiceOwner::acquire(&journal).is_err());
        drop(next);
        let _released = ServiceOwner::acquire(&journal).unwrap();
    }
}
