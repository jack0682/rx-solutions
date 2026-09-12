//! Explicit cell-mode deployment entrypoint. Operator/P inputs cannot select files or programs.
use rx_domain::{canonical, types::*};
use rx_executor::{
    Client, PeerPin, TlsEndpoint,
    assignment_journal::{AssignmentJournal, Identity},
    cell_service::{CellService, Phase},
    clock::{Clock, LinuxBoottime},
    engine_process::Executable,
    service::{CoordinationMode, Options, PinnedPlanner},
    service_owner::ServiceOwner,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const CONFIG_LIMIT: u64 = 65_536;
const TLS_LIMIT: u64 = 1_048_576;
const ENGINE_LIMIT: u64 = 134_217_728;
const JOURNAL_FILE: &str = "assignment.sqlite3";
const INSTALLATION_FILE: &str = "cell-installation.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PinnedFile {
    path: PathBuf,
    sha256: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Platform {
    uri: String,
    server_name: String,
    ca: PinnedFile,
    certificate: PinnedFile,
    key: PinnedFile,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CellOptions {
    poll_ms: u64,
    communication_grace_ms: u64,
    stop_timeout_ms: u64,
}
impl Default for CellOptions {
    fn default() -> Self {
        let options = Options::default();
        Self {
            poll_ms: options.poll_ms,
            communication_grace_ms: options.communication_grace_ms,
            stop_timeout_ms: options.stop_timeout_ms,
        }
    }
}
impl CellOptions {
    fn validated(&self) -> Result<Options> {
        if !(10..=1000).contains(&self.poll_ms)
            || !(100..=60000).contains(&self.communication_grace_ms)
            || !(100..=60000).contains(&self.stop_timeout_ms)
        {
            return Err("executor cell timing options out of range".into());
        }
        Ok(Options {
            coordination: CoordinationMode::SerialProduction,
            poll_ms: self.poll_ms,
            communication_grace_ms: self.communication_grace_ms,
            stop_timeout_ms: self.stop_timeout_ms,
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CellConfig {
    schema: Name,
    service_root: PathBuf,
    expected_service: Identity,
    platform: Platform,
    engine: PinnedFile,
    #[serde(default)]
    options: CellOptions,
}
impl CellConfig {
    fn read(path: &Path) -> Result<Self> {
        let value: Self = canonical::decode_json(&read_regular(path, CONFIG_LIMIT, false)?)?;
        if value.schema.as_str() != "rx.executor-cell-service.v1"
            || !value.service_root.is_absolute()
            || value.platform.uri.len() > 2048
            || !value.platform.uri.starts_with("https://")
            || value.platform.server_name.is_empty()
            || value.platform.server_name.len() > 253
            || value.platform.server_name.chars().any(char::is_whitespace)
            || [
                &value.platform.ca,
                &value.platform.certificate,
                &value.platform.key,
                &value.engine,
            ]
            .iter()
            .any(|file| !file.path.is_absolute())
        {
            return Err("executor cell deployment configuration invalid".into());
        }
        tonic::transport::Endpoint::from_shared(value.platform.uri.clone())?;
        value.options.validated()?;
        Ok(value)
    }
    fn normalize_root(&mut self) -> Result<()> {
        self.service_root = fs::canonicalize(&self.service_root)?;
        Ok(())
    }
    fn digest(&self) -> Result<Digest> {
        Ok(canonical::digest("RX-EXECUTOR-CELL-CONFIG-v1", self)?)
    }
    fn material(&self) -> Result<TlsEndpoint> {
        let ca_pem = read_pin(&self.platform.ca, false)?;
        let certificate_pem = read_pin(&self.platform.certificate, false)?;
        let private_key_pem = read_pin(&self.platform.key, true)?;
        verify_engine(&self.engine)?;
        Ok(TlsEndpoint {
            uri: self.platform.uri.clone(),
            server_name: self.platform.server_name.clone(),
            ca_pem,
            certificate_pem,
            private_key_pem,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Installation {
    schema: Name,
    configuration_digest: Digest,
    service_root: PathBuf,
    expected_service: Identity,
}
impl Installation {
    fn from_config(config: &CellConfig) -> Result<Self> {
        Ok(Self {
            schema: Name::new("rx.executor-cell-installation.v1")?,
            configuration_digest: config.digest()?,
            service_root: config.service_root.clone(),
            expected_service: config.expected_service.clone(),
        })
    }
    fn verify(&self, config: &CellConfig) -> Result<()> {
        if self.schema.as_str() != "rx.executor-cell-installation.v1"
            || self.configuration_digest != config.digest()?
            || self.service_root != config.service_root
            || self.expected_service != config.expected_service
        {
            return Err("executor cell installation/configuration pin differs".into());
        }
        Ok(())
    }
}

pub(super) fn initialize(path: &Path) -> Result<()> {
    let mut config = CellConfig::read(path)?;
    let _owner = ServiceOwner::acquire(&config.service_root.join(JOURNAL_FILE))?;
    config.normalize_root()?;
    // A prior partial init is evidence, not permission to overwrite either file.
    for entry in fs::read_dir(&config.service_root)? {
        if entry?.file_name() != ".rx-executor-service.lock" {
            return Err("executor cell initialization requires an unused service root".into());
        }
    }
    fs::set_permissions(&config.service_root, fs::Permissions::from_mode(0o700))?;
    let _material = config.material()?;
    let installation = Installation::from_config(&config)?;
    let manifest = config.service_root.join(INSTALLATION_FILE);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&manifest)?;
    file.write_all(&canonical::bytes(&installation)?)?;
    file.sync_all()?;
    File::open(&config.service_root)?.sync_all()?;
    #[cfg(feature = "test-harness")]
    if std::env::var_os("RX_EXECUTOR_CELL_INIT_FAIL_AFTER_MANIFEST").is_some() {
        return Err("TEST_ONLY_EXECUTOR_CELL_INIT_AFTER_MANIFEST".into());
    }
    let journal = AssignmentJournal::initialize_file(
        &config.service_root.join(JOURNAL_FILE),
        config.expected_service.clone(),
    )?;
    drop(journal);
    File::open(&config.service_root)?.sync_all()?;
    println!("{}", serde_json::to_string(&installation)?);
    Ok(())
}

pub(super) async fn run(path: &Path) -> Result<bool> {
    let mut config = CellConfig::read(path)?;
    let _owner = ServiceOwner::acquire(&config.service_root.join(JOURNAL_FILE))?;
    config.normalize_root()?;
    let installation: Installation = canonical::decode_json(&read_regular(
        &config.service_root.join(INSTALLATION_FILE),
        CONFIG_LIMIT,
        false,
    )?)?;
    installation.verify(&config)?;
    let mut journal = AssignmentJournal::open_file_required(
        &config.service_root.join(JOURNAL_FILE),
        config.expected_service.clone(),
    )?;
    // Required-open every current run store before registering a new P peer. Preparing without
    // a creation marker is only a reservation; CellService will still recheck its P authority.
    drop(journal.recover_current_file(&config.service_root)?);
    let endpoint = config.material()?;
    let clock = Arc::new(LinuxBoottime::new()?);
    let scope = &config.expected_service.scope;
    let pin = PeerPin {
        principal: scope.principal.clone(),
        peer_boot: Id::new(uuid::Uuid::new_v4().to_string())?,
        installation: scope.installation.clone(),
        store_generation: scope.store_generation.clone(),
        release: scope.release,
        clock_id: clock.now()?.clock_id,
        cell: scope.cell.clone(),
        definition: scope.definition,
    };
    #[cfg(feature = "test-harness")]
    super::linux::before_connect_probe()?;

    let (stop, mut shutdown) = tokio::sync::watch::channel(false);
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let signals = tokio::spawn(async move {
        tokio::select! { _=term.recv()=>{}, _=interrupt.recv()=>{} };
        let _ = stop.send(true);
    });
    let outcome = async {
        let client = tokio::select! {
            biased;
            _=shutdown.changed()=>return Err::<bool, Box<dyn std::error::Error>>("executor cell startup interrupted before connection completed".into()),
            connected=Client::connect(endpoint, pin, clock.clone())=>connected?,
        };
        let service = CellService::new(
            client, journal, config.service_root.clone(),
            PinnedPlanner(Executable { path: config.engine.path.clone(), sha256: config.engine.sha256 }),
            clock, config.options.validated()?,
        )?;
        let (updates, mut states) = tokio::sync::watch::channel(service.initial_status());
        println!("{}", serde_json::to_string(&*states.borrow())?);
        let output = tokio::spawn(async move {
            while states.changed().await.is_ok() {
                if let Ok(text) = serde_json::to_string(&*states.borrow_and_update()) {
                    println!("{text}");
                }
            }
        });
        let report = service.run(shutdown, updates).await;
        output.await?;
        println!("{}", serde_json::to_string(&report)?);
        Ok(report.status.phase == Phase::Stopped)
    }.await;
    signals.abort();
    outcome
}

fn open_regular(path: &Path, limit: u64, secret: bool) -> Result<File> {
    if !path.is_absolute() {
        return Err("absolute deployment file path required".into());
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.len() > limit
        || (secret && metadata.permissions().mode() & 0o077 != 0)
    {
        return Err("deployment file type, size or private-key permission differs".into());
    }
    Ok(file)
}
fn read_regular(path: &Path, limit: u64, secret: bool) -> Result<Vec<u8>> {
    let mut value = Vec::new();
    open_regular(path, limit, secret)?
        .take(limit + 1)
        .read_to_end(&mut value)?;
    if value.len() as u64 > limit {
        return Err("deployment file grew beyond its size bound".into());
    }
    Ok(value)
}
fn read_pin(pin: &PinnedFile, secret: bool) -> Result<Vec<u8>> {
    let value = read_regular(&pin.path, TLS_LIMIT, secret)?;
    if Digest::from_bytes(Sha256::digest(&value).into()) != pin.sha256 {
        return Err("executor deployment file digest differs".into());
    }
    Ok(value)
}
fn verify_engine(pin: &PinnedFile) -> Result<()> {
    let mut file = open_regular(&pin.path, ENGINE_LIMIT, false)?;
    if file.metadata()?.permissions().mode() & 0o111 == 0 {
        return Err("release engine must be executable".into());
    }
    let mut hash = Sha256::new();
    let mut buffer = [0; 16384];
    let mut total = 0_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > ENGINE_LIMIT {
            return Err("release engine exceeded its size bound".into());
        }
        hash.update(&buffer[..read]);
    }
    if Digest::from_bytes(hash.finalize().into()) != pin.sha256 {
        return Err("release engine digest differs".into());
    }
    Ok(())
}
