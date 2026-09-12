//! Separate-process simulation of the actual service loop, including storage failure.
use rx_domain::{canonical, types::*};
use rx_executor::{
    Client, Error, PeerPin, TlsEndpoint, ValidatedSnapshot,
    clock::Clock,
    engine_process::{self, EngineProcess},
    journal::{Journal, Scope},
    service::{Options, PlannerFactory, RunService},
    worker::Worker,
};
use rx_ports::*;
use rx_storage::SqliteRepository;
use serde::Deserialize;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    uri: String,
    server_name: String,
    ca: PathBuf,
    certificate: PathBuf,
    key: PathBuf,
    pin: PeerPin,
    run: Id,
    visit: Counter,
    ticks: Counter,
    output: PathBuf,
    journal: PathBuf,
    ready: PathBuf,
    trigger: PathBuf,
    cpp_image: String,
    mode: String,
    scenario: String,
    clock_file: PathBuf,
    stop_file: PathBuf,
    status_file: PathBuf,
}
struct SimClock(PathBuf);
impl Clock for SimClock {
    fn now(&self) -> std::result::Result<TimePoint, Error> {
        let value: Counter = canonical::decode_json(
            &std::fs::read(&self.0).map_err(|e| Error::Invalid(e.to_string()))?,
        )
        .map_err(|e| Error::Invalid(e.to_string()))?;
        Ok(TimePoint {
            clock_id: "test-clock".into(),
            ticks_ns: value,
        })
    }
}
struct Factory {
    image: String,
    clock: PathBuf,
    starts: Arc<AtomicUsize>,
}
#[tonic::async_trait]
impl PlannerFactory for Factory {
    async fn spawn(
        &mut self,
        view: &ValidatedSnapshot,
    ) -> std::result::Result<EngineProcess, engine_process::Error> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let mut command = tokio::process::Command::new("docker");
        command
            .args([
                "run",
                "--rm",
                "--pull=never",
                "-i",
                "--network",
                "none",
                "--read-only",
                "--cap-drop",
                "ALL",
                "--security-opt",
                "no-new-privileges",
                "-v",
            ])
            .arg(format!(
                "{}:/rx-clock:ro",
                self.clock.parent().unwrap().display()
            ))
            .args(["--entrypoint", "/build/executor/rx-bt-engine-fixture"])
            .arg(&self.image)
            .arg(format!(
                "/rx-clock/{}",
                self.clock.file_name().unwrap().to_string_lossy()
            ));
        EngineProcess::spawn_simulation(command, view).await
    }
}
struct Repo {
    inner: SqliteRepository,
    fail: Option<PathBuf>,
}
impl Repository for Repo {
    fn transact<T>(&mut self, f: impl FnOnce(&mut dyn Transaction) -> Result<T>) -> Result<T> {
        if self.fail.as_ref().is_some_and(|p| p.exists()) {
            return Err(StoreError::Unavailable(
                "injected stop journal fault".into(),
            ));
        }
        self.inner.transact(f)
    }
    fn pending_outbox_after(&mut self, a: Option<&Id>, l: usize) -> Result<Vec<OutboxRecord>> {
        self.inner.pending_outbox_after(a, l)
    }
    fn control_events_after(&mut self, a: Counter, l: usize) -> Result<Vec<StoredEvent>> {
        self.inner.control_events_after(a, l)
    }
    fn control_snapshot(&mut self) -> Result<(Counter, Vec<Record>)> {
        self.inner.control_snapshot()
    }
    fn journal_head(&mut self) -> Result<Counter> {
        self.inner.journal_head()
    }
    fn pending_outbox(&mut self, l: usize) -> Result<Vec<OutboxRecord>> {
        self.inner.pending_outbox(l)
    }
    fn snapshot(&mut self) -> Result<(Counter, Vec<Record>)> {
        self.inner.snapshot()
    }
    fn events_after(&mut self, a: Counter, l: usize) -> Result<Vec<StoredEvent>> {
        self.inner.events_after(a, l)
    }
}
#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let config: Config = canonical::decode_json(&std::fs::read(
        std::env::args().nth(1).ok_or("config required")?,
    )?)?;
    if !config.uri.starts_with("https://127.0.0.1:")
        || config.pin.clock_id != "test-clock"
        || config.ticks != Counter(1000)
        || !config.cpp_image.starts_with("sha256:")
        || config.cpp_image.len() != 71
        || !config.cpp_image[7..]
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err("explicit pinned simulation only".into());
    }
    let _ = &config.trigger;
    std::fs::create_dir(&config.output)?;
    let clock = Arc::new(SimClock(config.clock_file.clone()));
    let mut client = Client::connect(
        TlsEndpoint {
            uri: config.uri,
            server_name: config.server_name,
            ca_pem: std::fs::read(config.ca)?,
            certificate_pem: std::fs::read(config.certificate)?,
            private_key_pem: std::fs::read(config.key)?,
        },
        config.pin.clone(),
        clock.clone(),
    )
    .await?;
    let view = client.inspect_run(&config.run).await?;
    let scope = Scope {
        installation: config.pin.installation,
        store_generation: config.pin.store_generation,
        principal: config.pin.principal,
        release: config.pin.release,
        cell: config.pin.cell,
        definition: config.pin.definition,
        run: config.run,
        resolved_digest: view.recipe,
    };
    let repo = Repo {
        inner: SqliteRepository::open(&config.journal)?,
        fail: (config.scenario == "SERVICE_STORE_FAULT" && config.mode != "RECOVER")
            .then_some(config.stop_file.clone()),
    };
    let worker = Worker::new(client, Journal::open(repo, scope.clone())?)?;
    let starts = Arc::new(AtomicUsize::new(0));
    let options = Options {
        coordination: if config.scenario.starts_with("PRODUCTION_") {
            rx_executor::service::CoordinationMode::SerialProduction
        } else {
            rx_executor::service::CoordinationMode::ManualVisit
        },
        poll_ms: 25,
        communication_grace_ms: 1000,
        stop_timeout_ms: if config.scenario == "SERVICE_PAUSE_UNAVAILABLE"
            && config.mode != "RECOVER"
        {
            600
        } else {
            10000
        },
    };
    let mut service = RunService::new(
        worker,
        Factory {
            image: config.cpp_image,
            clock: config.clock_file,
            starts: starts.clone(),
        },
        clock,
        config.visit,
        options,
    )?;
    if config.scenario == "SERVICE_CRASH_STOP" && config.mode != "RECOVER" {
        let output = config.output.clone();
        service = service.with_stop_hook(Arc::new(move |control| {
            assert!(control.durability_fault.is_none());
            std::fs::write(
                output.join("boundary.json"),
                serde_json::to_vec(&control.record).unwrap(),
            )
            .unwrap();
            std::process::exit(88);
        }));
    }
    let (updates, mut status) = tokio::sync::watch::channel(service.initial_status());
    std::fs::write(&config.ready, serde_json::to_vec(&*status.borrow())?)?;
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    let stop_path = config.stop_file.clone();
    let stop_task = tokio::spawn(async move {
        while !stop_path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let _ = stop.send(true);
    });
    let status_path = config.status_file;
    let status_task = tokio::spawn(async move {
        while status.changed().await.is_ok() {
            std::fs::write(
                &status_path,
                serde_json::to_vec(&*status.borrow_and_update()).unwrap(),
            )
            .unwrap();
        }
    });
    let report = service.run(shutdown, updates).await;
    stop_task.abort();
    status_task.await?;
    let mut journal = Journal::open(SqliteRepository::open(&config.journal)?, scope)?;
    let saved = journal.stop_record()?.map(|s| s.record);
    let coordination_attempts = journal
        .test_attempts()?
        .into_iter()
        .filter(|e| {
            matches!(
                e.logical.stage,
                rx_executor::journal::Stage::BeginPart | rx_executor::journal::Stage::CompletePart
            )
        })
        .collect::<Vec<_>>();
    std::fs::write(
        config.output.join("report.json"),
        serde_json::to_vec(
            &serde_json::json!({"report":report,"saved":saved,"planner_starts":starts.load(Ordering::SeqCst),"coordination_attempts":coordination_attempts}),
        )?,
    )?;
    Ok(())
}
