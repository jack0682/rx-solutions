//! Explicit local source freeze/export. Does not contact P, launch processes or adopt PIDs.
use rx_domain::types::{Counter, Id};
use rx_storage::SqliteRepository;
use rx_supervisor::registration::{FreezeRequest, Registry};
use std::{fs, path::Path};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !matches!(
        args.first().map(String::as_str),
        Some("freeze" | "inspect" | "history")
    ) || !(match args.first().map(String::as_str) {
        Some("freeze") => args.len() == 4,
        Some("inspect") => args.len() == 2,
        Some("history") => args.len() == 5,
        _ => false,
    }) {
        return Err("usage: rx-registration-transfer freeze REGISTRY_DB REQUEST_ID TARGET_INSTALLATION; inspect REGISTRY_DB; history REGISTRY_DB FREEZE_ID AFTER LIMIT".into());
    }
    let path = Path::new(&args[1]);
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() == 0 {
        return Err("existing regular registry database required".into());
    }
    let mut registry = Registry::new(SqliteRepository::open(path)?);
    match args[0].as_str() {
        "freeze" => println!(
            "{}",
            serde_json::to_string(&registry.freeze_for_transfer(FreezeRequest {
                id: Id::new(&args[2])?,
                target_installation: Id::new(&args[3])?
            })?)?
        ),
        "inspect" => println!("{}", serde_json::to_string(&registry.frozen_source()?)?),
        "history" => println!(
            "{}",
            serde_json::to_string(&registry.frozen_history(
                &Id::new(&args[2])?,
                Counter(args[3].parse()?),
                args[4].parse()?
            )?)?
        ),
        _ => unreachable!(),
    }
    registry.into_repository().close()?;
    Ok(())
}
