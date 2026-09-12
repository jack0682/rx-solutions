#![cfg(unix)]
use rx_domain::types::*;
use rx_host::{
    Clock,
    ros_jtc::{
        process::{Executable, Process},
        protocol::*,
    },
    simulation::ManualClock,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicU64},
    time::Duration,
};
const SCRIPT: &str = r#"#!/usr/bin/python3
import sys,json,time
c=json.load(open(sys.argv[1]));instance='11111111-1111-4111-8111-111111111111'
def reply(seq,state,value,owner=instance):
 print(json.dumps({'schema':'rx.ros-jtc-reply.v1','bridge_instance':owner,'sequence':seq,'clock_id':'test/pipe','ticks_ns':'1000','state':state,'value':value,'fault':None}),flush=True)
reply(None,'READY',{'support_id':c['support_id'],'model':'omy_f3m','controller':c['controller'],'action':c['namespace']+'/'+c['controller']+'/follow_joint_trajectory','joints':['joint'+str(i) for i in range(1,7)],'source_observed_only':True,'catalog_sha256':c['catalog_sha256'],'controller_generation_known':False})
for line in sys.stdin:
 r=json.loads(line)
 if '/hang/' in c['controller_manager']:time.sleep(.4)
 if '/oversize/' in c['controller_manager']:print('x'*1048577,flush=True);continue
 owner='22222222-2222-4222-8222-222222222222' if '/wrong/' in c['controller_manager'] else instance
 reply(r['sequence'],'OBSERVED',{},owner)
"#;
fn config(mode: &str) -> Configuration {
    Configuration {
        schema: Name::new("rx.ros-jtc-bridge.v1").unwrap(),
        catalog_sha256: catalog_digest(),
        support_id: Name::new("OM-06").unwrap(),
        controller: "arm_controller".into(),
        namespace: "/robot".into(),
        controller_manager: format!("/{mode}/controller_manager"),
        domain_id: 171,
        timeout_ms: 50,
        capacity: 32,
    }
}
fn clock() -> ManualClock {
    ManualClock {
        clock_id: "test/pipe".into(),
        ticks: Arc::new(AtomicU64::new(1000)),
    }
}
fn spawn(root: &std::path::Path, mode: &str) -> Process<ManualClock> {
    let path = root.join("script");
    std::fs::write(&path, SCRIPT).unwrap();
    Process::spawn(
        Executable {
            path,
            sha256: rx_package::content_digest(SCRIPT.as_bytes()),
            environment: BTreeMap::new(),
        },
        &config(mode),
        root,
        clock(),
    )
    .unwrap()
}
#[test]
fn owned_executable_copy_and_bounded_pipes_preserve_context_and_close_without_kill() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = spawn(dir.path(), "ok");
    std::fs::write(dir.path().join("script"), b"changed original").unwrap();
    let until = TimePoint {
        clock_id: clock().now().clock_id,
        ticks_ns: Counter(50_001_000),
    };
    assert!(matches!(
        p.exchange("inspect", serde_json::json!({}), &until)
            .unwrap()
            .state,
        State::Observed
    ));
    for _ in 0..50 {
        if p.try_close().unwrap() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("child did not exit after EOF");
}
#[test]
fn timeout_oversize_and_wrong_instance_permanently_fault_the_pipe() {
    for mode in ["hang", "oversize", "wrong"] {
        let dir = tempfile::tempdir().unwrap();
        let mut p = spawn(dir.path(), mode);
        let until = TimePoint {
            clock_id: "test/pipe".into(),
            ticks_ns: Counter(20_001_000),
        };
        assert!(
            p.exchange("inspect", serde_json::json!({}), &until)
                .is_err()
        );
        assert!(p.is_faulted());
        assert!(
            p.exchange("inspect", serde_json::json!({}), &until)
                .is_err()
        );
    }
}
#[test]
fn executable_pin_and_configuration_are_checked_before_spawn() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("script");
    std::fs::write(&path, SCRIPT).unwrap();
    assert!(
        Process::spawn(
            Executable {
                path,
                sha256: Digest::from_bytes([0; 32]),
                environment: BTreeMap::new()
            },
            &config("ok"),
            dir.path(),
            clock()
        )
        .is_err()
    );
}
