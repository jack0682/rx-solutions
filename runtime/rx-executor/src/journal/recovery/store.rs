//! Bounded copies of exclusively held source files. SQLite never opens the originals.
use rx_domain::{canonical, types::*};
use rx_ports::*;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

pub(crate) const MAX_RECORDS: usize = 8192;
const MAX_EVENTS: usize = 16384;
const MAX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_DOCUMENT: u64 = 1_048_576;
const MAX_DB: u64 = 64 * 1024 * 1024;
const MAX_SHM: u64 = 4 * 1024 * 1024;

fn failed(message: impl ToString) -> StoreError {
    StoreError::Integrity(message.to_string())
}
fn unavailable(message: impl ToString) -> StoreError {
    StoreError::Unavailable(message.to_string())
}
fn limit() -> StoreError {
    StoreError::Invalid("RECOVERY_INSPECT_LIMIT_EXCEEDED".into())
}

pub fn validate_path(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(failed(
            "absolute recovery path without parent traversal required",
        ));
    }
    for parent in path.ancestors().skip(1) {
        let metadata = fs::symlink_metadata(parent).map_err(failed)?;
        if !metadata.file_type().is_dir() {
            return Err(failed(
                "recovery path contains a symlink or non-directory parent",
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FileDigest {
    pub name: String,
    pub bytes: Counter,
    pub sha256: Digest,
}

struct Source {
    path: PathBuf,
    file: File,
    metadata: fs::Metadata,
    digest: FileDigest,
}
#[cfg(unix)]
fn open_existing(path: &Path, write: bool) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = OpenOptions::new()
        .read(true)
        .write(write)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32)
        .open(path)
        .map_err(failed)?;
    if !file.metadata().map_err(failed)?.is_file() {
        return Err(failed("regular existing recovery source required"));
    }
    Ok(file)
}
#[cfg(not(unix))]
fn open_existing(_: &Path, _: bool) -> Result<File> {
    Err(failed("recovery source locking requires Unix"))
}

fn hash_file(file: &mut File, maximum: u64) -> Result<(u64, Digest)> {
    file.seek(SeekFrom::Start(0)).map_err(failed)?;
    let mut hash = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 16384];
    loop {
        let count = file.read(&mut buffer).map_err(failed)?;
        if count == 0 {
            break;
        }
        bytes = bytes.checked_add(count as u64).ok_or_else(limit)?;
        if bytes > maximum {
            return Err(limit());
        }
        hash.update(&buffer[..count]);
    }
    Ok((bytes, Digest::from_bytes(hash.finalize().into())))
}
fn same_metadata(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.is_file()
            && b.is_file()
            && a.dev() == b.dev()
            && a.ino() == b.ino()
            && a.len() == b.len()
            && a.mtime() == b.mtime()
            && a.mtime_nsec() == b.mtime_nsec()
            && a.ctime() == b.ctime()
            && a.ctime_nsec() == b.ctime_nsec()
    }
    #[cfg(not(unix))]
    {
        a.is_file() && b.is_file() && a.len() == b.len() && a.modified().ok() == b.modified().ok()
    }
}
impl Source {
    fn capture(
        path: PathBuf,
        mut file: File,
        maximum: u64,
        destination: Option<&Path>,
    ) -> Result<Self> {
        let metadata = file.metadata().map_err(failed)?;
        if !metadata.is_file() || metadata.len() > maximum {
            return Err(limit());
        }
        let mut copied = destination
            .map(|path| OpenOptions::new().write(true).create_new(true).open(path))
            .transpose()
            .map_err(failed)?;
        let mut hash = Sha256::new();
        let mut bytes = 0_u64;
        let mut buffer = [0_u8; 16384];
        loop {
            let count = file.read(&mut buffer).map_err(failed)?;
            if count == 0 {
                break;
            }
            bytes = bytes.checked_add(count as u64).ok_or_else(limit)?;
            if bytes > maximum {
                return Err(limit());
            }
            hash.update(&buffer[..count]);
            if let Some(copied) = &mut copied {
                copied.write_all(&buffer[..count]).map_err(failed)?;
            }
        }
        if bytes != metadata.len() || !same_metadata(&metadata, &file.metadata().map_err(failed)?) {
            return Err(failed("recovery source changed while copying"));
        }
        let digest = FileDigest {
            name: path
                .file_name()
                .and_then(|v| v.to_str())
                .ok_or_else(|| failed("UTF-8 source name required"))?
                .into(),
            bytes: Counter(bytes),
            sha256: Digest::from_bytes(hash.finalize().into()),
        };
        Ok(Self {
            path,
            file,
            metadata,
            digest,
        })
    }
    fn verify(&mut self) -> Result<()> {
        let path_metadata = fs::symlink_metadata(&self.path).map_err(failed)?;
        if !same_metadata(&self.metadata, &path_metadata)
            || !same_metadata(&self.metadata, &self.file.metadata().map_err(failed)?)
        {
            return Err(failed("recovery source identity or mtime changed"));
        }
        let (bytes, digest) = hash_file(&mut self.file, self.metadata.len())?;
        if bytes != self.digest.bytes.0 || digest != self.digest.sha256 {
            return Err(failed("recovery source content changed"));
        }
        if !same_metadata(&self.metadata, &self.file.metadata().map_err(failed)?)
            || !same_metadata(
                &self.metadata,
                &fs::symlink_metadata(&self.path).map_err(failed)?,
            )
        {
            return Err(failed("recovery source changed while verifying"));
        }
        Ok(())
    }
}

/// The held existing writer lock prevents ordinary SQLite writers across both file captures.
pub(crate) struct Snapshot {
    pub records: BTreeMap<Name, Record>,
    pub events: Vec<StoredEvent>,
    pub files: Vec<FileDigest>,
    sources: Vec<Source>,
    absent: Vec<PathBuf>,
    _copy: tempfile::TempDir,
}
impl Snapshot {
    pub fn open(path: &Path) -> Result<Self> {
        validate_path(path)?;
        let parent = path
            .parent()
            .ok_or_else(|| failed("source parent required"))?;
        if !parent.is_dir() {
            return Err(failed("existing recovery root required"));
        }
        let lock_path = path.with_extension("writer.lock");
        let lock = open_existing(&lock_path, true)?;
        lock.try_lock()
            .map_err(|e| unavailable(format!("RECOVERY_SOURCE_OWNED: {e}")))?;
        let copy = tempfile::tempdir().map_err(unavailable)?;
        let filename = path
            .file_name()
            .ok_or_else(|| failed("source filename required"))?;
        let copied_db = copy.path().join(filename);
        let mut sources = vec![Source::capture(lock_path, lock, 65_536, None)?];
        let mut absent = vec![];
        for (suffix, maximum) in [("", MAX_DB), ("-wal", MAX_DB), ("-shm", MAX_SHM)] {
            let source = PathBuf::from(format!("{}{suffix}", path.display()));
            match fs::symlink_metadata(&source) {
                Err(error)
                    if !suffix.is_empty() && error.kind() == std::io::ErrorKind::NotFound =>
                {
                    absent.push(source);
                }
                Err(error) => return Err(failed(error)),
                Ok(metadata) => {
                    if !metadata.file_type().is_file() || (suffix.is_empty() && metadata.len() == 0)
                    {
                        return Err(failed("nonempty database/regular sidecar required"));
                    }
                    let target = PathBuf::from(format!("{}{suffix}", copied_db.display()));
                    sources.push(Source::capture(
                        source.clone(),
                        open_existing(&source, false)?,
                        maximum,
                        Some(&target),
                    )?);
                }
            }
        }
        let mut snapshot = Self {
            records: BTreeMap::new(),
            events: vec![],
            files: sources.iter().map(|s| s.digest.clone()).collect(),
            sources,
            absent,
            _copy: copy,
        };
        snapshot.verify_sources()?;
        super::wal::validate_copy(&copied_db)?;
        // SHM is a transient index, not a durable commit watermark. Rebuild it
        // from the fully validated WAL in the private copy; never trust a copied
        // stale mxFrame to hide later complete commits or touch original SHM.
        let copied_shm = PathBuf::from(format!("{}-shm", copied_db.display()));
        match fs::remove_file(&copied_shm) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(failed(error)),
        }
        let connection = rusqlite::Connection::open_with_flags(
            &copied_db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(failed)?;
        connection
            .busy_timeout(std::time::Duration::from_millis(250))
            .map_err(failed)?;
        connection
            .execute_batch("PRAGMA trusted_schema=OFF; PRAGMA query_only=ON; BEGIN DEFERRED;")
            .map_err(failed)?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(failed)?;
        if version != 6 {
            return Err(failed(
                "recovery inspection requires exact existing SQLite schema 6; no migration",
            ));
        }
        let integrity: String = connection
            .query_row("PRAGMA quick_check", [], |r| r.get(0))
            .map_err(failed)?;
        if integrity != "ok" {
            return Err(failed(integrity));
        }
        for table in [
            "entities",
            "events",
            "requests",
            "outbox",
            "control_entities",
            "control_events",
        ] {
            let kind: String = connection
                .query_row(
                    "SELECT type FROM sqlite_schema WHERE name=?1",
                    [table],
                    |r| r.get(0),
                )
                .map_err(failed)?;
            if kind != "table" {
                return Err(failed("required recovery store table differs"));
            }
        }
        for table in ["requests", "outbox", "control_entities", "control_events"] {
            let count: i64 = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .map_err(failed)?;
            if count != 0 {
                return Err(failed(
                    "foreign request/outbox/control data in executor journal",
                ));
            }
        }
        let mut stored_bytes = 0_u64;
        for (table, maximum) in [("entities", MAX_RECORDS), ("events", MAX_EVENTS)] {
            let (count, bytes, largest): (i64, i64, i64) = connection.query_row(&format!("SELECT COUNT(*),COALESCE(SUM(length(document)),0),COALESCE(MAX(length(document)),0) FROM {table}"), [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(failed)?;
            if count < 0
                || count as usize > maximum
                || bytes < 0
                || largest < 0
                || largest as u64 > MAX_DOCUMENT
            {
                return Err(limit());
            }
            stored_bytes = stored_bytes.checked_add(bytes as u64).ok_or_else(limit)?;
            if stored_bytes > MAX_BYTES {
                return Err(limit());
            }
        }
        {
            let mut statement = connection
                .prepare("SELECT key,revision,document FROM entities ORDER BY key")
                .map_err(failed)?;
            let rows = statement
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, Vec<u8>>(2)?,
                    ))
                })
                .map_err(failed)?;
            for row in rows {
                let (key, revision, bytes) = row.map_err(failed)?;
                if revision <= 0 {
                    return Err(failed("positive record revision required"));
                }
                let key = Name::new(key).map_err(failed)?;
                let document: Document = canonical::decode_json(&bytes).map_err(failed)?;
                if snapshot
                    .records
                    .insert(
                        key.clone(),
                        Record {
                            key,
                            revision: Counter(revision as u64),
                            document,
                        },
                    )
                    .is_some()
                {
                    return Err(failed("duplicate entity key"));
                }
            }
        }
        {
            let mut statement = connection
                .prepare("SELECT seq,event_id,document FROM events ORDER BY seq")
                .map_err(failed)?;
            let rows = statement
                .query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Vec<u8>>(2)?,
                    ))
                })
                .map_err(failed)?;
            for row in rows {
                let (seq, id, bytes) = row.map_err(failed)?;
                if seq != snapshot.events.len() as i64 + 1 {
                    return Err(failed("event history has a missing or regressed sequence"));
                }
                snapshot.events.push(StoredEvent {
                    seq: Counter(seq as u64),
                    event_id: Id::new(id).map_err(failed)?,
                    document: canonical::decode_json(&bytes).map_err(failed)?,
                });
            }
        }
        connection.execute_batch("ROLLBACK").map_err(failed)?;
        drop(connection);
        snapshot.verify_sources()?;
        Ok(snapshot)
    }
    pub fn verify_sources(&mut self) -> Result<()> {
        for source in &mut self.sources {
            source.verify()?;
        }
        for path in &self.absent {
            match fs::symlink_metadata(path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                _ => return Err(failed("recovery sidecar appeared during inspection")),
            }
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<Digest> {
        canonical::digest("RX-E-RECOVERY-RECORDS-v1", &(&self.records, &self.events))
            .map_err(failed)
    }
}
impl Drop for Snapshot {
    fn drop(&mut self) {
        if let Some(lock) = self.sources.first() {
            let _ = lock.file.unlock();
        }
    }
}
impl Transaction for Snapshot {
    fn control_head(&mut self) -> Result<Counter> {
        Ok(Counter(0))
    }
    fn get(&mut self, key: &Name) -> Result<Option<Record>> {
        Ok(self.records.get(key).cloned())
    }
    fn scan(&mut self, prefix: &str) -> Result<Vec<Record>> {
        Ok(self
            .records
            .values()
            .filter(|r| r.key.as_str().starts_with(prefix))
            .cloned()
            .collect())
    }
    fn outbox(&mut self, _: &Id) -> Result<Option<OutboxRecord>> {
        Ok(None)
    }
    fn lookup(&mut self, _: &RequestScope) -> Result<Option<SavedRequest>> {
        Ok(None)
    }
    fn append_control(&mut self, _: &Id, _: &Record, _: &Document) -> Result<Counter> {
        Err(failed("read-only recovery snapshot"))
    }
    fn put(&mut self, _: &Name, _: Option<Counter>, _: &Document) -> Result<Record> {
        Err(failed("read-only recovery snapshot"))
    }
    fn remember(&mut self, _: &RequestScope, _: &SavedRequest) -> Result<()> {
        Err(failed("read-only recovery snapshot"))
    }
    fn append(&mut self, _: &Id, _: &Document) -> Result<Counter> {
        Err(failed("read-only recovery snapshot"))
    }
    fn enqueue(&mut self, _: &Id, _: &Document) -> Result<()> {
        Err(failed("read-only recovery snapshot"))
    }
    fn transition_outbox(&mut self, _: &Id, _: OutboxState, _: OutboxState) -> Result<()> {
        Err(failed("read-only recovery snapshot"))
    }
}
