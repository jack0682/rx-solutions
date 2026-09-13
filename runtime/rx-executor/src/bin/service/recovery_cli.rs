//! Offline local inspection only: no Client, clock, reporter, planner or native lifecycle.
use super::*;
use rx_executor::assignment_journal::recovery;

pub(super) fn inspect(path: &Path) -> Result<()> {
    recovery::validate_path(path)?;
    let mut config = CellConfig::read(path)?;
    recovery::validate_path(&config.service_root.join(JOURNAL_FILE))?;
    let metadata = fs::symlink_metadata(&config.service_root)?;
    if !metadata.file_type().is_dir() {
        return Err("existing real recovery service root required".into());
    }
    let _owner = ServiceOwner::acquire_existing(&config.service_root.join(JOURNAL_FILE))?;
    config.normalize_root()?;
    let installation: Installation = canonical::decode_json(&read_regular(
        &config.service_root.join(INSTALLATION_FILE),
        CONFIG_LIMIT,
        false,
    )?)?;
    installation.verify(&config)?;
    let report = recovery::inspect_current(
        &config.service_root,
        &config.expected_service,
        config.digest()?,
    )?;
    let output = canonical::bytes(&report)?;
    if output.len() > 8 * 1024 * 1024 {
        return Err("RECOVERY_INSPECT_LIMIT_EXCEEDED: output exceeds 8 MiB".into());
    }
    // Print exactly one deterministic JSON value only after every source has been verified.
    std::io::stdout().lock().write_all(&output)?;
    std::io::stdout().lock().write_all(b"\n")?;
    Ok(())
}
