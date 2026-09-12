//! Simulation-only process fixture for the durable worker. Never shipped as a production launcher.
use rx_domain::{canonical, types::*};
use rx_executor::{
    Client, Error, PeerPin, TlsEndpoint,
    clock::Clock,
    frame::{Request, RequestKind},
    journal::{Journal, Logical, Scope, Stage},
    worker::{Outcome, TestPoint, Worker},
};
use rx_storage::SqliteRepository;
use serde::Deserialize;
use std::{path::PathBuf, sync::Arc, time::Duration};
#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Mode {
    Run,
    Pause,
    Handover,
    CrashBeforeSubmit,
    CrashAfterSubmit,
    Recover,
}
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
    mode: Mode,
    #[serde(default)]
    pause_key: Option<Id>,
}
struct FrozenClock(TimePoint);
async fn wait_marker(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    while !path.exists() {
        if tokio::time::Instant::now() >= deadline {
            return Err("handover fixture timeout".into());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Ok(())
}
fn cpp_handover(
    config: &Config,
    clock_id: &str,
    view: &rx_executor::ValidatedSnapshot,
) -> Result<Vec<Request>, Box<dyn std::error::Error>> {
    std::fs::write(
        config.output.join("handover-frame.json"),
        canonical::bytes(&view.frame(&view.context_identity())?)?,
    )?;
    let result = std::process::Command::new("docker")
        .args([
            "run",
            "--rm",
            "--pull=never",
            "--network",
            "none",
            "--read-only",
            "--cap-drop",
            "ALL",
            "--security-opt",
            "no-new-privileges",
            "-v",
        ])
        .arg(format!("{}:/fixture:ro", config.output.display()))
        .args(["--entrypoint", "/build/executor/rx-bt-request-fixture"])
        .arg(&config.cpp_image)
        .args([
            "/fixture/resolved.json",
            "/fixture/process.bt.xml",
            "/fixture/handover-frame.json",
            clock_id,
            &config.ticks.0.to_string(),
        ])
        .output()?;
    if !result.status.success() {
        return Err(format!("C++ handover: {}", String::from_utf8_lossy(&result.stderr)).into());
    }
    let requests: Vec<Request> = canonical::decode_json(&result.stdout)?;
    if requests.len() != 1 || requests[0].kind != RequestKind::RequestHandover {
        return Err("expected real BT handover request".into());
    }
    std::fs::write(config.output.join("handover-requests.json"), &result.stdout)?;
    Ok(requests)
}
impl Clock for FrozenClock {
    fn now(&self) -> Result<TimePoint, Error> {
        Ok(self.0.clone())
    }
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config: Config = canonical::decode_json(&std::fs::read(
        std::env::args().nth(1).ok_or("config required")?,
    )?)?;
    if !config.uri.starts_with("https://127.0.0.1:")
        || !(config.pin.clock_id == "test-clock"
            || config.pin.clock_id.starts_with("simulation/")
            || config.pin.clock_id.starts_with("test/"))
    {
        return Err("explicit loopback simulation only".into());
    }
    if !config.cpp_image.starts_with("sha256:")
        || config.cpp_image.len() != 71
        || !config.cpp_image[7..]
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err("pinned local C++ image required".into());
    }
    let endpoint = TlsEndpoint {
        uri: config.uri.clone(),
        server_name: config.server_name.clone(),
        ca_pem: std::fs::read(&config.ca)?,
        certificate_pem: std::fs::read(&config.certificate)?,
        private_key_pem: std::fs::read(&config.key)?,
    };
    let clock = Arc::new(FrozenClock(TimePoint {
        clock_id: config.pin.clock_id.clone(),
        ticks_ns: config.ticks,
    }));
    let pin = config.pin.clone();
    let mut client = Client::connect(endpoint, config.pin.clone(), clock).await?;
    std::fs::write(
        &config.ready,
        serde_json::to_vec(&serde_json::json!({"session":client.session_id()}))?,
    )?;
    if config.mode != Mode::Recover {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        while !config.trigger.exists() {
            if tokio::time::Instant::now() >= deadline {
                return Err("fixture assignment timeout".into());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    let snapshot = client.snapshot(&config.run, config.visit).await?;
    let scope = Scope {
        installation: pin.installation,
        store_generation: pin.store_generation,
        principal: pin.principal,
        release: pin.release,
        cell: pin.cell,
        definition: pin.definition,
        run: config.run.clone(),
        resolved_digest: snapshot.data().resolved.sha256,
    };
    let mut worker = Worker::new(
        client,
        Journal::open(SqliteRepository::open(&config.journal)?, scope)?,
    )?;
    let mode = config.mode;
    let boundary_output = config.output.clone();
    worker = worker.with_test_hook(Arc::new(move |point, entry| {
        if entry.logical.stage == Stage::SubmitOperation
            && ((mode == Mode::CrashBeforeSubmit && point == TestPoint::Entered)
                || (mode == Mode::CrashAfterSubmit && point == TestPoint::RemoteReply))
        {
            std::fs::write(
                boundary_output.join("boundary.json"),
                serde_json::to_vec(entry).expect("boundary JSON"),
            )
            .expect("boundary file");
            std::process::exit(if mode == Mode::CrashBeforeSubmit {
                73
            } else {
                74
            });
        }
    }));
    std::fs::create_dir(&config.output)?;
    let node = snapshot.process().root.id.clone();
    if config.mode == Mode::Recover {
        worker.recover(&snapshot)?;
        if snapshot.data().request_admission_allowed {
            return Err("restart unexpectedly restored admission authority".into());
        }
        let pending = worker.journal().get(&Logical {
            visit: config.visit,
            node: node.clone(),
            stage: Stage::SubmitOperation,
            control: None,
        })?;
        let activation_request = worker.journal().get(&Logical {
            visit: config.visit,
            node: node.clone(),
            stage: Stage::ResolveActivation,
            control: None,
        })?;
        let pause_request = if let Some(key) = &config.pause_key {
            worker.journal().attempt(key)?
        } else {
            None
        };
        let handover_logical = Logical {
            visit: config.visit,
            node,
            stage: Stage::ReconcileOperation,
            control: None,
        };
        let handover_request = worker.journal().get(&handover_logical)?;
        let release_observed = worker.journal().observation(&handover_logical)?.is_some();
        std::fs::write(
            config.output.join("report.json"),
            serde_json::to_vec(
                &serde_json::json!({"recovery_only":true,"work_count":snapshot.data().progress.operations.len(),"request":pending,"activation_request":activation_request,"pause_request":pause_request,"handover_request":handover_request,"release_observed":release_observed}),
            )?,
        )?;
        return Ok(());
    }
    let frame = snapshot.frame(&snapshot.context_identity())?;
    for (file, bytes) in [
        ("resolved.json", canonical::bytes(snapshot.process())?),
        ("frame.json", canonical::bytes(&frame)?),
        (
            "process.bt.xml",
            rx_process::bt_xml::generate(snapshot.process())
                .map_err(std::io::Error::other)?
                .into_bytes(),
        ),
    ] {
        std::fs::write(config.output.join(file), bytes)?;
    }
    let fixture_dir = config.output.clone();
    let image = config.cpp_image.clone();
    let clock_id = pin.clock_id.clone();
    let ticks = config.ticks.0.to_string();
    let generated = tokio::task::spawn_blocking(move || {
        std::process::Command::new("docker")
            .args([
                "run",
                "--rm",
                "--pull=never",
                "--network",
                "none",
                "--read-only",
                "--cap-drop",
                "ALL",
                "--security-opt",
                "no-new-privileges",
                "-v",
            ])
            .arg(format!("{}:/fixture:ro", fixture_dir.display()))
            .args(["--entrypoint", "/build/executor/rx-bt-request-fixture"])
            .arg(image)
            .args([
                "/fixture/resolved.json",
                "/fixture/process.bt.xml",
                "/fixture/frame.json",
                &clock_id,
                &ticks,
            ])
            .output()
    })
    .await??;
    if !generated.status.success() {
        return Err(format!(
            "C++ request producer: {}",
            String::from_utf8_lossy(&generated.stderr)
        )
        .into());
    }
    let requests: Vec<Request> = canonical::decode_json(&generated.stdout)?;
    if requests.len() != 1 || requests[0].kind != RequestKind::SubmitOperation {
        return Err("fixture expects one real BT operation request".into());
    }
    std::fs::write(config.output.join("requests.json"), &generated.stdout)?;
    let request = requests[0].clone();
    let mut result = None;
    for _ in 0..10 {
        match worker.handle(request.clone()).await {
            Ok(Outcome::Admitted(operation) | Outcome::ObservedOperation(operation)) => {
                result = Some(operation);
                break;
            }
            Ok(Outcome::RefreshRequired) => tokio::time::sleep(Duration::from_millis(20)).await,
            Err(Error::Rpc(status))
                if matches!(
                    status.code(),
                    tonic::Code::Unavailable | tonic::Code::DeadlineExceeded
                ) =>
            {
                tokio::time::sleep(Duration::from_millis(20)).await
            }
            other => return Err(format!("worker fixture did not progress: {other:?}").into()),
        }
    }
    let operation = result.ok_or("worker did not finish admission")?;
    let mut handover_request = None;
    if config.mode == Mode::Handover {
        std::fs::write(
            config.output.join("admitted.json"),
            canonical::bytes(&operation)?,
        )?;
        wait_marker(&config.output.join("handover-ready")).await?;
        let view = worker.snapshot(config.visit).await?;
        let requests = cpp_handover(&config, &pin.clock_id, &view)?;
        match worker.handle(requests[0].clone()).await {
            Ok(Outcome::ReconciliationPending(_)) => {
                if !matches!(
                    worker.handle(requests[0].clone()).await?,
                    Outcome::ReconciliationPending(_)
                ) {
                    return Err("accepted query was not pending".into());
                }
            }
            Err(Error::Rpc(status)) if status.code() == tonic::Code::Unavailable => {}
            other => return Err(format!("handover query: {other:?}").into()),
        }
        std::fs::write(config.output.join("handover-requested"), b"query attempted")?;
        wait_marker(&config.output.join("handover-released")).await?;
        if !matches!(worker.handle(requests[0].clone()).await?, Outcome::ResourcesReleased(ref op) if *op == operation)
        {
            return Err("worker failed to observe P release".into());
        }
        handover_request = worker.journal().get(&Logical {
            visit: config.visit,
            node: node.clone(),
            stage: Stage::ReconcileOperation,
            control: None,
        })?;
    }
    let mut pause_request = None;
    if config.mode == Mode::Pause {
        let view = worker.snapshot(config.visit).await?;
        let context = view.context_identity();
        let node = view.process().root.id.clone();
        std::fs::write(
            config.output.join("pause-frame.json"),
            canonical::bytes(&view.frame(&context)?)?,
        )?;
        let generated = std::process::Command::new("docker")
            .args([
                "run",
                "--rm",
                "--pull=never",
                "--network",
                "none",
                "--read-only",
                "--cap-drop",
                "ALL",
                "--security-opt",
                "no-new-privileges",
                "-v",
            ])
            .arg(format!("{}:/fixture:ro", config.output.display()))
            .args(["--entrypoint", "/build/executor/rx-bt-request-fixture"])
            .arg(&config.cpp_image)
            .args([
                "/fixture/resolved.json",
                "/fixture/process.bt.xml",
                "/fixture/pause-frame.json",
                &pin.clock_id,
                &config.ticks.0.to_string(),
                "--halt",
            ])
            .output()?;
        if !generated.status.success() {
            return Err(format!(
                "pause producer: {}",
                String::from_utf8_lossy(&generated.stderr)
            )
            .into());
        }
        let pauses: Vec<Request> = canonical::decode_json(&generated.stdout)?;
        if pauses.len() != 1 || pauses[0].kind != RequestKind::PauseExecutor {
            return Err("expected C++ pause request".into());
        }
        std::fs::write(config.output.join("pause-requests.json"), &generated.stdout)?;
        let mut paused = false;
        for _ in 0..10 {
            match worker.handle(pauses[0].clone()).await {
                Ok(Outcome::Paused { .. }) => {
                    paused = true;
                    break;
                }
                Ok(Outcome::RefreshRequired) | Err(Error::Rpc(_)) => {
                    tokio::time::sleep(Duration::from_millis(20)).await
                }
                other => return Err(format!("pause did not complete: {other:?}").into()),
            }
        }
        if !paused {
            return Err("pause retry bound".into());
        }
        pause_request = worker.journal().get(&Logical {
            visit: config.visit,
            node,
            stage: Stage::PauseRun,
            control: Some(rx_executor::journal::ControlIdentity {
                session: context.executor_session,
                epoch: context.epoch,
            }),
        })?;
    }
    let entry = worker.journal().get(&Logical {
        visit: config.visit,
        node: node.clone(),
        stage: Stage::SubmitOperation,
        control: None,
    })?;
    let activation_request = worker.journal().get(&Logical {
        visit: config.visit,
        node,
        stage: Stage::ResolveActivation,
        control: None,
    })?;
    std::fs::write(
        config.output.join("report.json"),
        serde_json::to_vec(
            &serde_json::json!({"recovery_only":false,"operation":operation,"request":entry,"activation_request":activation_request,"pause_request":pause_request,"handover_request":handover_request}),
        )?,
    )?;
    Ok(())
}
