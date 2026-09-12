use crate::Error;
use rx_domain::types::*;
/// Trusted same-host clock adapter, not a timestamp supplied by a remote payload.
pub trait Clock: Send + Sync {
    fn now(&self) -> Result<TimePoint, Error>;
}
#[cfg(target_os = "linux")]
pub struct LinuxBoottime {
    id: String,
}
#[cfg(target_os = "linux")]
impl LinuxBoottime {
    pub fn new() -> Result<Self, Error> {
        let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .map_err(|e| Error::Invalid(format!("host boot identity: {e}")))?;
        let boot = Id::new(boot.trim()).map_err(|e| Error::Invalid(e.to_string()))?;
        Ok(Self {
            id: format!("linux-boottime/{boot}"),
        })
    }
}
#[cfg(target_os = "linux")]
impl Clock for LinuxBoottime {
    fn now(&self) -> Result<TimePoint, Error> {
        let time = rustix::time::clock_gettime(rustix::time::ClockId::Boottime);
        let seconds =
            u64::try_from(time.tv_sec).map_err(|_| Error::Invalid("negative boottime".into()))?;
        let nanos =
            u64::try_from(time.tv_nsec).map_err(|_| Error::Invalid("negative nanos".into()))?;
        let ticks = seconds
            .checked_mul(1_000_000_000)
            .and_then(|v| v.checked_add(nanos))
            .ok_or_else(|| Error::Invalid("boottime overflow".into()))?;
        Ok(TimePoint {
            clock_id: self.id.clone(),
            ticks_ns: Counter(ticks),
        })
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    #[test]
    fn linux_clock_uses_the_shared_boot_identity_and_non_decreasing_boottime() {
        let clock = LinuxBoottime::new().unwrap();
        let first = clock.now().unwrap();
        let second = clock.now().unwrap();
        assert!(first.clock_id.starts_with("linux-boottime/"));
        assert_eq!(first.clock_id, second.clock_id);
        assert!(second.ticks_ns >= first.ticks_ns);
    }
}
