//! Explicit source freeze, export and authenticated reconciliation; never launches processes.
use rx_domain::types::{Counter, Id};
use rx_storage::SqliteRepository;
use rx_supervisor::registration::{FreezeRequest, Registry};
use std::{fs, path::Path};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !matches!(
        args.first().map(String::as_str),
        Some("freeze" | "inspect" | "history" | "reconcile")
    ) || !(match args.first().map(String::as_str) {
        Some("freeze") => args.len() == 4,
        Some("inspect") => args.len() == 2,
        Some("history") => args.len() == 5,
        Some("reconcile") => args.len() == 3,
        _ => false,
    }) {
        return Err("usage: rx-registration-transfer freeze REGISTRY_DB REQUEST_ID TARGET_INSTALLATION; inspect REGISTRY_DB; history REGISTRY_DB FREEZE_ID AFTER LIMIT; reconcile REGISTRY_DB CONNECTION_FILE (read owner-issued component/scope JSON from stdin)".into());
    }
    let path = Path::new(&args[1]);
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() == 0 {
        return Err("existing regular registry database required".into());
    }
    let store = if args[0] == "reconcile" {
        SqliteRepository::open_sealed_existing(path)?
    } else {
        SqliteRepository::open(path)?
    };
    let mut registry = Registry::new(store);
    match args[0].as_str() {
        "reconcile" => {
            use std::io::{BufRead, Read, Write};
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Authorization {
                component: Id,
                scope: Id,
            }
            let original = registry.frozen_source()?;
            let connection = rx_supervisor::reporting::resident::connection(Path::new(&args[2]))?;
            let mut client = rx_supervisor::reporting::Client::connect(connection).await?;
            println!(
                "{}",
                serde_json::json!({"state":"AWAITING_OWNER_SCOPE","peer":client.peer(),"source":original.record()})
            );
            std::io::stdout().flush()?;
            let line = tokio::task::spawn_blocking(|| -> Result<String> {
                let mut line = String::new();
                std::io::stdin().lock().take(65537).read_line(&mut line)?;
                if line.len() > 65536 {
                    return Err("authorization input too large".into());
                }
                Ok(line)
            })
            .await??;
            let authorization: Authorization = rx_domain::canonical::decode_json(line.as_bytes())?;
            let scope = client
                .inspect_scope(
                    &authorization.scope,
                    &authorization.component,
                    &authorization.component,
                )
                .await?;
            let evidence = client
                .registration_acceptance(&scope, original.record())
                .await?;
            let recorded = registry.record_platform_acceptance(&evidence)?;
            println!(
                "{}",
                serde_json::json!({"state":"RECORDED_FROM_AUTHENTICATED_PLATFORM","acceptance":recorded,"current_observation":evidence.view(),"process_ownership":"NOT_TRANSFERRED","work_use_permission":"NOT_EVALUATED"})
            );
        }
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
