//! Device-free cross-repository fixture: real owned software child -> S registry -> mTLS P.
use rx_domain::{resident_reporting::*, types::*};
use rx_solution_catalog::DeviceCatalog;
use rx_storage::SqliteRepository;
use rx_supervisor::{
    model::*,
    process::{Backend, OsProcesses},
    registered::{RegisteredSupervisor, catalog_reference},
    registration::{Declaration, Registry},
    reporting::{Client, Connection, from_execution},
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
    let mut client = Client::connect(Connection {
        endpoint: config.endpoint,
        server_name: "localhost".into(),
        ca_pem: std::fs::read(config.ca)?,
        certificate_pem: std::fs::read(config.certificate)?,
        private_key_pem: std::fs::read(config.key)?,
        principal: name("reporter"),
        installation: config.installation,
        store_generation: config.store_generation,
        shared_clock_id: config.clock,
        release_digest: config.release,
    })
    .await?;
    write(
        &config.output.join("ready.json"),
        &serde_json::json!({"peer":client.peer(),"registration":registration}),
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    let authorization = config.output.join("authorization.json");
    while !authorization.is_file() {
        if Instant::now() > deadline {
            return Err("reporting scope not supplied".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    #[derive(Deserialize)]
    struct Authorization {
        scope: Id,
        component: Id,
    }
    let authorization: Authorization = serde_json::from_slice(&std::fs::read(authorization)?)?;
    let scope = client
        .inspect_scope(&authorization.scope, &authorization.component, &component)
        .await?;
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
        component,
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
    let views = supervisor.registrations()?;
    let execution = &views[&name("status")].executions[0];
    let first = from_execution(&scope, execution, Counter(1))?;
    assert_eq!(first.state, ExecutionState::Running);
    let key = id();
    let lost = client
        .publish(&scope, &key, first.clone())
        .await
        .unwrap_err();
    assert!(
        matches!(lost,rx_supervisor::reporting::Error::Rpc(ref s) if s.code()==tonic::Code::Unavailable)
    );
    let recovered = client.publish(&scope, &key, first).await?;
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
    let views = supervisor.registrations()?;
    let execution = &views[&name("status")].executions[0];
    let stopped = from_execution(&scope, execution, Counter(2))?;
    assert_eq!(stopped.state, ExecutionState::Exited);
    assert_eq!(stopped.exit_code, Some(0));
    let last = client.publish(&scope, &id(), stopped).await?;
    write(
        &config.output.join("result.json"),
        &serde_json::json!({"status":"PASS","recovered":recovered,"stopped":last,
        "scope":"Actual locally approved non-actuating Supervisor child and attributed mTLS reports; not Platform launch authority or registry migration"}),
    );
    Ok(())
}
