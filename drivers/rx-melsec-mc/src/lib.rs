//! Bounded MELSEC-Q MC 3E binary transport. A write acknowledgement is not a motion result.
//! No CLI, native Host registration, automatic reconnect, retry, or physical address defaults.
mod client;
mod codec;
mod profile;

pub use client::{Client, Failure, FailureStage, WriteAcknowledgement};
pub use profile::{AccessProfile, AddressRange, Configuration, Environment, Route};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid MC configuration: {0}")]
    Configuration(&'static str),
    #[error("MC address/operation is outside the configured access profile")]
    AccessDenied,
    #[error("MC connection is faulted; no further request can be sent")]
    Faulted,
    #[error("MC I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("malformed MC response: {0}")]
    Protocol(&'static str),
    #[error("PLC returned MC end code 0x{code:04x}")]
    Plc { code: u16, diagnostic: Vec<u8> },
}

pub type Result<T> = std::result::Result<T, Error>;
