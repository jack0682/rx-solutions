//! Deterministic process-package candidates and detached signing. No execution or trust installation.
mod candidate;
pub mod directory;
pub mod trust;
pub use candidate::*;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("process package: {0}")]
    Invalid(String),
    #[error("package verification: {0}")]
    Package(#[from] rx_package::Error),
    #[error("process compiler: {0}")]
    Compile(#[from] rx_process::Error),
    #[error("file I/O: {0}")]
    Io(#[from] std::io::Error),
}
pub type Result<T> = std::result::Result<T, Error>;

pub mod investigation;
pub mod review;
