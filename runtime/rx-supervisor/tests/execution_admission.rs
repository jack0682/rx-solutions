//! Positive application evidence here is explicitly simulated. Real OS probes
//! exercise rejection or declarations that request no resource enforcement.
use rx_domain::{canonical, types::*};
use rx_solution_catalog::DeviceCatalog;
use rx_storage::SqliteRepository;
use rx_supervisor::{
    Error, Result, Supervisor,
    execution::{
        Application, Capacity, Decision, Evidence, Receipt, Request, Requirement, Requirements,
        Unmet,
    },
    model::*,
    process::{Backend, OsProcesses, SpawnFailure},
};
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn support() -> DeviceCatalog {
    DeviceCatalog::decode(include_bytes!("../../../catalogs/device-support.v1.json")).unwrap()
}
fn requirements() -> Requirements {
    Requirements(
        [
            (
                name("cpu-ceiling"),
                Requirement::UpperBound {
                    resource: Capacity::CpuMillicores,
                    amount: Counter(3000),
                },
            ),
            (
                name("memory-reservation"),
                Requirement::ReservedCapacity {
                    resource: Capacity::MemoryBytes,
                    amount: Counter(4096),
                },
            ),
            (
                name("camera"),
                Requirement::ExclusiveAccess {
                    resource: name("camera/front"),
                },
            ),
            (
                name("telemetry"),
                Requirement::SharedAccess {
                    resource: name("telemetry/read"),
                },
            ),
        ]
        .into(),
    )
}
fn program(requested: Option<Requirements>) -> Program {
    Program {
        id: name("test/observer"),
        effect: Effect::NonActuating,
        executable: "/release/observer".into(),
        executable_sha256: Digest::from_bytes([1; 32]),
        files: BTreeMap::new(),
        fixed_arguments: vec![],
        arguments: BTreeMap::new(),
        ready: ReadyProbe::AliveOnly,
        execution_requirements: requested,
    }
}
fn plan() -> Plan {
    Plan {
        schema: name("rx.solutions-process-plan.v1"),
        id: id(),
        environment: Environment::Simulation,
        profiles: vec![],
        processes: vec![Process {
            id: name("observer"),
            program: name("test/observer"),
            parameters: BTreeMap::new(),
            depends_on: vec![],
            startup_timeout_ms: Counter(100),
            shutdown_timeout_ms: Counter(100),
            restart_limit: Counter(0),
            restart_backoff_ms: Counter(100),
        }],
    }
}

#[derive(Default)]
struct SimulatedState {
    spawned: Vec<Id>,
    applied: Vec<Request>,
    reservations: BTreeMap<Id, u64>,
    exclusive: BTreeMap<Name, Id>,
    shared: BTreeMap<Name, BTreeSet<Id>>,
}
#[derive(Clone, Default)]
struct Simulated {
    state: Arc<Mutex<SimulatedState>>,
    unsupported: BTreeSet<Name>,
    stale: Option<Receipt>,
    uncertain: bool,
    malformed_rejection: bool,
}
impl Backend for Simulated {
    fn spawn(
        &mut self,
        l: &Launch,
        authorize: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<u32, SpawnFailure> {
        if !authorize() {
            return Err(SpawnFailure::NotStarted(Error::Invalid(
                "startup permission unavailable".into(),
            )));
        }
        let mut state = self.state.lock().unwrap();
        state.spawned.push(l.instance.clone());
        Ok(state.spawned.len() as u32 + 100)
    }
    fn spawn_with_requirements(
        &mut self,
        l: &Launch,
        request: &Request,
        authorize: &mut dyn FnMut() -> bool,
    ) -> std::result::Result<Decision, SpawnFailure> {
        if self.malformed_rejection {
            return Ok(Decision::Rejected { unmet: vec![] });
        }
        // One lock and no application until the entire request has been assessed.
        let mut state = self.state.lock().unwrap();
        let mut unmet = vec![];
        let mut reserve = 0;
        for (key, r) in &request.requirements().0 {
            let reason = if self.unsupported.contains(key) {
                Some("fixture mechanism unavailable")
            } else {
                match r {
                    Requirement::ReservedCapacity {
                        resource: Capacity::CpuMillicores,
                        amount,
                    } => {
                        reserve += amount.0;
                        (state.reservations.values().sum::<u64>() + reserve > 4000)
                            .then_some("fixture CPU capacity exhausted")
                    }
                    Requirement::ExclusiveAccess { resource } => {
                        (state.exclusive.contains_key(resource)
                            || state.shared.get(resource).is_some_and(|s| !s.is_empty()))
                        .then_some("fixture access conflict")
                    }
                    Requirement::SharedAccess { resource } => state
                        .exclusive
                        .contains_key(resource)
                        .then_some("fixture exclusive owner exists"),
                    Requirement::Unknown => Some("unknown request"),
                    _ => None,
                }
            };
            if let Some(reason) = reason {
                unmet.push(Unmet {
                    requirement: key.clone(),
                    reason: reason.into(),
                });
            }
        }
        if !unmet.is_empty() {
            return Ok(Decision::Rejected { unmet });
        }
        if !authorize() {
            return Err(SpawnFailure::NotStarted(Error::Invalid(
                "authority lost before fixture application".into(),
            )));
        }
        if self.uncertain {
            return Err(SpawnFailure::Uncertain(Error::Reconciliation(
                "fixture lost the whole-boundary response".into(),
            )));
        }
        let receipt = self.stale.clone().unwrap_or_else(|| {
            Receipt::reported(
                request,
                Evidence::Simulation {
                    provider: name("test/atomic-admission"),
                    reference: "fixture commit; no OS enforcement".into(),
                },
            )
            .unwrap()
        });
        // Simulated application and simulated child publication occur in the same critical section.
        state.applied.push(request.clone());
        state.spawned.push(l.instance.clone());
        state.reservations.insert(l.instance.clone(), reserve);
        for r in request.requirements().0.values() {
            match r {
                Requirement::ExclusiveAccess { resource } => {
                    state.exclusive.insert(resource.clone(), l.instance.clone());
                }
                Requirement::SharedAccess { resource } => {
                    state
                        .shared
                        .entry(resource.clone())
                        .or_default()
                        .insert(l.instance.clone());
                }
                _ => {}
            }
        }
        Ok(Decision::Admitted {
            pid: state.spawned.len() as u32 + 100,
            receipt,
        })
    }
    fn pid(&self, i: &Id) -> Option<u32> {
        self.state
            .lock()
            .unwrap()
            .spawned
            .iter()
            .position(|x| x == i)
            .map(|n| n as u32 + 101)
    }
    fn owns(&self, i: &Id) -> bool {
        self.pid(i).is_some()
    }
    fn forget_exited(&mut self, _: &Id) -> Result<()> {
        Ok(())
    }
    fn exited(&mut self, _: &Id) -> Result<Option<Option<i32>>> {
        Ok(None)
    }
    fn ready(&mut self, _: &Launch) -> Result<bool> {
        Ok(true)
    }
    fn terminate(&mut self, _: &Id, _: bool) -> Result<()> {
        Ok(())
    }
}

fn open(
    backend: Simulated,
    p: Plan,
    program: Program,
) -> (
    tempfile::TempDir,
    Supervisor<SqliteRepository, Simulated, SoftwareOnly>,
) {
    let dir = tempfile::tempdir().unwrap();
    let s = Supervisor::open(
        SqliteRepository::open(dir.path().join("state.db")).unwrap(),
        backend,
        SoftwareOnly,
        p,
        [(program.id.clone(), program)].into(),
        &support(),
    )
    .unwrap();
    (dir, s)
}

#[test]
fn whole_bundle_all_available_all_unavailable_and_mixed_have_no_partial_application() {
    for unavailable in [
        vec![],
        vec!["cpu-ceiling", "memory-reservation", "camera", "telemetry"],
        vec!["camera"],
    ] {
        let backend = Simulated {
            unsupported: unavailable.iter().map(|s| name(s)).collect(),
            ..Default::default()
        };
        let observed = backend.state.clone();
        let (_dir, mut s) = open(backend, plan(), program(Some(requirements())));
        let status = s.tick().unwrap();
        let effects = observed.lock().unwrap();
        let applied = unavailable.is_empty();
        assert_eq!(effects.spawned.len(), usize::from(applied));
        assert_eq!(effects.applied.len(), usize::from(applied));
        if applied {
            assert_eq!(effects.applied[0].requirements(), &requirements());
            assert!(matches!(
                status.execution_admission[&name("observer")].application,
                Application::ReportedAtStart { .. }
            ));
        } else {
            assert!(
                effects.reservations.is_empty()
                    && effects.exclusive.is_empty()
                    && effects.shared.is_empty()
            );
            assert_eq!(
                status.state.records[&name("observer")].phase,
                Phase::StartFailed
            );
            let reasons = &status.execution_admission[&name("observer")].not_applied_reasons;
            assert_eq!(reasons.len(), unavailable.len());
            assert!(
                unavailable
                    .iter()
                    .all(|n| reasons.iter().any(|r| r.requirement == name(n)))
            );
        }
        println!(
            "D3 unavailable={unavailable:?} status={}",
            serde_json::to_string(&status.execution_admission).unwrap()
        );
    }
}

#[test]
fn upper_bounds_do_not_reserve_but_reserved_capacity_and_access_conflicts_are_atomic() {
    for reserved in [false, true] {
        let requirement = if reserved {
            Requirement::ReservedCapacity {
                resource: Capacity::CpuMillicores,
                amount: Counter(3000),
            }
        } else {
            Requirement::UpperBound {
                resource: Capacity::CpuMillicores,
                amount: Counter(3000),
            }
        };
        let mut p = plan();
        let mut second = p.processes[0].clone();
        second.id = name("second");
        p.processes.push(second);
        let backend = Simulated::default();
        let effects = backend.state.clone();
        let (_dir, mut s) = open(
            backend,
            p,
            program(Some(Requirements([(name("cpu"), requirement)].into()))),
        );
        s.tick().unwrap();
        assert_eq!(
            effects.lock().unwrap().spawned.len(),
            if reserved { 1 } else { 2 }
        );
    }
    for exclusive in [false, true] {
        let requirement = if exclusive {
            Requirement::ExclusiveAccess {
                resource: name("camera"),
            }
        } else {
            Requirement::SharedAccess {
                resource: name("camera"),
            }
        };
        let mut p = plan();
        let mut second = p.processes[0].clone();
        second.id = name("second");
        p.processes.push(second);
        let backend = Simulated::default();
        let effects = backend.state.clone();
        let (_dir, mut s) = open(
            backend,
            p,
            program(Some(Requirements([(name("access"), requirement)].into()))),
        );
        s.tick().unwrap();
        assert_eq!(
            effects.lock().unwrap().spawned.len(),
            if exclusive { 1 } else { 2 }
        );
    }
}

#[test]
fn unknown_is_rejected_even_by_a_capable_backend_and_not_required_is_distinct() {
    for unknown in [false, true] {
        let requirement = if unknown {
            Requirement::Unknown
        } else {
            Requirement::NotRequired
        };
        let backend = Simulated::default();
        let effects = backend.state.clone();
        let (_dir, mut s) = open(
            backend,
            plan(),
            program(Some(Requirements([(name("memory"), requirement)].into()))),
        );
        let status = s.tick().unwrap();
        assert_eq!(effects.lock().unwrap().spawned.len(), usize::from(!unknown));
        if unknown {
            assert!(
                status
                    .blocked
                    .iter()
                    .any(|x| x.contains("memory") && x.contains("unknown"))
            );
        }
    }
}

#[test]
fn site_overrides_and_lowered_catalog_are_rejected_in_three_distinct_paths() {
    let p = plan();
    let recipe = program(Some(requirements()));
    let catalog = [(recipe.id.clone(), recipe.clone())].into();
    let mut site = serde_json::to_value(&p).unwrap();
    site["processes"][0]["execution_requirements"] = serde_json::json!({});
    let error = canonical::decode_json::<Plan>(&serde_json::to_vec(&site).unwrap())
        .err()
        .unwrap();
    println!("D4 site declaration rejected: {error}");
    let mut parameters = p.clone();
    parameters.processes[0]
        .parameters
        .insert(name("memory-reservation"), "0".into());
    let error = parameters.validate(&catalog, &support()).unwrap_err();
    println!("D4 parameter downgrade rejected: {error}");
    let (dir, s) = open(Simulated::default(), p.clone(), recipe.clone());
    drop(s);
    for requirements in [None, Some(Requirements(BTreeMap::new()))] {
        let mut lower = recipe.clone();
        lower.execution_requirements = requirements;
        let error = Supervisor::open(
            SqliteRepository::open(dir.path().join("state.db")).unwrap(),
            Simulated::default(),
            SoftwareOnly,
            p.clone(),
            [(lower.id.clone(), lower)].into(),
            &support(),
        )
        .err()
        .expect("catalog downgrade must conflict");
        println!("D4 catalog downgrade rejected: {error}");
    }
    let mut zero = recipe;
    zero.execution_requirements = Some(Requirements(
        [(
            name("memory"),
            Requirement::ReservedCapacity {
                resource: Capacity::MemoryBytes,
                amount: Counter(0),
            },
        )]
        .into(),
    ));
    assert!(
        p.validate(&[(zero.id.clone(), zero)].into(), &support())
            .is_err()
    );
}

#[test]
fn legacy_serialization_is_unchanged_and_explicit_empty_has_its_own_identity() {
    let legacy = program(None);
    let empty = program(Some(Requirements(BTreeMap::new())));
    let p = plan();
    assert!(
        serde_json::to_value(&legacy)
            .unwrap()
            .get("execution_requirements")
            .is_none()
    );
    assert_ne!(
        p.validate(&[(legacy.id.clone(), legacy.clone())].into(), &support())
            .unwrap(),
        p.validate(&[(empty.id.clone(), empty.clone())].into(), &support())
            .unwrap()
    );
    for recipe in [legacy, empty] {
        let declared = recipe.execution_requirements.is_some();
        let backend = Simulated::default();
        let effects = backend.state.clone();
        let (_dir, mut s) = open(backend, p.clone(), recipe);
        let status = s.tick().unwrap();
        assert_eq!(effects.lock().unwrap().spawned.len(), 1);
        assert_eq!(
            status.execution_admission[&name("observer")]
                .requested
                .is_some(),
            declared
        );
    }
}

#[test]
fn diagnostic_query_does_not_enter_admission_or_spawn() {
    let backend = Simulated::default();
    let effects = backend.state.clone();
    let (_dir, mut supervisor) = open(backend, plan(), program(Some(requirements())));
    let before = serde_json::to_value(supervisor.state().unwrap()).unwrap();
    let first = serde_json::to_value(supervisor.execution_admission().unwrap()).unwrap();
    let second = serde_json::to_value(supervisor.execution_admission().unwrap()).unwrap();
    assert_eq!(first, second);
    assert_eq!(
        before,
        serde_json::to_value(supervisor.state().unwrap()).unwrap()
    );
    assert!(effects.lock().unwrap().spawned.is_empty());
    assert!(effects.lock().unwrap().applied.is_empty());
    let started = supervisor.tick().unwrap();
    assert_eq!(
        serde_json::to_value(started.execution_admission).unwrap(),
        serde_json::to_value(supervisor.execution_admission().unwrap()).unwrap()
    );
}

#[test]
fn stale_receipts_and_lost_application_responses_remain_unknown_without_retry() {
    let backend = Simulated::default();
    let (_dir, mut first) = open(backend.clone(), plan(), program(Some(requirements())));
    let first_status = first.tick().unwrap();
    let Application::ReportedAtStart { receipt } = first_status.execution_admission
        [&name("observer")]
        .application
        .clone()
    else {
        panic!()
    };
    for uncertain in [false, true] {
        let backend = Simulated {
            stale: Some(receipt.clone()),
            uncertain,
            ..Default::default()
        };
        let effects = backend.state.clone();
        let (_dir, mut s) = open(backend, plan(), program(Some(requirements())));
        let status = s.tick().unwrap();
        assert_eq!(
            status.state.records[&name("observer")].phase,
            Phase::Unknown
        );
        assert!(matches!(
            status.execution_admission[&name("observer")].application,
            Application::Unconfirmed { .. }
        ));
        let count = effects.lock().unwrap().spawned.len();
        s.tick().unwrap();
        assert_eq!(effects.lock().unwrap().spawned.len(), count);
    }
}

#[test]
fn malformed_rejection_is_not_falsely_reported_as_confirmed_non_application() {
    let backend = Simulated {
        malformed_rejection: true,
        ..Default::default()
    };
    let effects = backend.state.clone();
    let (_dir, mut supervisor) = open(backend, plan(), program(Some(requirements())));
    let result = supervisor.tick().unwrap();
    assert_eq!(
        result.state.records[&name("observer")].phase,
        Phase::Unknown
    );
    assert!(matches!(
        result.execution_admission[&name("observer")].application,
        Application::Unconfirmed { .. }
    ));
    assert!(
        result.execution_admission[&name("observer")]
            .not_applied_reasons
            .is_empty()
    );
    supervisor.tick().unwrap();
    assert!(effects.lock().unwrap().spawned.is_empty());
}

#[test]
fn reopening_does_not_restore_application_evidence_or_resource_ownership() {
    let p = plan();
    let recipe = program(Some(requirements()));
    let backend = Simulated::default();
    let effects = backend.state.clone();
    let (dir, mut s) = open(backend.clone(), p.clone(), recipe.clone());
    s.tick().unwrap();
    drop(s);
    let mut reopened = Supervisor::open(
        SqliteRepository::open(dir.path().join("state.db")).unwrap(),
        backend,
        SoftwareOnly,
        p,
        [(recipe.id.clone(), recipe)].into(),
        &support(),
    )
    .unwrap();
    let status = reopened.tick().unwrap();
    assert_eq!(
        status.state.records[&name("observer")].phase,
        Phase::Unknown
    );
    assert!(matches!(
        status.execution_admission[&name("observer")].application,
        Application::Unconfirmed { .. }
    ));
    assert_eq!(effects.lock().unwrap().spawned.len(), 1);
}

struct ChangingAuthority(AtomicUsize);
impl LifecycleAuthority for ChangingAuthority {
    fn may_start(&self, _: &Plan, _: &Process, _: &Launch) -> bool {
        self.0.fetch_add(1, Ordering::SeqCst) < 2
    }
    fn may_stop(&self, _: &Plan, _: &Process, _: &Launch) -> bool {
        false
    }
}
#[test]
fn authority_recheck_precedes_any_simulated_resource_application() {
    let dir = tempfile::tempdir().unwrap();
    let recipe = program(Some(requirements()));
    let backend = Simulated::default();
    let effects = backend.state.clone();
    let mut s = Supervisor::open(
        SqliteRepository::open(dir.path().join("state.db")).unwrap(),
        backend,
        ChangingAuthority(AtomicUsize::new(0)),
        plan(),
        [(recipe.id.clone(), recipe)].into(),
        &support(),
    )
    .unwrap();
    s.tick().unwrap();
    let effects = effects.lock().unwrap();
    assert!(effects.applied.is_empty() && effects.spawned.is_empty());
}

#[cfg(unix)]
#[test]
fn real_backend_rejects_required_policies_before_exec_and_reports_no_application() {
    for req in [
        requirements(),
        Requirements([(name("unknown-memory"), Requirement::Unknown)].into()),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        let mut recipe = program(Some(req));
        recipe.executable = "/usr/bin/true".into();
        recipe.executable_sha256 =
            Digest::from_bytes(Sha256::digest(std::fs::read(&recipe.executable).unwrap()).into());
        let mut s = Supervisor::open(
            SqliteRepository::open(dir.path().join("state.db")).unwrap(),
            OsProcesses::new(logs.clone()).unwrap(),
            SoftwareOnly,
            plan(),
            [(recipe.id.clone(), recipe)].into(),
            &support(),
        )
        .unwrap();
        let status = s.tick().unwrap();
        println!(
            "D7 REAL DEFAULT REJECTION {}",
            serde_json::to_string(&status.execution_admission).unwrap()
        );
        assert_eq!(
            status.state.records[&name("observer")].phase,
            Phase::StartFailed
        );
        assert!(status.state.records[&name("observer")].pid.is_none());
        assert!(matches!(
            status.execution_admission[&name("observer")].application,
            Application::NotApplied
        ));
        assert_eq!(std::fs::read_dir(logs).unwrap().count(), 0);
        let (_, backend, _) = s.into_parts();
        assert!(backend.owned_instances().is_empty());
    }
}

#[cfg(unix)]
#[test]
fn real_backend_keeps_legacy_and_explicit_no_policy_startup_working() {
    for requested in [
        None,
        Some(Requirements(BTreeMap::new())),
        Some(Requirements(
            [(name("memory"), Requirement::NotRequired)].into(),
        )),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut recipe = program(requested.clone());
        recipe.executable = "/usr/bin/true".into();
        recipe.executable_sha256 =
            Digest::from_bytes(Sha256::digest(std::fs::read(&recipe.executable).unwrap()).into());
        let mut s = Supervisor::open(
            SqliteRepository::open(dir.path().join("state.db")).unwrap(),
            OsProcesses::new(dir.path().join("logs")).unwrap(),
            SoftwareOnly,
            plan(),
            [(recipe.id.clone(), recipe)].into(),
            &support(),
        )
        .unwrap();
        let status = s.tick().unwrap();
        assert_eq!(
            status.state.records[&name("observer")].phase,
            Phase::Starting
        );
        assert_eq!(
            status.execution_admission[&name("observer")].requested,
            requested
        );
        if requested.is_some() {
            let value =
                serde_json::to_value(&status.execution_admission[&name("observer")]).unwrap();
            assert_eq!(
                value["application"]["receipt"]["evidence"]["basis"],
                "NO_REQUIREMENTS"
            );
        } else {
            assert!(matches!(
                status.execution_admission[&name("observer")].application,
                Application::LegacyNotDeclared
            ));
        }
        println!(
            "D6 REAL NO-POLICY {}",
            serde_json::to_string(&status.execution_admission).unwrap()
        );
        s.request_stop().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            if s.tick().unwrap().all_exited {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        s.tick().unwrap();
        let (_, backend, _) = s.into_parts();
        assert!(backend.owned_instances().is_empty());
    }
}
