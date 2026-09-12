//! Acquire immutable candidate bytes and publish a new directory without replacing an existing artifact.
use crate::{Candidate, Error, Result};
use rx_package::{PackagePath, SignatureEnvelope};
use std::{collections::BTreeMap, path::Path};
pub fn candidate(path: &Path) -> Result<Candidate> {
    crate::from_files(rx_package::directory::acquire_directory(
        path,
        32,
        4 * 1024 * 1024,
    )?)
}
pub fn publish(
    candidate: &Candidate,
    signature: Option<&SignatureEnvelope>,
    out: &Path,
) -> Result<()> {
    let mut files = candidate.files().clone();
    files.insert(
        PackagePath::new("manifest.json").expect("literal"),
        rx_package::manifest_bytes(candidate.manifest())?,
    );
    if let Some(signature) = signature {
        files.insert(
            PackagePath::new("manifest.sig.json").expect("literal"),
            rx_domain::canonical::bytes(signature).map_err(|e| Error::Invalid(e.to_string()))?,
        );
    }
    publish_files(files, out)
}
pub fn publish_files(files: BTreeMap<PackagePath, Vec<u8>>, out: &Path) -> Result<()> {
    rx_package::directory::publish_files(files, out).map_err(Into::into)
}
