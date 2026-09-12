//! Separate-process integration fixture for the actual resident service, with simulated P time.
//! The release CLI uses LinuxBoottime; this fixture is never installed in a product image.
use rx_domain::{canonical, types::*};
use rx_executor::{
    Client, Error, PeerPin, TlsEndpoint, ValidatedSnapshot,
    assignment_journal::{AssignmentJournal, Identity, ServiceScope},
    cell_service::CellService,
    clock::Clock,
    engine_process::{self, EngineProcess},
    service::{CoordinationMode, Options, PlannerFactory},
    service_owner::ServiceOwner,
};
use rx_ports::Repository;
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
    service_journal: Id,
    root: PathBuf,
    clock_file: PathBuf,
    cpp_image: String,
    ready: PathBuf,
    stop_file: PathBuf,
    status_file: PathBuf,
    output: PathBuf,
    initialize: bool,
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

fn publish(
    path: &std::path::Path,
    value: &impl serde::Serialize,
) -> Result<(), Box<dyn std::error::Error>> {
    let temporary = path.with_extension("writing");
    std::fs::write(&temporary, serde_json::to_vec(value)?)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config: Config = canonical::decode_json(&std::fs::read(
        std::env::args().nth(1).ok_or("config required")?,
    )?)?;
    std::fs::create_dir_all(&config.output)?;
    let journal_path = config.root.join("assignment.sqlite3");
    let _owner = ServiceOwner::acquire(&journal_path)?;
    let identity = Identity {
        journal: config.service_journal,
        scope: ServiceScope {
            installation: config.pin.installation.clone(),
            store_generation: config.pin.store_generation.clone(),
            principal: config.pin.principal.clone(),
            release: config.pin.release,
            cell: config.pin.cell.clone(),
            definition: config.pin.definition,
        },
    };
    if config.initialize {
        drop(AssignmentJournal::initialize_file(
            &journal_path,
            identity.clone(),
        )?);
    }
    let journal = AssignmentJournal::open_file_required(&journal_path, identity.clone())?;
    let clock = Arc::new(SimClock(config.clock_file.clone()));
    let client = Client::connect(
        TlsEndpoint {
            uri: config.uri,
            server_name: config.server_name,
            ca_pem: std::fs::read(config.ca)?,
            certificate_pem: std::fs::read(config.certificate)?,
            private_key_pem: std::fs::read(config.key)?,
        },
        config.pin,
        clock.clone(),
    )
    .await?;
    let starts = Arc::new(AtomicUsize::new(0));
    let service = CellService::new(
        client,
        journal,
        config.root.clone(),
        Factory {
            image: config.cpp_image,
            clock: config.clock_file,
            starts: starts.clone(),
        },
        clock,
        Options {
            coordination: CoordinationMode::SerialProduction,
            poll_ms: 25,
            communication_grace_ms: 1000,
            stop_timeout_ms: 10000,
        },
    )?;
    let (updates, mut states) = tokio::sync::watch::channel(service.initial_status());
    let initial = serde_json::json!({"status":&*states.borrow(),"planner_starts":starts.load(Ordering::SeqCst)});
    publish(&config.ready, &initial)?;
    publish(&config.status_file, &initial)?;
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    let stop_file = config.stop_file;
    let stop_task = tokio::spawn(async move {
        while !stop_file.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let _ = stop.send(true);
    });
    let status_file = config.status_file;
    let observed_starts = starts.clone();
    let statuses = tokio::spawn(async move {
        while states.changed().await.is_ok() {
            publish(&status_file,&serde_json::json!({"status":&*states.borrow_and_update(),"planner_starts":observed_starts.load(Ordering::SeqCst)})).unwrap();
        }
    });
    let report = service.run(shutdown, updates).await;
    println!("{}", serde_json::to_string(&report)?);
    stop_task.abort();
    statuses.await?;
    let mut journal = AssignmentJournal::open_file_required(&journal_path, identity.clone())?;
    let current = journal.current()?.map(|v| v.value);
    let mut repository = journal.into_repository();
    let attachments = repository.transact(|tx| tx.scan("attachment/run/"))?;
    let mut journal = AssignmentJournal::open_required(repository, identity)?;
    let mut stops = Vec::new();
    for row in &attachments {
        let attachment: rx_executor::assignment_journal::Attachment =
            serde_json::from_value(row.document.value.clone())?;
        let run = journal.open_run_file_required(&config.root, &attachment.preparation.id)?;
        let mut repository = run.into_repository();
        let stop =
            repository.transact(|tx| tx.get(&Name::new("executor-stop/current").unwrap()))?;
        stops.push(serde_json::json!({"run":attachment.preparation.scope.run,"stop":stop}));
    }
    publish(
        &config.output.join("report.json"),
        &serde_json::json!({"report":report,"planner_starts":starts.load(Ordering::SeqCst),"current":current,"attachments":attachments,"stops":stops}),
    )?;
    Ok(())
}
