//! Product Host process composition. No test clock, fault injection or native startup action.
pub mod binding_change;
pub mod config;
pub mod device_package;
mod factory;
pub mod jtc_package;
pub mod maintenance;
use crate::{Binding, Clock, Environment, Host, NativeAdapter};
use config::{Backend, Loaded};
pub use factory::{Builtin, BuiltinAdapter};
use rx_domain::{canonical, types::*};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
/// A release-owned factory, never an executable path or dynamic library from site input.
/// Opening must be passive and safe to drop before any admitted lifecycle operation.
pub trait AdapterFactory<C: Clock>: Send + Sync {
    type Adapter: NativeAdapter + 'static;
    fn validate(&self, backend: &Backend, bindings: &[Binding]) -> Result<()>;
    /// Create passive native metadata inside the unexposed installation staging directory.
    fn initialize_metadata(
        &self,
        _backend: &Backend,
        _data: &Path,
    ) -> Result<Option<NativeInstallation>> {
        Ok(None)
    }
    fn open_passive(&self, backend: &Backend, data: &Path, clock: C) -> Result<Self::Adapter>;
}
struct MetadataOnly(Environment);
impl crate::native::LocalProtection for MetadataOnly {
    fn react(&self, _: crate::ProtectionIncident) {}
}
impl NativeAdapter for MetadataOnly {
    fn environment(&self) -> Environment {
        self.0
    }
    fn protection(&self) -> Arc<dyn crate::native::LocalProtection> {
        Arc::new(MetadataOnly(self.0))
    }
    fn guard(&self, _: &rx_domain::intent::Intent, _: &TimePoint) -> crate::Result<crate::Guard> {
        Err(crate::HostError::Guard)
    }
    fn can_handover(&self, _: &[Name]) -> bool {
        false
    }
    fn submit(
        &mut self,
        _: &Id,
        _: &Id,
        _: &rx_domain::intent::Intent,
    ) -> crate::Result<crate::NativeCapture> {
        Err(crate::HostError::Guard)
    }
    fn lookup(&mut self, _: &Id, _: &Id) -> crate::Result<Option<crate::NativeCapture>> {
        Err(crate::HostError::Guard)
    }
}
struct AdmissionOwner<N: NativeAdapter, C: Clock> {
    host: Arc<Host<N, C>>,
    graceful: bool,
}
impl<N: NativeAdapter, C: Clock> Drop for AdmissionOwner<N, C> {
    fn drop(&mut self) {
        if !self.graceful {
            self.host.service_owner_lost();
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum NativeInstallation {
    Jtc {
        identity: crate::ros_jtc::Identity,
        manifest_digest: Digest,
    },
    Melsec {
        identity: crate::melsec::Identity,
        manifest_digest: Digest,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Installation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    maintenance_protocol: Option<Name>,
    schema: Name,
    identity: Digest,
    installation: Id,
    host: Name,
    delivery_journal: Id,
    evidence_journal: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    native: Option<NativeInstallation>,
}
#[derive(Serialize)]
pub struct Status {
    pub schema: Name,
    pub phase: Name,
    pub installation: Id,
    pub host: Name,
    pub instance: Id,
    pub host_boot: Id,
    pub endpoint: String,
    pub clock_id: String,
    pub admission_open: bool,
    pub publication: String,
    pub stop: Option<crate::gate::StopSnapshot>,
    pub pending_evidence: Option<Counter>,
    pub qualification_or_arm_restored: bool,
}
fn name(s: &str) -> Name {
    Name::new(s).expect("service literal")
}
fn private_write(path: &Path, data: &[u8]) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options.open(path)?;
    f.write_all(data)?;
    f.sync_all()?;
    Ok(())
}
fn real_directory(path: &Path) -> Result<()> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir() || m.file_type().is_symlink() {
        return Err("real owned directory required".into());
    }
    Ok(())
}
fn validate_state_files(path: &Path) -> Result<()> {
    real_directory(path)?;
    for file in [
        "host.db",
        "host.db-wal",
        "host.db-shm",
        "host.writer.lock",
        "installation.json",
    ] {
        let p = path.join(file);
        match fs::symlink_metadata(&p) {
            Ok(m) if !m.is_file() || m.file_type().is_symlink() => {
                return Err("invalid Host state file".into());
            }
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    let device = path.join("device");
    if device.exists() {
        real_directory(&device)?;
        for file in ["device.lock", "device-session", "effects.jsonl"] {
            let p = device.join(file);
            if let Ok(m) = fs::symlink_metadata(&p)
                && (!m.is_file() || m.file_type().is_symlink())
            {
                return Err("invalid simulation state file".into());
            }
        }
    }
    let native = path.join("native-melsec");
    if native.exists() {
        real_directory(&native)?;
        for file in [
            "native.sqlite3",
            "native.sqlite3-wal",
            "native.sqlite3-shm",
            "native.writer.lock",
        ] {
            if let Ok(m) = fs::symlink_metadata(native.join(file))
                && (!m.is_file() || m.file_type().is_symlink())
            {
                return Err("invalid native state file".into());
            }
        }
    }
    Ok(())
}
fn status(path: &Path, value: &Status) -> Result<()> {
    let target = path.join("host-status.json");
    if let Ok(m) = fs::symlink_metadata(&target) {
        if !m.is_file() || m.file_type().is_symlink() {
            return Err("invalid status destination".into());
        }
        let old: serde_json::Value = canonical::decode_json(&fs::read(&target)?)?;
        if old["installation"] != value.installation.to_string()
            || old["host"] != value.host.as_str()
        {
            return Err("status belongs to another installation/Host".into());
        }
    }
    let temporary = path.join(format!(".host-status-{}", uuid::Uuid::new_v4()));
    private_write(&temporary, &canonical::bytes(value)?)?;
    fs::rename(temporary, target)?;
    Ok(())
}
pub fn inspect(loaded: &Loaded) -> Result<serde_json::Value> {
    <Builtin as AdapterFactory<crate::service_clock::SystemClock>>::validate(
        &Builtin,
        &loaded.config.backend,
        &loaded.bindings,
    )?;
    Ok(
        serde_json::json!({"schema":"rx.host-installation-inspection.v1","installation":loaded.config.installation,"host":loaded.config.host,"backend":loaded.config.backend,"identity":loaded.identity,"binding_cells":loaded.bindings.iter().map(|b|b.cell.clone()).collect::<Vec<_>>(),"activation_authorized":false,"native_processes_started":0,"physical_backend_available":matches!(loaded.config.backend,Backend::MelsecPackage {..})&&loaded.bindings.iter().any(|b|b.environment==Environment::Physical),"physical_qualification_verified":false,"runtime_qualification_required":loaded.bindings.iter().any(|b|b.environment==Environment::Physical),"control_provider":if matches!(loaded.config.backend,Backend::JtcPackage {..}){"NOT_CONFIGURED"}else{"BUILTIN"}}),
    )
}
pub fn initialize_with<C: Clock + Clone + 'static, F: AdapterFactory<C>>(
    loaded: &Loaded,
    clock: C,
    factory: &F,
) -> Result<()> {
    factory.validate(&loaded.config.backend, &loaded.bindings)?;
    validate_installation_material(loaded)?;
    if loaded.config.data_directory.exists() {
        return Err("Host installation already exists".into());
    }
    let parent = loaded.config.data_directory.parent().ok_or("data parent")?;
    real_directory(parent)?;
    let stage = parent.join(format!(".rx-host-init-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&stage)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&stage, fs::Permissions::from_mode(0o700))?;
    }
    let result = (|| {
        // Initialization creates only the Host journal. No device adapter is opened.
        let host = Host::open(
            stage.join("host.db"),
            MetadataOnly(loaded.bindings[0].environment),
            clock,
            loaded.bindings.clone(),
        )?;
        let journals = host.journals()?;
        drop(host);
        let native = factory.initialize_metadata(&loaded.config.backend, &stage)?;
        private_write(
            &stage.join("installation.json"),
            &canonical::bytes(&Installation {
                maintenance_protocol: Some(name("rx.host-maintenance.v1")),
                schema: name("rx.host-installation.v2"),
                identity: loaded.identity,
                installation: loaded.config.installation.clone(),
                host: loaded.config.host.clone(),
                delivery_journal: journals.delivery_journal,
                evidence_journal: journals.evidence_journal,
                native,
            })?,
        )?;
        for dir in ["native-melsec", "native-jtc"] {
            if stage.join(dir).exists() {
                fs::File::open(stage.join(dir))?.sync_all()?;
            }
        }
        fs::File::open(&stage)?.sync_all()?;
        publish_installation(parent, &stage, &loaded.config.data_directory)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}
fn validate_installation_material(loaded: &Loaded) -> Result<()> {
    if let Backend::JtcPackage { .. } = &loaded.config.backend {
        let device = jtc_package::load(&loaded.config.backend)?;
        if device.profile.installation != loaded.config.installation {
            return Err("JTC installation differs from Host".into());
        }
    }
    if let Backend::MelsecPackage { .. } = &loaded.config.backend {
        let device = device_package::load(&loaded.config.backend)?;
        if device.profile.installation != loaded.config.installation {
            return Err("device installation differs from Host".into());
        }
    }
    Ok(())
}
fn publish_installation(parent: &Path, stage: &Path, target: &Path) -> Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let directory = fs::File::open(parent)?;
        rustix::fs::renameat_with(
            &directory,
            stage.file_name().ok_or("stage name")?,
            &directory,
            target.file_name().ok_or("installation name")?,
            rustix::fs::RenameFlags::NOREPLACE,
        )?;
        directory.sync_all()?;
        Ok(())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (parent, stage, target);
        Err("atomic no-replace installation publication unsupported".into())
    }
}

fn runtime_owner(directory: &Path) -> Result<std::fs::File> {
    real_directory(directory)?;
    let lock_path = directory.join("host.lock");
    if let Ok(m) = fs::symlink_metadata(&lock_path)
        && (!m.is_file() || m.file_type().is_symlink())
    {
        return Err("invalid runtime lock".into());
    }
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let owner = options.open(lock_path)?;
    owner
        .try_lock()
        .map_err(|_| "another process owns this Host runtime")?;
    Ok(owner)
}
pub async fn run_with<C: Clock + Clone + Send + Sync + 'static, F: AdapterFactory<C>>(
    loaded: Loaded,
    clock: C,
    factory: F,
    stop: impl std::future::Future<Output = ()> + Send,
) -> Result<()> {
    factory.validate(&loaded.config.backend, &loaded.bindings)?;
    validate_installation_material(&loaded)?;
    validate_state_files(&loaded.config.data_directory)?;
    real_directory(&loaded.config.runtime_directory)?;
    let descriptor_path = loaded.config.data_directory.join("installation.json");
    if fs::symlink_metadata(&descriptor_path)?.len() > 1_048_576 {
        return Err("installation descriptor too large".into());
    }
    let descriptor: Installation = canonical::decode_json(&fs::read(descriptor_path)?)?;
    if !matches!(
        (
            descriptor.schema.as_str(),
            descriptor.maintenance_protocol.as_ref().map(Name::as_str)
        ),
        ("rx.host-installation.v1", None)
            | ("rx.host-installation.v2", Some("rx.host-maintenance.v1"))
    ) || descriptor.identity != loaded.identity
        || descriptor.installation != loaded.config.installation
        || descriptor.host != loaded.config.host
    {
        return Err("Host installation identity differs".into());
    }
    if !loaded.config.data_directory.join("host.db").is_file() {
        return Err("Host journal missing; explicit restore required".into());
    }
    let _owner = runtime_owner(&loaded.config.runtime_directory)?;
    let startup_attempt = crate::journal::id();
    let startup_configuration_digest = maintenance::startup_digest(&loaded.config)?;
    // Verify existing journal identities before Host::open could initialize missing rows.
    {
        use rx_ports::Repository;
        let mut store =
            rx_storage::SqliteRepository::open(loaded.config.data_directory.join("host.db"))?;
        store.transact(|tx| {
            let meta: crate::HostMeta = crate::journal::decode(
                &tx.get(&name("host/meta"))?
                    .ok_or(rx_ports::StoreError::Integrity(
                        "Host journal metadata missing".into(),
                    ))?,
                "rx.host.meta.v1",
            )?;
            if meta.delivery_journal != descriptor.delivery_journal
                || meta.evidence_journal != descriptor.evidence_journal
            {
                return Err(rx_ports::StoreError::Integrity(
                    "Host journal generation differs".into(),
                ));
            }
            for binding in &loaded.bindings {
                let state: crate::CellState = crate::journal::decode(
                    &tx.get(&crate::journal::key("cell", &binding.cell))?.ok_or(
                        rx_ports::StoreError::Integrity("Host cell state missing".into()),
                    )?,
                    "rx.host.cell.v1",
                )?;
                if state.epoch.0 == 0
                    || state
                        .scopes
                        .keys()
                        .collect::<std::collections::BTreeSet<_>>()
                        != binding.scope_ids.iter().collect()
                    || state.scopes.values().any(|v| v.0 == 0)
                {
                    return Err(rx_ports::StoreError::Integrity(
                        "Host cell generation differs".into(),
                    ));
                }
            }
            maintenance::begin_startup(
                tx,
                loaded.identity,
                startup_configuration_digest,
                &startup_attempt,
            )?;
            Ok(())
        })?;
    }
    let listener = tokio::net::TcpListener::bind(loaded.config.bind).await?;
    let address = listener.local_addr()?;
    let native = factory.open_passive(
        &loaded.config.backend,
        &loaded.config.data_directory,
        clock.clone(),
    )?;
    let host = Arc::new(Host::open(
        loaded.config.data_directory.join("host.db"),
        native,
        clock.clone(),
        loaded.bindings,
    )?);
    let mut admission_owner = AdmissionOwner {
        host: host.clone(),
        graceful: false,
    };
    let journals = host.journals()?;
    if descriptor.delivery_journal != journals.delivery_journal
        || descriptor.evidence_journal != journals.evidence_journal
    {
        return Err("Host journal generation differs from installation".into());
    }
    let rpc = crate::rpc::RpcHost::new(
        host.clone(),
        crate::rpc::Configuration {
            installation: loaded.config.installation.clone(),
            release_digest: loaded.config.release_digest,
            allowed_certificates: loaded.config.allowed_platform_certificates,
        },
    )?;
    let instance = std::env::var("RX_PROCESS_INSTANCE_ID")
        .ok()
        .map(Id::new)
        .transpose()?
        .unwrap_or_else(crate::journal::id);
    let mut view = Status {
        schema: name("rx.host-service-status.v1"),
        phase: name("SOFTWARE_READY_UNARMED"),
        installation: loaded.config.installation,
        host: loaded.config.host,
        instance,
        host_boot: host.boot_id()?,
        endpoint: format!("https://{address}"),
        clock_id: clock.now().clock_id,
        admission_open: true,
        publication: "DISABLED".into(),
        stop: None,
        pending_evidence: None,
        qualification_or_arm_restored: false,
    };
    let (stop_rpc, rpc_stop) = tokio::sync::oneshot::channel();
    let mut serving = tokio::spawn(rpc.serve(listener, loaded.tls, async {
        let _ = rpc_stop.await;
    }));
    let (stop_publisher, publisher_stop) = tokio::sync::watch::channel(false);
    let destination = loaded.publisher.as_ref().map(|p| p.destination.clone());
    let mut publisher_status = None;
    let mut publisher_task = None;
    if let Some(config) = loaded.publisher {
        let publisher = crate::publication::Publisher::new(host.clone(), config);
        publisher_status = Some(publisher.subscribe());
        publisher_task = Some(tokio::spawn(publisher.run(publisher_stop)));
    }
    if let Err(error) = status(&loaded.config.runtime_directory, &view) {
        host.request_service_stop();
        view.phase = name("STATUS_WRITE_FAILED");
        view.admission_open = false;
        eprintln!("Host status failed; closing admission: {error}");
    }
    tokio::pin!(stop);
    let mut stopping = !view.admission_open;
    let mut server_done = false;
    let mut service_error = None;
    let mut check: Option<tokio::task::JoinHandle<crate::Result<crate::gate::StopSnapshot>>> = None;
    let mut drain_started = None;
    let mut last = String::new();
    loop {
        if stopping {
            host.request_service_stop();
            view.admission_open = false;
            view.phase = name("STOP_WAITING_FOR_ADAPTER");
            if check.is_none() {
                let h = host.clone();
                check = Some(tokio::task::spawn_blocking(move || {
                    h.service_stop_snapshot()
                }));
            }
        }
        if !stopping && publisher_task.as_ref().is_some_and(|p| p.is_finished()) {
            stopping = true;
            service_error = Some("configured evidence publisher stopped".into());
        }
        if let Some(s) = &publisher_status {
            view.publication = format!("{:?}", s.borrow().clone());
        }
        if check.as_ref().is_some_and(|h| h.is_finished()) {
            match check.take().unwrap().await {
                Ok(Ok(report)) => {
                    let safe = report.safe_to_drop;
                    view.stop = Some(report);
                    if safe {
                        let start = drain_started.get_or_insert_with(Instant::now);
                        let pending = if let Some(destination) = &destination {
                            let h = host.clone();
                            let d = destination.clone();
                            match tokio::task::spawn_blocking(move || h.publication_chunk(&d)).await
                            {
                                Ok(Ok(chunk)) => Some(Counter(
                                    chunk.tail.0.saturating_sub(chunk.cursor.through.0),
                                )),
                                _ => None,
                            }
                        } else {
                            view.stop.as_ref().map(|s| s.retained_evidence)
                        };
                        view.pending_evidence = pending;
                        if destination.is_none()
                            || pending == Some(Counter(0))
                            || start.elapsed()
                                >= Duration::from_millis(loaded.config.publication_drain_ms.0)
                        {
                            break;
                        }
                        view.phase = name("STOP_DRAINING_EVIDENCE");
                    }
                }
                Ok(Err(e)) => {
                    view.phase = name("STOP_STATE_UNAVAILABLE");
                    eprintln!("Host stop remains unproven: {e}");
                }
                Err(e) => {
                    view.phase = name("STOP_WORKER_FAILED");
                    eprintln!("Host stop worker failed: {e}");
                }
            }
        }
        let text = serde_json::to_string(&view)?;
        if text != last {
            if let Err(e) = status(&loaded.config.runtime_directory, &view) {
                stopping = true;
                eprintln!("Host status write failed: {e}");
            }
            last = text;
        }
        tokio::select! {
         _=&mut stop,if !stopping=>{stopping=true;},
         result=&mut serving,if !server_done=>{server_done=true;stopping=true;if !matches!(result,Ok(Ok(()))){service_error=Some(format!("Host RPC service failed: {result:?}"));}},
         _=tokio::time::sleep(Duration::from_millis(100))=>{}
        }
    }
    // The adapter explicitly proved it can be dropped. New admission has already been latched off.
    let _ = stop_rpc.send(());
    if !server_done {
        match serving.await {
            Ok(Ok(())) => {}
            other => service_error = Some(format!("Host RPC shutdown: {other:?}")),
        }
    }
    let _ = stop_publisher.send(true);
    if let Some(task) = publisher_task {
        match task.await {
            Ok(Ok(())) => {}
            other => service_error = Some(format!("publisher shutdown: {other:?}")),
        }
    }
    loop {
        let h = host.clone();
        let checked = tokio::task::spawn_blocking(move || h.service_stop_snapshot()).await;
        match checked {
            Ok(Ok(snapshot)) if snapshot.safe_to_drop => {
                view.stop = Some(snapshot);
                break;
            }
            Ok(Ok(snapshot)) => view.stop = Some(snapshot),
            _ => {}
        }
        view.phase = name("STOP_WAITING_FINAL_ADAPTER_PROOF");
        let _ = status(&loaded.config.runtime_directory, &view);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    view.phase = name(if service_error.is_some() {
        "STOPPED_WITH_SERVICE_ERROR"
    } else if view.pending_evidence.is_none_or(|n| n.0 > 0)
        || view
            .stop
            .as_ref()
            .is_some_and(|s| !s.pending_operations.is_empty())
    {
        "STOPPED_WITH_RECONCILIATION_REQUIRED"
    } else {
        "STOPPED"
    });
    if service_error.is_none() {
        host.seal_service_stop(
            loaded.identity,
            startup_configuration_digest,
            &startup_attempt,
        )?;
    }
    view.admission_open = false;
    admission_owner.graceful = true;
    status(&loaded.config.runtime_directory, &view)?;
    if let Some(error) = service_error {
        return Err(error.into());
    }
    Ok(())
}
