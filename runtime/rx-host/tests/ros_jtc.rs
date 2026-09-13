use rx_domain::{canonical, intent::*, types::*};
use rx_host::{native::*, ros_jtc::protocol::*, ros_jtc::*, simulation::ManualClock, *};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
static SERIAL: Mutex<()> = Mutex::new(());
fn n(v: &str) -> Name {
    Name::new(v).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn d(v: u8) -> Digest {
    Digest::from_bytes([v; 32])
}
fn clock() -> ManualClock {
    ManualClock {
        clock_id: "test/jtc-clock".into(),
        ticks: Arc::new(AtomicU64::new(1_000_000_000)),
    }
}
fn profile() -> Profile {
    let bridge = Configuration {
        schema: n("rx.ros-jtc-bridge.v1"),
        catalog_sha256: catalog_digest(),
        support_id: n("SIM-JTC-6DOF"),
        controller: "arm_controller".into(),
        namespace: "/robot".into(),
        controller_manager: "/robot/controller_manager".into(),
        domain_id: 171,
        timeout_ms: 100,
        capacity: 32,
    };
    let joints = bridge.joints().unwrap();
    let tolerance = joints
        .iter()
        .map(|j| Tolerance {
            name: j.clone(),
            position: 0.1,
            velocity: 0.1,
            acceleration: 0.1,
        })
        .collect::<Vec<_>>();
    let goal = Goal {
        joints: joints.clone(),
        points: vec![Point {
            positions: vec![0.1; joints.len()],
            velocities: vec![],
            accelerations: vec![],
            time_ns: Counter(100_000_000),
        }],
        path_tolerance: tolerance.clone(),
        goal_tolerance: tolerance,
        goal_time_ns: Counter(100_000_000),
    };
    let bytes = canonical::bytes(&goal).unwrap();
    Profile {
        schema: n("rx.ros-jtc-profile.v1"),
        installation: id(),
        cell: n("cell/robot"),
        target: n("robot/arm"),
        site_config: d(1),
        calibrations: vec![d(2)],
        resources: vec![n("robot/controller")],
        environment: Environment::Simulation,
        bridge,
        conditions: [n("robot/ready")].into(),
        trajectories: vec![TrajectoryAsset {
            reference: ArtifactRef {
                sha256: rx_package::content_digest(&bytes),
                schema_id: n("rx.ros-jtc.goal.v1"),
                size_bytes: Counter(bytes.len() as u64),
            },
            joint_group: n("arm"),
            tool: d(3),
            goal,
        }],
        authority_max_age_ms: Counter(100),
    }
}
fn intent(p: &Profile) -> Intent {
    let t = &p.trajectories[0];
    Intent {
        kind: Kind::FiniteAction,
        target: p.target.clone(),
        profile_digest: p.digest().unwrap(),
        site_config_digest: p.site_config,
        calibration_digests: p.calibrations.clone(),
        resource_set: p.resources.clone(),
        execution_timeout_ms: Counter(1000),
        prepare_validity_ms: Counter(1000),
        completion_rule: n("rx.ros-jtc.terminal-result.v1"),
        cancel_rule: n("rx.ros-jtc.exact-cancel.v1"),
        body: Body::Trajectory(TrajectoryGoal {
            trajectory: t.reference.clone(),
            joint_group: t.joint_group.clone(),
            tool_digest: t.tool,
        }),
    }
}
struct Protected(AtomicBool);
impl LocalProtection for Protected {
    fn react(&self, _: ProtectionIncident) {
        self.0.store(true, Ordering::SeqCst);
    }
}
#[derive(Clone)]
struct Auth<C: Clock + Clone = ManualClock> {
    state: Arc<Mutex<AuthoritySnapshot>>,
    protection: Arc<Protected>,
    clock: C,
}
impl<C: Clock + Clone> Authority for Auth<C> {
    fn snapshot(&self) -> Result<AuthoritySnapshot> {
        let mut s = self.state.lock().unwrap().clone();
        s.observed_at = self.clock.now();
        Ok(s)
    }
    fn protection(&self) -> Arc<dyn LocalProtection> {
        self.protection.clone()
    }
}
fn authority<C: Clock + Clone>(p: &Profile, c: &C) -> Auth<C> {
    Auth {
        state: Arc::new(Mutex::new(AuthoritySnapshot {
            controller_session: id(),
            observed_at: c.now(),
            uncertainty_ns: Counter(0),
            resources: p.resources.clone(),
            conditions: p.conditions.clone(),
            exclusive_control: true,
            no_external_goals: true,
            control_available: true,
            support_stable: true,
            client_drop_allowed: true,
        })),
        protection: Arc::new(Protected(AtomicBool::new(false))),
        clock: c.clone(),
    }
}
#[derive(Default)]
struct Native {
    sent: Vec<Id>,
    status: u8,
    code: i32,
    reply_lost: bool,
    close_ready: bool,
    close_calls: u32,
    inspect_ready: bool,
}
struct Fake {
    instance: Id,
    state: Arc<Mutex<Native>>,
    clock: ManualClock,
    effects: Option<std::path::PathBuf>,
}
impl Transport for Fake {
    fn instance(&self) -> &Id {
        &self.instance
    }
    fn exchange(&mut self, command: &str, body: serde_json::Value, _: &TimePoint) -> Result<Reply> {
        let mut s = self.state.lock().unwrap();
        let t = self.clock.now();
        let (state, value) = match command {
            "inspect" => (
                State::Observed,
                serde_json::json!({"controllers":[],"selected_matches":s.inspect_ready,"observed_start_ns":t.ticks_ns,"observed_end_ns":t.ticks_ns,"controller_generation_known":false,"physical_readiness_proven":false,"send_service_ready":true,"result_service_ready":true,"cancel_service_ready":true}),
            ),
            "send" => {
                let inv: Id = serde_json::from_value(body["invocation"].clone()).unwrap();
                s.sent.push(inv.clone());
                if let Some(path) = &self.effects {
                    use std::io::Write;
                    let mut f = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                        .unwrap();
                    writeln!(f, "{inv}").unwrap();
                    f.sync_all().unwrap();
                }
                if s.reply_lost {
                    return Err(HostError::NativeUnknown("lost after send".into()));
                }
                (
                    State::SendRecorded,
                    serde_json::json!({"state":"ACCEPTED","goal_id":inv,"accepted":true,"entered_at_ns":t.ticks_ns,"captured_at_ns":t.ticks_ns,"stamp_sec":0,"stamp_nanosec":0}),
                )
            }
            "result" => (
                if s.status >= 4 {
                    State::ResultCaptured
                } else {
                    State::ResultUnknown
                },
                serde_json::json!({"goal_id":body["invocation"],"known_to_bridge":false,"ros_goal_status":s.status,"controller_error_code":s.code,"controller_error_string":"test","controller_generation_known":false,"query_started_at_ns":t.ticks_ns,"captured_at_ns":t.ticks_ns}),
            ),
            _ => panic!("unexpected command"),
        };
        Ok(Reply {
            schema: n("rx.ros-jtc-reply.v1"),
            bridge_instance: self.instance.clone(),
            sequence: Some(Counter(1)),
            clock_id: t.clock_id,
            ticks_ns: t.ticks_ns,
            state,
            value,
            fault: None,
        })
    }
    fn try_close(&mut self) -> Result<bool> {
        let mut s = self.state.lock().unwrap();
        s.close_calls += 1;
        Ok(s.close_ready)
    }
}
fn fake(c: &ManualClock, state: &Arc<Mutex<Native>>) -> Fake {
    Fake {
        instance: id(),
        state: state.clone(),
        clock: c.clone(),
        effects: None,
    }
}
fn native() -> Arc<Mutex<Native>> {
    Arc::new(Mutex::new(Native {
        inspect_ready: true,
        close_ready: true,
        ..Default::default()
    }))
}

#[test]
fn native_journal_prevents_resend_and_new_id_bypass_after_reply_loss_and_reopen() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let p = profile();
    let c = clock();
    let a = authority(&p, &c);
    let s = native();
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("native");
    let identity = Jtc::<Fake, ManualClock, Auth>::initialize(&dir, &p).unwrap();
    let op = id();
    let inv = id();
    let mut j = Jtc::open(
        &dir,
        &identity,
        p.clone(),
        fake(&c, &s),
        c.clone(),
        a.clone(),
    )
    .unwrap();
    s.lock().unwrap().reply_lost = true;
    assert!(j.submit(&op, &inv, &intent(&p)).is_err());
    assert_eq!(s.lock().unwrap().sent.len(), 1);
    assert!(j.submit(&op, &inv, &intent(&p)).is_err());
    drop(j);
    let mut j = Jtc::open(&dir, &identity, p.clone(), fake(&c, &s), c.clone(), a).unwrap();
    assert!(j.submit(&op, &inv, &intent(&p)).is_err());
    assert!(matches!(
        j.submit(&id(), &id(), &intent(&p)),
        Err(HostError::Busy)
    ));
    assert_eq!(s.lock().unwrap().sent.len(), 1);
    assert!(j.lookup(&op, &inv).unwrap().is_none());
    s.lock().unwrap().status = 4;
    let capture = j.lookup(&op, &inv).unwrap().unwrap();
    assert_eq!(capture.status_schema.as_str(), "rx.ros-jtc.succeeded.v1");
    assert_eq!(
        p.outcome_table()
            .unwrap()
            .resolve(&capture.status_schema, Integer(capture.status))
            .unwrap(),
        Some(rx_process_contract::native_outcome::NativeConclusion::Succeeded)
    );
    assert_eq!(s.lock().unwrap().sent.len(), 1);
}
#[test]
fn independent_generation_ownership_and_drop_proof_are_required() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let p = profile();
    let c = clock();
    let a = authority(&p, &c);
    let s = native();
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("native");
    let identity = Jtc::<Fake, ManualClock, Auth>::initialize(&dir, &p).unwrap();
    let mut j = Jtc::open(
        &dir,
        &identity,
        p.clone(),
        fake(&c, &s),
        c.clone(),
        a.clone(),
    )
    .unwrap();
    a.state.lock().unwrap().exclusive_control = false;
    assert!(j.guard(&intent(&p), &c.now()).is_err());
    assert!(j.submit(&id(), &id(), &intent(&p)).is_err());
    assert!(s.lock().unwrap().sent.is_empty());
    a.state.lock().unwrap().exclusive_control = true;
    let op = id();
    let inv = id();
    assert!(j.submit(&op, &inv, &intent(&p)).is_err());
    a.state.lock().unwrap().controller_session = id();
    s.lock().unwrap().status = 4;
    assert!(j.lookup(&op, &inv).unwrap().is_none());
    assert!(j.prepare_shutdown(&p.resources).is_err());
    assert_eq!(s.lock().unwrap().close_calls, 0);
}
#[test]
fn action_completion_does_not_close_bridge_until_support_and_child_exit_are_proven() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let p = profile();
    let c = clock();
    let a = authority(&p, &c);
    let s = native();
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("native");
    let identity = Jtc::<Fake, ManualClock, Auth>::initialize(&dir, &p).unwrap();
    let mut j = Jtc::open(
        &dir,
        &identity,
        p.clone(),
        fake(&c, &s),
        c.clone(),
        a.clone(),
    )
    .unwrap();
    let op = id();
    let inv = id();
    assert!(j.submit(&op, &inv, &intent(&p)).is_err());
    s.lock().unwrap().status = 4;
    j.lookup(&op, &inv).unwrap().unwrap();
    a.state.lock().unwrap().client_drop_allowed = false;
    assert!(j.prepare_shutdown(&p.resources).is_err());
    assert_eq!(s.lock().unwrap().close_calls, 0);
    a.state.lock().unwrap().client_drop_allowed = true;
    s.lock().unwrap().close_ready = false;
    assert!(j.prepare_shutdown(&p.resources).is_err());
    assert!(!j.shutdown_snapshot(&p.resources).unwrap().safe_to_drop);
    s.lock().unwrap().close_ready = true;
    j.prepare_shutdown(&p.resources).unwrap();
    assert!(j.shutdown_snapshot(&p.resources).unwrap().safe_to_drop);
    assert!(j.submit(&id(), &id(), &intent(&p)).is_err());
}
#[test]
fn contradictory_result_is_stored_without_success_capture() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let p = profile();
    let c = clock();
    let a = authority(&p, &c);
    let s = native();
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("native");
    let identity = Jtc::<Fake, ManualClock, Auth>::initialize(&dir, &p).unwrap();
    let mut j = Jtc::open(&dir, &identity, p.clone(), fake(&c, &s), c, a).unwrap();
    let op = id();
    let inv = id();
    assert!(j.submit(&op, &inv, &intent(&p)).is_err());
    {
        let mut s = s.lock().unwrap();
        s.status = 4;
        s.code = -4;
    }
    assert!(j.lookup(&op, &inv).is_err());
    let e = j.entry(&op).unwrap().unwrap();
    assert!(e.capture.is_none() && e.result.is_some() && e.issue.is_some());
    s.lock().unwrap().code = 0;
    assert!(j.lookup(&op, &inv).is_err());
    assert!(j.entry(&op).unwrap().unwrap().disputed);
}
#[test]
fn missing_authority_or_changed_artifact_never_dispatches() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let mut p = profile();
    let c = clock();
    let s = native();
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("native");
    let identity = Jtc::<Fake, ManualClock, UnavailableAuthority>::initialize(&dir, &p).unwrap();
    let mut j = Jtc::open(
        &dir,
        &identity,
        p.clone(),
        fake(&c, &s),
        c,
        UnavailableAuthority,
    )
    .unwrap();
    assert!(j.submit(&id(), &id(), &intent(&p)).is_err());
    assert!(s.lock().unwrap().sent.is_empty());
    p.trajectories[0].goal.points[0].positions[0] = 99.;
    assert!(p.validate().is_err());
}

struct ChangeAfterEntry {
    clock: ManualClock,
    state: Arc<Mutex<AuthoritySnapshot>>,
    generation: bool,
}
impl Boundary for ChangeAfterEntry {
    fn after_entry(&self) {
        if self.generation {
            self.state.lock().unwrap().controller_session = id();
        } else {
            self.clock.ticks.fetch_add(20_000_000, Ordering::SeqCst);
        }
    }
}
#[test]
fn host_dispatch_context_survives_native_journal_and_rejects_expiry_or_generation_change() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    for generation in [false, true] {
        let p = profile();
        let c = clock();
        let a = authority(&p, &c);
        let state = native();
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("native");
        let identity = Jtc::<Fake, ManualClock, Auth>::initialize(&dir, &p).unwrap();
        let context = NativeDispatch {
            device_session: a.state.lock().unwrap().controller_session.clone(),
            expires_at: TimePoint {
                clock_id: c.clock_id.clone(),
                ticks_ns: Counter(c.now().ticks_ns.0 + 10_000_000),
            },
        };
        let hook = ChangeAfterEntry {
            clock: c.clone(),
            state: a.state.clone(),
            generation,
        };
        let mut j =
            Jtc::with_boundary(&dir, &identity, p.clone(), fake(&c, &state), c, a, hook).unwrap();
        let op = id();
        assert!(
            j.submit_with_context(&op, &id(), &intent(&p), &context)
                .is_err()
        );
        assert!(j.entry(&op).unwrap().is_some());
        assert!(state.lock().unwrap().sent.is_empty());
    }
}

struct Crash(&'static str);
impl Boundary for Crash {
    fn after_entry(&self) {
        if self.0 == "entry" {
            std::process::exit(86);
        }
    }
    fn after_send(&self) {
        if self.0 == "send" {
            std::process::exit(86);
        }
    }
    fn after_capture(&self) {
        if self.0 == "capture" {
            std::process::exit(86);
        }
    }
}
#[test]
#[ignore = "child-only abrupt-exit fixture"]
fn crash_jtc_child() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let file = std::path::PathBuf::from(std::env::var("RX_JTC_CRASH_INPUT").unwrap());
    let (path, profile, identity, controller, op, inv, stage): (
        std::path::PathBuf,
        Profile,
        Identity,
        Id,
        Id,
        Id,
        String,
    ) = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    let c = clock();
    let a = authority(&profile, &c);
    a.state.lock().unwrap().controller_session = controller;
    let state = native();
    state.lock().unwrap().status = 4;
    let mut transport = fake(&c, &state);
    transport.effects = Some(path.with_extension("effects"));
    let stage = match stage.as_str() {
        "entry" => "entry",
        "send" => "send",
        "capture" => "capture",
        _ => panic!(),
    };
    let mut adapter = Jtc::with_boundary(
        &path,
        &identity,
        profile.clone(),
        transport,
        c,
        a,
        Crash(stage),
    )
    .unwrap();
    let _ = adapter.submit(&op, &inv, &intent(&profile));
    let _ = adapter.lookup(&op, &inv);
    panic!("crash hook not reached");
}
#[test]
fn abrupt_exit_at_each_native_boundary_preserves_entry_and_never_replays() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    for (stage, writes, captured) in [
        ("entry", 0, false),
        ("send", 1, false),
        ("capture", 1, true),
    ] {
        let p = profile();
        let c = clock();
        let a = authority(&p, &c);
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("native");
        let identity = Jtc::<Fake, ManualClock, Auth>::initialize(&path, &p).unwrap();
        let op = id();
        let inv = id();
        let input = root.path().join("input.json");
        std::fs::write(
            &input,
            canonical::bytes(&(
                path.clone(),
                p.clone(),
                identity.clone(),
                a.state.lock().unwrap().controller_session.clone(),
                op.clone(),
                inv.clone(),
                stage,
            ))
            .unwrap(),
        )
        .unwrap();
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "crash_jtc_child", "--ignored", "--nocapture"])
            .env("RX_JTC_CRASH_INPUT", input)
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(86),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let state = native();
        let mut j = Jtc::open(&path, &identity, p.clone(), fake(&c, &state), c, a).unwrap();
        assert_eq!(j.submit(&op, &inv, &intent(&p)).is_ok(), captured);
        assert!(state.lock().unwrap().sent.is_empty());
        let effects = std::fs::read_to_string(path.with_extension("effects")).unwrap_or_default();
        assert_eq!(effects.lines().count(), writes);
        if !captured {
            assert!(matches!(
                j.submit(&id(), &id(), &intent(&p)),
                Err(HostError::Busy)
            ));
        }
    }
}
#[test]
fn journal_identity_loss_and_corrupt_capture_cannot_be_adopted() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    use rx_ports::Repository;
    let p = profile();
    let c = clock();
    let a = authority(&p, &c);
    let state = native();
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("native");
    let identity = Jtc::<Fake, ManualClock, Auth>::initialize(&dir, &p).unwrap();
    let op = id();
    let inv = id();
    let mut j = Jtc::open(
        &dir,
        &identity,
        p.clone(),
        fake(&c, &state),
        c.clone(),
        a.clone(),
    )
    .unwrap();
    assert!(j.submit(&op, &inv, &intent(&p)).is_err());
    state.lock().unwrap().status = 4;
    j.lookup(&op, &inv).unwrap().unwrap();
    drop(j);
    let mut wrong = identity.clone();
    wrong.journal = id();
    assert!(
        Jtc::open(
            &dir,
            &wrong,
            p.clone(),
            fake(&c, &state),
            c.clone(),
            a.clone()
        )
        .is_err()
    );
    let mut store = rx_storage::SqliteRepository::open(dir.join("native.sqlite3")).unwrap();
    store
        .transact(|tx| {
            let key = journal::key("jtc-operation", &op);
            let row = tx.get(&key)?.unwrap();
            let mut e: Entry = journal::decode(&row, "rx.jtc.entry.v1")?;
            e.capture.as_mut().unwrap().status = 99;
            tx.put(
                &key,
                Some(row.revision),
                &journal::doc("rx.jtc.entry.v1", &e)?,
            )?;
            Ok(())
        })
        .unwrap();
    drop(store);
    assert!(
        Jtc::open(
            &dir,
            &identity,
            p.clone(),
            fake(&c, &state),
            c.clone(),
            a.clone()
        )
        .is_err()
    );
    std::fs::remove_file(dir.join("native.sqlite3")).unwrap();
    assert!(Jtc::open(&dir, &identity, p, fake(&c, &state), c, a).is_err());
}

#[cfg(target_os = "linux")]
fn real_process(
    p: &Profile,
    dir: &std::path::Path,
    c: &service_clock::SystemClock,
) -> ros_jtc::process::Process<service_clock::SystemClock> {
    let path = std::path::PathBuf::from(std::env::var("RX_JTC_BRIDGE_BINARY").unwrap());
    let bytes = std::fs::read(&path).unwrap();
    let environment = [
        "LD_LIBRARY_PATH",
        "AMENT_PREFIX_PATH",
        "COLCON_PREFIX_PATH",
        "RMW_IMPLEMENTATION",
        "ROS_AUTOMATIC_DISCOVERY_RANGE",
        "ROS_LOG_DIR",
    ]
    .into_iter()
    .filter_map(|k| std::env::var(k).ok().map(|v| (k.into(), v)))
    .collect();
    ros_jtc::process::Process::spawn(
        ros_jtc::process::Executable {
            path,
            sha256: rx_package::content_digest(&bytes),
            environment,
        },
        &p.bridge,
        dir,
        c.clone(),
    )
    .unwrap()
}
#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires the isolated real ROS simulator and shipped bridge binary"]
fn actual_rust_host_to_cpp_bridge_to_ros_action_preserves_original_goal() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    use std::collections::BTreeSet;
    let p = profile();
    let c = service_clock::SystemClock::new().unwrap();
    let a = authority(&p, &c);
    let mock: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var("RX_JTC_SERVER_STATE").unwrap()).unwrap(),
    )
    .unwrap();
    a.state.lock().unwrap().controller_session =
        serde_json::from_value(mock["controller_session"].clone()).unwrap();
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("native");
    let identity = Jtc::<
        ros_jtc::process::Process<service_clock::SystemClock>,
        service_clock::SystemClock,
        Auth<service_clock::SystemClock>,
    >::initialize(&directory, &p)
    .unwrap();
    let adapter = Jtc::open(
        &directory,
        &identity,
        p.clone(),
        real_process(&p, root.path(), &c),
        c.clone(),
        a,
    )
    .unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while adapter.guard(&intent(&p), &c.now()).is_err() {
        assert!(std::time::Instant::now() < until);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let b = Binding {
        host: n("host/jtc"),
        platform: n("platform"),
        cell: p.cell.clone(),
        definition: ArtifactRef {
            sha256: d(20),
            schema_id: n("rx.cell-definition.v1"),
            size_bytes: Counter(1),
        },
        envelope: ArtifactRef {
            sha256: d(21),
            schema_id: n("rx.operating-envelope.v1"),
            size_bytes: Counter(1),
        },
        qualification: id(),
        qualification_revision: Counter(1),
        allowed_intents: vec![intent(&p)],
        scope_ids: vec![n("scope/jtc")],
        condition_ids: p.conditions.iter().cloned().collect(),
        environment: Environment::Simulation,
        purposes: [Purpose::Production].into(),
    };
    let host = Host::open(
        root.path().join("host.db"),
        adapter,
        c.clone(),
        vec![b.clone()],
    )
    .unwrap();
    let who = Caller {
        peer: b.platform.clone(),
        session: id(),
    };
    host.bind_platform(who.clone()).unwrap();
    let scopes = [(n("scope/jtc"), Counter(1))].into();
    host.arm(&who, id(), &p.cell, Counter(1), &scopes, &BTreeSet::new())
        .unwrap();
    let grant = host
        .acquire_grant(
            &who,
            id(),
            p.resources.clone(),
            Counter(1),
            Counter(10_000_000_000),
        )
        .unwrap();
    let op = id();
    let i = intent(&p);
    let digest = i.digest().unwrap();
    let request = Request {
        operation: op.clone(),
        intent: i,
        digest,
        grant: grant.id.clone(),
        permit: Permit {
            id: id(),
            operation: op.clone(),
            digest,
            cell: p.cell.clone(),
            epoch: Counter(1),
            scopes,
            envelope: b.envelope.sha256,
            qualification: b.qualification,
            qualification_revision: Counter(1),
            grant: grant.id,
            host_boot: host.boot_id().unwrap(),
            conditions: p.conditions.clone(),
            expires_at: TimePoint {
                clock_id: c.now().clock_id,
                ticks_ns: Counter(c.now().ticks_ns.0 + 3_000_000_000),
            },
            purpose: Purpose::Production,
            parent: PermitParent::Mandate(id()),
            source_digest: None,
        },
    };
    let prepared = host.prepare(&who, request.clone()).unwrap();
    let invocation = prepared.invocation.unwrap();
    assert!(host.authorize(&who, request.clone(), &invocation).is_err());
    assert_eq!(
        host.authorize(&who, request, &invocation).unwrap().state,
        ReceiptState::SendEntered
    );
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if host.reconcile(&who, &op).unwrap().state == ReceiptState::ResultCaptured {
            break;
        }
        assert!(std::time::Instant::now() < until);
    }
    let evidence = host.evidence_after(&who, Counter(0), 128).unwrap();
    assert_eq!(evidence.len(), 1);
    assert_eq!(
        evidence[0].1.capture.native_id,
        Some(invocation.to_string())
    );
    let mock: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var("RX_JTC_SERVER_STATE").unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        mock["goals"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v.as_str() == Some(invocation.as_str()))
            .count(),
        1
    );
    host.request_service_stop();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if host.service_stop_snapshot().unwrap().safe_to_drop {
            break;
        }
        assert!(std::time::Instant::now() < until);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn bound_jtc_outcome_table_interprets_native_captures_without_code_zero_fallback() {
    use rx_process_contract::native_outcome::NativeConclusion;
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    for (status, code, expected) in [
        (4, 0, Some(NativeConclusion::Succeeded)),
        (5, 0, Some(NativeConclusion::Canceled)),
        (5, -4, Some(NativeConclusion::Canceled)),
        (6, 0, Some(NativeConclusion::Failed)),
        (6, -5, Some(NativeConclusion::Failed)),
        (6, 99, None),
    ] {
        let p = profile();
        let c = clock();
        let a = authority(&p, &c);
        let s = native();
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("native");
        let identity = Jtc::<Fake, ManualClock, Auth>::initialize(&dir, &p).unwrap();
        let mut j = Jtc::open(&dir, &identity, p.clone(), fake(&c, &s), c, a).unwrap();
        let op = id();
        let inv = id();
        assert!(j.submit(&op, &inv, &intent(&p)).is_err());
        {
            let mut state = s.lock().unwrap();
            state.status = status;
            state.code = code;
        }
        let capture = j.lookup(&op, &inv).unwrap().unwrap();
        let table = p.outcome_table().unwrap();
        assert_eq!(table.profile_digest, p.digest().unwrap());
        assert_eq!(table.completion_rule, intent(&p).completion_rule);
        assert_eq!(
            table
                .resolve(&capture.status_schema, Integer(capture.status))
                .unwrap(),
            expected
        );
        assert_eq!(capture.native_id, Some(inv.to_string()));
        assert_eq!(s.lock().unwrap().sent.len(), 1);
        assert_eq!(
            table
                .resolve(&n("rx.ros-jtc.accepted.v1"), Integer(0))
                .unwrap(),
            None
        );
        assert_eq!(
            table
                .resolve(&n("rx.ros-jtc.succeeded.v1"), Integer(-4))
                .unwrap(),
            None
        );
        assert_eq!(
            table
                .resolve(&n("rx.ros-jtc.goal-rejected.v1"), Integer(0))
                .unwrap(),
            Some(NativeConclusion::Failed)
        );
    }
}
