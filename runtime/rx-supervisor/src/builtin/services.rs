use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationPin {
    pub path: PathBuf,
    pub sha256: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceInput {
    pub configuration: ConfigurationPin,
    pub scope: rx_service_status::GuardedScope,
}
pub type ServiceConfigurations = BTreeMap<Name, ServiceInput>;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ServiceRole {
    Host,
    Executor,
}
/// Generated from a named deployment input and a fixed release binary, never deserialized argv.
#[derive(Clone, Debug, Serialize)]
pub struct Initializer {
    pub id: Name,
    pub role: ServiceRole,
    pub executable: PathBuf,
    pub executable_sha256: Digest,
    pub files: BTreeMap<PathBuf, Digest>,
    pub arguments: Vec<String>,
    pub data_directory: PathBuf,
    pub runtime_directory: Option<PathBuf>,
    pub installation: Id,
    pub principal: Name,
    pub cells: BTreeMap<Name, Digest>,
}

pub fn add_guarded_services(
    root: &Path,
    configurations: &ServiceConfigurations,
    programs: &mut BTreeMap<Name, Program>,
) -> Result<Vec<Initializer>> {
    if configurations.is_empty() || configurations.len() > 32 {
        return Err(Error::Invalid(
            "service configuration count must be 1..32".into(),
        ));
    }
    let inventory: Inventory =
        canonical::decode_json(&std::fs::read(root.join("manifests/runtime-files.json"))?)
            .map_err(|e| Error::Invalid(e.to_string()))?;
    if inventory.schema != "rx.solutions-runtime-files.v1" {
        return Err(Error::Invalid("release inventory schema".into()));
    }
    let mut commands = Vec::new();
    for (service, input) in configurations {
        if service.as_str().len() > 64
            || !service
                .as_str()
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return Err(Error::Invalid(
                "service name must be a flat alphanumeric, underscore or hyphen label".into(),
            ));
        }
        let mut config = read_configuration(&input.configuration)?;
        let status_path = PathBuf::from("/run/rx-solutions").join(format!("{service}.json"));
        let (
            role,
            binary,
            args,
            init_args,
            data_directory,
            runtime_directory,
            installation,
            principal,
            cells,
            status,
        ) = match &input.scope {
            rx_service_status::GuardedScope::Host {
                installation,
                host,
                installation_identity,
            } => {
                let actual_installation: Id = field(&config, "installation")?;
                let actual_host: Name = field(&config, "host")?;
                let bindings: ConfigurationPin = field(&config, "bindings")?;
                let actual_identity = canonical::digest(
                    "RX-HOST-INSTALLATION-CONFIG-v1",
                    &(
                        &actual_installation,
                        &actual_host,
                        &config["backend"],
                        bindings.sha256,
                    ),
                )
                .map_err(|e| Error::Invalid(e.to_string()))?;
                if config["schema"] != "rx.host-startup.v1"
                    || actual_installation != *installation
                    || actual_host != *host
                    || actual_identity != *installation_identity
                {
                    return Err(Error::Invalid("Host configuration scope differs".into()));
                }
                let bindings = read_configuration(&bindings)?;
                let rows = bindings
                    .as_array()
                    .filter(|v| !v.is_empty() && v.len() <= 64)
                    .ok_or_else(|| Error::Invalid("Host bindings array required".into()))?;
                let mut cells = BTreeMap::new();
                for row in rows {
                    let cell: Name = field(row, "cell")?;
                    let definition: ArtifactRef = field(row, "definition")?;
                    if cells
                        .insert(cell, definition.sha256)
                        .is_some_and(|old| old != definition.sha256)
                    {
                        return Err(Error::Invalid("Host cell definitions disagree".into()));
                    }
                }
                (
                    ServiceRole::Host,
                    "rx-hostd",
                    vec!["run"],
                    vec!["init"],
                    normalize_location(&field::<PathBuf>(&config, "data_directory")?)?,
                    Some(normalize_location(&field::<PathBuf>(
                        &config,
                        "runtime_directory",
                    )?)?),
                    installation.clone(),
                    host.clone(),
                    cells,
                    GuardedStatusBinding::Host {
                        path: status_path,
                        installation: installation.clone(),
                        host: host.clone(),
                        installation_identity: *installation_identity,
                    },
                )
            }
            rx_service_status::GuardedScope::Executor {
                installation,
                cell,
                service_journal,
                configuration_digest,
            } => {
                if config["schema"] != "rx.executor-cell-service.v1" {
                    return Err(Error::Invalid(
                        "Executor configuration schema differs".into(),
                    ));
                }
                let data_directory =
                    normalize_location(&field::<PathBuf>(&config, "service_root")?)?;
                config["service_root"] = serde_json::to_value(&data_directory)
                    .map_err(|e| Error::Invalid(e.to_string()))?;
                if config.get("options").is_none() {
                    config["options"] = serde_json::json!({"poll_ms":50,"communication_grace_ms":5000,"stop_timeout_ms":10000});
                }
                let expected = &config["expected_service"];
                let expected_journal: Id = field(expected, "journal")?;
                let expected_installation: Id = field(&expected["scope"], "installation")?;
                let expected_cell: Name = field(&expected["scope"], "cell")?;
                let definition: Digest = field(&expected["scope"], "definition")?;
                let principal: Name = field(&expected["scope"], "principal")?;
                let actual_digest = canonical::digest("RX-EXECUTOR-CELL-CONFIG-v1", &config)
                    .map_err(|e| Error::Invalid(e.to_string()))?;
                if expected_journal != *service_journal
                    || expected_installation != *installation
                    || expected_cell != *cell
                    || actual_digest != *configuration_digest
                {
                    return Err(Error::Invalid(
                        "Executor configuration scope/digest differs".into(),
                    ));
                }
                (
                    ServiceRole::Executor,
                    "rx-executor-service",
                    vec!["cell", "run"],
                    vec!["cell", "init"],
                    data_directory,
                    None,
                    installation.clone(),
                    principal,
                    [(cell.clone(), definition)].into(),
                    GuardedStatusBinding::Executor {
                        path: status_path,
                        installation: installation.clone(),
                        cell: cell.clone(),
                        service_journal: service_journal.clone(),
                        configuration_digest: *configuration_digest,
                    },
                )
            }
        };
        let id = Name::new(format!("rx/service/{service}"))
            .map_err(|e| Error::Invalid(e.to_string()))?;
        let executable = root.join("bin").join(binary);
        let hash = *inventory
            .files
            .get(&format!("bin/{binary}"))
            .ok_or_else(|| Error::Invalid("required release daemon absent".into()))?;
        verify(&executable, hash)?;
        let path = input
            .configuration
            .path
            .to_str()
            .ok_or_else(|| Error::Invalid("UTF-8 configuration path required".into()))?
            .to_owned();
        let files: BTreeMap<PathBuf, Digest> =
            [(input.configuration.path.clone(), input.configuration.sha256)].into();
        let mut arguments = init_args.into_iter().map(str::to_owned).collect::<Vec<_>>();
        arguments.push(path.clone());
        let mut fixed_arguments = args.into_iter().map(str::to_owned).collect::<Vec<_>>();
        fixed_arguments.push(path);
        commands.push(Initializer {
            id: id.clone(),
            role,
            executable: executable.clone(),
            executable_sha256: hash,
            files: files.clone(),
            arguments,
            data_directory,
            runtime_directory,
            installation,
            principal,
            cells,
        });
        if programs
            .insert(
                id.clone(),
                Program {
                    id,
                    effect: Effect::ProtocolGuardedService,
                    executable,
                    executable_sha256: hash,
                    files,
                    fixed_arguments,
                    arguments: BTreeMap::new(),
                    ready: ReadyProbe::GuardedStatus(status),
                },
            )
            .is_some()
        {
            return Err(Error::Invalid("duplicate release service recipe".into()));
        }
    }
    Ok(commands)
}

/// Return only selected services in metadata initialization dependency order.
pub fn validate_service_plan(plan: &Plan, commands: &[Initializer]) -> Result<Vec<Initializer>> {
    let mut selected = Vec::new();
    let mut owners = BTreeSet::new();
    let mut executor_cells = BTreeSet::new();
    let mut roots: Vec<PathBuf> = Vec::new();
    for command in commands {
        let processes = plan
            .processes
            .iter()
            .filter(|p| p.program == command.id)
            .collect::<Vec<_>>();
        if processes.is_empty() {
            continue;
        }
        if processes.len() != 1
            || !owners.insert((command.installation.clone(), command.principal.clone()))
        {
            return Err(Error::Invalid(
                "each selected service/principal has one process owner".into(),
            ));
        }
        if command.role == ServiceRole::Executor {
            for cell in command.cells.keys() {
                if !executor_cells.insert((command.installation.clone(), cell.clone())) {
                    return Err(Error::Invalid(
                        "each installation/cell has one selected Executor service".into(),
                    ));
                }
            }
        }
        for path in std::iter::once(&command.data_directory).chain(command.runtime_directory.iter())
        {
            if roots
                .iter()
                .any(|other| path.starts_with(other) || other.starts_with(path))
            {
                return Err(Error::Invalid(
                    "selected service data/runtime roots overlap".into(),
                ));
            }
            roots.push(path.clone());
        }
        selected.push(command.clone());
    }
    if selected.is_empty() {
        return Err(Error::Invalid("no configured service selected".into()));
    }
    for executor in selected.iter().filter(|c| c.role == ServiceRole::Executor) {
        let process = plan
            .processes
            .iter()
            .find(|p| p.program == executor.id)
            .unwrap();
        for (cell, definition) in &executor.cells {
            let hosts = selected
                .iter()
                .filter(|h| {
                    h.role == ServiceRole::Host
                        && h.installation == executor.installation
                        && h.cells.contains_key(cell)
                })
                .collect::<Vec<_>>();
            if hosts.is_empty() {
                return Err(Error::Invalid(
                    "selected Executor requires its managed Host providers".into(),
                ));
            }
            for host in hosts {
                let provider = plan
                    .processes
                    .iter()
                    .find(|p| p.program == host.id)
                    .unwrap();
                if host.cells[cell] != *definition || !process.depends_on.contains(&provider.id) {
                    return Err(Error::Invalid(
                        "Executor must depend on every matching Host with the same cell definition"
                            .into(),
                    ));
                }
            }
        }
    }
    let diagnostics = plan
        .processes
        .iter()
        .filter(|p| p.program.as_str() == "rx/status-http")
        .collect::<Vec<_>>();
    for host in selected.iter().filter(|h| h.role == ServiceRole::Host) {
        let process = plan
            .processes
            .iter()
            .find(|p| p.program == host.id)
            .unwrap();
        if diagnostics
            .iter()
            .any(|status| !process.depends_on.contains(&status.id))
        {
            return Err(Error::Invalid(
                "diagnostics must remain available until managed Hosts stop".into(),
            ));
        }
    }
    let mut ordered = Vec::new();
    let mut done: BTreeSet<Name> = plan
        .processes
        .iter()
        .filter(|p| !selected.iter().any(|c| c.id == p.program))
        .map(|p| p.id.clone())
        .collect();
    while ordered.len() < selected.len() {
        let before = ordered.len();
        for process in &plan.processes {
            if done.contains(&process.id) || !process.depends_on.iter().all(|d| done.contains(d)) {
                continue;
            }
            if let Some(command) = selected.iter().find(|c| c.id == process.program) {
                ordered.push(command.clone());
                done.insert(process.id.clone());
            }
        }
        if ordered.len() == before {
            return Err(Error::Invalid(
                "managed initializer dependency cycle".into(),
            ));
        }
    }
    Ok(ordered)
}
fn field<T: serde::de::DeserializeOwned>(value: &serde_json::Value, field: &str) -> Result<T> {
    let value = value
        .get(field)
        .ok_or_else(|| Error::Invalid(format!("missing configuration field {field}")))?;
    canonical::decode_json(&canonical::bytes(value).map_err(|e| Error::Invalid(e.to_string()))?)
        .map_err(|e| Error::Invalid(e.to_string()))
}
fn normalize_location(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute()
        || path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err(Error::Invalid(
            "absolute deployment path without traversal required".into(),
        ));
    }
    let mut current = path.to_path_buf();
    let mut suffix = Vec::new();
    loop {
        match std::fs::symlink_metadata(&current) {
            Ok(_) => {
                let mut normalized = std::fs::canonicalize(&current)?;
                if !normalized.is_dir() {
                    return Err(Error::Invalid(
                        "data/runtime ancestor must be a directory".into(),
                    ));
                }
                for part in suffix.into_iter().rev() {
                    normalized.push(part);
                }
                return Ok(normalized);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                suffix.push(
                    current
                        .file_name()
                        .ok_or_else(|| Error::Invalid("data ancestor missing".into()))?
                        .to_os_string(),
                );
                if !current.pop() {
                    return Err(Error::Invalid("data ancestor missing".into()));
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
}
fn read_configuration(pin: &ConfigurationPin) -> Result<serde_json::Value> {
    if !pin.path.is_absolute() {
        return Err(Error::Invalid("absolute configuration pin required".into()));
    }
    let leaf = pin
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| Error::Invalid("configuration filename required".into()))?;
    let relative = rx_package::PackagePath::new(leaf).map_err(|e| Error::Invalid(e.to_string()))?;
    let bytes =
        rx_package::directory::read_relative_file(pin.path.parent().unwrap(), &relative, 1_048_576)
            .map_err(|e| Error::Invalid(e.to_string()))?;
    if rx_package::content_digest(&bytes) != pin.sha256 {
        return Err(Error::Invalid("configuration pin differs".into()));
    }
    canonical::decode_json(&bytes).map_err(|e| Error::Invalid(e.to_string()))
}
