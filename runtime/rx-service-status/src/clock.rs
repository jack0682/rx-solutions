#[cfg(not(target_os = "linux"))]
use crate::Error;
use crate::Result;
#[cfg(target_os = "linux")]
use crate::invalid;
use rx_domain::types::TimePoint;
#[cfg(target_os = "linux")]
use rx_domain::types::{Counter, Id};

pub trait Clock: Send + Sync {
    fn now(&self) -> Result<TimePoint>;
}

pub struct LinuxBoottime {
    #[cfg(target_os = "linux")]
    id: String,
}
impl LinuxBoottime {
    #[cfg(target_os = "linux")]
    pub fn new() -> Result<Self> {
        let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
        let boot = Id::new(boot.trim()).map_err(|e| invalid(e.to_string()))?;
        Ok(Self {
            id: format!("linux-boottime/{boot}"),
        })
    }
    #[cfg(not(target_os = "linux"))]
    pub fn new() -> Result<Self> {
        Err(Error::Unsupported)
    }
}
impl Clock for LinuxBoottime {
    #[cfg(target_os = "linux")]
    fn now(&self) -> Result<TimePoint> {
        let time = rustix::time::clock_gettime(rustix::time::ClockId::Boottime);
        let seconds =
            u64::try_from(time.tv_sec).map_err(|_| invalid("negative BOOTTIME seconds"))?;
        let nanos = u64::try_from(time.tv_nsec).map_err(|_| invalid("negative BOOTTIME nanos"))?;
        let ticks = seconds
            .checked_mul(1_000_000_000)
            .and_then(|n| n.checked_add(nanos))
            .ok_or_else(|| invalid("BOOTTIME overflow"))?;
        Ok(TimePoint {
            clock_id: self.id.clone(),
            ticks_ns: Counter(ticks),
        })
    }
    #[cfg(not(target_os = "linux"))]
    fn now(&self) -> Result<TimePoint> {
        Err(Error::Unsupported)
    }
}
