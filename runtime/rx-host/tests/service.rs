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

#[test]
fn binding_inspection_compares_full_cohort_without_creating_installation_or_restoring_authority() {
    use rx_process_contract::host_binding_plan::{HostTarget, Plan};
    let (_dir, file, _) = fixture();
    let current = Loaded::read(&file).unwrap();
    let mut proposed = Loaded::read(&file).unwrap();
    let b = &current.bindings[0];
    let plan = Plan {
        schema: name("rx.host-binding-plan.v1"),
        installation: current.config.installation.clone(),
        cell: b.cell.clone(),
        device_context_digest: Digest::from_bytes([11; 32]),
        process_review_digest: Digest::from_bytes([12; 32]),
        before_configuration: b.definition.clone(),
        after_configuration: b.definition.clone(),
        definition: b.definition.clone(),
        envelope: b.envelope.clone(),
        environment: name("SIMULATION"),
        scopes: b.scope_ids.iter().cloned().collect(),
        hosts: [(
            b.host.clone(),
            HostTarget {
                required_intents: b.allowed_intents.clone(),
                required_conditions: b.condition_ids.iter().cloned().collect(),
                device_packages: vec![],
                other_affected_cells: Default::default(),
            },
        )]
        .into(),
    };
    let checked = service::binding_change::inspect(&plan, &current, &proposed).unwrap();
    assert!(checked.software_matches);
    assert!(!checked.activation_authorized && !checked.installation_changed);
    assert_eq!(checked.native_processes_started, Counter(0));
    assert!(!current.config.data_directory.exists());
    proposed.bindings[0].qualification_revision = Counter(2);
    assert!(
        !service::binding_change::inspect(&plan, &current, &proposed)
            .unwrap()
            .software_matches
    );
    proposed.bindings[0] = b.clone();
    proposed.bindings[0]
        .allowed_intents
        .push(b.allowed_intents[0].clone());
    assert!(
        !service::binding_change::inspect(&plan, &current, &proposed)
            .unwrap()
            .software_matches
    );
    proposed.bindings[0] = b.clone();
    proposed.config.data_directory = current.config.runtime_directory.join("other");
    assert!(
        !service::binding_change::inspect(&plan, &current, &proposed)
            .unwrap()
            .software_matches
    );
    proposed.config = current.config.clone();
    let mut partial = plan.clone();
    partial
        .hosts
        .get_mut(&b.host)
        .unwrap()
        .other_affected_cells
        .insert(name("cell/missing"));
    assert!(
        !service::binding_change::inspect(&partial, &current, &proposed)
            .unwrap()
            .software_matches
    );
    let mut wrong = plan;
    wrong.installation = id();
    assert!(service::binding_change::inspect(&wrong, &current, &proposed).is_err());
}

fn maintenance_plan(loaded: &Loaded) -> rx_process_contract::host_binding_plan::Plan {
    use rx_process_contract::host_binding_plan::*;
    let b = &loaded.bindings[0];
    Plan {
        schema: name("rx.host-binding-plan.v1"),
        installation: loaded.config.installation.clone(),
        cell: b.cell.clone(),
        device_context_digest: Digest::from_bytes([23; 32]),
        process_review_digest: Digest::from_bytes([24; 32]),
        before_configuration: b.definition.clone(),
        after_configuration: b.definition.clone(),
        definition: b.definition.clone(),
        envelope: b.envelope.clone(),
        environment: name("SIMULATION"),
        scopes: b.scope_ids.iter().cloned().collect(),
        hosts: [(
            b.host.clone(),
            HostTarget {
                required_intents: b.allowed_intents.clone(),
                required_conditions: b.condition_ids.iter().cloned().collect(),
                device_packages: vec![],
                other_affected_cells: Default::default(),
            },
        )]
        .into(),
    }
}
#[tokio::test]
async fn durable_maintenance_requires_real_stop_recovers_same_request_and_blocks_startup_until_cancelled()
 {
    use service::maintenance as m;
    let (_dir, file, clock) = fixture();
    let loaded = Loaded::read(&file).unwrap();
    let plan = maintenance_plan(&loaded);
    service::initialize_with(&loaded, clock.clone(), &service::Builtin).unwrap();
    let request = id();
    assert!(m::prepare(&plan, &loaded, &loaded, &request).is_err());
    let (stop, signal) = tokio::sync::oneshot::channel();
    let c = clock.clone();
    let running = Loaded::read(&file).unwrap();
    let task = tokio::spawn(service::run_with(running, c, service::Builtin, async {
        let _ = signal.await;
    }));
    status(
        &loaded.config.runtime_directory.join("host-status.json"),
        "SOFTWARE_READY_UNARMED",
    )
    .await;
    assert!(m::prepare(&plan, &loaded, &loaded, &request).is_err());
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    let prepared = m::prepare(&plan, &loaded, &loaded, &request).unwrap();
    assert_eq!(prepared.state, m::State::Prepared);
    assert!(!prepared.installation_changed && !prepared.activation_authorized);
    assert_eq!(
        canonical::bytes(&prepared).unwrap(),
        canonical::bytes(&m::prepare(&plan, &loaded, &loaded, &request).unwrap()).unwrap()
    );
    assert_eq!(
        m::lookup(&loaded, &request)
            .unwrap()
            .unwrap()
            .request_digest,
        prepared.request_digest
    );
    assert!(m::prepare(&plan, &loaded, &loaded, &id()).is_err());
    let mut changed = plan.clone();
    changed.process_review_digest = Digest::from_bytes([25; 32]);
    assert!(m::prepare(&changed, &loaded, &loaded, &request).is_err());
    assert!(
        service::run_with(
            Loaded::read(&file).unwrap(),
            clock.clone(),
            service::Builtin,
            async {}
        )
        .await
        .is_err()
    );
    assert_eq!(
        m::cancel(&loaded, &request).unwrap().state,
        m::State::Cancelled
    );
    assert_eq!(
        m::cancel(&loaded, &request).unwrap().state,
        m::State::Cancelled
    );
    assert_eq!(
        m::prepare(&plan, &loaded, &loaded, &request).unwrap().state,
        m::State::Cancelled
    );
    service::run_with(
        Loaded::read(&file).unwrap(),
        clock,
        service::Builtin,
        async {},
    )
    .await
    .unwrap();
    let next = m::prepare(&plan, &loaded, &loaded, &id()).unwrap();
    assert_ne!(next.stop.attempt, prepared.stop.attempt);
    assert_eq!(
        next.stop.journal_tail, prepared.stop.journal_tail,
        "maintenance history is not native evidence"
    );
}
#[tokio::test]
async fn failed_startup_invalidates_old_stop_even_when_old_status_file_still_says_stopped() {
    use service::maintenance as m;
    let (_dir, file, clock) = fixture();
    let loaded = Loaded::read(&file).unwrap();
    let plan = maintenance_plan(&loaded);
    service::initialize_with(&loaded, clock.clone(), &service::Builtin).unwrap();
    service::run_with(
        Loaded::read(&file).unwrap(),
        clock.clone(),
        service::Builtin,
        async {},
    )
    .await
    .unwrap();
    let status_file = loaded.config.runtime_directory.join("host-status.json");
    let old_status = std::fs::read(&status_file).unwrap();
    let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut failing = Loaded::read(&file).unwrap();
    failing.config.bind = occupied.local_addr().unwrap();
    assert!(
        service::run_with(failing, clock.clone(), service::Builtin, async {})
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(&status_file).unwrap(), old_status);
    assert!(m::prepare(&plan, &loaded, &loaded, &id()).is_err());
    drop(occupied);
    service::run_with(
        Loaded::read(&file).unwrap(),
        clock,
        service::Builtin,
        async {},
    )
    .await
    .unwrap();
    assert!(m::prepare(&plan, &loaded, &loaded, &id()).is_ok());
}
#[tokio::test]
async fn operational_state_changes_after_stop_invalidate_preparation_without_reinitializing_the_journal()
 {
    use rx_ports::{Document, Repository};
    use service::maintenance as m;
    let (_dir, file, clock) = fixture();
    let loaded = Loaded::read(&file).unwrap();
    let plan = maintenance_plan(&loaded);
    service::initialize_with(&loaded, clock.clone(), &service::Builtin).unwrap();
    service::run_with(
        Loaded::read(&file).unwrap(),
        clock,
        service::Builtin,
        async {},
    )
    .await
    .unwrap();
    let mut store =
        rx_storage::SqliteRepository::open(loaded.config.data_directory.join("host.db")).unwrap();
    store
        .transact(|tx| {
            tx.put(
                &name("external/change"),
                None,
                &Document {
                    schema: name("test.change.v1"),
                    value: serde_json::json!({"changed":true}),
                },
            )?;
            Ok(())
        })
        .unwrap();
    drop(store);
    assert!(m::prepare(&plan, &loaded, &loaded, &id()).is_err());
    assert!(m::lookup(&loaded, &id()).unwrap().is_none());
}

#[cfg(feature = "test-harness")]
#[test]
#[ignore = "abrupt-exit helper invoked only by the maintenance crash test"]
fn maintenance_crash_child() {
    let file = std::env::var("RX_MAINTENANCE_TEST_CONFIG").unwrap();
    let loaded = Loaded::read(Path::new(&file)).unwrap();
    let plan = maintenance_plan(&loaded);
    let request = Id::new(std::env::var("RX_MAINTENANCE_TEST_REQUEST").unwrap()).unwrap();
    let point = std::env::var("RX_MAINTENANCE_TEST_POINT").unwrap();
    service::maintenance::prepare_with_boundary(
        &plan,
        &loaded,
        &loaded,
        &request,
        || {
            if point == "before" {
                std::process::exit(71)
            }
            Ok(())
        },
        || {
            if point == "after" {
                std::process::exit(72)
            }
        },
    )
    .unwrap();
}
#[cfg(feature = "test-harness")]
#[tokio::test]
async fn preparation_commit_crashes_preserve_atomicity_and_durable_same_request_recovery() {
    use service::maintenance as m;
    for point in ["before", "after"] {
        let (_dir, file, clock) = fixture();
        let loaded = Loaded::read(&file).unwrap();
        service::initialize_with(&loaded, clock.clone(), &service::Builtin).unwrap();
        service::run_with(
            Loaded::read(&file).unwrap(),
            clock,
            service::Builtin,
            async {},
        )
        .await
        .unwrap();
        let request = id();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "maintenance_crash_child"])
            .env("RX_MAINTENANCE_TEST_CONFIG", &file)
            .env("RX_MAINTENANCE_TEST_REQUEST", request.as_str())
            .env("RX_MAINTENANCE_TEST_POINT", point)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(if point == "before" { 71 } else { 72 }));
        assert_eq!(
            m::lookup(&loaded, &request).unwrap().is_some(),
            point == "after"
        );
        let plan = maintenance_plan(&loaded);
        let recovered = m::prepare(&plan, &loaded, &loaded, &request).unwrap();
        assert_eq!(recovered.state, m::State::Prepared);
        assert_eq!(
            m::lookup(&loaded, &request)
                .unwrap()
                .unwrap()
                .request_digest,
            recovered.request_digest
        );
        assert_eq!(
            m::cancel(&loaded, &request).unwrap().state,
            m::State::Cancelled
        );
    }
}
#[tokio::test]
async fn maintenance_requires_descriptor_feature_marker_and_does_not_silently_upgrade_legacy_installations()
 {
    let (_dir, file, clock) = fixture();
    let loaded = Loaded::read(&file).unwrap();
    service::initialize_with(&loaded, clock.clone(), &service::Builtin).unwrap();
    let descriptor = loaded.config.data_directory.join("installation.json");
    let bytes = std::fs::read(&descriptor).unwrap();
    let mut value: serde_json::Value = canonical::decode_json(&bytes).unwrap();
    assert_eq!(value["maintenance_protocol"], "rx.host-maintenance.v1");
    value
        .as_object_mut()
        .unwrap()
        .remove("maintenance_protocol");
    std::fs::write(&descriptor, canonical::bytes(&value).unwrap()).unwrap();
    assert!(
        service::run_with(
            Loaded::read(&file).unwrap(),
            clock.clone(),
            service::Builtin,
            async {}
        )
        .await
        .is_err()
    );
    value["schema"] = serde_json::json!("rx.host-installation.v1");
    std::fs::write(&descriptor, canonical::bytes(&value).unwrap()).unwrap();
    service::run_with(
        Loaded::read(&file).unwrap(),
        clock,
        service::Builtin,
        async {},
    )
    .await
    .unwrap();
    assert!(
        service::maintenance::prepare(&maintenance_plan(&loaded), &loaded, &loaded, &id()).is_err()
    );
    assert!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(&descriptor).unwrap())
            .unwrap()
            .get("maintenance_protocol")
            .is_none()
    );
}

#[tokio::test]
async fn preparation_cannot_swap_both_current_and_proposed_around_the_actual_stopped_startup() {
    let (_dir, file, clock) = fixture();
    let loaded = Loaded::read(&file).unwrap();
    service::initialize_with(&loaded, clock.clone(), &service::Builtin).unwrap();
    service::run_with(
        Loaded::read(&file).unwrap(),
        clock,
        service::Builtin,
        async {},
    )
    .await
    .unwrap();
    let plan = maintenance_plan(&loaded);
    for field in ["release", "bind", "drain"] {
        let mut swapped = Loaded::read(&file).unwrap();
        match field {
            "release" => swapped.config.release_digest = Digest::from_bytes([88; 32]),
            "bind" => swapped.config.bind = "127.0.0.1:17777".parse().unwrap(),
            _ => swapped.config.publication_drain_ms = Counter(9123),
        }
        assert_eq!(
            swapped.identity, loaded.identity,
            "old installation identity intentionally excludes these settings"
        );
        assert!(
            service::binding_change::inspect(&plan, &swapped, &swapped)
                .unwrap()
                .software_matches
        );
        let request = id();
        assert!(service::maintenance::prepare(&plan, &swapped, &swapped, &request).is_err());
        assert!(
            service::maintenance::lookup(&loaded, &request)
                .unwrap()
                .is_none()
        );
    }
    assert!(service::maintenance::prepare(&plan, &loaded, &loaded, &id()).is_ok());
}
