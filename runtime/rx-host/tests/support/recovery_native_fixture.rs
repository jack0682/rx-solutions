//! TEST FIXTURE README
//! Run: rx-host-recovery-fixture run CONFIG. Initialize using the actual rx-hostd init.
//! This is a distinct test executable, not a new product backend or the shipped rx-hostd.
//! It uses the unchanged Host service/RPC/receipt/publisher and real Linux BOOTTIME.
//! Only FILE_SIMULATION is accepted. It performs no P DB/authority/evidence seeding.
//!
//! FileDevice still appends/fsyncs every actual effect and deliberately does not deduplicate.
//! This wrapper withholds that successful capture with NativeUnknown so Host SEND_ENTERED
//! remains unresolved until an explicitly observed lookup. Before the empty regular marker
//! /data/test-native/result-visible exists, lookup returns None; afterward it delegates to
//! the original FileDevice lookup. The marker controls only simulated result visibility.
//! /data/test-native/native-calls.jsonl is an append+fsync trace of SUBMIT, SUBMIT_RESULT,
//! LOOKUP and LOOKUP_RESULT. Count these independently from device/effects.jsonl.
//! No wrapper deduplication hides an erroneous second native call. All other methods delegate.
//! This fixture does not validate physical equipment, H restart/rebind, new authority or resume.

#[cfg(target_os = "linux")]
mod linux {
    use rx_domain::{canonical, host_snapshot::SourceObservation, intent::Intent, types::*};
    use rx_host::{
        Binding, Environment, Guard, HostError, LocalHandover, NativeCapture,
        native::{LocalProtection, NativeAdapter, NativeDispatch, NativeShutdown},
        service::{
            self, AdapterFactory, Builtin,
            config::{Backend, Loaded},
        },
        service_clock::SystemClock,
        simulation::FileDevice,
    };
    use serde::Serialize;
    use std::{
        fs::{self, File, OpenOptions},
        io::Write,
        os::unix::fs::OpenOptionsExt,
        path::Path,
        sync::Arc,
    };

    const ROOT: &str = "/data/test-native";
    const CALLS: &str = "/data/test-native/native-calls.jsonl";
    const VISIBLE: &str = "/data/test-native/result-visible";

    #[derive(Serialize)]
    struct Call<'a> {
        schema: &'static str,
        kind: &'static str,
        process_id: u32,
        operation: &'a Id,
        invocation: &'a Id,
        observed_at: TimePoint,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<&'static str>,
    }
    struct Trace {
        file: File,
        clock: SystemClock,
    }
    impl Trace {
        fn open(clock: SystemClock) -> service::Result<Self> {
            fs::create_dir_all(ROOT)?;
            let metadata = fs::symlink_metadata(ROOT)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err("test trace requires a real directory".into());
            }
            let file = OpenOptions::new()
                .append(true)
                .create(true)
                .mode(0o600)
                .custom_flags(
                    (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32,
                )
                .open(CALLS)?;
            if !file.metadata()?.is_file() {
                return Err("test trace requires a regular file".into());
            }
            file.try_lock()
                .map_err(|_| "another fixture owns the native trace")?;
            // Existing bytes are preserved. A restarted test cannot erase prior native calls.
            Ok(Self { file, clock })
        }
        fn append(
            &mut self,
            kind: &'static str,
            operation: &Id,
            invocation: &Id,
            result: Option<&'static str>,
        ) -> rx_host::Result<()> {
            let mut bytes = canonical::bytes(&Call {
                schema: "rx.test-native-call.v1",
                kind,
                process_id: std::process::id(),
                operation,
                invocation,
                observed_at: self.clock.read()?,
                result,
            })
            .map_err(|e| HostError::Invalid(e.to_string()))?;
            bytes.push(b'\n');
            self.file
                .write_all(&bytes)
                .and_then(|_| self.file.sync_all())
                .map_err(|e| HostError::NativeUnknown(format!("test trace write failed: {e}")))
        }
    }
    fn visible() -> rx_host::Result<bool> {
        match fs::symlink_metadata(VISIBLE) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Ok(metadata) if metadata.file_type().is_file() && metadata.len() == 0 => Ok(true),
            Ok(_) => Err(HostError::Invalid(
                "test result-visible marker must be an empty regular file".into(),
            )),
            Err(error) => Err(HostError::NativeUnknown(format!(
                "test marker read failed: {error}"
            ))),
        }
    }

    struct DelayedFileDevice {
        inner: FileDevice<SystemClock>,
        trace: Trace,
    }
    impl DelayedFileDevice {
        fn withhold(
            &mut self,
            operation: &Id,
            invocation: &Id,
            captured: rx_host::Result<NativeCapture>,
        ) -> rx_host::Result<NativeCapture> {
            match captured {
                Ok(_) => {
                    self.trace.append(
                        "SUBMIT_RESULT",
                        operation,
                        invocation,
                        Some("EFFECT_SAVED_RETURN_WITHHELD"),
                    )?;
                    Err(HostError::NativeUnknown("TEST_ONLY: durable FileDevice effect exists; capture withheld until lookup".into()))
                }
                Err(error) => {
                    self.trace
                        .append("SUBMIT_RESULT", operation, invocation, Some("ERROR"))?;
                    Err(error)
                }
            }
        }
    }
    impl NativeAdapter for DelayedFileDevice {
        fn prepare_shutdown(&mut self, resources: &[Name]) -> rx_host::Result<()> {
            self.inner.prepare_shutdown(resources)
        }
        fn shutdown_snapshot(&self, resources: &[Name]) -> rx_host::Result<NativeShutdown> {
            self.inner.shutdown_snapshot(resources)
        }
        fn environment(&self) -> Environment {
            self.inner.environment()
        }
        fn protection(&self) -> Arc<dyn LocalProtection> {
            self.inner.protection()
        }
        fn observe_sources(
            &self,
            cell: &Name,
            sources: &[Name],
        ) -> rx_host::Result<Vec<SourceObservation>> {
            self.inner.observe_sources(cell, sources)
        }
        fn guard(&self, intent: &Intent, now: &TimePoint) -> rx_host::Result<Guard> {
            self.inner.guard(intent, now)
        }
        fn can_handover(&self, resources: &[Name]) -> bool {
            self.inner.can_handover(resources)
        }
        fn handover_snapshot(&self, resources: &[Name]) -> rx_host::Result<LocalHandover> {
            self.inner.handover_snapshot(resources)
        }
        fn submit(
            &mut self,
            operation: &Id,
            invocation: &Id,
            intent: &Intent,
        ) -> rx_host::Result<NativeCapture> {
            self.trace.append("SUBMIT", operation, invocation, None)?;
            let captured = self.inner.submit(operation, invocation, intent);
            self.withhold(operation, invocation, captured)
        }
        fn submit_with_context(
            &mut self,
            operation: &Id,
            invocation: &Id,
            intent: &Intent,
            context: &NativeDispatch,
        ) -> rx_host::Result<NativeCapture> {
            self.trace.append("SUBMIT", operation, invocation, None)?;
            let captured = self
                .inner
                .submit_with_context(operation, invocation, intent, context);
            self.withhold(operation, invocation, captured)
        }
        fn lookup(
            &mut self,
            operation: &Id,
            invocation: &Id,
        ) -> rx_host::Result<Option<NativeCapture>> {
            self.trace.append("LOOKUP", operation, invocation, None)?;
            let (result, label) = match visible() {
                Ok(false) => (Ok(None), "HIDDEN"),
                Ok(true) => {
                    let result = self.inner.lookup(operation, invocation);
                    let label = match &result {
                        Ok(Some(_)) => "FOUND",
                        Ok(None) => "MISSING",
                        Err(_) => "ERROR",
                    };
                    (result, label)
                }
                Err(error) => (Err(error), "ERROR"),
            };
            self.trace
                .append("LOOKUP_RESULT", operation, invocation, Some(label))?;
            result
        }
    }

    struct FixtureFactory;
    impl AdapterFactory<SystemClock> for FixtureFactory {
        type Adapter = DelayedFileDevice;
        fn validate(&self, backend: &Backend, bindings: &[Binding]) -> service::Result<()> {
            if !matches!(backend, Backend::FileSimulation)
                || bindings
                    .iter()
                    .any(|binding| binding.environment != Environment::Simulation)
            {
                return Err("recovery native fixture accepts FILE_SIMULATION only; physical/device backends are forbidden".into());
            }
            <Builtin as AdapterFactory<SystemClock>>::validate(&Builtin, backend, bindings)
        }
        fn open_passive(
            &self,
            backend: &Backend,
            data: &Path,
            clock: SystemClock,
        ) -> service::Result<Self::Adapter> {
            if !matches!(backend, Backend::FileSimulation) {
                return Err("FILE_SIMULATION required".into());
            }
            let trace = Trace::open(clock.clone())?;
            let inner = FileDevice::open(data.join("device"), clock)?;
            Ok(DelayedFileDevice { inner, trace })
        }
    }

    pub(super) async fn run() -> service::Result<()> {
        let arguments: Vec<_> = std::env::args_os().skip(1).collect();
        let [command, config] = arguments.as_slice() else {
            return Err(
                "usage: rx-host-recovery-fixture run CONFIG; initialize with rx-hostd init".into(),
            );
        };
        if command != "run" {
            return Err(
                "fixture supports run only; initialize with the actual rx-hostd init".into(),
            );
        }
        let loaded = Loaded::read(Path::new(config))?;
        let clock = SystemClock::new()?;
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let shutdown = async move {
            tokio::select! { _ = term.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
        };
        service::run_with(loaded, clock, FixtureFactory, shutdown).await
    }
}

#[tokio::main]
async fn main() -> rx_host::service::Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::run().await
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err("recovery native fixture requires Linux BOOTTIME".into())
    }
}
