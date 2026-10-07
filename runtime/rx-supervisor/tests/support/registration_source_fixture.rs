//! Actual legacy registry writer and source fence for cross-repository intake tests.
use rx_domain::types::{Counter, Id, Name};
use rx_storage::SqliteRepository;
use rx_supervisor::registration::{FreezeRequest, Registry};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("fixture DB TARGET_INSTALLATION FREEZE_ID".into());
    }
    let mut registry = Registry::new(SqliteRepository::open(&args[0])?);
    let first = registry.list()?.into_iter().next().ok_or("source empty")?;
    let mut declaration = first.registration.declaration;
    declaration.label = Name::new("source-history-revised")?;
    registry.update(&first.registration.id, first.revision, declaration)?;
    let frozen = registry.freeze_for_transfer(FreezeRequest {
        id: Id::new(&args[2])?,
        target_installation: Id::new(&args[1])?,
    })?;
    let mut history = Vec::new();
    let mut after = Counter(0);
    loop {
        let page = registry.frozen_history(&frozen.record().request.id, after, 128)?;
        if page.is_empty() {
            break;
        }
        after = page.last().unwrap().seq;
        history.extend(page);
    }
    println!("{}", serde_json::json!({"freeze":frozen,"history":history}));
    registry.into_repository().close()?;
    Ok(())
}
