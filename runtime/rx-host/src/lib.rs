//! Device-side delivery facts and gate. This crate never assigns platform outcomes.
pub mod gate;
pub mod journal;
pub mod melsec;
pub mod model;
pub mod native;
pub mod publication;
pub mod ros_jtc;
pub mod rpc;
pub mod simulation;
pub use gate::Host;
pub use model::*;

pub mod service;
pub mod service_clock;
pub use native::NativeAdapter;
