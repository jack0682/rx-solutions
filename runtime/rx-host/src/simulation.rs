//! A separate file-backed simulated device. It deliberately does not deduplicate calls.
//! Device effects and the Host journal use different persistence paths.
use crate::{model::*, native::*};
use rx_domain::{canonical, intent::Intent, types::*};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone)]
pub struct ManualClock {
    pub clock_id: String,
    pub ticks: Arc<AtomicU64>,
}
impl Clock for ManualClock {
    fn now(&self) -> TimePoint {
        TimePoint {
            clock_id: self.clock_id.clone(),
            ticks_ns: Counter(self.ticks.load(Ordering::SeqCst)),
        }
    }
}
pub struct ProtectionCounter(pub AtomicU64);
impl LocalProtection for ProtectionCounter {
    fn react(&self, _: ProtectionIncident) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceEffect {
    pub operation: Id,
    pub invocation: Id,
    pub capture: NativeCapture,
}
pub struct FileDevice<C: Clock = ManualClock> {
    path: PathBuf,
    device_session: Id,
    clock: C,
    pub protection: Arc<ProtectionCounter>,
    _ownership: std::fs::File,
}
impl<C: Clock> FileDevice<C> {
    pub fn open(directory: impl AsRef<Path>, clock: C) -> Result<Self> {
        fs::create_dir_all(&directory).map_err(|e| HostError::NativeUnknown(e.to_string()))?;
        let ownership = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.as_ref().join("device.lock"))
            .map_err(|e| HostError::NativeUnknown(e.to_string()))?;
        ownership.try_lock().map_err(|_| HostError::Busy)?;
        let session_path = directory.as_ref().join("device-session");
        let device_session = if session_path.exists() {
            Id::new(
                fs::read_to_string(&session_path)
                    .map_err(|e| HostError::NativeUnknown(e.to_string()))?,
            )
            .map_err(|e| HostError::Invalid(e.to_string()))?
        } else {
            let value = crate::journal::id();
            fs::write(&session_path, value.as_str())
                .map_err(|e| HostError::NativeUnknown(e.to_string()))?;
            value
        };
        Ok(Self {
            path: directory.as_ref().join("effects.jsonl"),
            device_session,
            clock,
            protection: Arc::new(ProtectionCounter(AtomicU64::new(0))),
            _ownership: ownership,
        })
    }
}
impl FileDevice {
    pub fn effects(directory: impl AsRef<Path>) -> Result<Vec<DeviceEffect>> {
        let path = directory.as_ref().join("effects.jsonl");
        if !path.exists() {
            return Ok(vec![]);
        }
        fs::read_to_string(path)
            .map_err(|e| HostError::NativeUnknown(e.to_string()))?
            .lines()
            .map(|line| {
                canonical::decode_json(line.as_bytes())
                    .map_err(|e| HostError::Invalid(e.to_string()))
            })
            .collect()
    }
}
impl<C: Clock> NativeAdapter for FileDevice<C> {
    fn shutdown_snapshot(&self, resources: &[Name]) -> Result<NativeShutdown> {
        FileDevice::effects(self.path.parent().ok_or(HostError::Guard)?)?;
        Ok(NativeShutdown {
            device_session: self.device_session.clone(),
            observed_at: self.clock.now(),
            uncertainty_ns: Counter(0),
            resources: resources.to_vec(),
            no_pending_commands: true,
            safe_to_drop: true,
        })
    }
    fn environment(&self) -> Environment {
        Environment::Simulation
    }
    fn observe_sources(
        &self,
        _cell: &Name,
        sources: &[Name],
    ) -> Result<Vec<rx_domain::host_snapshot::SourceObservation>> {
        if sources
            .iter()
            .any(|s| !matches!(s.as_str(), "ready" | "sim/ready"))
        {
            return Err(HostError::Guard);
        }
        // Explicit file-device simulation health, never a physical interlock observation.
        FileDevice::effects(
            self.path
                .parent()
                .ok_or_else(|| HostError::Invalid("device path".into()))?,
        )?;
        Ok(sources
            .iter()
            .map(|source| rx_domain::host_snapshot::SourceObservation {
                source: source.clone(),
                generation: self.device_session.clone(),
                schema: crate::journal::name("boolean/v1"),
                unit: crate::journal::name("unitless"),
                value: TypedValue::Boolean(true),
                acquired_at: self.clock.now(),
                uncertainty_ns: Counter(0),
                quality_good: true,
                origin_age_bounded: true,
                disputed: false,
                evidence_id: crate::journal::id(),
            })
            .collect())
    }
    fn protection(&self) -> Arc<dyn LocalProtection> {
        self.protection.clone()
    }
    fn guard(&self, _: &Intent, now: &TimePoint) -> Result<Guard> {
        Ok(Guard {
            device_session: self.device_session.clone(),
            valid_until: TimePoint {
                clock_id: now.clock_id.clone(),
                ticks_ns: Counter(now.ticks_ns.0.saturating_add(1_000_000_000)),
            },
            satisfied: [crate::journal::name("sim/ready")].into_iter().collect(),
        })
    }
    fn can_handover(&self, _: &[Name]) -> bool {
        true
    }
    fn handover_snapshot(&self, _: &[Name]) -> Result<LocalHandover> {
        // This file-device has no asynchronous native queue or physical gravity/material load.
        // A different adapter must supply its own measured support/queue observations.
        FileDevice::effects(
            self.path
                .parent()
                .ok_or_else(|| HostError::Invalid("device directory".into()))?,
        )?;
        Ok(LocalHandover {
            device_session: self.device_session.clone(),
            observed_at: self.clock.now(),
            uncertainty_ns: Counter(0),
            no_pending_commands: true,
            control_available: true,
            support_stable: true,
        })
    }
    fn submit(&mut self, operation: &Id, invocation: &Id, _: &Intent) -> Result<NativeCapture> {
        let capture = NativeCapture {
            status_schema: crate::journal::name("rx.sim.completed.v1"),
            status: 0,
            native_id: Some(invocation.to_string()),
            captured_at: self.clock.now(),
            device_session: self.device_session.clone(),
        };
        let mut bytes = canonical::bytes(&DeviceEffect {
            operation: operation.clone(),
            invocation: invocation.clone(),
            capture: capture.clone(),
        })
        .map_err(|e| HostError::Invalid(e.to_string()))?;
        bytes.push(b'\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| HostError::NativeUnknown(e.to_string()))?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| HostError::NativeUnknown(e.to_string()))?;
        Ok(capture)
    }
    fn lookup(&mut self, operation: &Id, invocation: &Id) -> Result<Option<NativeCapture>> {
        Ok(FileDevice::effects(
            self.path
                .parent()
                .ok_or_else(|| HostError::Invalid("device directory".into()))?,
        )?
        .into_iter()
        .find(|r| &r.operation == operation && &r.invocation == invocation)
        .map(|r| r.capture))
    }
}
