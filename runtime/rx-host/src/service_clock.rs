//! Real host clock. Production execution currently targets Linux CLOCK_BOOTTIME.
use crate::{Clock, HostError, Result};
#[cfg(target_os = "linux")]
use rx_domain::types::Id;
use rx_domain::types::{Counter, TimePoint};
#[derive(Clone)]
pub struct SystemClock {
    #[cfg(target_os = "linux")]
    clock_id: String,
}
impl SystemClock {
    #[cfg(target_os = "linux")]
    pub fn new() -> Result<Self> {
        let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .map_err(|e| HostError::Invalid(e.to_string()))?;
        let boot = Id::new(boot.trim()).map_err(|e| HostError::Invalid(e.to_string()))?;
        let clock = Self {
            clock_id: format!("linux-boottime/{boot}"),
        };
        clock.read()?;
        Ok(clock)
    }
    #[cfg(not(target_os = "linux"))]
    pub fn new() -> Result<Self> {
        Err(HostError::Invalid(
            "Host service currently requires the validated Linux boottime clock".into(),
        ))
    }
    #[cfg(target_os = "linux")]
    pub fn read(&self) -> Result<TimePoint> {
        let value = rustix::time::clock_gettime(rustix::time::ClockId::Boottime);
        if value.tv_sec < 0 || value.tv_nsec < 0 {
            return Err(HostError::Invalid("negative boottime".into()));
        }
        let ticks = (value.tv_sec as u64)
            .checked_mul(1_000_000_000)
            .and_then(|t| t.checked_add(value.tv_nsec as u64))
            .ok_or(HostError::Invalid("boottime overflow".into()))?;
        Ok(TimePoint {
            clock_id: self.clock_id.clone(),
            ticks_ns: Counter(ticks),
        })
    }
    #[cfg(not(target_os = "linux"))]
    pub fn read(&self) -> Result<TimePoint> {
        Err(HostError::Invalid("unsupported host clock".into()))
    }
}
impl Clock for SystemClock {
    fn healthy(&self) -> bool {
        self.read().is_ok()
    }
    fn now(&self) -> TimePoint {
        self.read().unwrap_or_else(|_| TimePoint {
            clock_id: "unavailable/host-clock".into(),
            ticks_ns: Counter(u64::MAX),
        })
    }
}
