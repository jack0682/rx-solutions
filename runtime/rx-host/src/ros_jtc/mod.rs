//! ROS ROS action adapter. Physical authority must come from a separate release-owned provider.
mod adapter;
pub mod authoring;
mod journal;
mod outcome;
pub mod process;
mod profile;
pub use adapter::{Boundary, Jtc, NoBoundary};
pub use journal::{Entry, Identity};
pub use profile::{Authority, AuthoritySnapshot, Profile, TrajectoryAsset, UnavailableAuthority};
pub mod protocol;
pub fn initialize_journal(
    directory: &std::path::Path,
    profile: &Profile,
) -> crate::Result<Identity> {
    journal::initialize(directory, profile)
}
