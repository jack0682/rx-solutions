//! Local daemon metadata, never P admission, resource handover or physical stop proof.
mod clock;
mod file;
mod reporter;
mod support;
pub use clock::{Clock, LinuxBoottime};
pub use reporter::Reporter;
use rx_domain::{canonical, types::*};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
pub use support::{SupportRefusal, storage_ownership_refusal};

pub const SCHEMA: &str = "rx.protocol-guarded-status.v1";
pub const MAX_BYTES: usize = 65_536;
pub const MAX_AGE_NS: u64 = 2_000_000_000;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("guarded status I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid guarded status: {0}")]
    Invalid(String),
    #[error("guarded status requires Linux BOOTTIME")]
    Unsupported,
}
pub type Result<T> = std::result::Result<T, Error>;
fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum GuardedScope {
    Host {
        installation: Id,
        host: Name,
        installation_identity: Digest,
    },
    Executor {
        installation: Id,
        cell: Name,
        service_journal: Id,
        configuration_digest: Digest,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum GuardedStatusBinding {
    Host {
        path: PathBuf,
        installation: Id,
        host: Name,
        installation_identity: Digest,
    },
    Executor {
        path: PathBuf,
        installation: Id,
        cell: Name,
        service_journal: Id,
        configuration_digest: Digest,
    },
}
impl GuardedStatusBinding {
    pub fn path(&self) -> &Path {
        match self {
            Self::Host { path, .. } | Self::Executor { path, .. } => path,
        }
    }
    pub fn scope(&self) -> GuardedScope {
        match self {
            Self::Host {
                installation,
                host,
                installation_identity,
                ..
            } => GuardedScope::Host {
                installation: installation.clone(),
                host: host.clone(),
                installation_identity: *installation_identity,
            },
            Self::Executor {
                installation,
                cell,
                service_journal,
                configuration_digest,
                ..
            } => GuardedScope::Executor {
                installation: installation.clone(),
                cell: cell.clone(),
                service_journal: service_journal.clone(),
                configuration_digest: *configuration_digest,
            },
        }
    }
    /// The supervisor can derive a unique file for one owned launch without changing scope.
    pub fn with_path(&self, path: PathBuf) -> Self {
        let mut binding = self.clone();
        match &mut binding {
            Self::Host { path: current, .. } | Self::Executor { path: current, .. } => {
                *current = path
            }
        }
        binding
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum GuardedState {
    Starting,
    Ready,
    Stopping,
    Stopped { reconciliation_required: bool },
    Attention,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardedObservation {
    pub state: GuardedState,
    pub sequence: Counter,
    pub observed_at: TimePoint,
    pub payload_digest: Digest,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema: Name,
    scope: GuardedScope,
    instance: Id,
    pid: u32,
    sequence: Counter,
    observed_at: TimePoint,
    state: GuardedState,
}
impl Envelope {
    fn validate(&self, scope: &GuardedScope, instance: &Id, pid: u32) -> Result<()> {
        if self.schema.as_str() != SCHEMA
            || self.scope != *scope
            || self.instance != *instance
            || self.pid != pid
            || pid == 0
            || self.sequence.0 == 0
            || self.observed_at.clock_id.is_empty()
        {
            return Err(invalid(
                "schema, role, owner, scope, sequence or clock differs",
            ));
        }
        Ok(())
    }
    fn observation(&self, bytes: &[u8]) -> GuardedObservation {
        GuardedObservation {
            state: self.state,
            sequence: self.sequence,
            observed_at: self.observed_at.clone(),
            payload_digest: Digest::from_bytes(Sha256::digest(bytes).into()),
        }
    }
}

/// Typed clock injection is for library users and tests; the product read() uses BOOTTIME.
pub struct Reader {
    clock: Arc<dyn Clock>,
}
impl Reader {
    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Self { clock }
    }
    pub fn read(
        &self,
        binding: &GuardedStatusBinding,
        instance: &Id,
        pid: u32,
    ) -> Result<Option<GuardedObservation>> {
        if pid == 0 {
            return Err(invalid("positive owned process ID required"));
        }
        let Some(bytes) = file::read(binding.path())? else {
            return Ok(None);
        };
        let envelope: Envelope =
            canonical::decode_json(&bytes).map_err(|e| invalid(e.to_string()))?;
        envelope.validate(&binding.scope(), instance, pid)?;
        let now = self.clock.now()?;
        if now.clock_id != envelope.observed_at.clock_id
            || now
                .ticks_ns
                .0
                .checked_sub(envelope.observed_at.ticks_ns.0)
                .is_none_or(|age| age > MAX_AGE_NS)
        {
            return Err(invalid(
                "clock differs or source observation is stale/future",
            ));
        }
        Ok(Some(envelope.observation(&bytes)))
    }
}

pub fn read(
    binding: &GuardedStatusBinding,
    instance: &Id,
    pid: u32,
) -> Result<Option<GuardedObservation>> {
    Reader::with_clock(Arc::new(LinuxBoottime::new()?)).read(binding, instance, pid)
}

#[cfg(all(test, unix))]
mod tests;
