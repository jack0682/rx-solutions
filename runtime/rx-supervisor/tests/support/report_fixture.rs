//! Device-free cross-repository fixture: real owned software child -> S registry -> mTLS P.
use rx_domain::{resident_reporting::*, types::*};
use rx_ports::Repository;
use rx_solution_catalog::DeviceCatalog;
use rx_storage::SqliteRepository;
use rx_supervisor::{
    model::*,
    process::{Backend, OsProcesses},
    registered::{RegisteredSupervisor, catalog_reference},
    registration::{Declaration, Registry},
    reporting::{
        outbox::Outbox,
        resident::{Configuration as ReportingConfiguration, Resident},
    },
};
use serde::Deserialize;
use sha2::Digest as _;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    endpoint: String,
    ca: PathBuf,
    certificate: PathBuf,
    key: PathBuf,
    output: PathBuf,
    installation: Id,
    store_generation: Id,
    clock: String,
    release: Digest,
    #[serde(default)]
    outage: bool,
}
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn hash(p: &std::path::Path) -> Digest {
    Digest::from_bytes(sha2::Sha256::digest(std::fs::read(p).unwrap()).into())
}
fn write(p: &std::path::Path, value: &impl serde::Serialize) {
    let pending = p.with_extension("pending");
    std::fs::write(&pending, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    std::fs::rename(pending, p).unwrap();
}

// Fixture cleanup is separate from the asserted cooperative stop; these are owned non-actuating children only.
struct Owned(OsProcesses);
impl Drop for Owned {
    fn drop(&mut self) {
        for id in self.0.owned_instances() {
            let _ = self.0.terminate(&id, true);
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                if matches!(self.0.exited(&id), Ok(Some(_))) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}
impl Backend for Owned {
    fn spawn(
        &mut self,
        l: &Launch,
        a: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<u32, rx_supervisor::process::SpawnFailure> {
        self.0.spawn(l, a)
    }
    fn pid(&self, id: &Id) -> Option<u32> {
        self.0.pid(id)
    }
    fn owns(&self, id: &Id) -> bool {
        self.0.owns(id)
    }
    fn forget_exited(&mut self, id: &Id) -> rx_supervisor::Result<()> {
        self.0.forget_exited(id)
    }
    fn exited(&mut self, id: &Id) -> rx_supervisor::Result<Option<Option<i32>>> {
        self.0.exited(id)
    }
    fn ready(&mut self, l: &Launch) -> rx_supervisor::Result<bool> {
        self.0.ready(l)
    }
    fn terminate(&mut self, id: &Id, force: bool) -> rx_supervisor::Result<()> {
        self.0.terminate(id, force)
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config: Config = serde_json::from_slice(&std::fs::read(
        std::env::args().nth(1).ok_or("config required")?,
    )?)?;
    std::fs::create_dir_all(&config.output)?;
    let script = config.output.join("status.py");
    std::fs::write(
        &script,
        r#"import os,json,sys,signal
from http.server import BaseHTTPRequestHandler,HTTPServer
signal.signal(signal.SIGTERM,lambda *_:sys.exit(0))
class Handler(BaseHTTPRequestHandler):
 def log_message(self,*args):pass
 def do_GET(self):
  body=json.dumps({'schema':'rx.solutions-status.v1','phase':'SOFTWARE_READY_UNCOMMISSIONED','supervisor_instance':os.environ['RX_PROCESS_INSTANCE_ID']}).encode()
  self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
HTTPServer(('127.0.0.1',int(sys.argv[2])),Handler).serve_forever()
"#,
    )?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    let python = PathBuf::from("/usr/bin/python3");
    let program = Program {
        id: name("test/resident-status"),
        effect: Effect::NonActuating,
        executable: python.clone(),
        executable_sha256: hash(&python),
        files: [(script.clone(), hash(&script))].into(),
        fixed_arguments: vec![script.to_string_lossy().into_owned()],
        arguments: [(
            name("port"),
            Argument::Port {
                flag: "--port".into(),
            },
        )]
        .into(),
        ready: ReadyProbe::HttpStatus {
            port_parameter: name("port"),
        },
        execution_requirements: Some(rx_supervisor::execution::Requirements(BTreeMap::new())),
        functional_readiness: None,
        decision_policy: None,
    };
    let mut registry = Registry::new(SqliteRepository::open(
        config.output.join("registration.db"),
    )?);
    let registration = registry.register(Declaration {
        label: name("status"),
        catalog: catalog_reference(&program)?,
    })?;
    let component = registration.registration.id.clone();
    let connection = config.output.join("connection.json");
    write(
        &connection,
        &serde_json::json!({"schema":"rx.resident-report-connection.v1",
        "endpoint":config.endpoint,"server_name":"localhost","ca":config.ca,"certificate":config.certificate,"private_key":config.key,
        "principal":"reporter","installation":config.installation,"store_generation":config.store_generation,"shared_clock_id":config.clock,"release_digest":config.release}),
    );
    let scopes = config.output.join("scopes.json");
    let journal = config.output.join("reporting.db");
    let reporting_config = ReportingConfiguration {
        connection,
        scopes: scopes.clone(),
    };
    let mut reporter =
        Resident::start(reporting_config.clone(), journal.clone()).map_err(|e| e.to_string())?;
    let plan = Plan {
        schema: name("rx.solutions-process-plan.v1"),
        id: id(),
        environment: Environment::Simulation,
        profiles: vec![name("SIM-JTC-6DOF")],
        processes: vec![Process {
            id: name("status"),
            program: program.id.clone(),
            parameters: [(name("port"), port.to_string())].into(),
            depends_on: vec![],
            startup_timeout_ms: Counter(10000),
            shutdown_timeout_ms: Counter(3000),
            restart_limit: Counter(0),
            restart_backoff_ms: Counter(100),
        }],
    };
    let support = DeviceCatalog::decode(include_bytes!(
        "../../../../catalogs/device-support.v1.json"
    ))?;
    let mut supervisor = RegisteredSupervisor::open(
        SqliteRepository::open(config.output.join("execution.db"))?,
        Owned(OsProcesses::new(config.output.join("logs"))?),
        SoftwareOnly,
        plan,
        BTreeMap::from([(program.id.clone(), program)]),
        &support,
        registry,
        component.clone(),
    )?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let report = supervisor.tick()?;
        if report.state.records[&name("status")].phase == Phase::ProcessReady {
            break;
        }
        if Instant::now() > deadline {
            return Err("software child not ready".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // The child becomes ready before an owner scope exists; reporting cannot gate local execution.
    reporter
        .observe(supervisor.reporting_snapshot()?)
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while reporter.status().peer.is_none() {
        if Instant::now() > deadline {
            return Err("reporter peer absent".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    write(
        &config.output.join("ready.json"),
        &serde_json::json!({"peer":reporter.status().peer,"registration":registration}),
    );
    let authorization = config.output.join("authorization.json");
    while !authorization.is_file() {
        if Instant::now() > deadline {
            return Err("reporting scope not supplied".into());
        }
        supervisor.tick()?;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let authorization: serde_json::Value = serde_json::from_slice(&std::fs::read(authorization)?)?;
    write(
        &scopes,
        &serde_json::json!({"schema":"rx.resident-report-scopes.v1","selections":{"status":{
        "component":authorization["component"],"scope":authorization["scope"],"registration":component}}}),
    );
    while reporter.status().delivered_snapshots != 1 {
        if Instant::now() > deadline {
            return Err(format!("report not delivered: {:?}", reporter.status()).into());
        }
        supervisor.tick()?;
        reporter
            .observe(supervisor.reporting_snapshot()?)
            .map_err(|e| e.to_string())?;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    if config.outage {
        write(
            &config.output.join("running.json"),
            &serde_json::json!({"delivered":reporter.status().delivered_snapshots}),
        );
        while !config.output.join("stop-approved.json").is_file() {
            if Instant::now() > deadline {
                return Err("outage control absent".into());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    let stop_started = Instant::now();
    supervisor.request_stop()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let state = supervisor.tick()?;
        if state.all_exited {
            break;
        }
        if Instant::now() > deadline {
            return Err("owned child did not stop".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let local_stop_ms = stop_started.elapsed().as_millis();
    assert!(local_stop_ms < 2000, "local stop was delayed by reporting");
    let snapshots = supervisor.reporting_snapshot()?;
    let instance = snapshots[0].binding.instance.clone();
    let status = reporter.finish(snapshots.clone()).await;
    let mut retained_during_outage = false;
    if config.outage {
        assert!(status.stopped && status.error.is_some(), "{status:?}");
        let mut retained = Outbox::new(SqliteRepository::open(&journal)?);
        let row = retained
            .inspect(&instance)?
            .ok_or("outage snapshot absent")?;
        assert_eq!(row.latest.last_observed.state, ExecutionState::Exited);
        assert_eq!(
            row.accepted.as_ref().unwrap().report.state,
            ExecutionState::Running
        );
        retained_during_outage = true;
        retained.into_repository().close()?;
        reporter = Resident::start(reporting_config, journal.clone()).map_err(|e| e.to_string())?;
        reporter
            .observe(snapshots.clone())
            .map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(25);
        while reporter.status().peer.is_none() {
            if Instant::now() > deadline {
                return Err("restart peer absent".into());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        write(
            &config.output.join("restart-ready.json"),
            &reporter.status().peer.unwrap(),
        );
        while !config.output.join("authorization-restart.json").is_file() {
            if Instant::now() > deadline {
                return Err("restart authorization absent".into());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let authorization: serde_json::Value = serde_json::from_slice(&std::fs::read(
            config.output.join("authorization-restart.json"),
        )?)?;
        write(
            &scopes,
            &serde_json::json!({"schema":"rx.resident-report-scopes.v1","selections":{"status":{
            "component":authorization["component"],"scope":authorization["scope"],"registration":component}}}),
        );
        let final_status = reporter.finish(snapshots).await;
        assert!(
            final_status.stopped && final_status.error.is_none(),
            "{final_status:?}"
        );
    } else {
        assert!(status.stopped && status.error.is_none(), "{status:?}");
    }
    let mut store = SqliteRepository::open(journal)?;
    let events = store.control_events_after(Counter(0), 100)?;
    let recovered: Receipt = events
        .iter()
        .filter_map(|e| serde_json::from_value::<Receipt>(e.document.value["receipt"].clone()).ok())
        .find(|r| r.report.state == ExecutionState::Running)
        .ok_or("accepted running receipt missing")?;
    let mut outbox = Outbox::new(store);
    let row = outbox.inspect(&instance)?.ok_or("outbox absent")?;
    assert!(row.pending.is_none());
    let last = row.accepted.ok_or("exit receipt absent")?;
    assert_eq!(last.report.state, ExecutionState::Exited);
    assert_eq!(last.report.exit_code, Some(0));
    write(
        &config.output.join("result.json"),
        &serde_json::json!({"status":"PASS","recovered":recovered,"stopped":last,"local_stop_ms":local_stop_ms,"retained_during_outage":retained_during_outage,
        "scope":"Actual locally approved non-actuating Supervisor child and attributed mTLS reports; not Platform launch authority or registry migration"}),
    );
    Ok(())
}
