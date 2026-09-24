use rx_domain::types::*;
use rx_ports::{Repository, Transaction};
use rx_storage::SqliteRepository;
use rx_supervisor::{
    Result,
    decision::Policy,
    execution,
    model::*,
    process::{Backend, OsProcesses, SpawnFailure},
    registered::{RegisteredSupervisor, catalog_reference},
    registration::{Declaration, Registry},
    use_assessment::*,
    work_use::Task,
};
use sha2::{Digest as _, Sha256};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};
pub fn n(s: &str) -> Name {
    Name::new(s).unwrap()
}
pub fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn hash(p: &Path) -> Digest {
    Digest::from_bytes(Sha256::digest(std::fs::read(p).unwrap()).into())
}
#[derive(Clone)]
pub struct Owned(pub Rc<RefCell<OsProcesses>>);
impl Backend for Owned {
    fn spawn(
        &mut self,
        l: &Launch,
        a: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<u32, SpawnFailure> {
        self.0.borrow_mut().spawn(l, a)
    }
    fn spawn_with_requirements(
        &mut self,
        l: &Launch,
        r: &execution::Request,
        a: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<execution::Decision, SpawnFailure> {
        self.0.borrow_mut().spawn_with_requirements(l, r, a)
    }
    fn pid(&self, i: &Id) -> Option<u32> {
        self.0.borrow().pid(i)
    }
    fn owns(&self, i: &Id) -> bool {
        self.0.borrow().owns(i)
    }
    fn forget_exited(&mut self, i: &Id) -> Result<()> {
        self.0.borrow_mut().forget_exited(i)
    }
    fn exited(&mut self, i: &Id) -> Result<Option<Option<i32>>> {
        self.0.borrow_mut().exited(i)
    }
    fn ready(&mut self, l: &Launch) -> Result<bool> {
        self.0.borrow_mut().ready(l)
    }
    fn terminate(&mut self, i: &Id, f: bool) -> Result<()> {
        self.0.borrow_mut().terminate(i, f)
    }
    fn observe_status(
        &mut self,
        l: &Launch,
        r: &StatusObservationRequest,
    ) -> Result<StatusObservationResult> {
        self.0.borrow_mut().observe_status(l, r)
    }
}
// Hook runs only after a new result has been staged and the product's logical
// cut passed. It models rollback, delayed commit IO, and response loss precisely.
pub type Hook = Box<dyn FnOnce() -> rx_ports::Result<bool>>;
#[derive(Default)]
pub struct Fault {
    pub operation: Option<Id>,
    pub after_cut: Option<Hook>,
}
pub struct Store {
    pub inner: SqliteRepository,
    pub fault: Rc<RefCell<Fault>>,
}
impl Repository for Store {
    fn transact<T>(
        &mut self,
        f: impl FnOnce(&mut dyn Transaction) -> rx_ports::Result<T>,
    ) -> rx_ports::Result<T> {
        let fault = self.fault.clone();
        let mut lose_reply = false;
        let value = self.inner.transact(|tx| {
            let value = f(tx)?;
            let operation = fault.borrow().operation.clone();
            if let Some(operation) = operation
                && tx.get(&n(&format!("work/result/{operation}")))?.is_some()
            {
                let hook = fault.borrow_mut().after_cut.take();
                if let Some(hook) = hook {
                    lose_reply = hook()?;
                }
            }
            Ok(value)
        })?;
        if lose_reply {
            Err(rx_ports::StoreError::Unavailable(
                "test response lost after successful commit".into(),
            ))
        } else {
            Ok(value)
        }
    }
    fn pending_outbox_after(
        &mut self,
        a: Option<&Id>,
        l: usize,
    ) -> rx_ports::Result<Vec<rx_ports::OutboxRecord>> {
        self.inner.pending_outbox_after(a, l)
    }
    fn control_events_after(
        &mut self,
        a: Counter,
        l: usize,
    ) -> rx_ports::Result<Vec<rx_ports::StoredEvent>> {
        self.inner.control_events_after(a, l)
    }
    fn control_snapshot(&mut self) -> rx_ports::Result<(Counter, Vec<rx_ports::Record>)> {
        self.inner.control_snapshot()
    }
    fn journal_head(&mut self) -> rx_ports::Result<Counter> {
        self.inner.journal_head()
    }
    fn pending_outbox(&mut self, l: usize) -> rx_ports::Result<Vec<rx_ports::OutboxRecord>> {
        self.inner.pending_outbox(l)
    }
    fn snapshot(&mut self) -> rx_ports::Result<(Counter, Vec<rx_ports::Record>)> {
        self.inner.snapshot()
    }
    fn events_after(
        &mut self,
        a: Counter,
        l: usize,
    ) -> rx_ports::Result<Vec<rx_ports::StoredEvent>> {
        self.inner.events_after(a, l)
    }
}
pub type Managed = RegisteredSupervisor<SqliteRepository, Owned, SoftwareOnly, Store>;
pub struct Fixture {
    pub dir: tempfile::TempDir,
    pub managed: Option<Managed>,
    pub fault: Rc<RefCell<Fault>>,
    pub component: Id,
    pub program: Program,
    pub plan: Plan,
    pub installed: bool,
    owner: Owned,
}
impl Fixture {
    pub fn new(policy: Option<Policy>, installed: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("report.py");
        let data = dir.path().join("counts.json");
        std::fs::write(&data, r#"{"native_packages":10,"support_profiles":4}"#).unwrap();
        std::fs::write(&script,r#"import json,os,sys
from pathlib import Path
from http.server import BaseHTTPRequestHandler,HTTPServer
class H(BaseHTTPRequestHandler):
 def log_message(self,*a):pass
 def do_GET(self):
  v=json.loads(Path(sys.argv[1]).read_text());v.update(schema='rx.solutions-status.v1',phase='SOFTWARE_READY_UNCOMMISSIONED',supervisor_instance=os.environ['RX_PROCESS_INSTANCE_ID'])
  b=json.dumps(v).encode();self.send_response(200);self.send_header('Content-Length',str(len(b)));self.end_headers();self.wfile.write(b)
HTTPServer(('127.0.0.1',int(sys.argv[3])),H).serve_forever()
"#).unwrap();
        let python = PathBuf::from("/usr/bin/python3");
        let conditions = [
            (n("instance"), ReadinessCondition::InstanceMatches),
            (
                n("schema"),
                ReadinessCondition::Equals {
                    field: n("schema"),
                    expected: ExpectedValue::Text("rx.solutions-status.v1".into()),
                    origin: ReportOrigin::ReleaseDeclaration,
                },
            ),
            (
                n("native"),
                ReadinessCondition::Unsigned {
                    field: n("native_packages"),
                    origin: ReportOrigin::StartupAuditDerived,
                },
            ),
            (
                n("profiles"),
                ReadinessCondition::Unsigned {
                    field: n("support_profiles"),
                    origin: ReportOrigin::StartupCatalogDerived,
                },
            ),
        ]
        .into();
        let mut program = if installed {
            rx_supervisor::builtin::release_programs(Path::new("/opt/rx"))
                .unwrap()
                .remove(&n("rx/status-http"))
                .unwrap()
        } else {
            Program {
                id: n("test/work-source"),
                effect: Effect::NonActuating,
                executable: python.clone(),
                executable_sha256: hash(&python),
                files: [(script.clone(), hash(&script))].into(),
                fixed_arguments: vec![
                    script.to_string_lossy().into(),
                    data.to_string_lossy().into(),
                ],
                arguments: [(
                    n("port"),
                    Argument::Port {
                        flag: "--port".into(),
                    },
                )]
                .into(),
                ready: ReadyProbe::HttpStatus {
                    port_parameter: n("port"),
                },
                execution_requirements: Some(execution::Requirements(BTreeMap::new())),
                functional_readiness: Some(ReadinessContract(
                    [(
                        n("diagnostics/support-summary"),
                        ReadinessProfile {
                            endpoint: StatusEndpoint::SupportSummary,
                            conditions,
                        },
                    )]
                    .into(),
                )),
                decision_policy: None,
            }
        };
        // Test-authored policy only. The shipping catalog remains unmodified.
        program.decision_policy = policy;
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut parameters: BTreeMap<_, _> = [(n("port"), port.to_string())].into();
        if installed {
            parameters.insert(n("bind"), "127.0.0.1".into());
        }
        let plan = Plan {
            schema: n("rx.solutions-process-plan.v1"),
            id: id(),
            environment: Environment::Simulation,
            profiles: vec![],
            processes: vec![Process {
                id: n("status"),
                program: program.id.clone(),
                parameters,
                depends_on: vec![],
                startup_timeout_ms: Counter(5000),
                shutdown_timeout_ms: Counter(1000),
                restart_limit: Counter(0),
                restart_backoff_ms: Counter(100),
            }],
        };
        let fault = Rc::new(RefCell::new(Fault::default()));
        let mut registry = Registry::new(Store {
            inner: SqliteRepository::open(dir.path().join("registration.db")).unwrap(),
            fault: fault.clone(),
        });
        let component = registry
            .register(Declaration {
                label: n("status"),
                catalog: catalog_reference(&program).unwrap(),
            })
            .unwrap()
            .registration
            .id;
        let owner = Owned(Rc::new(RefCell::new(
            OsProcesses::new(dir.path().join("logs")).unwrap(),
        )));
        let managed = RegisteredSupervisor::open(
            SqliteRepository::open(dir.path().join("supervisor.db")).unwrap(),
            owner.clone(),
            SoftwareOnly,
            plan.clone(),
            [(program.id.clone(), program.clone())].into(),
            &rx_solution_catalog::DeviceCatalog::decode(include_bytes!(
                "../../../../catalogs/device-support.v1.json"
            ))
            .unwrap(),
            registry,
            component.clone(),
        )
        .unwrap();
        let mut this = Self {
            dir,
            managed: Some(managed),
            fault,
            component,
            program,
            plan,
            installed,
            owner,
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let state = this.manager().tick().unwrap();
            if state.state.records[&n("status")].phase == Phase::ProcessReady {
                break;
            }
            assert!(Instant::now() < deadline, "{:?}", state.blocked);
            std::thread::sleep(Duration::from_millis(10));
        }
        this
    }
    pub fn manager(&mut self) -> &mut Managed {
        self.managed.as_mut().unwrap()
    }
    pub fn task(&self, native: u64, profiles: u64) -> Task {
        Task {
            operation: id(),
            selection: n("status"),
            operating_area: n("test/diagnostics"),
            required_native_packages: Counter(native),
            required_support_profiles: Counter(profiles),
        }
    }
    pub fn counts(&self, native: u64, profiles: u64) {
        assert!(!self.installed);
        std::fs::write(
            self.dir.path().join("counts.json"),
            format!("{{\"native_packages\":{native},\"support_profiles\":{profiles}}}"),
        )
        .unwrap();
    }
    pub fn hook(&self, operation: Id, hook: Hook) {
        *self.fault.borrow_mut() = Fault {
            operation: Some(operation),
            after_cut: Some(hook),
        };
    }
    pub fn observed_counts(&self) -> (u64, u64) {
        let port = &self.plan.processes[0].parameters[&n("port")];
        let output=std::process::Command::new("/usr/bin/python3").args(["-I","-B","-c",
            &format!("import json,urllib.request; print(json.dumps(json.load(urllib.request.urlopen('http://127.0.0.1:{port}/api/v1/solution/support'))))")]).output().unwrap();
        assert!(output.status.success());
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        (
            value["native_packages"].as_u64().unwrap(),
            value["support_profiles"].as_u64().unwrap(),
        )
    }
    pub fn reopen(&mut self) {
        let (store, backend, authority, registry) = self.managed.take().unwrap().into_parts();
        drop(store);
        self.managed = Some(
            RegisteredSupervisor::open(
                SqliteRepository::open(self.dir.path().join("supervisor.db")).unwrap(),
                backend,
                authority,
                self.plan.clone(),
                [(self.program.id.clone(), self.program.clone())].into(),
                &rx_solution_catalog::DeviceCatalog::decode(include_bytes!(
                    "../../../../catalogs/device-support.v1.json"
                ))
                .unwrap(),
                registry,
                self.component.clone(),
            )
            .unwrap(),
        );
    }
    pub fn rows(&mut self) -> Vec<rx_ports::Record> {
        self.manager()
            .history()
            .unwrap()
            .iter()
            .map(|e| serde_json::from_value(e.document.value["entity"].clone()).unwrap())
            .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let instances = self.owner.0.borrow().owned_instances();
        for instance in instances {
            let _ = self.owner.0.borrow_mut().terminate(&instance, true);
            let end = Instant::now() + Duration::from_secs(3);
            while Instant::now() < end {
                if matches!(self.owner.0.borrow_mut().exited(&instance), Ok(Some(_))) {
                    let _ = self.owner.0.borrow_mut().forget_exited(&instance);
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
