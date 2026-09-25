//! One Host-owned fictional DYNAMIXEL instance. Native result lookup never dispatches.
mod process;
pub mod profile;
use crate::{
    journal::{decode, doc, id, key, name},
    model::*,
    native::*,
};
use rx_domain::{intent::Intent, types::*};
use rx_ports::{Repository, StoreError};
use rx_storage::SqliteRepository;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicU64},
};
fn unknown(e: impl std::fmt::Display) -> HostError {
    HostError::NativeUnknown(e.to_string())
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub instance: Id,
    pub source: Digest,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Meta {
    identity: Identity,
    pending: Option<Id>,
    entries: Counter,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    operation: Id,
    invocation: Id,
    instance: Id,
    intent: Digest,
    request: serde_json::Value,
    capture: Option<NativeCapture>,
    transcript: Option<serde_json::Value>,
}
pub fn initialize(directory: &Path) -> Result<Identity> {
    std::fs::create_dir(directory).map_err(unknown)?;
    let identity = Identity {
        instance: id(),
        source: profile::source_digest(),
    };
    let mut store = SqliteRepository::open(directory.join("native.sqlite3"))?;
    store.transact(|tx| {
        tx.put(
            &name("dynamixel/meta"),
            None,
            &doc(
                "rx.dynamixel.meta.v1",
                &Meta {
                    identity: identity.clone(),
                    pending: None,
                    entries: Counter(0),
                },
            )?,
        )?;
        Ok(())
    })?;
    Ok(identity)
}
pub struct Dynamixel<C: Clock> {
    store: SqliteRepository,
    directory: PathBuf,
    identity: Identity,
    clock: C,
    pin: Digest,
    pending: bool,
    protection: Arc<crate::simulation::ProtectionCounter>,
}
impl<C: Clock> Dynamixel<C> {
    pub fn open(directory: &Path, identity: Identity, pin: Digest, clock: C) -> Result<Self> {
        let path = directory.join("native.sqlite3");
        let metadata = std::fs::symlink_metadata(&path).map_err(unknown)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() == 0 {
            return Err(unknown("DXL_NATIVE_IDENTITY_MISSING"));
        }
        let mut store = SqliteRepository::open(path)?;
        store.check_integrity()?;
        let pending = store.transact(|tx| {
            let row = tx
                .get(&name("dynamixel/meta"))?
                .ok_or(StoreError::Integrity("DXL_NATIVE_IDENTITY_MISSING".into()))?;
            let meta: Meta = decode(&row, "rx.dynamixel.meta.v1")?;
            if meta.identity != identity || identity.source != profile::source_digest() {
                return Err(StoreError::Integrity("DXL_NATIVE_INSTANCE_MISMATCH".into()));
            }
            let entries = tx.scan("dynamixel-operation/")?;
            let mut pending = vec![];
            if entries.len() as u64 != meta.entries.0 {
                return Err(StoreError::Integrity("DXL_ENTRY_COUNT".into()));
            }
            for row in entries {
                let entry: Entry = decode(&row, "rx.dynamixel.entry.v1")?;
                if row.key != key("dynamixel-operation", &entry.operation)
                    || entry.instance != identity.instance
                    || entry.capture.is_some() != entry.transcript.is_some()
                {
                    return Err(StoreError::Integrity("DXL_ENTRY_IDENTITY".into()));
                }
                let invocation = tx
                    .get(&key("dynamixel-invocation", &entry.invocation))?
                    .ok_or(StoreError::Integrity("DXL_INVOCATION_MISSING".into()))?;
                if decode::<Id>(&invocation, "rx.dynamixel.invocation.v1")? != entry.operation {
                    return Err(StoreError::Integrity("DXL_INVOCATION_MISMATCH".into()));
                }
                if entry.capture.is_none() {
                    pending.push(entry.operation);
                }
            }
            if pending != meta.pending.clone().into_iter().collect::<Vec<_>>() {
                return Err(StoreError::Integrity("DXL_PENDING_MISMATCH".into()));
            }
            Ok(meta.pending.is_some())
        })?;
        Ok(Self {
            store,
            directory: directory.to_owned(),
            identity,
            clock,
            pin,
            pending,
            protection: Arc::new(crate::simulation::ProtectionCounter(AtomicU64::new(0))),
        })
    }
    fn dispatch(
        &mut self,
        operation: &Id,
        invocation: &Id,
        intent: &Intent,
        context: &NativeDispatch,
    ) -> Result<NativeCapture> {
        profile::validate_intent(intent)?;
        let now = self.clock.now();
        if self.pending
            || context.device_session != self.identity.instance
            || context.expires_at.clock_id != now.clock_id
            || context.expires_at.ticks_ns <= now.ticks_ns
        {
            return Err(HostError::Guard);
        }
        let request = serde_json::json!({"operation":operation,"invocation":invocation,"instance":self.identity.instance,"deadline_ns":context.expires_at.ticks_ns});
        let mut entry = Entry {
            operation: operation.clone(),
            invocation: invocation.clone(),
            instance: self.identity.instance.clone(),
            intent: intent.digest().map_err(unknown)?,
            request: request.clone(),
            capture: None,
            transcript: None,
        };
        self.store.transact(|tx| {
            let row = tx
                .get(&name("dynamixel/meta"))?
                .ok_or(StoreError::Integrity("DXL_META".into()))?;
            let mut meta: Meta = decode(&row, "rx.dynamixel.meta.v1")?;
            if meta.pending.is_some() || meta.entries.0 >= 10_000 {
                return Err(StoreError::Invalid("DXL_PENDING_OR_CAPACITY".into()));
            }
            tx.put(
                &key("dynamixel-operation", operation),
                None,
                &doc("rx.dynamixel.entry.v1", &entry)?,
            )?;
            tx.put(
                &key("dynamixel-invocation", invocation),
                None,
                &doc("rx.dynamixel.invocation.v1", operation)?,
            )?;
            tx.append(&id(), &doc("rx.dynamixel.entry.v1", &entry)?)?;
            meta.pending = Some(operation.clone());
            meta.entries.0 += 1;
            tx.put(
                &name("dynamixel/meta"),
                Some(row.revision),
                &doc("rx.dynamixel.meta.v1", &meta)?,
            )?;
            Ok(())
        })?;
        self.pending = true;
        let transcript = process::ping(&self.directory, self.pin, &request)?;
        let capture = NativeCapture {
            status_schema: name(profile::COMPLETION),
            status: 0,
            native_id: Some(invocation.to_string()),
            captured_at: self.clock.now(),
            device_session: self.identity.instance.clone(),
        };
        entry.capture = Some(capture.clone());
        entry.transcript = Some(transcript);
        self.store.transact(|tx| {
            let row = tx
                .get(&key("dynamixel-operation", operation))?
                .ok_or(StoreError::Integrity("DXL_ENTRY".into()))?;
            let old: Entry = decode(&row, "rx.dynamixel.entry.v1")?;
            if old.capture.is_some() || old.invocation != *invocation {
                return Err(StoreError::KeyConflict);
            }
            tx.put(
                &row.key,
                Some(row.revision),
                &doc("rx.dynamixel.entry.v1", &entry)?,
            )?;
            tx.append(&id(), &doc("rx.dynamixel.entry.v1", &entry)?)?;
            let row = tx
                .get(&name("dynamixel/meta"))?
                .ok_or(StoreError::Integrity("DXL_META".into()))?;
            let mut meta: Meta = decode(&row, "rx.dynamixel.meta.v1")?;
            if meta.pending.as_ref() != Some(operation) {
                return Err(StoreError::KeyConflict);
            }
            meta.pending = None;
            tx.put(
                &row.key,
                Some(row.revision),
                &doc("rx.dynamixel.meta.v1", &meta)?,
            )?;
            Ok(())
        })?;
        self.pending = false;
        Ok(capture)
    }
}
impl<C: Clock> NativeAdapter for Dynamixel<C> {
    fn environment(&self) -> Environment {
        Environment::Simulation
    }
    fn protection(&self) -> Arc<dyn LocalProtection> {
        self.protection.clone()
    }
    fn observe_sources(
        &self,
        _: &Name,
        sources: &[Name],
    ) -> Result<Vec<rx_domain::host_snapshot::SourceObservation>> {
        if sources
            .iter()
            .any(|s| !matches!(s.as_str(), "ready" | "sim/ready"))
        {
            return Err(HostError::Guard);
        }
        // Passive software readiness. This does not perform a diagnostic Ping.
        Ok(sources
            .iter()
            .map(|source| rx_domain::host_snapshot::SourceObservation {
                source: source.clone(),
                generation: self.identity.instance.clone(),
                schema: name("boolean/v1"),
                unit: name("unitless"),
                value: TypedValue::Boolean(!self.pending),
                acquired_at: self.clock.now(),
                uncertainty_ns: Counter(0),
                quality_good: true,
                origin_age_bounded: true,
                disputed: false,
                evidence_id: id(),
            })
            .collect())
    }
    fn guard(&self, intent: &Intent, now: &TimePoint) -> Result<Guard> {
        profile::validate_intent(intent)?;
        if self.pending {
            return Err(HostError::Guard);
        }
        Ok(Guard {
            device_session: self.identity.instance.clone(),
            valid_until: TimePoint {
                clock_id: now.clock_id.clone(),
                ticks_ns: Counter(now.ticks_ns.0.saturating_add(1_000_000_000)),
            },
            satisfied: [name("sim/ready")].into_iter().collect(),
        })
    }
    fn can_handover(&self, _: &[Name]) -> bool {
        !self.pending
    }
    fn handover_snapshot(&self, _: &[Name]) -> Result<LocalHandover> {
        Ok(LocalHandover {
            device_session: self.identity.instance.clone(),
            observed_at: self.clock.now(),
            uncertainty_ns: Counter(0),
            no_pending_commands: !self.pending,
            control_available: !self.pending,
            support_stable: true,
        })
    }
    fn shutdown_snapshot(&self, resources: &[Name]) -> Result<NativeShutdown> {
        Ok(NativeShutdown {
            device_session: self.identity.instance.clone(),
            observed_at: self.clock.now(),
            uncertainty_ns: Counter(0),
            resources: resources.to_vec(),
            no_pending_commands: !self.pending,
            safe_to_drop: !self.pending,
        })
    }
    fn submit(&mut self, _: &Id, _: &Id, _: &Intent) -> Result<NativeCapture> {
        Err(HostError::Guard)
    }
    fn submit_with_context(
        &mut self,
        op: &Id,
        inv: &Id,
        intent: &Intent,
        context: &NativeDispatch,
    ) -> Result<NativeCapture> {
        self.dispatch(op, inv, intent, context)
    }
    fn lookup(&mut self, op: &Id, inv: &Id) -> Result<Option<NativeCapture>> {
        let entry = self.store.transact(|tx| {
            tx.get(&key("dynamixel-operation", op))?
                .map(|r| decode::<Entry>(&r, "rx.dynamixel.entry.v1"))
                .transpose()
        })?;
        match entry {
            Some(entry) if entry.invocation != *inv => Err(HostError::Guard),
            Some(entry) => Ok(entry.capture),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::ManualClock;
    fn clock() -> ManualClock {
        ManualClock {
            clock_id: "test/dynamixel".into(),
            ticks: Arc::new(AtomicU64::new(1)),
        }
    }
    fn intent() -> Intent {
        Intent {
            kind: rx_domain::intent::Kind::FiniteAction,
            target: name("device/dynamixel-simulation"),
            profile_digest: profile::reference("profile").sha256,
            site_config_digest: Digest::from_bytes([1; 32]),
            calibration_digests: vec![],
            resource_set: vec![name("controller/simulation")],
            execution_timeout_ms: Counter(5000),
            prepare_validity_ms: Counter(1000),
            completion_rule: name(profile::COMPLETION),
            cancel_rule: name("rx.dynamixel.no-motion.v1"),
            body: rx_domain::intent::Body::Program(rx_domain::intent::ProgramGoal {
                program: profile::reference("program"),
                parameter_set: profile::reference("parameters"),
            }),
        }
    }
    #[test]
    fn exact_ping_contract_rejects_other_program_parameters_target_and_completion() {
        let good = intent();
        profile::validate_intent(&good).unwrap();
        let mut bad = good.clone();
        bad.target = name("device/file-simulation");
        assert!(profile::validate_intent(&bad).is_err());
        let mut bad = good.clone();
        bad.completion_rule = name("rx.sim.completed.v1");
        assert!(profile::validate_intent(&bad).is_err());
        let mut bad = good.clone();
        let rx_domain::intent::Body::Program(g) = &mut bad.body else {
            panic!()
        };
        g.program.sha256 = Digest::from_bytes([9; 32]);
        assert!(profile::validate_intent(&bad).is_err());
        let mut bad = good;
        let rx_domain::intent::Body::Program(g) = &mut bad.body else {
            panic!()
        };
        g.parameter_set.size_bytes.0 += 1;
        assert!(profile::validate_intent(&bad).is_err());
    }
    #[test]
    fn endpoint_allowlist_is_named_and_checked_before_any_release_file() {
        for endpoint in [
            "/dev/ttyUSB0",
            "/dev/ttyACM0",
            "tcp://localhost:1234",
            "simulation/dynamixel/id-2",
        ] {
            let backend = crate::service::config::Backend::ValidatedDriver {
                profile: name(profile::PROFILE),
                driver_digest: profile::source_digest(),
                endpoint: endpoint.into(),
            };
            assert!(
                profile::validate(&backend, &[])
                    .unwrap_err()
                    .to_string()
                    .contains("DXL_REAL_ENDPOINT_UNSUPPORTED")
            );
        }
    }
    #[test]
    fn instance_and_live_writer_refusal_survive_restart_without_new_identity() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("native");
        let identity = initialize(&path).unwrap();
        let pin = Digest::from_bytes([0; 32]);
        let mut owner = Dynamixel::open(&path, identity.clone(), pin, clock()).unwrap();
        assert!(Dynamixel::open(&path, identity.clone(), pin, clock()).is_err());
        assert!(owner.lookup(&id(), &id()).unwrap().is_none());
        drop(owner);
        let mut wrong = identity.clone();
        wrong.instance = id();
        assert!(Dynamixel::open(&path, wrong, pin, clock()).is_err());
        let owner = Dynamixel::open(&path, identity.clone(), pin, clock()).unwrap();
        assert_eq!(owner.identity, identity);
    }
    #[test]
    fn missing_native_result_stays_unknown_without_retry_after_restart() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("native");
        let identity = initialize(&path).unwrap();
        let pin = Digest::from_bytes([0; 32]);
        let mut owner = Dynamixel::open(&path, identity.clone(), pin, clock()).unwrap();
        let op = id();
        let inv = id();
        let context = NativeDispatch {
            device_session: identity.instance.clone(),
            expires_at: TimePoint {
                clock_id: "test/dynamixel".into(),
                ticks_ns: Counter(100),
            },
        };
        // No authenticated helper exists in a library fixture. Entered intent cannot be retried.
        assert!(
            owner
                .submit_with_context(&op, &inv, &intent(), &context)
                .is_err()
        );
        assert!(owner.pending);
        assert!(owner.lookup(&op, &inv).unwrap().is_none());
        assert!(!owner.can_handover(&[]));
        drop(owner);
        let mut reopened = Dynamixel::open(&path, identity, pin, clock()).unwrap();
        assert!(reopened.lookup(&op, &inv).unwrap().is_none());
        assert!(
            reopened
                .submit_with_context(&op, &inv, &intent(), &context)
                .is_err()
        );
        assert!(!path.join("helper-invocations.jsonl").exists());
        let count = reopened
            .store
            .transact(|tx| Ok(tx.scan("dynamixel-operation/")?.len()))
            .unwrap();
        assert_eq!(count, 1);
    }
}
