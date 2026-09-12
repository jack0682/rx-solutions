use crate::*;
use std::path::Path;
pub fn candidate(path: &Path) -> Result<Candidate> {
    from_files(rx_package::directory::acquire_directory(
        path,
        10,
        4 * 1024 * 1024,
    )?)
}
pub fn publish(
    candidate: &Candidate,
    signature: Option<&SignatureEnvelope>,
    output: &Path,
) -> Result<()> {
    let mut files = candidate.files().clone();
    files.insert(path("manifest.json"), manifest_bytes(candidate.manifest())?);
    if let Some(signature) = signature {
        files.insert(path("manifest.sig.json"), bytes(signature)?);
    } else {
        files.insert(path("candidate-recipe.json"), bytes(candidate.recipe())?);
    }
    rx_package::directory::publish_files(files, output)?;
    Ok(())
}
