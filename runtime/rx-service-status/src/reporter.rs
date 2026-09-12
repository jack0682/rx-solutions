use crate::*;
use std::ffi::OsString;

pub struct Reporter {
    path: PathBuf,
    scope: GuardedScope,
    instance: Id,
    pid: u32,
    clock: Arc<dyn Clock>,
    sequence: Counter,
    last_time: Option<TimePoint>,
    last_payload: Option<Vec<u8>>,
    _owner: file::Owner,
}
impl Reporter {
    /// No STATUS_PATH opts out of guarded reporting, preserving legacy INSTANCE_ID use.
    /// Once STATUS_PATH exists, invalid/missing instance identity is always an error.
    pub fn from_environment(scope: GuardedScope) -> Result<Option<Self>> {
        let Some((path, instance)) = environment(
            std::env::var_os("RX_PROCESS_STATUS_PATH"),
            std::env::var_os("RX_PROCESS_INSTANCE_ID"),
        )?
        else {
            return Ok(None);
        };
        Self::with_clock(
            path,
            scope,
            instance,
            std::process::id(),
            Arc::new(LinuxBoottime::new()?),
        )
        .map(Some)
    }
    /// Explicit typed clock injection for library callers/tests, never an environment fallback.
    pub fn with_clock(
        path: PathBuf,
        scope: GuardedScope,
        instance: Id,
        pid: u32,
        clock: Arc<dyn Clock>,
    ) -> Result<Self> {
        if pid == 0 {
            return Err(invalid("positive reporter process ID required"));
        }
        let owner = file::Owner::acquire(&path)?;
        let last_payload = file::read(&path)?;
        let (sequence, last_time) = if let Some(bytes) = &last_payload {
            let envelope: Envelope =
                canonical::decode_json(bytes).map_err(|e| invalid(e.to_string()))?;
            envelope.validate(&scope, &instance, pid)?;
            (envelope.sequence, Some(envelope.observed_at))
        } else {
            (Counter(0), None)
        };
        Ok(Self {
            path,
            scope,
            instance,
            pid,
            clock,
            sequence,
            last_time,
            last_payload,
            _owner: owner,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn scope(&self) -> &GuardedScope {
        &self.scope
    }
    pub fn instance(&self) -> &Id {
        &self.instance
    }
    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn publish(&mut self, state: GuardedState) -> Result<GuardedObservation> {
        if file::read(&self.path)? != self.last_payload {
            return Err(invalid("status ownership or content changed"));
        }
        let observed_at = self.clock.now()?;
        if observed_at.clock_id.is_empty()
            || self.last_time.as_ref().is_some_and(|previous| {
                previous.clock_id != observed_at.clock_id
                    || observed_at.ticks_ns < previous.ticks_ns
            })
        {
            return Err(invalid("reporter clock changed or regressed"));
        }
        let sequence = self
            .sequence
            .0
            .checked_add(1)
            .map(Counter)
            .ok_or_else(|| invalid("status sequence exhausted"))?;
        let envelope = Envelope {
            schema: Name::new(SCHEMA).expect("internal status schema"),
            scope: self.scope.clone(),
            instance: self.instance.clone(),
            pid: self.pid,
            sequence,
            observed_at: observed_at.clone(),
            state,
        };
        let bytes = canonical::bytes(&envelope).map_err(|e| invalid(e.to_string()))?;
        file::replace(&self.path, &bytes, self.last_payload.as_deref())?;
        let observation = envelope.observation(&bytes);
        self.sequence = sequence;
        self.last_time = Some(observed_at);
        self.last_payload = Some(bytes);
        Ok(observation)
    }
}

pub(crate) fn environment(
    path: Option<OsString>,
    instance: Option<OsString>,
) -> Result<Option<(PathBuf, Id)>> {
    let Some(path) = path else {
        return Ok(None);
    };
    if path.is_empty() {
        return Err(invalid("empty guarded status path"));
    }
    let instance = instance
        .ok_or_else(|| invalid("guarded status instance required"))?
        .into_string()
        .map_err(|_| invalid("guarded status instance must be UTF-8"))?;
    let instance = Id::new(instance).map_err(|e| invalid(e.to_string()))?;
    Ok(Some((PathBuf::from(path), instance)))
}
