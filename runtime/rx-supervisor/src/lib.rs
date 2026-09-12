pub mod builtin;
pub mod initialization;
pub mod model;
mod plan;
pub mod process;
pub mod supervisor;
pub use supervisor::Supervisor;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid supervisor input: {0}")]
    Invalid(String),
    #[error("storage: {0}")]
    Storage(#[from] rx_ports::StoreError),
    #[error("process: {0}")]
    Io(#[from] std::io::Error),
    #[error("reconciliation required: {0}")]
    Reconciliation(String),
}
pub type Result<T> = std::result::Result<T, Error>;
