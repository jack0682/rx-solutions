use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, net::SocketAddrV4, time::Duration};

/// Every routing byte is explicit. Configuration does not discover or modify the PLC.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub network: u8,
    pub pc: u8,
    pub module_io: u16,
    pub station: u8,
}
impl Route {
    pub(crate) fn bytes(self) -> [u8; 5] {
        let io = self.module_io.to_le_bytes();
        [self.network, self.pc, io[0], io[1], self.station]
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Environment {
    Simulation,
    Physical,
}

/// Inclusive range in decimal device numbers, not byte offsets.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddressRange {
    pub first: u32,
    pub last: u32,
}
impl AddressRange {
    fn validate(self, limit: u32) -> Result<()> {
        if self.first > self.last || self.last > limit {
            return Err(Error::Configuration(
                "address range exceeds configured device limit",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessProfile {
    /// Must come from the reviewed CPU parameter/device allocation, not the 24-bit wire limit.
    pub m_last: u32,
    pub d_last: u32,
    pub read_m: Vec<AddressRange>,
    pub read_d: Vec<AddressRange>,
    /// Individual request bits only. No word write, Y/X write, or arbitrary device code.
    pub write_m: BTreeSet<u32>,
}
impl AccessProfile {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.m_last > 0xff_ffff || self.d_last > 0xff_ffff {
            return Err(Error::Configuration(
                "device limit exceeds 24-bit addressing",
            ));
        }
        for (ranges, limit) in [(&self.read_m, self.m_last), (&self.read_d, self.d_last)] {
            if ranges.len() > 64 {
                return Err(Error::Configuration("too many read ranges"));
            }
            let mut previous = None;
            for range in ranges {
                range.validate(limit)?;
                if previous.is_some_and(|last| range.first <= last) {
                    return Err(Error::Configuration(
                        "read ranges must be sorted and disjoint",
                    ));
                }
                previous = Some(range.last);
            }
        }
        if self.write_m.len() > 64 || self.write_m.iter().any(|n| *n > self.m_last) {
            return Err(Error::Configuration(
                "write addresses exceed profile bounds",
            ));
        }
        Ok(())
    }
    pub(crate) fn permits(ranges: &[AddressRange], first: u32, count: u16, max: u16) -> bool {
        if count == 0 || count > max {
            return false;
        }
        let Some(last) = first.checked_add(u32::from(count) - 1) else {
            return false;
        };
        ranges.iter().any(|r| r.first <= first && last <= r.last)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub environment: Environment,
    pub endpoint: SocketAddrV4,
    pub route: Route,
    /// PLC monitoring timer in 250 ms units; zero (unbounded) is forbidden.
    pub monitoring_timer: u16,
    pub connect_timeout_ms: u32,
    /// One total deadline for request write + full response, including fragmented input.
    pub exchange_timeout_ms: u32,
    pub access: AccessProfile,
}
impl Configuration {
    /// Exact bytes of an allowlisted single-bit request for a caller's durable send record.
    /// Pure encoding only; there is still no API to send arbitrary bytes.
    pub fn write_m_frame(&self, address: u32, value: bool) -> Result<Vec<u8>> {
        self.validate()?;
        if !self.access.write_m.contains(&address) {
            return Err(Error::AccessDenied);
        }
        Ok(crate::codec::Request::WriteM { address, value }
            .encode(self.route, self.monitoring_timer))
    }
    pub fn validate(&self) -> Result<()> {
        let ip = self.endpoint.ip();
        if ip.is_unspecified()
            || ip.is_multicast()
            || ip.is_broadcast()
            || self.endpoint.port() == 0
        {
            return Err(Error::Configuration(
                "explicit unicast IPv4 endpoint required",
            ));
        }
        if self.environment == Environment::Simulation && !ip.is_loopback() {
            return Err(Error::Configuration("simulation endpoint must be loopback"));
        }
        if !(1..=5000).contains(&self.connect_timeout_ms)
            || !(1..=5000).contains(&self.exchange_timeout_ms)
            || !(1..=20).contains(&self.monitoring_timer)
        {
            return Err(Error::Configuration(
                "finite timeout required (maximum 5 seconds)",
            ));
        }
        self.access.validate()
    }
    pub(crate) fn exchange_timeout(&self) -> Duration {
        Duration::from_millis(self.exchange_timeout_ms.into())
    }
}
