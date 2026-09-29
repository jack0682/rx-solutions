use rx_domain::{canonical, host_snapshot::SourceObservation, types::*};
use rx_host::{
    native::*,
    observation::{Adapter, Declaration, Provider, Source, StopObservation, ValueType},
    service::{self, AdapterFactory, config::*},
    simulation::ManualClock,
    *,
};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
#[path = "support/host_service_fixture.rs"]
mod fixture;
use fixture::{id, name};
struct Protection;
impl LocalProtection for Protection {
    fn react(&self, _: ProtectionIncident) {}
}
#[derive(Clone)]
struct Reader {
    sample: Arc<Mutex<SourceObservation>>,
    reads: Arc<AtomicUsize>,
    stopped: Arc<AtomicBool>,
    clock: ManualClock,
}
impl Provider for Reader {
    fn environment(&self) -> Environment {
        Environment::Simulation
    }
    fn protection(&self) -> Arc<dyn LocalProtection> {
        Arc::new(Protection)
    }
    fn observe(&self, _: &Name, _: &[Name]) -> rx_host::Result<Vec<SourceObservation>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(vec![self.sample.lock().unwrap().clone()])
    }
    fn stop(&mut self) -> rx_host::Result<()> {
        self.stopped.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn stop_observation(&self) -> rx_host::Result<StopObservation> {
        Ok(StopObservation {
            generation: self.sample.lock().unwrap().generation.clone(),
            observed_at: self.clock.now(),
            uncertainty_ns: Counter(0),
            stopped: self.stopped.load(Ordering::SeqCst),
        })
    }
}
fn observer(binding: &Binding, clock: ManualClock) -> (Binding, Reader) {
    let mut b = binding.clone();
    b.allowed_intents.clear();
    b.condition_ids.clear();
    b.purposes.clear();
    b.qualification = None;
    b.qualification_revision = Counter(0);
    b.observation_only = Some(Declaration {
        schema: name("rx.observation-only-binding.v1"),
        sources: vec![Source {
            id: name("measurement"),
            schema: name("integer/v1"),
            unit: name("count"),
            value_type: ValueType::Integer,
        }],
    });
    let reader = Reader {
        sample: Arc::new(Mutex::new(SourceObservation {
            source: name("measurement"),
            generation: id(),
            schema: name("integer/v1"),
            unit: name("count"),
            value: TypedValue::Integer(Integer(42)),
            acquired_at: clock.now(),
            uncertainty_ns: Counter(0),
            quality_good: true,
            origin_age_bounded: true,
            disputed: false,
            evidence_id: id(),
        })),
        reads: Arc::new(AtomicUsize::new(0)),
        stopped: Arc::new(AtomicBool::new(false)),
        clock,
    };
    (b, reader)
}
#[test]
fn declared_observer_reads_without_qualification_grant_or_arm_and_preserves_native_metadata() {
    let (_dir, path, clock) = fixture::fixture();
    let loaded = Loaded::read(&path).unwrap();
    let (binding, reader) = observer(&loaded.bindings[0], clock.clone());
    let initial = reader.sample.lock().unwrap().clone();
    let root = tempfile::tempdir().unwrap();
    let host = Host::open(
        root.path().join("h.db"),
        Adapter::new(reader.clone()),
        clock.clone(),
        vec![binding.clone()],
    )
    .unwrap();
    let caller = Caller {
        peer: binding.platform.clone(),
        session: id(),
    };
    host.bind_platform(caller.clone()).unwrap();
    let first = host
        .bootstrap_snapshot(&caller, &binding.cell, &[name("measurement")])
        .unwrap();
    assert_eq!(
        canonical::bytes(&first.observations[0]).unwrap(),
        canonical::bytes(&initial).unwrap()
    );
    assert!(
        first.resource_fences.is_empty()
            && first.pending_operations.is_empty()
            && first.pending_permits.is_empty()
    );
    assert!(matches!(
        host.acquire_grant(
            &caller,
            id(),
            vec![name("fictional/resource")],
            Counter(1),
            Counter(1000)
        ),
        Err(HostError::Forbidden)
    ));
    assert!(matches!(
        host.renew_grant(&caller, &id(), Counter(1)),
        Err(HostError::Forbidden)
    ));
    assert!(matches!(
        host.void_before_send(&caller, id(), Digest::from_bytes([8; 32])),
        Err(HostError::Forbidden)
    ));
    let control = &loaded.bindings[0];
    let intent = control.allowed_intents[0].clone();
    let digest = intent.digest().unwrap();
    let operation = id();
    let grant = id();
    let forged = Request {
        operation: operation.clone(),
        intent,
        digest,
        grant: grant.clone(),
        permit: Permit {
            id: id(),
            operation,
            digest,
            cell: binding.cell.clone(),
            epoch: Counter(1),
            scopes: binding
                .scope_ids
                .iter()
                .map(|s| (s.clone(), Counter(1)))
                .collect(),
            envelope: binding.envelope.sha256,
            qualification: control.qualification.clone().unwrap(),
            qualification_revision: control.qualification_revision,
            grant,
            host_boot: host.boot_id().unwrap(),
            conditions: control.condition_ids.iter().cloned().collect(),
            expires_at: TimePoint {
                clock_id: clock.now().clock_id,
                ticks_ns: Counter(1_000_000),
            },
            source_digest: None,
            purpose: Purpose::Production,
            parent: PermitParent::Mandate(id()),
        },
    };
    assert!(matches!(
        host.prepare(&caller, forged.clone()),
        Err(HostError::Forbidden)
    ));
    assert!(matches!(
        host.authorize(&caller, forged, &id()),
        Err(HostError::Forbidden)
    ));
    let scopes = binding
        .scope_ids
        .iter()
        .map(|s| (s.clone(), Counter(1)))
        .collect::<BTreeMap<_, _>>();
    assert!(matches!(
        host.arm(
            &caller,
            id(),
            &binding.cell,
            Counter(1),
            &scopes,
            &Default::default()
        ),
        Err(HostError::Forbidden)
    ));
    assert!(
        host.bootstrap_snapshot(&caller, &binding.cell, &[name("undeclared")])
            .is_err()
    );
    assert_eq!(reader.reads.load(Ordering::SeqCst), 1);
    clock.ticks.store(10_000, Ordering::SeqCst);
    let stale = host
        .bootstrap_snapshot(&caller, &binding.cell, &[name("measurement")])
        .unwrap();
    assert_eq!(stale.observations[0].acquired_at, initial.acquired_at);
    assert_eq!(stale.observations[0].evidence_id, initial.evidence_id);
    reader.sample.lock().unwrap().generation = id();
    let changed = host
        .bootstrap_snapshot(&caller, &binding.cell, &[name("measurement")])
        .unwrap();
    assert_ne!(changed.observations[0].generation, initial.generation);
    host.request_service_stop();
    let stopped = host.service_stop_snapshot().unwrap();
    assert!(stopped.safe_to_drop && !stopped.physical_shutdown_assessed);
    assert!(reader.stopped.load(Ordering::SeqCst));
    assert!(
        host.bootstrap_snapshot(&caller, &binding.cell, &[name("measurement")])
            .is_err()
    );
}
#[test]
fn observer_declaration_cannot_smuggle_control_or_mismatched_samples() {
    for bad in 0..7 {
        let (_dir, path, clock) = fixture::fixture();
        let loaded = Loaded::read(&path).unwrap();
        let (mut b, reader) = observer(&loaded.bindings[0], clock.clone());
        match bad {
            0 => b.qualification = Some(id()),
            1 => b.allowed_intents = loaded.bindings[0].allowed_intents.clone(),
            2 => b.purposes = loaded.bindings[0].purposes.clone(),
            3 => {
                b.observation_only.as_mut().unwrap().sources[0].id =
                    name("handover/reserved/control")
            }
            4 => reader.sample.lock().unwrap().schema = name("wrong/v1"),
            5 => reader.sample.lock().unwrap().unit = name("meter"),
            _ => reader.sample.lock().unwrap().value = TypedValue::Boolean(true),
        }
        let root = tempfile::tempdir().unwrap();
        let host = Host::open(
            root.path().join("h.db"),
            Adapter::new(reader.clone()),
            clock,
            vec![b.clone()],
        );
        if bad < 4 {
            assert!(host.is_err());
        } else {
            let host = host.unwrap();
            let caller = Caller {
                peer: b.platform.clone(),
                session: id(),
            };
            host.bind_platform(caller.clone()).unwrap();
            assert!(
                host.bootstrap_snapshot(&caller, &b.cell, &[name("measurement")])
                    .is_err()
            );
        }
    }
}
struct Factory(Reader);
impl AdapterFactory<ManualClock> for Factory {
    type Adapter = Adapter<Reader>;
    fn validate(&self, _: &Backend, b: &[Binding]) -> service::Result<()> {
        if b.iter().any(|b| b.observation_only.is_none()) {
            return Err("observer mode required".into());
        }
        Ok(())
    }
    fn open_passive(
        &self,
        _: &Backend,
        _: &Path,
        _: ManualClock,
    ) -> service::Result<Self::Adapter> {
        Ok(Adapter::new(self.0.clone()))
    }
}
#[tokio::test]
async fn observer_service_initializes_runs_and_stops_without_restoring_qualification() {
    let (_dir, path, clock) = fixture::fixture();
    let old = Loaded::read(&path).unwrap();
    let legacy = canonical::bytes(&old.bindings).unwrap();
    assert!(
        !String::from_utf8(legacy.clone())
            .unwrap()
            .contains("observation_only")
    );
    assert_eq!(
        canonical::bytes(&canonical::decode_json::<Vec<Binding>>(&legacy).unwrap()).unwrap(),
        legacy
    );
    let (binding, reader) = observer(&old.bindings[0], clock.clone());
    let value = serde_json::to_value(&binding).unwrap();
    assert!(
        value.get("qualification").is_none()
            && value.get("allowed_intents").is_none()
            && value.get("purposes").is_none()
    );
    let mut cfg = old.config;
    let raw = canonical::bytes(&vec![binding]).unwrap();
    std::fs::write(&cfg.bindings.path, &raw).unwrap();
    cfg.bindings.sha256 = rx_package::content_digest(&raw);
    std::fs::write(&path, canonical::bytes(&cfg).unwrap()).unwrap();
    let loaded = Loaded::read(&path).unwrap();
    let factory = Factory(reader.clone());
    service::initialize_with(&loaded, clock.clone(), &factory).unwrap();
    let status_file = loaded.config.runtime_directory.join("host-status.json");
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(service::run_with(loaded, clock, factory, async {
        let _ = stopped.await;
    }));
    let status = fixture::status(&status_file, "OBSERVATION_READY").await;
    assert_eq!(status["qualification_or_arm_restored"], false);
    stop.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(reader.stopped.load(Ordering::SeqCst));
}
