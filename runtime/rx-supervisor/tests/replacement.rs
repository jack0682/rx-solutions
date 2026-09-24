#![cfg(unix)]
#[path = "support/external_signer.rs"]
mod external_signer;
#[path = "support/decision_fixture.rs"]
mod fixture;
#[path = "support/replacement_store.rs"]
mod storage;
use external_signer::ExternalSigner;
use fixture::*;
use rx_domain::types::*;
use rx_supervisor::{
    decision::*,
    registration::{Declaration, Registry, diagnostic::*},
    use_assessment::*,
};
use std::{
    sync::{Arc, Barrier},
    time::Duration,
};
use storage::*;
static CASES: std::sync::Mutex<()> = std::sync::Mutex::new(());
struct Issuer {
    signer: ExternalSigner,
    ttl: u64,
}
impl Issuer {
    fn new() -> Self {
        Self {
            signer: ExternalSigner::new(),
            ttl: 30_000,
        }
    }
}
impl BindingJudgment for Issuer {
    fn assess(&self, r: &AcceptanceRequest) -> AcceptanceReply {
        let challenge = r
            .decision_request()
            .unwrap()
            .challenge(&self.signer.key_id())
            .unwrap();
        let signed = self
            .signer
            .decision(challenge.claim(uid(), Counter(self.ttl)));
        AcceptanceReply::Verified(Box::new(challenge.verify(&signed).unwrap()))
    }
}
fn source() -> Fixture {
    let mut f = Fixture::new(None);
    f.ready();
    assert!(f.dir.path().is_dir());
    assert_eq!(f.program.id, f.reference.catalog.program);
    let generation = f.generation();
    assert_eq!(f.plan.id, generation.run);
    f
}
fn consumer(
    path: &std::path::Path,
    policy: Option<Policy>,
) -> (Consumer<SharedStore>, SharedStore, Id, Catalog) {
    let store = SharedStore::new(path);
    let mut registry = Registry::new(store.clone());
    let mut catalog = catalog();
    catalog.decision_policy = policy;
    let component = registry
        .register(Declaration {
            label: n("replacement/consumer"),
            catalog: catalog.reference().unwrap(),
        })
        .unwrap()
        .registration
        .id;
    (
        Consumer::open(registry, component.clone(), catalog.clone()).unwrap(),
        store,
        component,
        catalog,
    )
}
fn start(c: &mut Consumer<SharedStore>, binding: &Id, s: &mut impl Source) -> Run {
    let run = c.assign(binding, s).unwrap().run.unwrap();
    c.begin(&run.binding, &run.id, s).unwrap();
    run
}
fn finish(c: &mut Consumer<SharedStore>, run: &Run, s: &mut impl Source) -> DiagnosticResult {
    c.finish(&run.binding, &run.id, s).unwrap().result.unwrap()
}
fn intent(root: &Id, b: &mut Fixture) -> ReplacementIntent {
    ReplacementIntent {
        application: uid(),
        binding: root.clone(),
        proposed_provider: b.reference.clone(),
        proposed_generation: b.generation(),
    }
}
fn application_key(component: &Id, id: &Id) -> Name {
    n(&format!(
        "components/diagnostic/application/{component}/{id}"
    ))
}
fn message(e: impl std::fmt::Display) -> String {
    let t = e.to_string();
    println!("replacement_refusal={t}");
    t
}
#[test]
fn route_switch_midflight_keeps_old_version_and_new_assignments_use_new_then_return() {
    let _case = CASES.lock().unwrap_or_else(|p| p.into_inner());
    let issuer = Issuer::new();
    let mut a = source();
    let mut b = source();
    let dir = tempfile::tempdir().unwrap();
    let (mut c, store, _, _) = consumer(
        &dir.path().join("consumer.db"),
        Some(issuer.signer.policy()),
    );
    let original = c
        .track(
            n("snapshot-at-generation"),
            n("test/diagnostics"),
            Some(a.reference.clone()),
        )
        .unwrap();
    // First establish the original generation. Replacement never backfills an
    // unobserved origin. The tested in-flight run still has no captured samples.
    let seed = start(&mut c, &original.id, a.manager());
    let historical = finish(&mut c, &seed, a.manager());
    let old = start(&mut c, &original.id, a.manager());
    assert!(old.samples.is_empty());
    let before = store.rows();
    let proposed = intent(&original.id, &mut b);
    let prepared = c.prepare_replacement(proposed.clone(), &issuer).unwrap();
    assert_eq!(
        store.rows(),
        before,
        "approval alone must have no application effect"
    );
    let receipt = c.apply_replacement(&prepared, b.manager()).unwrap();
    assert_ne!(receipt.next.id, original.id);
    assert_eq!(c.routing(&original.id).unwrap().active, receipt.next);
    let new = c
        .assign_current(&original.id, b.manager())
        .unwrap()
        .run
        .unwrap();
    c.begin(&new.binding, &new.id, b.manager()).unwrap();
    let old_result = finish(&mut c, &old, a.manager());
    let new_result = finish(&mut c, &new, b.manager());
    assert_eq!(
        old_result.body["samples"][0]["provider"]["registration"],
        serde_json::json!(a.reference.registration)
    );
    assert_eq!(
        new_result.body["samples"][0]["provider"]["registration"],
        serde_json::json!(b.reference.registration)
    );
    assert_eq!(c.recorded_result(&historical.id).unwrap(), historical);
    assert!(message(c.assign(&original.id, a.manager()).unwrap_err()).contains("superseded"));
    let old_view = c
        .inspect(
            &original.id,
            Some(&old.id),
            Some(&old_result.id),
            a.manager(),
        )
        .unwrap();
    assert_eq!(old_view.new_assignment.state, ConditionState::NotMet);
    assert!(old_view.checkpoints.cadence.contains("NO_TIMER"));
    assert_eq!(old_view.checkpoints.maximum_detection_delay_ms, None);
    let back = intent(&original.id, &mut a);
    let p = c.prepare_replacement(back, &issuer).unwrap();
    let returned = c.apply_replacement(&p, a.manager()).unwrap();
    assert_eq!(returned.previous.id, receipt.next.id);
    assert_eq!(returned.next.provider, Some(a.reference.clone()));
    assert_ne!(returned.next.id, original.id);
    assert_eq!(
        c.recorded_replacement(&proposed.application).unwrap(),
        receipt
    );
    println!(
        "replacement_midflight={}",
        serde_json::json!({"original":original.id,"old_run":old_result,"new_run":new_result,"application":receipt,"return":returned,"active":c.routing(&original.id).unwrap()})
    );
}
#[test]
fn partial_staging_rollback_and_lost_reply_leave_one_route_and_preserve_receipt() {
    let _case = CASES.lock().unwrap_or_else(|p| p.into_inner());
    let issuer = Issuer::new();
    let mut a = source();
    let mut b = source();
    let dir = tempfile::tempdir().unwrap();
    let (mut c, store, component, _) = consumer(
        &dir.path().join("consumer.db"),
        Some(issuer.signer.policy()),
    );
    let original = c
        .track(
            n("current-report-collection"),
            n("test/diagnostics"),
            Some(a.reference.clone()),
        )
        .unwrap();
    let old = start(&mut c, &original.id, a.manager());
    let proposed = intent(&original.id, &mut b);
    let p = c.prepare_replacement(proposed.clone(), &issuer).unwrap();
    let before = store.rows();
    store.fail_after_put(&format!("components/diagnostic/binding/{component}/"));
    assert!(message(c.apply_replacement(&p, b.manager()).unwrap_err()).contains("rollback"));
    assert_eq!(store.rows(), before);
    assert_eq!(c.routing(&original.id).unwrap().active.id, original.id);
    store.hook(
        application_key(&component, &proposed.application),
        Box::new(|| Ok(true)),
    );
    assert!(message(c.apply_replacement(&p, b.manager()).unwrap_err()).contains("reply lost"));
    let receipt = c.recorded_replacement(&proposed.application).unwrap();
    assert_eq!(c.routing(&original.id).unwrap().active, receipt.next);
    assert!(c.apply_replacement(&p, b.manager()).is_err());
    let routes = store
        .rows()
        .into_iter()
        .filter(|r| r.key.as_str().contains("/route/"))
        .count();
    assert_eq!(routes, 1);
    assert_eq!(finish(&mut c, &old, a.manager()).binding, original.id);
    println!(
        "replacement_faults={}",
        serde_json::json!({"rollback_unchanged":true,"lost_reply_recovered":receipt,"active_routes":routes})
    );
}
#[test]
fn authority_expiry_revocation_restart_and_candidate_generation_are_rechecked() {
    let _case = CASES.lock().unwrap_or_else(|p| p.into_inner());
    let mut issuer = Issuer::new();
    let mut a = source();
    let mut b = source();
    let dir = tempfile::tempdir().unwrap();
    let (mut c, store, component, catalog) = consumer(
        &dir.path().join("consumer.db"),
        Some(issuer.signer.policy()),
    );
    let original = c
        .track(
            n("current-report-collection"),
            n("test/diagnostics"),
            Some(a.reference.clone()),
        )
        .unwrap();
    let _old = start(&mut c, &original.id, a.manager());
    let proposed = intent(&original.id, &mut b);
    assert!(
        c.prepare_replacement(proposed.clone(), &NoBindingJudgment)
            .is_err()
    );
    issuer.ttl = 1000;
    let p = c.prepare_replacement(proposed, &issuer).unwrap();
    while p.decision().check_live().is_ok() {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(message(c.apply_replacement(&p, b.manager()).unwrap_err()).contains("expired"));
    issuer.ttl = 30_000;
    let p = c
        .prepare_replacement(intent(&original.id, &mut b), &issuer)
        .unwrap();
    let rev = issuer
        .signer
        .revocation(p.decision().revocation_claim(n("test/revoke")));
    p.decision().revoke(&rev).unwrap();
    assert!(message(c.apply_replacement(&p, b.manager()).unwrap_err()).contains("revoked"));
    let p = c
        .prepare_replacement(intent(&original.id, &mut b), &issuer)
        .unwrap();
    b.restart();
    assert!(
        message(c.apply_replacement(&p, b.manager()).unwrap_err())
            .contains("candidate-unavailable")
    );
    let p = c
        .prepare_replacement(intent(&original.id, &mut b), &issuer)
        .unwrap();
    let registry = c.into_registry();
    let mut c = Consumer::open(registry, component, catalog).unwrap();
    assert_eq!(p.decision().check_live(), Err(Failure::Epoch));
    assert!(message(c.apply_replacement(&p, b.manager()).unwrap_err()).contains("epoch"));
    assert_eq!(c.routing(&original.id).unwrap().active.id, original.id);
    assert!(
        !store
            .rows()
            .iter()
            .any(|r| r.key.as_str().contains("/route/"))
    );
}
#[test]
fn two_concurrent_applications_from_one_base_cannot_both_win() {
    let _case = CASES.lock().unwrap_or_else(|p| p.into_inner());
    let issuer = Issuer::new();
    let mut a = source();
    let mut b = source();
    let dir = tempfile::tempdir().unwrap();
    let (mut c1, store, component, catalog) = consumer(
        &dir.path().join("consumer.db"),
        Some(issuer.signer.policy()),
    );
    let root = c1
        .track(
            n("current-report-collection"),
            n("test/diagnostics"),
            Some(a.reference.clone()),
        )
        .unwrap();
    let _old = start(&mut c1, &root.id, a.manager());
    let mut c2 = Consumer::open(Registry::new(store.clone()), component.clone(), catalog).unwrap();
    let p1 = c1
        .prepare_replacement(intent(&root.id, &mut b), &issuer)
        .unwrap();
    let p2 = c2
        .prepare_replacement(intent(&root.id, &mut b), &issuer)
        .unwrap();
    type Envelope = (Probe, std::sync::mpsc::Sender<SourceReply>);
    struct Bridge {
        requests: std::sync::mpsc::Sender<Envelope>,
        ready: Arc<Barrier>,
    }
    impl Source for Bridge {
        fn observe(&mut self, p: &Probe) -> SourceReply {
            let (send, recv) = std::sync::mpsc::channel();
            self.requests.send((p.clone(), send)).unwrap();
            let reply = recv.recv_timeout(Duration::from_secs(5)).unwrap();
            self.ready.wait();
            reply
        }
    }
    let (send, recv) = std::sync::mpsc::channel::<Envelope>();
    let ready = Arc::new(Barrier::new(2));
    let mut s1 = Bridge {
        requests: send.clone(),
        ready: ready.clone(),
    };
    let mut s2 = Bridge {
        requests: send,
        ready,
    };
    let t1 = std::thread::spawn(move || {
        let result = c1.apply_replacement(&p1, &mut s1);
        (c1, result)
    });
    let t2 = std::thread::spawn(move || {
        let result = c2.apply_replacement(&p2, &mut s2);
        (c2, result)
    });
    for _ in 0..2 {
        let (probe, response) = recv.recv_timeout(Duration::from_secs(5)).unwrap();
        response.send(b.manager().observe(&probe)).unwrap();
    }
    let (mut c1, r1) = t1.join().unwrap();
    let (_, r2) = t2.join().unwrap();
    assert_ne!(r1.is_ok(), r2.is_ok());
    let failure = if let Err(e) = r1 { e } else { r2.unwrap_err() };
    assert!(message(failure).contains("CAS-conflict"));
    let rows = store.rows();
    assert_eq!(
        rows.iter()
            .filter(|r| r.key.as_str().contains("/route/"))
            .count(),
        1
    );
    assert_eq!(
        rows.iter()
            .filter(|r| r.key.as_str().contains("/application/"))
            .count(),
        1
    );
    assert_eq!(
        rows.iter()
            .filter(|r| r.key.as_str().contains("/replacement-consumed/"))
            .count(),
        1
    );
    println!(
        "concurrent_replacement_single_winner={}",
        serde_json::to_string(&c1.routing(&root.id).unwrap()).unwrap()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn manager_loss_during_replacement_recovers_old_or_new_without_ownership() {
    use rustix::process::{Pid, PidfdFlags, Signal, WaitOptions};
    use std::{
        path::PathBuf,
        process::{Child, Command, Stdio},
        time::Instant,
    };
    const PHASE: &str = "RX_REPLACEMENT_FAULT_PHASE";
    const DATA: &str = "RX_REPLACEMENT_FAULT_DATA";
    if let Ok(phase) = std::env::var(PHASE) {
        let root = PathBuf::from(std::env::var_os(DATA).unwrap());
        let issuer = Issuer::new();
        let mut a = source();
        let mut b = source();
        let (mut c, store, component, _) =
            consumer(&root.join("consumer.db"), Some(issuer.signer.policy()));
        let binding = c
            .track(
                n("current-report-collection"),
                n("test/diagnostics"),
                Some(a.reference.clone()),
            )
            .unwrap();
        let old = start(&mut c, &binding.id, a.manager());
        let intent = intent(&binding.id, &mut b);
        let prepared = c.prepare_replacement(intent.clone(), &issuer).unwrap();
        let public = issuer.signer.policy().authorities[&issuer.signer.key_id()].public_key;
        let marker = serde_json::json!({"phase":phase,"component":component,"root":binding.id,"application":intent.application,
            "run":old,"public_key":public,"provider_pids":[a.generation().pid,b.generation().pid],
            "prepared_history":prepared,"scope":"consumer application manager and owned test sources; no production ownership restored"});
        let pause = move || {
            std::fs::write(
                root.join("marker.tmp"),
                serde_json::to_vec(&marker).unwrap(),
            )
            .unwrap();
            std::fs::rename(root.join("marker.tmp"), root.join("marker.json")).unwrap();
            loop {
                std::thread::sleep(Duration::from_secs(1));
            }
        };
        if phase == "before" {
            store.hook(
                application_key(&component, &intent.application),
                Box::new(pause),
            );
            c.apply_replacement(&prepared, b.manager()).unwrap();
        } else {
            c.apply_replacement(&prepared, b.manager()).unwrap();
            pause().unwrap();
        }
        unreachable!("the parent must kill this actual manager at the checkpoint");
    }
    let _case = CASES.lock().unwrap_or_else(|p| p.into_inner());
    // Fixture-only orphan cleanup. This is not an RX process ownership/adoption API.
    struct Reaper(Option<Pid>);
    impl Drop for Reaper {
        fn drop(&mut self) {
            let _ = rustix::process::set_child_subreaper(self.0);
        }
    }
    let _reaper = Reaper(rustix::process::child_subreaper().unwrap());
    rustix::process::set_child_subreaper(Pid::from_raw(1)).unwrap();
    struct Worker(Child);
    impl Drop for Worker {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut scenes = vec![];
    for phase in ["before", "after"] {
        let dir = tempfile::tempdir().unwrap();
        let log = std::fs::File::create(dir.path().join("manager.log")).unwrap();
        let mut worker = Worker(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "manager_loss_during_replacement_recovers_old_or_new_without_ownership",
                    "--nocapture",
                ])
                .env(PHASE, phase)
                .env(DATA, dir.path())
                .stdout(Stdio::from(log.try_clone().unwrap()))
                .stderr(Stdio::from(log))
                .spawn()
                .unwrap(),
        );
        let end = Instant::now() + Duration::from_secs(20);
        while !dir.path().join("marker.json").exists() {
            assert!(
                worker.0.try_wait().unwrap().is_none(),
                "manager failed before checkpoint: {}",
                std::fs::read_to_string(dir.path().join("manager.log")).unwrap()
            );
            assert!(Instant::now() < end, "checkpoint timeout");
            std::thread::sleep(Duration::from_millis(10));
        }
        let marker: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("marker.json")).unwrap())
                .unwrap();
        let pids: Vec<u32> = serde_json::from_value(marker["provider_pids"].clone()).unwrap();
        // Open observer handles before loss, solely to clean up the fixture.
        let handles: Vec<_> = pids
            .iter()
            .map(|p| {
                rustix::process::pidfd_open(Pid::from_raw(*p as i32).unwrap(), PidfdFlags::empty())
                    .unwrap()
            })
            .collect();
        let manager_pid = worker.0.id();
        worker.0.kill().unwrap();
        assert!(!worker.0.wait().unwrap().success());
        let component: Id = serde_json::from_value(marker["component"].clone()).unwrap();
        let root: Id = serde_json::from_value(marker["root"].clone()).unwrap();
        let application: Id = serde_json::from_value(marker["application"].clone()).unwrap();
        let old: Run = serde_json::from_value(marker["run"].clone()).unwrap();
        // Test-authored public policy reconstruction for reading the same catalog;
        // no private key is loaded and no new positive is requested after loss.
        let template = ExternalSigner::new();
        let mut policy = template.policy();
        policy
            .authorities
            .get_mut(&template.key_id())
            .unwrap()
            .public_key = serde_json::from_value(marker["public_key"].clone()).unwrap();
        let mut catalog = catalog();
        catalog.decision_policy = Some(policy);
        let store = SharedStore::new(dir.path().join("consumer.db"));
        let mut recovered =
            Consumer::open(Registry::new(store.clone()), component, catalog).unwrap();
        let route = recovered.routing(&root).unwrap();
        let receipt = recovered.recorded_replacement(&application);
        if phase == "before" {
            assert_eq!(route.generation, Counter(0));
            assert_eq!(route.active.id, root);
            assert!(receipt.is_err());
        } else {
            let receipt = receipt.unwrap();
            assert_eq!(route.generation, Counter(1));
            assert_eq!(route.active, receipt.next);
            assert!(receipt.current_authority.contains("NONE"));
        }
        assert!(
            recovered
                .assign_current(&root, &mut NoSource)
                .unwrap()
                .run
                .is_none(),
            "saved routing must not restore an available source"
        );
        let view = recovered
            .inspect(&old.binding, Some(&old.id), None, &mut NoSource)
            .unwrap();
        assert_eq!(view.run.unwrap().binding, old.binding);
        assert!(
            store
                .rows()
                .iter()
                .filter(|r| r.key.as_str().contains("/route/"))
                .count()
                <= 1
        );
        scenes.push(serde_json::json!({"phase":phase,"killed_manager_pid":manager_pid,"observer_pid":std::process::id(),"route":route,"old_run":old.id,"source_ownership":"NOT_RESTORED"}));
        for (pid, handle) in pids.into_iter().zip(handles) {
            let _ = rustix::process::pidfd_send_signal(&handle, Signal::KILL);
            let end = Instant::now() + Duration::from_secs(3);
            loop {
                match rustix::process::waitpid(Pid::from_raw(pid as i32), WaitOptions::NOHANG) {
                    Ok(Some(_)) | Err(rustix::io::Errno::CHILD) => break,
                    Ok(None) => {
                        assert!(Instant::now() < end);
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("fixture reap: {e}"),
                }
            }
        }
    }
    println!(
        "replacement_manager_loss={}",
        serde_json::json!({"scenes":scenes,"physical_handover":"NOT_ASSESSED"})
    );
}

#[test]
fn changed_consumer_revision_preserves_history_but_cannot_reinterpret_old_route() {
    let _case = CASES.lock().unwrap_or_else(|p| p.into_inner());
    let issuer = Issuer::new();
    let mut a = source();
    let mut b = source();
    let dir = tempfile::tempdir().unwrap();
    let (mut c, _, component, catalog) = consumer(
        &dir.path().join("consumer.db"),
        Some(issuer.signer.policy()),
    );
    let original = c
        .track(
            n("current-report-collection"),
            n("test/diagnostics"),
            Some(a.reference.clone()),
        )
        .unwrap();
    let _old = start(&mut c, &original.id, a.manager());
    let request = intent(&original.id, &mut b);
    let prepared = c.prepare_replacement(request.clone(), &issuer).unwrap();
    let receipt = c.apply_replacement(&prepared, b.manager()).unwrap();
    let mut registry = c.into_registry();
    registry
        .update(
            &component,
            Counter(1),
            Declaration {
                label: n("reviewed-new-label"),
                catalog: catalog.reference().unwrap(),
            },
        )
        .unwrap();
    let mut c = Consumer::open(registry, component, catalog).unwrap();
    assert_eq!(
        c.recorded_replacement(&request.application).unwrap(),
        receipt
    );
    assert!(
        message(
            c.prepare_replacement(intent(&original.id, &mut a), &issuer)
                .unwrap_err()
        )
        .contains("consumer-binding-revision")
    );
}

#[test]
fn old_source_loss_withholds_only_its_interval_while_new_and_independent_work_continue() {
    let _case = CASES.lock().unwrap_or_else(|p| p.into_inner());
    let issuer = Issuer::new();
    let mut a = source();
    let mut b = source();
    let dir = tempfile::tempdir().unwrap();
    let (mut c, _, _, _) = consumer(
        &dir.path().join("consumer.db"),
        Some(issuer.signer.policy()),
    );
    let original = c
        .track(
            n("current-report-collection"),
            n("test/diagnostics"),
            Some(a.reference.clone()),
        )
        .unwrap();
    let completed = start(&mut c, &original.id, a.manager());
    let historical = finish(&mut c, &completed, a.manager());
    let old = start(&mut c, &original.id, a.manager());
    let before = c
        .inspect(&old.binding, Some(&old.id), None, a.manager())
        .unwrap()
        .run
        .unwrap();
    let p = c
        .prepare_replacement(intent(&original.id, &mut b), &issuer)
        .unwrap();
    c.apply_replacement(&p, b.manager()).unwrap();
    let independent = c
        .track(n("catalog-summary"), n("test/diagnostics"), None)
        .unwrap();
    let independent_run = start(&mut c, &independent.id, &mut NoSource);
    a.manager().request_stop().unwrap();
    let end = std::time::Instant::now() + Duration::from_secs(4);
    while !a.manager().tick().unwrap().all_exited {
        assert!(std::time::Instant::now() < end);
        std::thread::sleep(Duration::from_millis(10));
    }
    let withheld = c.poll(&old.binding, &old.id, a.manager()).unwrap();
    assert_ne!(withheld.assessment.state, ConditionState::Satisfied);
    assert_eq!(withheld.run.unwrap(), before);
    assert!(
        c.finish(&old.binding, &old.id, a.manager())
            .unwrap()
            .result
            .is_none()
    );
    assert!(
        c.consume_result(&historical.id, a.manager())
            .unwrap()
            .result
            .is_none()
    );
    assert_eq!(c.recorded_result(&historical.id).unwrap(), historical);
    let new = c
        .assign_current(&original.id, b.manager())
        .unwrap()
        .run
        .unwrap();
    c.begin(&new.binding, &new.id, b.manager()).unwrap();
    let new_result = finish(&mut c, &new, b.manager());
    let unaffected = finish(&mut c, &independent_run, &mut NoSource);
    assert_eq!(
        new_result.body["samples"][0]["provider"]["registration"],
        serde_json::json!(b.reference.registration)
    );
    assert_eq!(unaffected.profile, n("catalog-summary"));
    println!(
        "replacement_loss_impact={}",
        serde_json::json!({"old_withheld":true,"old_result_preserved":historical.id,"new_completed":new_result.id,"independent_completed":unaffected.id,"detection":"ONLY_AT_EXPLICIT_REQUIRED_CHECKPOINT"})
    );
}

#[test]
fn default_catalog_does_not_apply_a_replacement_or_block_diagnostic_completion() {
    let _case = CASES.lock().unwrap_or_else(|p| p.into_inner());
    let mut a = source();
    let mut b = source();
    let dir = tempfile::tempdir().unwrap();
    let (mut c, store, _, _) = consumer(&dir.path().join("consumer.db"), None);
    let binding = c
        .track(
            n("current-report-collection"),
            n("test/diagnostics"),
            Some(a.reference.clone()),
        )
        .unwrap();
    let run = start(&mut c, &binding.id, a.manager());
    let before = store.rows();
    assert!(
        message(
            c.prepare_replacement(intent(&binding.id, &mut b), &NoBindingJudgment)
                .unwrap_err()
        )
        .contains("author-policy-absent")
    );
    assert_eq!(store.rows(), before);
    assert_eq!(c.routing(&binding.id).unwrap().active.id, binding.id);
    assert_eq!(
        finish(&mut c, &run, a.manager()).purpose,
        Purpose::DiagnosticOnly
    );
}

#[test]
fn assignment_observed_before_switch_cannot_commit_against_superseded_route() {
    let _case = CASES.lock().unwrap_or_else(|p| p.into_inner());
    let issuer = Issuer::new();
    let mut a = source();
    let mut b = source();
    let dir = tempfile::tempdir().unwrap();
    let (mut c, store, component, catalog) = consumer(
        &dir.path().join("consumer.db"),
        Some(issuer.signer.policy()),
    );
    let binding = c
        .track(
            n("current-report-collection"),
            n("test/diagnostics"),
            Some(a.reference.clone()),
        )
        .unwrap();
    let seed = start(&mut c, &binding.id, a.manager());
    finish(&mut c, &seed, a.manager());
    let mut other = Consumer::open(Registry::new(store.clone()), component, catalog).unwrap();
    let prepared = other
        .prepare_replacement(intent(&binding.id, &mut b), &issuer)
        .unwrap();
    struct Interleave<'a> {
        old: &'a mut fixture::Managed,
        new: &'a mut fixture::Managed,
        consumer: &'a mut Consumer<SharedStore>,
        prepared: &'a PreparedReplacement,
    }
    impl Source for Interleave<'_> {
        fn observe(&mut self, p: &Probe) -> SourceReply {
            let observed = self.old.observe(p);
            self.consumer
                .apply_replacement(self.prepared, self.new)
                .unwrap();
            observed
        }
    }
    let mut interleave = Interleave {
        old: a.manager(),
        new: b.manager(),
        consumer: &mut other,
        prepared: &prepared,
    };
    assert!(
        message(c.assign_current(&binding.id, &mut interleave).unwrap_err()).contains("superseded")
    );
    assert_eq!(
        store
            .rows()
            .iter()
            .filter(|r| r.key.as_str().contains("/run/"))
            .count(),
        1
    );
    let current = c.routing(&binding.id).unwrap();
    let next = c
        .assign_current(&binding.id, b.manager())
        .unwrap()
        .run
        .unwrap();
    assert_eq!(next.binding, current.active.id);
    println!("stale_assignment_refused=true");
}
