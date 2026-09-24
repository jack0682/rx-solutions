#![cfg(target_os = "linux")]
use rx_domain::types::*;
use rx_solution_catalog::DeviceCatalog;
use rx_storage::SqliteRepository;
use rx_supervisor::{
    Error, Result, Supervisor,
    model::*,
    process::{Backend, OsProcesses, SpawnFailure},
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};
fn n(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn hash(p: &std::path::Path) -> Digest {
    rx_package::content_digest(&std::fs::read(p).unwrap())
}
struct LostDelivery {
    real: OsProcesses,
    exit: PathBuf,
    fail: bool,
}
impl Backend for LostDelivery {
    fn spawn(
        &mut self,
        l: &Launch,
        a: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<u32, SpawnFailure> {
        self.real.spawn(l, a)
    }
    fn pid(&self, i: &Id) -> Option<u32> {
        self.real.pid(i)
    }
    fn owns(&self, i: &Id) -> bool {
        self.real.owns(i)
    }
    fn forget_exited(&mut self, i: &Id) -> Result<()> {
        self.real.forget_exited(i)
    }
    fn exited(&mut self, i: &Id) -> Result<Option<Option<i32>>> {
        self.real.exited(i)
    }
    fn ready(&mut self, l: &Launch) -> Result<bool> {
        self.real.ready(l)
    }
    fn guarded_status(&mut self, l: &Launch) -> Result<Option<GuardedObservation>> {
        self.real.guarded_status(l)
    }
    fn terminate(&mut self, i: &Id, force: bool) -> Result<()> {
        assert!(!force, "guarded timeout must never escalate");
        if self.fail {
            self.fail = false;
            std::fs::write(&self.exit, b"exit")?;
            return Err(Error::Reconciliation(
                "injected failed TERM response while owned service exits".into(),
            ));
        }
        self.real.terminate(i, force)
    }
}
impl Drop for LostDelivery {
    fn drop(&mut self) {
        let _ = std::fs::write(&self.exit, b"exit");
        for i in self.real.owned_instances() {
            let end = Instant::now() + Duration::from_secs(3);
            while Instant::now() < end && !matches!(self.real.exited(&i), Ok(Some(_))) {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}
#[test]
fn failed_guarded_delivery_never_settles_without_owned_exit_and_current_final_report() {
    let mut scenes = vec![];
    for final_report in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let script = root.path().join("service.py");
        let exit = root.path().join("exit");
        std::fs::write(&script,r#"import os,sys,json,time,signal
from pathlib import Path
exit=Path(sys.argv[1]);final=sys.argv[2]=='true';scope=json.loads(sys.argv[3]);seq=0
signal.signal(signal.SIGTERM,lambda *_: exit.write_text('exit'))
path=Path(os.environ['RX_PROCESS_STATUS_PATH']);clock='linux-boottime/'+Path('/proc/sys/kernel/random/boot_id').read_text().strip()
def report(kind):
 global seq
 seq+=1;state={'kind':kind}
 if kind=='STOPPED':state['reconciliation_required']=False
 v={'schema':'rx.protocol-guarded-status.v1','scope':scope,'instance':os.environ['RX_PROCESS_INSTANCE_ID'],'pid':os.getpid(),'sequence':str(seq),'observed_at':{'clock_id':clock,'ticks_ns':str(time.clock_gettime_ns(time.CLOCK_BOOTTIME))},'state':state}
 tmp=path.with_suffix('.tmp');tmp.write_text(json.dumps(v));tmp.replace(path)
while not exit.exists():report('READY');time.sleep(.01)
if final:report('STOPPED')
"#).unwrap();
        let binding = GuardedStatusBinding::Host {
            path: root.path().join("status.json"),
            installation: id(),
            host: n("test/host"),
            installation_identity: Digest::from_bytes([1; 32]),
        };
        let python = PathBuf::from("/usr/bin/python3");
        let program = Program {
            id: n("test/guarded"),
            effect: Effect::ProtocolGuardedService,
            executable: python.clone(),
            executable_sha256: hash(&python),
            files: [(script.clone(), hash(&script))].into(),
            fixed_arguments: vec![
                script.to_string_lossy().into(),
                exit.to_string_lossy().into(),
                final_report.to_string(),
                serde_json::to_string(&binding.scope()).unwrap(),
            ],
            arguments: BTreeMap::new(),
            ready: ReadyProbe::GuardedStatus(binding),
            functional_readiness: None,
            decision_policy: None,
            execution_requirements: None,
        };
        let process = Process {
            id: n("service"),
            program: program.id.clone(),
            parameters: BTreeMap::new(),
            depends_on: vec![],
            startup_timeout_ms: Counter(3000),
            shutdown_timeout_ms: Counter(100),
            restart_limit: Counter(0),
            restart_backoff_ms: Counter(100),
        };
        let plan = Plan {
            schema: n("rx.solutions-process-plan.v1"),
            id: id(),
            environment: Environment::Simulation,
            profiles: vec![],
            processes: vec![process],
        };
        let catalog =
            DeviceCatalog::decode(include_bytes!("../../../catalogs/device-support.v1.json"))
                .unwrap();
        let backend = LostDelivery {
            real: OsProcesses::new(root.path().join("logs")).unwrap(),
            exit,
            fail: true,
        };
        let mut supervisor = Supervisor::open(
            SqliteRepository::open(root.path().join("supervisor.db")).unwrap(),
            backend,
            GuardedServices,
            plan,
            [(program.id.clone(), program)].into(),
            &catalog,
        )
        .unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let v = supervisor.tick().unwrap();
            if v.state.records[&n("service")].phase == Phase::ProcessReady {
                break;
            }
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(10));
        }
        let instance = supervisor.state().unwrap().records[&n("service")]
            .instance
            .clone();
        supervisor.request_stop().unwrap();
        let error = supervisor.tick().unwrap_err().to_string();
        assert!(error.contains("injected failed TERM"));
        let after = supervisor.state().unwrap();
        assert_eq!(after.records[&n("service")].phase, Phase::StopRequested);
        assert_eq!(after.records[&n("service")].instance, instance);
        let end = Instant::now() + Duration::from_secs(3);
        let report = loop {
            let r = supervisor.tick().unwrap();
            if r.all_exited {
                break r;
            }
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(report.guarded_shutdown_confirmed, final_report);
        assert_eq!(report.state.records[&n("service")].instance, instance);
        scenes.push(serde_json::json!({"final_report":final_report,"injected_delivery_failure":error,"status":report}));
    }
    println!("guarded_support_limit={}", serde_json::json!(scenes));
}
