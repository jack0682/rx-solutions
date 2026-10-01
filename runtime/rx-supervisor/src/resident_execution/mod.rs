//! P-authorized process lifecycle, using the existing Supervisor and OS admission boundaries.
mod catalog;
mod client;
mod clock;
mod delivery;
mod grant;
pub mod run;
pub use crate::reporting::Connection;
use crate::{Error, Result};
pub use catalog::{Catalog, Prepared};
pub use client::Client;
pub use clock::{Clock, SystemClock};
pub use delivery::Delivery;
pub(crate) use grant::Lease;
pub use grant::{Authority, LiveGrant};
use rx_domain::{canonical, resident_execution as data, types::*};
use std::sync::Arc;
fn invalid(value: impl std::fmt::Display) -> Error {
    Error::Invalid(value.to_string())
}

#[cfg(test)]
mod tests;
