use crate::{MAX_BYTES, Result, invalid};
#[cfg(unix)]
use std::{
    ffi::OsString,
    fs::OpenOptions,
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
};
use std::{
    fs::{self, File},
    path::{Component, Path, PathBuf},
};

fn checked_path(path: &Path) -> Result<()> {
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(invalid("absolute status file path required"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid("status parent missing"))?;
    let mut current = PathBuf::new();
    for component in parent.components() {
        match component {
            Component::RootDir | Component::Normal(_) => current.push(component.as_os_str()),
            _ => {
                return Err(invalid(
                    "status path must not contain aliases or parent traversal",
                ));
            }
        }
        let metadata = fs::symlink_metadata(&current)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(invalid("status parent must be a real directory"));
        }
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn read(path: &Path) -> Result<Option<Vec<u8>>> {
    checked_path(path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
            return Err(invalid("status path must be a regular file"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES as u64 {
        return Err(invalid("status handle or size differs"));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    (&mut file)
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES
        || bytes.len() as u64 != metadata.len()
        || file.metadata()?.len() != metadata.len()
    {
        return Err(invalid("status size changed or exceeds bound"));
    }
    Ok(Some(bytes))
}
#[cfg(not(unix))]
pub(crate) fn read(_path: &Path) -> Result<Option<Vec<u8>>> {
    Err(crate::Error::Unsupported)
}

pub(crate) struct Owner {
    #[cfg(unix)]
    lock: File,
}
impl Owner {
    #[cfg(unix)]
    pub(crate) fn acquire(path: &Path) -> Result<Self> {
        checked_path(path)?;
        let mut filename = OsString::from(".");
        filename.push(
            path.file_name()
                .ok_or_else(|| invalid("status filename missing"))?,
        );
        filename.push(".rx-status.lock");
        let lock_path = path.with_file_name(filename);
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .custom_flags(
                (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32,
            )
            .mode(0o600)
            .open(lock_path)?;
        if !lock.metadata()?.is_file() {
            return Err(invalid("status ownership lock must be a regular file"));
        }
        lock.try_lock()
            .map_err(|e| invalid(format!("status writer already owned: {e}")))?;
        Ok(Self { lock })
    }
    #[cfg(not(unix))]
    pub(crate) fn acquire(_path: &Path) -> Result<Self> {
        Err(crate::Error::Unsupported)
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        #[cfg(unix)]
        let _ = self.lock.unlock();
    }
}

#[cfg(unix)]
pub(crate) fn replace(path: &Path, bytes: &[u8], expected: Option<&[u8]>) -> Result<()> {
    checked_path(path)?;
    if bytes.len() > MAX_BYTES {
        return Err(invalid("status payload exceeds bound"));
    }
    let mut filename = OsString::from(".");
    filename.push(
        path.file_name()
            .ok_or_else(|| invalid("status filename missing"))?,
    );
    filename.push(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let temporary = Temporary(path.with_file_name(filename));
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32)
        .mode(0o600)
        .open(&temporary.0)?;
    output.write_all(bytes)?;
    output.sync_all()?;
    if read(path)?.as_deref() != expected {
        return Err(invalid("status owner payload changed before publication"));
    }
    fs::rename(&temporary.0, path)?;
    File::open(
        path.parent()
            .ok_or_else(|| invalid("status parent missing"))?,
    )?
    .sync_all()?;
    Ok(())
}
#[cfg(not(unix))]
pub(crate) fn replace(_path: &Path, _bytes: &[u8], _expected: Option<&[u8]>) -> Result<()> {
    Err(crate::Error::Unsupported)
}
#[cfg(unix)]
struct Temporary(PathBuf);
#[cfg(unix)]
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
