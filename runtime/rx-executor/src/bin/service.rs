//! Linux run/visit service. Operator admission and part coordination remain P responsibilities.
#[cfg(target_os = "linux")]
mod linux {
    use rx_domain::{canonical, types::*};
    use rx_executor::{
        Client, PeerPin, TlsEndpoint,
        clock::{Clock, LinuxBoottime},
        engine_process::Executable,
        journal::{Journal, Scope},
        lifecycle::StopPhase,
        service::{Options, PinnedPlanner, RunService},
        worker::Worker,
    };
    use rx_storage::SqliteRepository;
    use serde::Deserialize;
    use std::{path::PathBuf, sync::Arc};
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
        journal: PathBuf,
        engine: PathBuf,
        engine_sha256: Digest,
        #[serde(default)]
        options: Options,
    }
    pub async fn run() -> Result<bool, Box<dyn std::error::Error>> {
        let path = std::env::args()
            .nth(1)
            .ok_or("configuration file required")?;
        if std::env::args().len() != 2 {
            return Err("exactly one configuration path required".into());
        }
        let mut config: Config = canonical::decode_json(&std::fs::read(path)?)?;
        let clock = Arc::new(LinuxBoottime::new()?);
        if config.pin.clock_id != clock.now()?.clock_id {
            return Err("configured clock must match the local Linux boot".into());
        }
        // Boot identity is generated once for this process, never replayed from a saved job file.
        config.pin.peer_boot = Id::new(uuid::Uuid::new_v4().to_string())?;
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
        let run = client.inspect_run(&config.run).await?;
        let scope = Scope {
            installation: config.pin.installation,
            store_generation: config.pin.store_generation,
            principal: config.pin.principal,
            release: config.pin.release,
            cell: config.pin.cell,
            definition: config.pin.definition,
            run: config.run,
            resolved_digest: run.recipe,
        };
        let worker = Worker::new(
            client,
            Journal::open(SqliteRepository::open(config.journal)?, scope)?,
        )?;
        let service = RunService::new(
            worker,
            PinnedPlanner(Executable {
                path: config.engine,
                sha256: config.engine_sha256,
            }),
            clock,
            config.visit,
            config.options,
        )?;
        let (updates, mut states) = tokio::sync::watch::channel(service.initial_status());
        let (stop, shutdown) = tokio::sync::watch::channel(false);
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        let signals = tokio::spawn(async move {
            tokio::select! {_=term.recv()=>{},_=interrupt.recv()=>{}};
            let _ = stop.send(true);
        });
        println!("{}", serde_json::to_string(&*states.borrow())?);
        let output = tokio::spawn(async move {
            while states.changed().await.is_ok() {
                if let Ok(text) = serde_json::to_string(&*states.borrow_and_update()) {
                    println!("{text}");
                }
            }
        });
        let report = service.run(shutdown, updates).await;
        signals.abort();
        output.await?;
        println!("{}", serde_json::to_string(&report)?);
        Ok(matches!(
            report.phase,
            StopPhase::PauseObserved | StopPhase::Superseded
        ) && report.durability_fault.is_none())
    }
}
#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if linux::run().await? {
        Ok(())
    } else {
        Err("executor stop needs attention; inspect persistent intent and P state".into())
    }
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("rx-executor-service requires Linux BOOTTIME; no substitute clock is used");
    std::process::exit(2);
}
