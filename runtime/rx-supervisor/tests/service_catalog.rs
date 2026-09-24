use rx_domain::{canonical, types::*};
use rx_service_status::GuardedScope;
use rx_supervisor::{
    builtin::{
        ConfigurationPin, ServiceConfigurations, ServiceInput, add_guarded_services,
        validate_service_plan,
    },
    model::*,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
fn n(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id(v: u64) -> Id {
    Id::new(format!("00000000-0000-4000-8000-{v:012}")).unwrap()
}
fn d(v: u8) -> Digest {
    Digest::from_bytes([v; 32])
}
fn write_pin(path: &Path, value: &serde_json::Value) -> ConfigurationPin {
    let bytes = canonical::bytes(value).unwrap();
    std::fs::write(path, &bytes).unwrap();
    ConfigurationPin {
        path: path.into(),
        sha256: rx_package::content_digest(&bytes),
    }
}
fn fixture(root: &Path) -> (ServiceConfigurations, Plan) {
    let root = std::fs::canonicalize(root).unwrap();
    std::fs::create_dir(root.join("bin")).unwrap();
    std::fs::create_dir(root.join("manifests")).unwrap();
    let mut files = BTreeMap::new();
    for binary in ["rx-hostd", "rx-executor-service"] {
        let bytes = format!("test-only release inventory entry: {binary}").into_bytes();
        std::fs::write(root.join("bin").join(binary), &bytes).unwrap();
        files.insert(format!("bin/{binary}"), rx_package::content_digest(&bytes));
    }
    std::fs::write(root.join("manifests/runtime-files.json"), serde_json::to_vec(&serde_json::json!({"schema":"rx.solutions-runtime-files.v1","files":files,"external_files":{}})).unwrap()).unwrap();
    // Pre-signed inert fixture: no private key, runtime key override or alternate root.
    std::fs::write(
        root.join("manifests/release.json"),
        include_bytes!("fixtures/service-release/release.json"),
    )
    .unwrap();
    std::fs::write(
        root.join("manifests/revocations.json"),
        include_bytes!("fixtures/service-release/revocations.json"),
    )
    .unwrap();
    let mut services = BTreeMap::new();
    let mut processes = Vec::new();
    // This fixture validates the catalog projection only. No daemon, TLS or device is started.
    for index in 0..2 {
        let cell = n(&format!("cell/{index}"));
        let host = n(&format!("host/{index}"));
        let definition = d(10 + index);
        let binding_pin = write_pin(
            &root.join(format!("bindings-{index}.json")),
            &serde_json::json!([{
                "cell":cell,"definition":{"schema_id":"rx.cell-definition.v1","sha256":definition,"size_bytes":"1"}
            }]),
        );
        let backend = serde_json::json!({"kind":"FILE_SIMULATION"});
        let identity = canonical::digest(
            "RX-HOST-INSTALLATION-CONFIG-v1",
            &(&id(1), &host, &backend, binding_pin.sha256),
        )
        .unwrap();
        let host_pin = write_pin(
            &root.join(format!("host-{index}.json")),
            &serde_json::json!({
                "schema":"rx.host-startup.v1","installation":id(1),"host":host,"backend":backend,"bindings":binding_pin,
                "data_directory":root.join(format!("host-data-{index}")),"runtime_directory":root.join(format!("host-runtime-{index}"))
            }),
        );
        let mut executor = serde_json::json!({
            "schema":"rx.executor-cell-service.v1","service_root":root.join(format!("executor-data-{index}")),
            "expected_service":{"journal":id(20+u64::from(index)),"scope":{"installation":id(1),"store_generation":id(2),"principal":format!("executor/{index}"),"release":d(3),"cell":cell,"definition":definition}},
            "platform":{"uri":"https://test.invalid:7443","server_name":"test.invalid","ca":{"path":"/test/ca","sha256":d(4)},"certificate":{"path":"/test/cert","sha256":d(5)},"key":{"path":"/test/key","sha256":d(6)}},
            "engine":{"path":"/test/engine","sha256":d(7)}
        });
        let executor_pin = write_pin(&root.join(format!("executor-{index}.json")), &executor);
        executor["options"] =
            serde_json::json!({"poll_ms":50,"communication_grace_ms":5000,"stop_timeout_ms":10000});
        let configuration_digest =
            canonical::digest("RX-EXECUTOR-CELL-CONFIG-v1", &executor).unwrap();
        services.insert(
            n(&format!("host-{index}")),
            ServiceInput {
                configuration: host_pin,
                scope: GuardedScope::Host {
                    installation: id(1),
                    host,
                    installation_identity: identity,
                },
            },
        );
        services.insert(
            n(&format!("executor-{index}")),
            ServiceInput {
                configuration: executor_pin,
                scope: GuardedScope::Executor {
                    installation: id(1),
                    cell,
                    service_journal: id(20 + u64::from(index)),
                    configuration_digest,
                },
            },
        );
        for (name, dependencies) in [
            (format!("host-{index}"), vec![]),
            (
                format!("executor-{index}"),
                vec![n(&format!("host-{index}"))],
            ),
        ] {
            processes.push(Process {
                id: n(&name),
                program: n(&format!("rx/service/{name}")),
                parameters: BTreeMap::new(),
                depends_on: dependencies,
                startup_timeout_ms: Counter(1000),
                shutdown_timeout_ms: Counter(1000),
                restart_limit: Counter(0),
                restart_backoff_ms: Counter(100),
            });
        }
    }
    (
        services,
        Plan {
            schema: n("rx.solutions-process-plan.v1"),
            id: id(99),
            environment: Environment::Simulation,
            profiles: vec![],
            processes,
        },
    )
}
#[test]
fn two_hosts_and_two_cells_resolve_fixed_binaries_and_independent_dependency_order() {
    let root = tempfile::tempdir().unwrap();
    let (services, plan) = fixture(root.path());
    let mut programs = BTreeMap::new();
    let commands = add_guarded_services(root.path(), &services, &mut programs).unwrap();
    assert_eq!(programs.len(), 4);
    let commands = validate_service_plan(&plan, &commands).unwrap();
    assert_eq!(commands.len(), 4);
    for (service, input) in &services {
        let program = &programs[&n(&format!("rx/service/{service}"))];
        assert_eq!(program.effect, Effect::ProtocolGuardedService);
        assert!(program.arguments.is_empty());
        let expected = match input.scope {
            GuardedScope::Host { .. } => "rx-hostd",
            GuardedScope::Executor { .. } => "rx-executor-service",
        };
        assert_eq!(program.executable.file_name().unwrap(), expected);
        assert_eq!(
            program.fixed_arguments.last().unwrap(),
            input.configuration.path.to_str().unwrap()
        );
    }
    for index in 0..2 {
        let position = |role: &str| {
            commands
                .iter()
                .position(|c| c.id == n(&format!("rx/service/{role}-{index}")))
                .unwrap()
        };
        assert!(position("host") < position("executor"));
    }
}
#[test]
fn each_executor_requires_matching_host_edges_and_definitions_without_cross_cell_shortcuts() {
    let root = tempfile::tempdir().unwrap();
    let (services, mut plan) = fixture(root.path());
    let mut programs = BTreeMap::new();
    let commands = add_guarded_services(root.path(), &services, &mut programs).unwrap();
    plan.processes
        .iter_mut()
        .find(|p| p.id == n("executor-0"))
        .unwrap()
        .depends_on = vec![n("host-1")];
    assert!(validate_service_plan(&plan, &commands).is_err());
    plan.processes
        .iter_mut()
        .find(|p| p.id == n("executor-0"))
        .unwrap()
        .depends_on = vec![n("host-0")];
    let mut changed = commands.clone();
    changed
        .iter_mut()
        .find(|c| c.id == n("rx/service/host-0"))
        .unwrap()
        .cells
        .insert(n("cell/0"), d(99));
    assert!(validate_service_plan(&plan, &changed).is_err());
    let mut shared = commands.clone();
    let principal = shared
        .iter()
        .find(|c| c.id == n("rx/service/executor-0"))
        .unwrap()
        .principal
        .clone();
    shared
        .iter_mut()
        .find(|c| c.id == n("rx/service/executor-1"))
        .unwrap()
        .principal = principal;
    assert!(validate_service_plan(&plan, &shared).is_err());
}
#[test]
fn service_names_cannot_be_paths_or_choose_programs() {
    let root = tempfile::tempdir().unwrap();
    let (mut services, _) = fixture(root.path());
    let input = services.remove(&n("host-0")).unwrap();
    services.insert(n("outside/host"), input);
    assert!(add_guarded_services(root.path(), &services, &mut BTreeMap::new()).is_err());
    let mut value = serde_json::to_value(&services).unwrap();
    value["outside/host"]["executable"] = serde_json::json!(PathBuf::from("/bin/sh"));
    assert!(
        canonical::decode_json::<ServiceConfigurations>(&serde_json::to_vec(&value).unwrap())
            .is_err()
    );
}

#[test]
fn distinct_executor_principals_cannot_compete_for_the_same_installation_cell() {
    let root = tempfile::tempdir().unwrap();
    let (services, mut plan) = fixture(root.path());
    let mut programs = BTreeMap::new();
    let mut commands = add_guarded_services(root.path(), &services, &mut programs).unwrap();
    let original = commands
        .iter()
        .find(|c| c.id == n("rx/service/executor-0"))
        .unwrap()
        .clone();
    let second = commands
        .iter_mut()
        .find(|c| c.id == n("rx/service/executor-1"))
        .unwrap();
    assert_ne!(original.principal, second.principal);
    assert_ne!(original.data_directory, second.data_directory);
    second.cells = original.cells.clone();
    plan.processes
        .iter_mut()
        .find(|p| p.id == n("executor-1"))
        .unwrap()
        .depends_on = vec![n("host-0")];
    let error = validate_service_plan(&plan, &commands)
        .unwrap_err()
        .to_string();
    assert!(error.contains("one selected Executor service"), "{error}");

    // Several independent Hosts may still serve the same cell if its single
    // Executor depends on every matching provider.
    commands.retain(|c| c.id != n("rx/service/executor-1"));
    plan.processes.retain(|p| p.id != n("executor-1"));
    commands
        .iter_mut()
        .find(|c| c.id == n("rx/service/host-1"))
        .unwrap()
        .cells = original.cells;
    plan.processes
        .iter_mut()
        .find(|p| p.id == n("executor-0"))
        .unwrap()
        .depends_on
        .push(n("host-1"));
    assert_eq!(validate_service_plan(&plan, &commands).unwrap().len(), 3);
}
