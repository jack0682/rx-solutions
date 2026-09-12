use rx_domain::{canonical, intent::*, types::*};
use rx_host::{
    native::*,
    service::{self, config::*},
    simulation::*,
    *,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
#[path = "support/host_service_fixture.rs"]
mod fixture;
use fixture::*;

#[tokio::test]
async fn product_composition_has_immutable_installation_real_owner_and_clean_software_stop() {
    let (dir, file, clock) = fixture();
    let loaded = Loaded::read(&file).unwrap();
    service::initialize_with(&loaded, clock.clone(), &service::Builtin).unwrap();
    assert!(!loaded.config.data_directory.join("device").exists());
    assert!(service::initialize_with(&loaded, clock.clone(), &service::Builtin).is_err());
    let status_file = loaded.config.runtime_directory.join("host-status.json");
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let c = clock.clone();
    let task = tokio::spawn(service::run_with(loaded, c, service::Builtin, async {
        let _ = stopped.await;
    }));
    let first = status(&status_file, "SOFTWARE_READY_UNARMED").await;
    assert_eq!(first["admission_open"], true);
    assert_eq!(first["qualification_or_arm_restored"], false);
    let duplicate = service::run_with(
        Loaded::read(&file).unwrap(),
        clock.clone(),
        service::Builtin,
        std::future::pending(),
    )
    .await;
    assert!(duplicate.is_err());
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let stopped = status(&status_file, "STOPPED").await;
    assert_eq!(stopped["stop"]["safe_to_drop"], true);
    assert_eq!(stopped["stop"]["physical_shutdown_assessed"], false);
    assert!(!dir.path().join("data/device/effects.jsonl").exists());
    std::fs::remove_file(dir.path().join("data/host.db")).unwrap();
    assert!(
        service::run_with(
            Loaded::read(&file).unwrap(),
            clock,
            service::Builtin,
            std::future::pending()
        )
        .await
        .is_err()
    );
}
#[test]
fn startup_rejects_policy_tampering_unknown_fields_and_unimplemented_physical_backend() {
    let (dir, file, clock) = fixture();
    let loaded = Loaded::read(&file).unwrap();
    std::fs::write(&loaded.config.bindings.path, b"[]").unwrap();
    assert!(Loaded::read(&file).is_err());
    assert!(!dir.path().join("data").exists());
    let (_dir, file, clock2) = fixture();
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    value["ticks"] = serde_json::json!(1000);
    std::fs::write(&file, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(Loaded::read(&file).is_err());
    let _ = (clock, clock2);
    let (dir, file, clock) = fixture();
    let mut loaded = Loaded::read(&file).unwrap();
    loaded.config.backend = Backend::ValidatedDriver {
        profile: name("robotis/omy"),
        driver_digest: Digest::from_bytes([1; 32]),
    };
    assert!(service::initialize_with(&loaded, clock, &service::Builtin).is_err());
    assert!(!dir.path().join("data").exists());
}
struct Held {
    native: FileDevice,
    safe: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
}
impl Drop for Held {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}
impl NativeAdapter for Held {
    fn environment(&self) -> Environment {
        Environment::Simulation
    }
    fn protection(&self) -> Arc<dyn LocalProtection> {
        self.native.protection()
    }
    fn guard(&self, i: &Intent, t: &TimePoint) -> Result<Guard> {
        self.native.guard(i, t)
    }
    fn can_handover(&self, r: &[Name]) -> bool {
        self.native.can_handover(r)
    }
    fn shutdown_snapshot(&self, r: &[Name]) -> Result<NativeShutdown> {
        let mut v = self.native.shutdown_snapshot(r)?;
        v.safe_to_drop = self.safe.load(Ordering::SeqCst);
        Ok(v)
    }
    fn submit(&mut self, o: &Id, v: &Id, i: &Intent) -> Result<NativeCapture> {
        self.native.submit(o, v, i)
    }
    fn lookup(&mut self, o: &Id, v: &Id) -> Result<Option<NativeCapture>> {
        self.native.lookup(o, v)
    }
}
struct Factory {
    safe: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
    opened: Arc<AtomicU64>,
}
impl service::AdapterFactory<ManualClock> for Factory {
    type Adapter = Held;
    fn validate(&self, b: &Backend, bindings: &[Binding]) -> service::Result<()> {
        <service::Builtin as service::AdapterFactory<ManualClock>>::validate(
            &service::Builtin,
            b,
            bindings,
        )
    }
    fn open_passive(&self, _: &Backend, p: &Path, c: ManualClock) -> service::Result<Held> {
        self.opened.fetch_add(1, Ordering::SeqCst);
        Ok(Held {
            native: FileDevice::open(p.join("device"), c)?,
            safe: self.safe.clone(),
            dropped: self.dropped.clone(),
        })
    }
}
#[tokio::test]
async fn normal_stop_retains_adapter_until_drop_permission_is_explicit() {
    let (_dir, file, clock) = fixture();
    let loaded = Loaded::read(&file).unwrap();
    let status_file = loaded.config.runtime_directory.join("host-status.json");
    let safe = Arc::new(AtomicBool::new(false));
    let dropped = Arc::new(AtomicBool::new(false));
    let opened = Arc::new(AtomicU64::new(0));
    let factory = Factory {
        safe: safe.clone(),
        dropped: dropped.clone(),
        opened: opened.clone(),
    };
    service::initialize_with(&loaded, clock.clone(), &factory).unwrap();
    assert_eq!(opened.load(Ordering::SeqCst), 0);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(service::run_with(loaded, clock, factory, async {
        let _ = stopped.await;
    }));
    status(&status_file, "SOFTWARE_READY_UNARMED").await;
    stop.send(()).unwrap();
    let waiting = status(&status_file, "STOP_WAITING_FOR_ADAPTER").await;
    assert_eq!(waiting["admission_open"], false);
    assert!(!dropped.load(Ordering::SeqCst));
    assert!(!task.is_finished());
    safe.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(dropped.load(Ordering::SeqCst));
}

#[test]
#[ignore = "exports isolated container fixture"]
fn export_host_image_fixture() {
    let (_dir, path, _clock) = fixture();
    let target = std::path::PathBuf::from(std::env::var("RX_HOST_IMAGE_FIXTURE").unwrap());
    assert!(!target.exists());
    std::fs::create_dir_all(&target).unwrap();
    let mut config: Configuration = canonical::decode_json(&std::fs::read(&path).unwrap()).unwrap();
    for file in [
        "bindings.json",
        "server.pem",
        "server.key",
        "ca.pem",
        "client.pem",
        "client.key",
    ] {
        std::fs::copy(path.parent().unwrap().join(file), target.join(file)).unwrap();
    }
    for file in [
        &mut config.bindings,
        &mut config.tls.certificate,
        &mut config.tls.key,
        &mut config.tls.ca,
    ] {
        file.path = PathBuf::from("/config").join(file.path.file_name().unwrap());
    }
    config.data_directory = PathBuf::from("/data/host");
    config.runtime_directory = PathBuf::from("/data/runtime");
    config.bind = "0.0.0.0:7444".parse().unwrap();
    std::fs::write(
        target.join("startup.json"),
        canonical::bytes(&config).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn empty_replacement_database_is_not_reinitialized_as_a_fresh_host() {
    use rx_ports::Repository;
    let (dir, file, clock) = fixture();
    let loaded = Loaded::read(&file).unwrap();
    service::initialize_with(&loaded, clock.clone(), &service::Builtin).unwrap();
    let db = dir.path().join("data/host.db");
    std::fs::remove_file(&db).unwrap();
    drop(rx_storage::SqliteRepository::open(&db).unwrap());
    assert!(
        service::run_with(
            Loaded::read(&file).unwrap(),
            clock,
            service::Builtin,
            std::future::pending()
        )
        .await
        .is_err()
    );
    let mut store = rx_storage::SqliteRepository::open(&db).unwrap();
    assert!(
        store
            .transact(|tx| tx.get(&name("host/meta")))
            .unwrap()
            .is_none()
    );
}

struct InstallationRace(PathBuf);
impl service::AdapterFactory<ManualClock> for InstallationRace {
    type Adapter = FileDevice;
    fn validate(&self, b: &Backend, bindings: &[Binding]) -> service::Result<()> {
        <service::Builtin as service::AdapterFactory<ManualClock>>::validate(
            &service::Builtin,
            b,
            bindings,
        )
    }
    fn initialize_metadata(
        &self,
        _: &Backend,
        _: &Path,
    ) -> service::Result<Option<service::NativeInstallation>> {
        std::fs::create_dir(&self.0)?;
        Ok(None)
    }
    fn open_passive(&self, _: &Backend, _: &Path, _: ManualClock) -> service::Result<FileDevice> {
        panic!("initialization must not open adapter")
    }
}
#[test]
fn initialization_cannot_replace_a_concurrently_created_empty_destination() {
    let (_dir, file, c) = fixture();
    let loaded = Loaded::read(&file).unwrap();
    let destination = loaded.config.data_directory.clone();
    assert!(service::initialize_with(&loaded, c, &InstallationRace(destination.clone())).is_err());
    assert!(destination.is_dir());
    assert_eq!(std::fs::read_dir(&destination).unwrap().count(), 0);
    assert!(
        !std::fs::read_dir(destination.parent().unwrap())
            .unwrap()
            .any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".rx-host-init-"))
    );
}

#[cfg(unix)]
#[tokio::test]
async fn startup_rejects_a_symlink_at_the_actual_host_writer_lock_path() {
    let (dir, file, c) = fixture();
    let loaded = Loaded::read(&file).unwrap();
    service::initialize_with(&loaded, c.clone(), &service::Builtin).unwrap();
    let lock = loaded.config.data_directory.join("host.writer.lock");
    std::fs::remove_file(&lock).unwrap();
    let outside = dir.path().join("outside-lock");
    std::fs::write(&outside, b"unchanged").unwrap();
    std::os::unix::fs::symlink(&outside, &lock).unwrap();
    assert!(
        service::run_with(loaded, c, service::Builtin, std::future::pending())
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(&outside).unwrap(), b"unchanged");
}
