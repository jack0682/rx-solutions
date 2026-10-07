//! Explicit installed-daemon path. Preparation and network delivery do not create a second manager.
use super::*;
use crate::{
    model::Phase, process::OsProcesses, registered::RegisteredSupervisor, registration::Registry,
};
use rx_package::PackagePath;
use rx_storage::SqliteRepository;
use serde::Deserialize;
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    schema: String,
    state_subdirectory: PackagePath,
    #[serde(default)]
    connection: Option<PathBuf>,
    #[serde(default)]
    assignment: Option<Id>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectionFile {
    schema: String,
    endpoint: String,
    server_name: String,
    ca: PathBuf,
    certificate: PathBuf,
    private_key: PathBuf,
    principal: Name,
    installation: Id,
    store_generation: Id,
    shared_clock_id: String,
    release_digest: Digest,
}
fn read(path: &Path) -> Result<Vec<u8>> {
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 1_048_576 {
        return Err(invalid("execution configuration file type/size"));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(1_048_577)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1_048_576 {
        return Err(invalid("execution configuration too large"));
    }
    Ok(bytes)
}
fn connection(path: &Path) -> Result<Connection> {
    let v: ConnectionFile = canonical::decode_json(&read(path)?).map_err(invalid)?;
    if v.schema != "rx.resident-execution-connection.v1" {
        return Err(invalid("execution connection schema"));
    }
    Ok(Connection {
        endpoint: v.endpoint,
        server_name: v.server_name,
        ca_pem: read(&v.ca)?,
        certificate_pem: read(&v.certificate)?,
        private_key_pem: read(&v.private_key)?,
        principal: v.principal,
        installation: v.installation,
        store_generation: v.store_generation,
        shared_clock_id: v.shared_clock_id,
        release_digest: v.release_digest,
    })
}
fn directory(path: &Path, create: bool) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && create => {
            Ok(std::fs::create_dir(path)?)
        }
        Err(e) => Err(e.into()),
        _ => Err(invalid("managed execution directory type")),
    }
}
fn data_root(relative: &PackagePath) -> Result<PathBuf> {
    let mut path = PathBuf::from("/var/lib/rx-solutions");
    directory(&path, false)?;
    for item in Path::new(relative.as_str()).components() {
        let std::path::Component::Normal(item) = item else {
            return Err(invalid("execution state path"));
        };
        path.push(item);
        directory(&path, true)?;
    }
    Ok(path)
}
fn database(path: &Path) -> Result<SqliteRepository> {
    for file in [path.to_path_buf(), path.with_extension("writer.lock")] {
        match std::fs::symlink_metadata(&file) {
            Ok(m) if !m.is_file() || m.file_type().is_symlink() => {
                return Err(invalid("managed execution state file type"));
            }
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    Ok(SqliteRepository::open(path)?)
}
/// `catalog` prints verified authoring data; `platform-run` uses an owner-approved P assignment.
pub async fn execute(mode: &str, path: &Path) -> Result<()> {
    let c: Configuration = canonical::decode_json(&read(path)?).map_err(invalid)?;
    if c.schema != "rx.resident-execution-runtime.v1" {
        return Err(invalid("execution runtime schema"));
    }
    let state = data_root(&c.state_subdirectory)?;
    let mut release_store = database(Path::new("/var/lib/rx-solutions/release.db"))?;
    let catalog = Catalog::load(Path::new("/opt/rx"), &mut release_store)?;
    release_store.close()?;
    let mut registry = Registry::new(database(&state.join("registration.db"))?);
    if mode == "catalog" {
        println!(
            "{}",
            serde_json::json!({"schema":"rx.resident-execution-catalog.v1","enrollment":catalog.enrollment(registry.platform_binding()?)?,"basis":"ACTUAL_RELEASE_VERIFIED_AT_CHECKPOINT; NOT_EXECUTION_PERMISSION"})
        );
        registry.into_repository().close()?;
        return Ok(());
    }
    if !matches!(mode, "platform-run" | "platform-investigate") {
        return Err(invalid("unknown managed execution mode"));
    }
    let assignment = c
        .assignment
        .ok_or_else(|| invalid("managed execution requires an assignment ID"))?;
    let connection_path = c
        .connection
        .as_deref()
        .ok_or_else(|| invalid("managed execution requires its execution connection"))?;
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new()?);
    let mut client = Client::connect(connection(connection_path)?, clock, &registry).await?;
    if mode == "platform-investigate" {
        let inspection = client
            .investigate(&assignment, catalog, &mut registry)
            .await?;
        println!("{}", serde_json::to_string(&inspection).map_err(invalid)?);
        registry.into_repository().close()?;
        return Ok(());
    }
    let prepared = client.prepare(&assignment, catalog, &mut registry).await?;
    println!(
        "{}",
        serde_json::json!({"schema":"rx.resident-execution-prepared.v1","peer":client.peer(),"preparation":prepared.offer(),"state":"AWAITING_OWNER_APPROVAL"})
    );
    let grant = loop {
        let view = client.inspect(&assignment).await?;
        match view.assignment.phase {
            data::Phase::Granted => break client.take_grant(&prepared).await?,
            data::Phase::Proposed | data::Phase::Prepared if !view.assignment.stop_requested => {}
            _ => {
                return Err(invalid(
                    "assignment cancelled, stopped or requires reconciliation before start",
                ));
            }
        }
        tokio::select! {_=tokio::time::sleep(Duration::from_millis(200))=>{},_=tokio::signal::ctrl_c()=>return Err(invalid("preparation interrupted; original P assignment retained"))}
    };
    let lease = grant.lease.clone();
    let runs = state.join("platform-runs");
    directory(&runs, true)?;
    let run = runs.join(assignment.as_str());
    directory(&run, true)?;
    let store = database(&run.join("supervisor.db"))?;
    let backend = OsProcesses::new(run.join("logs"))?;
    let mut supervisor =
        RegisteredSupervisor::open_platform(store, backend, prepared, grant, registry)?;
    let delivery = Delivery::start(client, lease.clone(), run.join("delivery.db"))?;
    #[cfg(unix)]
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut stop = false;
    let mut stop_committed = false;
    let mut last = String::new();
    loop {
        stop |= delivery.status().stop_requested;
        if !lease.start_window() {
            match supervisor.state() {
                Ok(state)
                    if state
                        .records
                        .values()
                        .any(|r| matches!(r.phase, Phase::Pending | Phase::Prepared)) =>
                {
                    stop = true
                }
                Err(error) => {
                    eprintln!(
                        "execution state unavailable; retain local manager and request stop: {error}"
                    );
                    stop = true;
                }
                _ => {}
            }
        }
        if stop && !stop_committed {
            match supervisor.request_stop() {
                Ok(()) => stop_committed = true,
                Err(e) => eprintln!("stop persistence unavailable: {e}"),
            }
        }
        match supervisor.tick() {
            Ok(report) => {
                let observed = match supervisor.platform_observation() {
                    Ok(value) => {
                        if let Err(e) = delivery.observe(value.clone()) {
                            eprintln!(
                                "execution delivery attention; local management continues: {e}"
                            );
                        }
                        Some(value)
                    }
                    Err(e) => {
                        eprintln!(
                            "execution observation unavailable; local management continues: {e}"
                        );
                        None
                    }
                };
                let text=serde_json::json!({"schema":"rx.resident-execution-runtime-status.v1","assignment":assignment,"supervisor":report,"delivery":delivery.status(),"work_use_permission":"NOT_GRANTED_BY_PROCESS_EXECUTION"}).to_string();
                if text != last {
                    println!("{text}");
                    last = text;
                }
                if report.all_exited {
                    let observed=observed.ok_or_else(||invalid("owned children stopped but the original observation needs reconciliation"))?;
                    let delivered = delivery.finish(observed).await;
                    println!(
                        "{}",
                        serde_json::json!({"schema":"rx.resident-execution-stopped.v1","assignment":assignment,"delivery":delivered,"source_stopped":true,"reconciliation_required":report.reconciliation_required,"work_use_permission":"NOT_GRANTED_BY_PROCESS_EXECUTION"})
                    );
                    if report.reconciliation_required {
                        return Err(invalid(
                            "processes stopped; residuals require explicit reconciliation",
                        ));
                    }
                    return Ok(());
                }
            }
            Err(e) => {
                let text = format!("managed execution attention; owned processes retained: {e}");
                if text != last {
                    eprintln!("{text}");
                    last = text;
                }
            }
        }
        #[cfg(unix)]
        tokio::select! {_=tokio::time::sleep(Duration::from_millis(50))=>{},_=term.recv()=>stop=true,_=tokio::signal::ctrl_c()=>stop=true}
        #[cfg(not(unix))]
        tokio::select! {_=tokio::time::sleep(Duration::from_millis(50))=>{},_=tokio::signal::ctrl_c()=>stop=true}
    }
}
