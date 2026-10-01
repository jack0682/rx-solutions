use super::*;
pub trait Clock: Send + Sync {
    fn now(&self) -> Result<TimePoint>;
}
pub struct SystemClock {
    #[cfg(target_os = "linux")]
    id: String,
}
impl SystemClock {
    pub fn new() -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let boot = Id::new(std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?.trim())
                .map_err(invalid)?;
            Ok(Self {
                id: format!("linux-boottime/{boot}"),
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(invalid(
                "resident execution requires the shared Linux boot clock",
            ))
        }
    }
}
impl Clock for SystemClock {
    fn now(&self) -> Result<TimePoint> {
        #[cfg(target_os = "linux")]
        {
            let t = rustix::time::clock_gettime(rustix::time::ClockId::Boottime);
            let ticks = u64::try_from(t.tv_sec)
                .map_err(invalid)?
                .checked_mul(1_000_000_000)
                .and_then(|v| v.checked_add(u64::try_from(t.tv_nsec).ok()?))
                .ok_or_else(|| invalid("boot clock range"))?;
            Ok(TimePoint {
                clock_id: self.id.clone(),
                ticks_ns: Counter(ticks),
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(invalid(
                "resident execution requires the shared Linux boot clock",
            ))
        }
    }
}
