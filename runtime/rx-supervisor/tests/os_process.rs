#![cfg(unix)]
use rx_domain::types::*;
use rx_solution_catalog::OwnPlatformCatalog;
use rx_storage::SqliteRepository;
use rx_supervisor::{
    Result, Supervisor,
    model::*,
    process::{Backend, OsProcesses, SpawnFailure},
};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn hash(p: &Path) -> Digest {
    Digest::from_bytes(Sha256::digest(std::fs::read(p).unwrap()).into())
}
struct TestProcesses(OsProcesses);
impl Drop for TestProcesses {
    fn drop(&mut self) {
        for id in self.0.owned_instances() {
            let _ = self.0.terminate(&id, true);
            let limit = Instant::now() + Duration::from_secs(1);
            while Instant::now() < limit {
                if matches!(self.0.exited(&id), Ok(Some(_))) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
impl Backend for TestProcesses {
    fn spawn(
        &mut self,
        l: &Launch,
        a: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<u32, SpawnFailure> {
        self.0.spawn(l, a)
    }
    fn pid(&self, id: &Id) -> Option<u32> {
        self.0.pid(id)
    }
    fn owns(&self, id: &Id) -> bool {
        self.0.owns(id)
    }
    fn forget_exited(&mut self, id: &Id) -> Result<()> {
        self.0.forget_exited(id)
    }
    fn exited(&mut self, id: &Id) -> Result<Option<Option<i32>>> {
        self.0.exited(id)
    }
    fn ready(&mut self, l: &Launch) -> Result<bool> {
        self.0.ready(l)
    }
    fn terminate(&mut self, id: &Id, force: bool) -> Result<()> {
        self.0.terminate(id, force)
    }
}
fn fixture(wrong_token: bool) -> (tempfile::TempDir, Plan, Program) {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("service.py");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let token = if wrong_token {
        "'not-this-instance'"
    } else {
        "os.environ['RX_PROCESS_INSTANCE_ID']"
    };
    std::fs::write(&script,format!(r#"import os,json,sys
from http.server import BaseHTTPRequestHandler,HTTPServer
class Handler(BaseHTTPRequestHandler):
 def log_message(self,*args):pass
 def do_GET(self):
  body=json.dumps({{'schema':'rx.solutions-status.v1','phase':'SOFTWARE_READY_UNCOMMISSIONED','supervisor_instance':{token}}}).encode()
  self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
HTTPServer(('127.0.0.1',int(sys.argv[2])),Handler).serve_forever()
"#)).unwrap();
    let python = PathBuf::from("/usr/bin/python3");
    let program = Program {
        id: name("test/http"),
        effect: Effect::NonActuating,
        executable: python.clone(),
        executable_sha256: hash(&python),
        files: [(script.clone(), hash(&script))].into_iter().collect(),
        fixed_arguments: vec![script.to_string_lossy().into_owned()],
        arguments: [(
            name("port"),
            Argument::Port {
                flag: "--port".into(),
            },
        )]
        .into_iter()
        .collect(),
        ready: ReadyProbe::HttpStatus {
            port_parameter: name("port"),
        },
    };
    let plan = Plan {
        schema: name("rx.solutions-process-plan.v1"),
        id: id(),
        environment: Environment::Simulation,
        profiles: vec![name("OM-05")],
        processes: vec![Process {
            id: name("main"),
            program: program.id.clone(),
            parameters: [(name("port"), port.to_string())].into_iter().collect(),
            depends_on: vec![],
            startup_timeout_ms: Counter(if wrong_token { 500 } else { 3000 }),
            shutdown_timeout_ms: Counter(500),
            restart_limit: Counter(0),
            restart_backoff_ms: Counter(100),
        }],
    };
    (dir, plan, program)
}
#[test]
fn real_process_readiness_is_instance_correlated_and_shutdown_reaps_the_owned_process() {
    for wrong in [false, true] {
        let (dir, plan, program) = fixture(wrong);
        let support =
            OwnPlatformCatalog::decode(include_bytes!("../../../catalogs/robotis-support.v1.json"))
                .unwrap();
        let backend = TestProcesses(OsProcesses::new(dir.path().join("logs")).unwrap());
        let mut s = Supervisor::open(
            SqliteRepository::open(dir.path().join("state.db")).unwrap(),
            backend,
            SoftwareOnly,
            plan,
            [(program.id.clone(), program)].into_iter().collect(),
            &support,
        )
        .unwrap();
        let limit = Instant::now() + Duration::from_secs(5);
        loop {
            let result = s.tick().unwrap();
            let phase = result.state.records[&name("main")].phase;
            if matches!(phase, Phase::ProcessReady | Phase::Unready) {
                assert_eq!(
                    phase,
                    if wrong {
                        Phase::Unready
                    } else {
                        Phase::ProcessReady
                    }
                );
                assert!(!result.control_prepared);
                break;
            }
            assert!(Instant::now() < limit);
            std::thread::sleep(Duration::from_millis(15));
        }
        s.request_stop().unwrap();
        let limit = Instant::now() + Duration::from_secs(3);
        loop {
            let result = s.tick().unwrap();
            if result.all_exited {
                assert!(!result.physical_shutdown_assessed);
                break;
            }
            assert!(Instant::now() < limit);
            std::thread::sleep(Duration::from_millis(15));
        }
        let (_, backend, _) = s.into_parts();
        assert!(
            backend.0.owned_instances().is_empty(),
            "confirmed exited child handles must be retired"
        );
    }
}
#[test]
fn tampered_program_file_is_not_started() {
    let (dir, plan, program) = fixture(false);
    let script = program.files.keys().next().unwrap();
    std::fs::write(script, b"raise SystemExit('changed')").unwrap();
    let support =
        OwnPlatformCatalog::decode(include_bytes!("../../../catalogs/robotis-support.v1.json"))
            .unwrap();
    let log = dir.path().join("logs");
    let mut s = Supervisor::open(
        SqliteRepository::open(dir.path().join("state.db")).unwrap(),
        TestProcesses(OsProcesses::new(log.clone()).unwrap()),
        SoftwareOnly,
        plan,
        [(program.id.clone(), program)]
            .into_iter()
            .collect::<BTreeMap<_, _>>(),
        &support,
    )
    .unwrap();
    let result = s.tick().unwrap();
    assert_eq!(
        result.state.records[&name("main")].phase,
        Phase::StartFailed
    );
    assert!(result.state.records[&name("main")].pid.is_none());
    assert_eq!(std::fs::read_dir(log).unwrap().count(), 0);
}
