//! Actual S client/worker/service against a loopback P SIMULATION fixture.
use rx_domain::{canonical, types::*};
use rx_executor::{
    Client, Error, PeerPin, TlsEndpoint, ValidatedSnapshot,
    clock::Clock,
    journal::{Journal, Scope},
    service::{CoordinationMode, Options, PlannerFactory, RunService},
    worker::Worker,
};
use serde::Deserialize;
use std::{path::PathBuf, sync::Arc};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    uri: String,
    ca: PathBuf,
    certificate: PathBuf,
    key: PathBuf,
    pin: PeerPin,
    run: Id,
    plan: Digest,
    ticks: Counter,
    ready: PathBuf,
    output: PathBuf,
    journal: PathBuf,
}
struct FrozenClock(TimePoint);
impl Clock for FrozenClock {
    fn now(&self) -> Result<TimePoint, Error> {
        Ok(self.0.clone())
    }
}
struct NoV1Planner;
#[tonic::async_trait]
impl PlannerFactory for NoV1Planner {
    async fn spawn(
        &mut self,
        _: &ValidatedSnapshot,
    ) -> Result<rx_executor::engine_process::EngineProcess, rx_executor::engine_process::Error>
    {
        Err(rx_executor::engine_process::Error::Invalid(
            "v1 planner reached by v2".into(),
        ))
    }
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config: Config = canonical::decode_json(&std::fs::read(
        std::env::args().nth(1).ok_or("config required")?,
    )?)?;
    if !config.uri.starts_with("https://127.0.0.1:") || !config.pin.clock_id.starts_with("test/") {
        return Err("loopback SIMULATION fixture only".into());
    }
    let clock = Arc::new(FrozenClock(TimePoint {
        clock_id: config.pin.clock_id.clone(),
        ticks_ns: config.ticks,
    }));
    let scope = Scope {
        installation: config.pin.installation.clone(),
        store_generation: config.pin.store_generation.clone(),
        principal: config.pin.principal.clone(),
        release: config.pin.release,
        cell: config.pin.cell.clone(),
        definition: config.pin.definition,
        run: config.run,
        resolved_digest: config.plan,
    };
    let client = Client::connect(
        TlsEndpoint {
            uri: config.uri,
            server_name: "localhost".into(),
            ca_pem: std::fs::read(config.ca)?,
            certificate_pem: std::fs::read(config.certificate)?,
            private_key_pem: std::fs::read(config.key)?,
        },
        config.pin,
        clock.clone(),
    )
    .await?;
    let journal = Journal::open(rx_storage::SqliteRepository::open(config.journal)?, scope)?;
    let mut worker = Worker::new(client, journal)?;
    worker.negotiate_execution().await?;
    std::fs::write(config.ready, worker.session_id().as_str())?;
    let service = RunService::new(
        worker,
        NoV1Planner,
        clock,
        Counter(1),
        Options {
            coordination: CoordinationMode::SerialExecutionV2,
            poll_ms: 10,
            ..Default::default()
        },
    )?;
    let (updates, status) = tokio::sync::watch::channel(service.initial_status());
    let (_stop, shutdown) = tokio::sync::watch::channel(false);
    let mut result = match tokio::time::timeout(
        std::time::Duration::from_secs(40),
        service.run_owned(shutdown, updates),
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            std::fs::write(
                config.output.with_extension("timeout.json"),
                serde_json::to_vec_pretty(&*status.borrow())?,
            )?;
            return Err(error.into());
        }
    };
    let attempts = result.worker.journal().test_attempts()?;
    std::fs::write(
        config.output,
        serde_json::to_vec_pretty(
            &serde_json::json!({"report":result.report,"attempts":attempts}),
        )?,
    )?;
    Ok(())
}
