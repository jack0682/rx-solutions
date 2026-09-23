#![cfg(target_os = "linux")]
use rx_domain::types::*;
use rx_supervisor::{
    model::*,
    process::{Backend, OsProcesses},
};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
fn n(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn hash(path: &Path) -> Digest {
    Digest::from_bytes(Sha256::digest(std::fs::read(path).unwrap()).into())
}
struct Cleanup {
    backend: OsProcesses,
    finish: PathBuf,
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::write(&self.finish, b"finish");
        for instance in self.backend.owned_instances() {
            let _ = self.backend.terminate(&instance, false);
            let until = Instant::now() + Duration::from_secs(3);
            while Instant::now() < until {
                if matches!(self.backend.exited(&instance), Ok(Some(_))) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}
#[test]
fn guarded_term_targets_only_the_owned_leader_and_force_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let script = root.path().join("guarded.py");
    let finish = root.path().join("finish");
    let child_term = root.path().join("child-was-signaled");
    let child_ready = root.path().join("child-ready");
    std::fs::write(&script, r#"import os,sys,json,time,signal,subprocess
from pathlib import Path
finish,child_term,child_ready=map(Path,sys.argv[1:4])
scope=json.loads(sys.argv[4]); stopping=False; sequence=0
def stop(_sig,_frame):
 global stopping
 stopping=True
signal.signal(signal.SIGTERM,stop)
code='''import signal,time,sys
from pathlib import Path
finish,term,ready=map(Path,sys.argv[1:])
def stop(s,f):
 term.write_text("TERM received")
 raise SystemExit(1)
signal.signal(signal.SIGTERM,stop);ready.write_text("ready")
while not finish.exists():time.sleep(.01)
'''
child=subprocess.Popen([sys.executable,'-c',code,str(finish),str(child_term),str(child_ready)])
while not child_ready.exists():time.sleep(.01)
path=Path(os.environ['RX_PROCESS_STATUS_PATH'])
clock='linux-boottime/'+Path('/proc/sys/kernel/random/boot_id').read_text().strip()
def report(kind,**fields):
 global sequence
 sequence+=1
 value={'schema':'rx.protocol-guarded-status.v1','scope':scope,'instance':os.environ['RX_PROCESS_INSTANCE_ID'],'pid':os.getpid(),'sequence':str(sequence),'observed_at':{'clock_id':clock,'ticks_ns':str(time.clock_gettime_ns(time.CLOCK_BOOTTIME))},'state':{'kind':kind,**fields}}
 temporary=path.with_suffix('.tmp');temporary.write_text(json.dumps(value));temporary.replace(path)
while not (stopping and finish.exists()):
 report('STOPPING' if stopping else 'READY');time.sleep(.05)
code=child.wait()
report('STOPPED',reconciliation_required=False) if code==0 else report('ATTENTION')
raise SystemExit(code)
"#).unwrap();
    let binding = GuardedStatusBinding::Host {
        path: root.path().join("status.json"),
        installation: id(),
        host: n("host/test"),
        installation_identity: Digest::from_bytes([1; 32]),
    };
    let python = PathBuf::from("/usr/bin/python3");
    let program = Program {
        functional_readiness: None,
        decision_policy: None,
        execution_requirements: None,
        id: n("test/guarded"),
        effect: Effect::ProtocolGuardedService,
        executable: python.clone(),
        executable_sha256: hash(&python),
        files: [(script.clone(), hash(&script))].into(),
        fixed_arguments: vec![
            script.to_string_lossy().into(),
            finish.to_string_lossy().into(),
            child_term.to_string_lossy().into(),
            child_ready.to_string_lossy().into(),
            serde_json::to_string(&binding.scope()).unwrap(),
        ],
        arguments: BTreeMap::new(),
        ready: ReadyProbe::GuardedStatus(binding),
    };
    let process = Process {
        id: n("test"),
        program: program.id.clone(),
        parameters: BTreeMap::new(),
        depends_on: vec![],
        startup_timeout_ms: Counter(3000),
        shutdown_timeout_ms: Counter(100),
        restart_limit: Counter(0),
        restart_backoff_ms: Counter(100),
    };
    let launch = process.launch(&program, id()).unwrap();
    let mut owned = Cleanup {
        backend: OsProcesses::new(root.path().join("logs")).unwrap(),
        finish,
    };
    assert!(owned.backend.spawn(&launch, &mut || true).is_ok());
    let until = Instant::now() + Duration::from_secs(3);
    while !owned.backend.ready(&launch).unwrap() {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(owned.backend.terminate(&launch.instance, true).is_err());
    owned.backend.terminate(&launch.instance, false).unwrap();
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        if owned
            .backend
            .guarded_status(&launch)
            .unwrap()
            .is_some_and(|o| o.state == GuardedState::Stopping)
        {
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !child_term.exists(),
        "TERM must not reach the planner/native-child process group"
    );
    assert!(owned.backend.exited(&launch.instance).unwrap().is_none());
    std::fs::write(&owned.finish, b"finish").unwrap();
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(code) = owned.backend.exited(&launch.instance).unwrap() {
            assert_eq!(code, Some(0));
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        owned
            .backend
            .guarded_status(&launch)
            .unwrap()
            .unwrap()
            .state,
        GuardedState::Stopped {
            reconciliation_required: false
        }
    );
    owned.backend.forget_exited(&launch.instance).unwrap();
}
