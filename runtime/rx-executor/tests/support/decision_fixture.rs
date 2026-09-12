//! Bounded simulation-only BT/decision worker. Never a production launcher.
use rx_domain::{canonical, types::*};
use rx_executor::{
    Client, Error, PeerPin, TlsEndpoint,
    clock::Clock,
    engine_process::{EngineProcess, State as EngineState},
    journal::{Journal, Logical, Scope, Stage},
    pending::{PendingRequests, StopState},
    worker::{Outcome, TestPoint, Worker},
};
use rx_storage::SqliteRepository;
use serde::Deserialize;
use std::{path::PathBuf, sync::Arc, time::Duration};
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
}
struct SimulationClock(PathBuf);
impl Clock for SimulationClock {
    fn now(&self) -> Result<TimePoint, Error> {
        let bytes = std::fs::read(&self.0).map_err(|e| Error::Invalid(e.to_string()))?;
        let ticks_ns: Counter =
            canonical::decode_json(&bytes).map_err(|e| Error::Invalid(e.to_string()))?;
        Ok(TimePoint {
            clock_id: "test-clock".into(),
            ticks_ns,
        })
    }
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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
        return Err("explicit pinned loopback simulation only".into());
    }
    let clock = Arc::new(SimulationClock(config.clock_file.clone()));
    let mut client = Client::connect(
        TlsEndpoint {
            uri: config.uri.clone(),
            server_name: config.server_name.clone(),
            ca_pem: std::fs::read(&config.ca)?,
            certificate_pem: std::fs::read(&config.certificate)?,
            private_key_pem: std::fs::read(&config.key)?,
        },
        config.pin.clone(),
        clock.clone(),
    )
    .await?;
    std::fs::write(
        &config.ready,
        serde_json::to_vec(&serde_json::json!({"session":client.session_id()}))?,
    )?;
    if config.mode != "RECOVER" {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
        while !config.trigger.exists() {
            if tokio::time::Instant::now() >= deadline {
                return Err("assignment timeout".into());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    let initial = client.snapshot(&config.run, config.visit).await?;
    let scope = Scope {
        installation: config.pin.installation.clone(),
        store_generation: config.pin.store_generation.clone(),
        principal: config.pin.principal.clone(),
        release: config.pin.release,
        cell: config.pin.cell.clone(),
        definition: config.pin.definition,
        run: config.run.clone(),
        resolved_digest: initial.data().resolved.sha256,
    };
    let mut worker = Worker::new(
        client,
        Journal::open(SqliteRepository::open(&config.journal)?, scope)?,
    )?;
    std::fs::create_dir(&config.output)?;
    let scenario = config.scenario.clone();
    let output = config.output.clone();
    if config.mode != "RECOVER" {
        worker = worker.with_test_hook(Arc::new(move |point, entry| {
            if entry.logical.stage.is_checkpoint()
                && ((scenario == "CHECKPOINT_CRASH_BEFORE" && point == TestPoint::Entered)
                    || (scenario == "CHECKPOINT_CRASH_AFTER" && point == TestPoint::RemoteReply))
            {
                std::fs::write(
                    output.join("boundary.json"),
                    serde_json::to_vec(entry).unwrap(),
                )
                .unwrap();
                std::process::exit(if scenario.ends_with("BEFORE") { 75 } else { 76 });
            }
        }));
    }
    let mut engine = if config.mode == "RECOVER" {
        None
    } else {
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
                config.clock_file.parent().unwrap().display()
            ))
            .args(["--entrypoint", "/build/executor/rx-bt-engine-fixture"])
            .arg(&config.cpp_image)
            .arg(format!(
                "/rx-clock/{}",
                config.clock_file.file_name().unwrap().to_string_lossy()
            ));
        Some(EngineProcess::spawn_simulation(command, &initial).await?)
    };
    let engine_pid = engine.as_ref().and_then(EngineProcess::pid);
    let mut pending = PendingRequests::new(
        initial.context_identity(),
        initial.process().root.id.clone(),
    );
    let mut total_cpp_requests = 0;
    let mut waiting = 0;
    for round in 0..32 {
        let view = worker.snapshot(config.visit).await?;
        worker.recover(&view)?;
        if config.mode == "RECOVER" {
            if view.data().request_admission_allowed {
                return Err("recovery restored authority".into());
            }
            break;
        }
        std::fs::write(
            config.output.join("resolved.json"),
            canonical::bytes(view.process())?,
        )?;
        std::fs::write(
            config.output.join("process.bt.xml"),
            rx_process::bt_xml::generate(view.process()).map_err(std::io::Error::other)?,
        )?;
        let frame_file = format!("frame-{round}.json");
        std::fs::write(
            config.output.join(&frame_file),
            canonical::bytes(&view.frame(&view.context_identity())?)?,
        )?;
        let process = engine.as_mut().ok_or("persistent engine missing")?;
        if process.pid() != engine_pid {
            return Err("planner process changed".into());
        }
        let reply = process.step(&view).await?;
        if reply.state == EngineState::Fault {
            return Err(format!("planner fault: {:?}", reply.fault).into());
        }
        std::fs::write(
            config.output.join(format!("engine-reply-{round}.json")),
            serde_json::to_vec(&reply)?,
        )?;
        total_cpp_requests += reply.requests.len();
        std::fs::write(
            config.output.join(format!("requests-{round}.json")),
            canonical::bytes(&reply.requests)?,
        )?;
        pending.accept(reply.requests, std::time::Instant::now())?;
        if !view.data().progress.operations.is_empty()
            || view.data().process_checkpoint.waits.values().any(|w| {
                matches!(
                    w,
                    rx_process_contract::frontier::WaitProgress::TimedOut { .. }
                )
            })
            || waiting >= 3
        {
            break;
        }
        if let Some(request) = pending.next(std::time::Instant::now()) {
            let outcome = worker.handle(request.clone()).await;
            if matches!(&outcome, Ok(Outcome::WaitingCondition)) {
                waiting += 1;
            }
            pending.complete(&request, outcome, std::time::Instant::now())?;
        }
        if pending.stop_state() != StopState::Running {
            return Err("planner requested attention before expected result".into());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let mut pause_observed = false;
    if let Some(process) = engine.take() {
        let closed = process.close().await?;
        pending.accept(closed.requests, std::time::Instant::now())?;
        for _ in 0..10 {
            if let Some(request) = pending.next(std::time::Instant::now()) {
                let outcome = worker.handle(request.clone()).await;
                pending.complete(&request, outcome, std::time::Instant::now())?;
            }
            if pending.stop_state() == StopState::PauseObserved {
                pause_observed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if !pause_observed {
            return Err("planner close did not observe P pause".into());
        }
    }
    let view = worker.snapshot(config.visit).await?;
    worker.recover(&view)?;
    let mut entries = vec![];
    let mut observations = vec![];
    for node in rx_process_contract::validation::nodes(view.process()) {
        for stage in [
            Stage::CheckpointBranch,
            Stage::CheckpointStartWait,
            Stage::CheckpointCheckWait,
            Stage::ResolveActivation,
            Stage::SubmitOperation,
        ] {
            let logical = Logical {
                node: node.id.clone(),
                visit: config.visit,
                stage,
                control: None,
            };
            if let Some(value) = worker.journal().get(&logical)? {
                entries.push(value);
            }
            if let Some(value) = worker.journal().observation(&logical)? {
                observations.push(value);
            }
        }
    }
    let attempts = worker.journal().test_attempts()?;
    std::fs::write(
        config.output.join("report.json"),
        serde_json::to_vec(
            &serde_json::json!({"work_count":view.data().progress.operations.len(),"admission":view.data().request_admission_allowed,"process_checkpoint":view.data().process_checkpoint,"entries":entries,"observations":observations,"attempts":attempts,"waiting":waiting,"engine_pid":engine_pid,"cpp_requests":total_cpp_requests,"pause_observed":pause_observed}),
        )?,
    )?;
    Ok(())
}
