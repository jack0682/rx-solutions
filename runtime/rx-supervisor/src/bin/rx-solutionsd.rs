//! Explicit managed mode. Only release-owned non-actuating recipes are enabled by this draft.
use rx_domain::canonical;
use rx_package::PackagePath;
use rx_solution_catalog::OwnPlatformCatalog;
use rx_storage::SqliteRepository;
use rx_supervisor::{
    Supervisor,
    builtin::release_programs,
    model::{Plan, SoftwareOnly},
    process::OsProcesses,
};
use serde::Deserialize;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    schema: String,
    state_subdirectory: PackagePath,
    plan: Plan,
}
fn read(path: &Path) -> Result<Configuration> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 1_048_576 {
        return Err("configuration file type/size".into());
    }
    let mut bytes = vec![];
    fs::File::open(path)?
        .take(1_048_577)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1_048_576 {
        return Err("configuration file too large".into());
    }
    let value: Configuration = canonical::decode_json(&bytes)?;
    if value.schema != "rx.solutions-startup.v1" {
        return Err("startup schema".into());
    }
    Ok(value)
}
fn state_directory(relative: &str) -> Result<PathBuf> {
    let mut path = PathBuf::from("/var/lib/rx-solutions");
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("state root must be a real directory".into());
    }
    for part in Path::new(relative).components() {
        let std::path::Component::Normal(part) = part else {
            return Err("state path component".into());
        };
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
            Ok(_) => return Err("state path is not an owned directory".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => fs::create_dir(&path)?,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(path)
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 2 || !matches!(args[0].as_str(), "inspect" | "run" | "activate") {
        return Err("usage: rx-solutionsd inspect|run|activate CONFIG".into());
    }
    let configuration = read(Path::new(&args[1]))?;
    let root = Path::new("/opt/rx");
    let programs = release_programs(root)?;
    let support =
        OwnPlatformCatalog::decode(&fs::read(root.join("catalogs/robotis-support.v1.json"))?)?;
    let plan_digest = configuration.plan.validate(&programs, &support)?;
    if args[0] == "inspect" {
        println!(
            "{}",
            serde_json::json!({"schema":"rx.solutions-plan-inspection.v1","plan_digest":plan_digest,"selected_profiles":configuration.plan.profiles,"control_prepared":false,"physical_qualification":"NOT_PERFORMED"})
        );
        return Ok(());
    }
    let state = state_directory(configuration.state_subdirectory.as_str())?;
    for filename in ["supervisor.db", "supervisor.writer.lock"] {
        match fs::symlink_metadata(state.join(filename)) {
            Ok(m) if !m.is_file() || m.file_type().is_symlink() => {
                return Err("invalid supervisor state file".into());
            }
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    let mut supervisor = Supervisor::open(
        SqliteRepository::open(state.join("supervisor.db"))?,
        OsProcesses::new(state.join("logs"))?,
        SoftwareOnly,
        configuration.plan,
        programs,
        &support,
    )?;
    if args[0] == "activate" {
        supervisor.rearm_software()?;
    }
    #[cfg(unix)]
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut stop = false;
    let mut stop_committed = false;
    let mut last_output = String::new();
    loop {
        if stop && !stop_committed {
            match supervisor.request_stop() {
                Ok(()) => stop_committed = true,
                Err(e) => eprintln!("stop request persistence unavailable: {e}"),
            }
        }
        match supervisor.tick() {
            Ok(report) => {
                let text = serde_json::to_string(&report)?;
                if text != last_output {
                    println!("{text}");
                    last_output = text;
                }
                if report.all_exited {
                    return Ok(());
                }
            }
            Err(e) => {
                let text = format!("supervisor attention; managed processes retained: {e}");
                if text != last_output {
                    eprintln!("{text}");
                    last_output = text;
                }
            }
        }
        #[cfg(unix)]
        tokio::select! {_=tokio::time::sleep(Duration::from_millis(50))=>{},_=term.recv()=>{stop=true;},_=tokio::signal::ctrl_c()=>{stop=true;}}
        #[cfg(not(unix))]
        tokio::select! {_=tokio::time::sleep(Duration::from_millis(50))=>{},_=tokio::signal::ctrl_c()=>{stop=true;}}
    }
}
