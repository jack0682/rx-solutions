use rx_domain::types::*;
use rx_solution_catalog::DeviceCatalog;
use rx_storage::SqliteRepository;
use rx_supervisor::{
    Result,
    decision::Policy,
    execution::Requirements,
    model::*,
    process::{Backend, OsProcesses, SpawnFailure},
    registered::{RegisteredSupervisor, catalog_reference},
    registration::{
        Declaration, Registry,
        diagnostic::{Generation, RegistrationRef},
    },
    use_assessment::*,
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
pub fn uid() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn hash(p: &Path) -> Digest {
    Digest::from_bytes(Sha256::digest(std::fs::read(p).unwrap()).into())
}
#[derive(Clone)]
pub struct Owned(Rc<RefCell<OsProcesses>>);
impl Backend for Owned {
    fn spawn(
        &mut self,
        l: &Launch,
        a: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<u32, SpawnFailure> {
        self.0.borrow_mut().spawn(l, a)
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
pub type Managed = RegisteredSupervisor<SqliteRepository, Owned, SoftwareOnly, SqliteRepository>;
pub struct Fixture {
    pub dir: tempfile::TempDir,
    pub managed: Option<Managed>,
    pub reference: RegistrationRef,
    pub program: Program,
    pub plan: Plan,
    backend: Owned,
}
impl Fixture {
    pub fn new(policy: Option<Policy>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("report.py");
        // Synthetic software report only; not an installed native audit.
        std::fs::write(&script,r#"import json,os,sys
from http.server import BaseHTTPRequestHandler,HTTPServer
class H(BaseHTTPRequestHandler):
 def log_message(self,*a):pass
 def do_GET(self):
  v={'schema':'rx.solutions-status.v1','phase':'SOFTWARE_READY_UNCOMMISSIONED','supervisor_instance':os.environ['RX_PROCESS_INSTANCE_ID'],'support_profiles':2,'operator_api_delegation':'NOT_CONNECTED'}
  b=json.dumps(v).encode();self.send_response(200);self.send_header('Content-Length',str(len(b)));self.end_headers();self.wfile.write(b)
HTTPServer(('127.0.0.1',int(sys.argv[2])),H).serve_forever()
"#).unwrap();
        let conditions: BTreeMap<_, _> = [
            (n("report/instance"), ReadinessCondition::InstanceMatches),
            (
                n("report/schema"),
                ReadinessCondition::Equals {
                    field: n("schema"),
                    expected: ExpectedValue::Text("rx.solutions-status.v1".into()),
                    origin: ReportOrigin::ReleaseDeclaration,
                },
            ),
        ]
        .into();
        let mut operator = conditions.clone();
        operator.insert(
            n("operator/connection"),
            ReadinessCondition::Equals {
                field: n("operator_api_delegation"),
                expected: ExpectedValue::Text("CONNECTED".into()),
                origin: ReportOrigin::ReleaseDeclaration,
            },
        );
        let python = PathBuf::from("/usr/bin/python3");
        let program = Program {
            id: n("test/decision-provider"),
            effect: Effect::NonActuating,
            executable: python.clone(),
            executable_sha256: hash(&python),
            files: [(script.clone(), hash(&script))].into(),
            fixed_arguments: vec![script.to_string_lossy().into()],
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
            execution_requirements: Some(Requirements(BTreeMap::new())),
            functional_readiness: Some(ReadinessContract(
                [
                    (
                        n("diagnostics/support-summary"),
                        ReadinessProfile {
                            endpoint: StatusEndpoint::SupportSummary,
                            conditions,
                        },
                    ),
                    (
                        n("diagnostics/operator-connected"),
                        ReadinessProfile {
                            endpoint: StatusEndpoint::SupportSummary,
                            conditions: operator,
                        },
                    ),
                ]
                .into(),
            )),
            decision_policy: policy,
        };
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let plan = Plan {
            schema: n("rx.solutions-process-plan.v1"),
            id: uid(),
            environment: Environment::Simulation,
            profiles: vec![],
            processes: vec![Process {
                id: n("provider"),
                program: program.id.clone(),
                parameters: [(n("port"), port.to_string())].into(),
                depends_on: vec![],
                startup_timeout_ms: Counter(3000),
                shutdown_timeout_ms: Counter(1000),
                restart_limit: Counter(0),
                restart_backoff_ms: Counter(100),
            }],
        };
        let mut registry =
            Registry::new(SqliteRepository::open(dir.path().join("registration.db")).unwrap());
        let registered = registry
            .register(Declaration {
                label: n("test-provider"),
                catalog: catalog_reference(&program).unwrap(),
            })
            .unwrap();
        let reference = RegistrationRef::from_registration(&registered);
        let backend = Owned(Rc::new(RefCell::new(
            OsProcesses::new(dir.path().join("logs")).unwrap(),
        )));
        let support = DeviceCatalog::decode(include_bytes!(
            "../../../../catalogs/device-support.v1.json"
        ))
        .unwrap();
        let managed = Managed::open(
            SqliteRepository::open(dir.path().join("execution.db")).unwrap(),
            backend.clone(),
            SoftwareOnly,
            plan.clone(),
            [(program.id.clone(), program.clone())].into(),
            &support,
            registry,
            registered.registration.id,
        )
        .unwrap();
        Self {
            dir,
            managed: Some(managed),
            reference,
            program,
            plan,
            backend,
        }
    }
    pub fn manager(&mut self) -> &mut Managed {
        self.managed.as_mut().unwrap()
    }
    pub fn ready(&mut self) {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let s = self.manager().tick().unwrap();
            if s.state.records[&n("provider")].phase == Phase::ProcessReady {
                return;
            }
            assert!(Instant::now() < end, "{s:?}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    pub fn restart(&mut self) {
        self.manager().request_stop().unwrap();
        let end = Instant::now() + Duration::from_secs(4);
        loop {
            if self.manager().tick().unwrap().all_exited {
                break;
            }
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(10));
        }
        self.manager().rearm_software().unwrap();
        self.ready();
    }
    pub fn generation(&mut self) -> Generation {
        let state = self.manager().state().unwrap();
        let r = &state.records[&n("provider")];
        Generation {
            run: state.plan,
            instance: r.instance.clone().unwrap(),
            pid: r.pid.unwrap(),
            configuration: state.plan_digest,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let mut b = self.backend.0.borrow_mut();
        for id in b.owned_instances() {
            let _ = b.terminate(&id, true);
            let end = Instant::now() + Duration::from_secs(2);
            while Instant::now() < end {
                if matches!(b.exited(&id), Ok(Some(_))) {
                    let _ = b.forget_exited(&id);
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
