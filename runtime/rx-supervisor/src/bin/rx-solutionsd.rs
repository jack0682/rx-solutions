//! Explicit management of release-owned diagnostics and protocol-guarded RX services.
use rx_domain::canonical;
use rx_package::PackagePath;
use rx_solution_catalog::DeviceCatalog;
use rx_storage::SqliteRepository;
use rx_supervisor::{
    builtin::{
        ServiceConfigurations, preflight_source_assets, programs_from_release, release_boundary,
        release_metadata, services_from_release, validate_service_plan, verify_release,
    },
    execution_store, initialization,
    model::{GuardedServices, Plan},
    process::OsProcesses,
    registered::RegisteredSupervisor,
    registration::{
        Disposition, DispositionKind, DispositionRequest, RecoveryAuthority, Registry,
        ResumeRequest,
    },
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
    #[serde(default)]
    services: Option<ServiceConfigurations>,
    #[serde(default)]
    operating_area_mailbox: Option<PathBuf>,
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
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
    Ok(canonical::decode_json(&bytes)?)
}
fn read(path: &Path) -> Result<Configuration> {
    let value: Configuration = read_json(path)?;
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
// The local invocation is the explicit software recovery decision, not physical
// authority. Kernel evidence and catalog checks remain mandatory downstream.
struct ExplicitLocalResume;
impl RecoveryAuthority for ExplicitLocalResume {
    fn may_dispose(&self, r: &DispositionRequest) -> bool {
        r.kind == DispositionKind::ConfirmedClosure
    }
    fn may_resume(&self, d: &Disposition, _: &ResumeRequest) -> bool {
        d.kind == DispositionKind::ConfirmedClosure
    }
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    if let Err(error) = run().await {
        if let Some(refusal) = rx_service_status::storage_ownership_refusal(error.as_ref()) {
            eprintln!("{}", serde_json::to_string(&refusal)?);
            return Err(refusal.condition.into());
        }
        let release_error = error
            .downcast_ref::<rx_package::release::Error>()
            .or_else(|| match error.downcast_ref::<rx_supervisor::Error>() {
                Some(rx_supervisor::Error::Release(release)) => Some(release),
                _ => None,
            });
        if let Some(release) = release_error {
            eprintln!(
                "{}",
                serde_json::json!({"schema":"rx.release-refusal.v1", "condition":release.condition(), "detail":release.to_string(), "current_permission":"NOT_GRANTED"})
            );
            return Err(release.condition().into());
        }
        return Err(error);
    }
    Ok(())
}
async fn run() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if !((args.len() == 2
        && matches!(
            args[0].as_str(),
            "inspect" | "init" | "run" | "activate" | "investigate"
        ))
        || (args.len() == 3 && matches!(args[0].as_str(), "resume" | "run" | "activate")))
    {
        return Err("usage: rx-solutionsd inspect|init|run|activate|investigate CONFIG; rx-solutionsd resume CURRENT_CONFIG NEXT_CONFIG; rx-solutionsd run|activate CONFIG WORK_TASK".into());
    }
    let configuration = read(Path::new(&args[1]))?;
    let mut work_task: Option<rx_supervisor::work_use::Task> =
        if matches!(args[0].as_str(), "run" | "activate") && args.len() == 3 {
            let task: rx_supervisor::work_use::Task = read_json(Path::new(&args[2]))?;
            if !configuration.plan.processes.iter().any(|p| {
                p.id == task.selection
                    && matches!(
                        p.program.as_str(),
                        "rx/status-http" | rx_package::operating_area::PROGRAM
                    )
            }) {
                return Err("work task requires a selected non-actuating status program".into());
            }
            Some(task)
        } else {
            None
        };
    let work_provider = match &configuration.operating_area_mailbox {
        Some(path) => rx_supervisor::operating_area::ResidentWorkProvider::Offline(Box::new(
            rx_supervisor::operating_area::OfflineWorkUseProvider::new(path),
        )),
        None => rx_supervisor::operating_area::ResidentWorkProvider::Unconfigured,
    };
    let mut prepared_work: Option<rx_supervisor::work_use::Prepared> = None;
    let mut last_work_observation = String::new();
    let root = Path::new("/opt/rx");
    // Capture signed metadata once. A plan-selected state subdirectory cannot
    // reset the installation-wide release floor. No runtime root-key override.
    let release_bytes = release_metadata(root, "release.json")?;
    let revocation_bytes = release_metadata(root, "revocations.json")?;
    preflight_source_assets(root)?;
    let mut release_store = if args[0] == "inspect" {
        None
    } else {
        let base = Path::new("/var/lib/rx-solutions");
        if !fs::symlink_metadata(base).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
            return Err("release/state-root-invalid".into());
        }
        for name in ["release.db", "release.writer.lock"] {
            match fs::symlink_metadata(base.join(name)) {
                Ok(m) if !m.is_file() || m.file_type().is_symlink() => {
                    return Err("release/state-file-invalid".into());
                }
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        let mut store = SqliteRepository::open(base.join("release.db"))?;
        rx_package::release::update_revocations(&mut store, &revocation_bytes)?;
        Some(store)
    };
    let release = verify_release(root, &release_bytes, &revocation_bytes)?;
    let mut programs = programs_from_release(root, &release)?;
    if configuration
        .plan
        .processes
        .iter()
        .any(|p| p.program.as_str() == rx_package::operating_area::PROGRAM)
    {
        let work = rx_supervisor::builtin::development_work_program(root, &release)?;
        programs.insert(work.id.clone(), work);
    }
    let initializers = if let Some(services) = &configuration.services {
        let initializers = services_from_release(root, services, &mut programs, &release)?;
        validate_service_plan(&configuration.plan, &initializers)?
    } else {
        Vec::new()
    };
    let support = DeviceCatalog::decode(&fs::read(root.join("catalogs/device-support.v1.json"))?)?;
    let plan_digest = configuration.plan.validate(&programs, &support)?;
    if args[0] == "inspect" {
        println!(
            "{}",
            serde_json::json!({"schema":"rx.solutions-plan-inspection.v1","plan_digest":plan_digest,"selected_profiles":configuration.plan.profiles,"protocol_guarded_services":!initializers.is_empty(),"release_boundary":release_boundary(),"control_prepared":false,"physical_qualification":"NOT_PERFORMED"})
        );
        return Ok(());
    }
    rx_package::release::admit(release_store.as_mut().expect("non-inspect store"), &release)?;
    // This is a recorded checkpoint, not continuous revocation monitoring.
    // Release the global writer only after the durable admission has committed.
    release_store.take().expect("non-inspect store").close()?;
    let state = state_directory(configuration.state_subdirectory.as_str())?;
    for initializer in &initializers {
        for path in
            std::iter::once(&initializer.data_directory).chain(initializer.runtime_directory.iter())
        {
            if path.starts_with(&state) || state.starts_with(path) {
                return Err("supervisor and daemon data/runtime roots must not overlap".into());
            }
        }
    }
    for filename in [
        "supervisor.db",
        "supervisor.writer.lock",
        "initialization.db",
        "initialization.writer.lock",
        "registration.db",
        "registration.writer.lock",
    ] {
        match fs::symlink_metadata(state.join(filename)) {
            Ok(m) if !m.is_file() || m.file_type().is_symlink() => {
                return Err("invalid supervisor state file".into());
            }
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    if args[0] == "init" {
        if initializers.is_empty() {
            return Err("init requires named Host/Executor service configuration".into());
        }
        let expected = initialization::digest(plan_digest, &initializers)?;
        let mut store = SqliteRepository::open(state.join("initialization.db"))?;
        let initialized = initialization::run(
            &mut store,
            expected,
            &initializers,
            &state.join("init-logs"),
        )
        .await?;
        println!("{}", serde_json::to_string(&initialized)?);
        return Ok(());
    }
    if !initializers.is_empty() {
        let path = state.join("initialization.db");
        if !fs::symlink_metadata(&path)
            .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink() && m.len() > 0)
        {
            return Err("required composition initialization journal absent; run never initializes services".into());
        }
        let mut store = SqliteRepository::open(path)?;
        store.check_integrity()?;
        initialization::require_complete(
            &mut store,
            initialization::digest(plan_digest, &initializers)?,
            &initializers,
        )?;
        for path in initializers
            .iter()
            .filter_map(|i| i.runtime_directory.as_ref())
        {
            fs::create_dir_all(path)?;
            if !fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
            {
                return Err("daemon runtime directory must be a real directory".into());
            }
        }
        let runtime = Path::new("/run/rx-solutions");
        if !fs::symlink_metadata(runtime).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
            return Err("owned /run/rx-solutions directory required for guarded status".into());
        }
    }
    let next = if args[0] == "resume" {
        let next = read(Path::new(&args[2]))?;
        if configuration.services.is_some()
            || next.services.is_some()
            || configuration.plan.processes.len() != 1
            || !matches!(
                configuration.plan.processes[0].program.as_str(),
                "rx/status-http" | rx_package::operating_area::PROGRAM
            )
            || configuration.state_subdirectory != next.state_subdirectory
            || configuration.plan.id == next.plan.id
        {
            return Err("resident-resume-requires-one-status-selection-same-root-new-run".into());
        }
        let mut reviewed = next.plan.clone();
        reviewed.id = configuration.plan.id.clone();
        if canonical::bytes(&reviewed)? != canonical::bytes(&configuration.plan)? {
            return Err("resident-resume-plan-must-be-unchanged-except-run-id".into());
        }
        next.plan.validate(&programs, &support)?;
        Some(next)
    } else {
        None
    };
    // This one lock owns registration across every old/new execution journal.
    let registry = Registry::new(SqliteRepository::open(state.join("registration.db"))?);
    let execution = execution_store::resolve(&state, &configuration.plan.id, args[0] == "run")?;
    let selection = configuration.plan.processes.first().map(|p| p.id.clone());
    let mut supervisor = RegisteredSupervisor::open_resident(
        SqliteRepository::open(execution.join("supervisor.db"))?,
        OsProcesses::new(execution.join("logs"))?,
        GuardedServices,
        configuration.plan,
        programs.clone(),
        &support,
        registry,
    )?;
    if args[0] == "investigate" {
        for selection in supervisor.registrations()?.keys() {
            println!(
                "{}",
                serde_json::json!({"schema":"rx.process-investigation.v1",
                "selection":selection, "finding":supervisor.investigate_current(selection)?,
                "ownership":"NOT_ADOPTED", "past_outcome":"UNRESOLVED"})
            );
        }
        return Ok(());
    }
    if let Some(next) = next {
        let selection = selection.expect("validated single selection");
        let permit = supervisor.prepare_investigated_resume(
            &selection,
            next.plan.id.clone(),
            rx_domain::types::Name::new("local/explicit-resume")?,
            &ExplicitLocalResume,
        )?;
        let (old_store, old_backend, _, registry) = supervisor.into_parts();
        drop(old_backend);
        drop(old_store);
        let execution = execution_store::create_resume(&state, &next.plan.id)?;
        supervisor = RegisteredSupervisor::open_resident_with_resume(
            SqliteRepository::open(execution.join("supervisor.db"))?,
            OsProcesses::new(execution.join("logs"))?,
            GuardedServices,
            next.plan,
            programs,
            &support,
            registry,
            permit,
        )?;
    }
    println!(
        "{}",
        serde_json::json!({
            "schema": "rx.resident-reconciliation.v1",
            "registrations": supervisor.registrations()?,
            "state": supervisor.state()?,
            "execution_admission": supervisor.execution_admission()?,
            "release_boundary": release_boundary(),
            "physical_qualification": "NOT_PERFORMED"
        })
    );
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
                    println!(
                        "{}",
                        serde_json::json!({
                            "schema": "rx.resident-registration-observation.v1",
                            "registrations": supervisor.registrations()?
                        })
                    );
                    last_output = text;
                }
                if work_task.as_ref().is_some_and(|task| {
                    !matches!(
                        report.state.records[&task.selection].phase,
                        rx_supervisor::model::Phase::Pending
                            | rx_supervisor::model::Phase::Prepared
                            | rx_supervisor::model::Phase::SpawnEntered
                            | rx_supervisor::model::Phase::Starting
                    )
                }) {
                    let task = work_task.as_ref().expect("checked task").clone();
                    if let Some(prepared) = prepared_work.take() {
                        let result =
                            work_provider.commit(&prepared, || supervisor.commit_work(&prepared));
                        match result {
                            Ok(result) => println!(
                                "{}",
                                serde_json::json!({"schema":"rx.work-use-result.v1","result":result,"operating_area_provider":work_provider.observation()})
                            ),
                            Err(error) => println!(
                                "{}",
                                serde_json::json!({"schema":"rx.work-use-result.v1","operation":task.operation,"selection":task.selection,"gate":"DENIED","reason":error.to_string(),"current_permission":"NONE","operating_area_provider":work_provider.observation()})
                            ),
                        }
                        work_task.take();
                    } else {
                        work_provider.begin_attempt();
                        match supervisor.prepare_work(task.clone(), &work_provider) {
                            Ok(prepared) => {
                                println!(
                                    "{}",
                                    serde_json::json!({"schema":"rx.work-use-judgment.v1","operation":task.operation,"work_use_permission":prepared.assessment(),"operating_area_provider":work_provider.observation(),"meaning":"JUDGMENT_OBSERVATION_ONLY; COMMIT_RECHECK_REQUIRED"})
                                );
                                prepared_work = Some(prepared);
                            }
                            Err(error) => {
                                let pending = work_provider.pending();
                                let output=serde_json::json!({"schema":"rx.work-use-result.v1","operation":task.operation,"selection":task.selection,"gate":if pending{"PENDING"}else{"DENIED"},"reason":error.to_string(),"current_permission":"NONE","operating_area_provider":work_provider.observation()}).to_string();
                                if output != last_work_observation {
                                    println!("{output}");
                                    last_work_observation = output;
                                }
                                if !pending {
                                    work_task.take();
                                }
                            }
                        }
                    }
                }
                if report.all_exited {
                    if !report.guarded_shutdown_confirmed {
                        return Err("managed daemons exited without confirmed cooperative-stop reports; reconciliation required".into());
                    }
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
