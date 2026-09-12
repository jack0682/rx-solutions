//! Black-box integration fixture. Only file simulation bindings are accepted.
use rx_domain::types::*;
use rx_host::{
    native::NativeAdapter,
    rpc::{Configuration, RpcHost, TlsMaterial},
    simulation::*,
    *,
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, atomic::AtomicU64},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default)]
    lose_configuration_reply_once: bool,
    #[serde(default)]
    lose_qualification_reply_once: bool,
    directory: PathBuf,
    bindings: Vec<Binding>,
    installation: Id,
    release_digest: Digest,
    client_fingerprint: Digest,
    server_certificate: PathBuf,
    server_key: PathBuf,
    client_ca: PathBuf,
    clock_id: String,
    ticks: u64,
    #[serde(default)]
    test_seed_evidence: Vec<EvidenceRecord>,
    publisher: Option<PublishConfig>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublishConfig {
    uri: String,
    server_name: String,
    server_ca: PathBuf,
    client_certificate: PathBuf,
    client_key: PathBuf,
    store_generation: Id,
}
#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let file = std::env::args().nth(1).ok_or("config file required")?;
    let config: Config = rx_domain::canonical::decode_json(&std::fs::read(file)?)?;
    if config
        .bindings
        .iter()
        .any(|b| b.environment != Environment::Simulation)
    {
        return Err("only simulation is supported by this fixture".into());
    }
    std::fs::create_dir_all(&config.directory)?;
    if !config.test_seed_evidence.is_empty() {
        use rx_ports::Repository;
        let path = config.directory.join("host.db");
        if path.exists() || config.test_seed_evidence.len() > 1024 {
            return Err("test seed requires a new Host journal and <=1024 records".into());
        }
        let mut store = rx_storage::SqliteRepository::open(path)?;
        for record in &config.test_seed_evidence {
            store.transact(|tx| {
                tx.append(
                    &record.evidence_id,
                    &rx_host::journal::doc("rx.host.evidence.v1", record)?,
                )
            })?;
        }
    }
    let clock = ManualClock {
        clock_id: config.clock_id,
        ticks: Arc::new(AtomicU64::new(config.ticks)),
    };
    let native = FileDevice::open(config.directory.join("device"), clock.clone())?;
    let source = native.guard(&config.bindings[0].allowed_intents[0], &clock.now())?;
    let platform = config.bindings[0].platform.clone();
    let host = Arc::new(Host::open(
        config.directory.join("host.db"),
        native,
        clock,
        config.bindings,
    )?);
    let adapter = RpcHost::new(
        host.clone(),
        Configuration {
            installation: config.installation.clone(),
            release_digest: config.release_digest,
            allowed_certificates: BTreeMap::from([(config.client_fingerprint, platform)]),
        },
    )?;
    if config.lose_qualification_reply_once {
        adapter.lose_next_qualification_reply();
    }
    if config.lose_configuration_reply_once {
        adapter.lose_next_configuration_reply();
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    std::fs::write(
        config.directory.join("ready.tmp"),
        serde_json::to_vec(
            &serde_json::json!({"address":format!("https://{address}"),"device_session":source.device_session}),
        )?,
    )?;
    std::fs::rename(
        config.directory.join("ready.tmp"),
        config.directory.join("ready.json"),
    )?;
    let mut publisher_lifetime = None;
    if let Some(target) = config.publisher {
        if !target.uri.starts_with("https://127.0.0.1:") {
            return Err("fixture publication is loopback only".into());
        }
        let publisher = rx_host::publication::Publisher::new(
            host.clone(),
            rx_host::publication::Configuration {
                destination: rx_host::publication::Destination {
                    platform: host.bindings()?[0].platform.clone(),
                    installation: config.installation.clone(),
                    store_generation: target.store_generation,
                },
                release_digest: config.release_digest,
                endpoint: rx_host::publication::Endpoint {
                    uri: target.uri,
                    server_name: target.server_name,
                    server_ca_pem: std::fs::read(target.server_ca)?,
                    client_certificate_pem: std::fs::read(target.client_certificate)?,
                    client_key_pem: std::fs::read(target.client_key)?,
                },
            },
        );
        let mut progress = publisher.subscribe();
        let directory = config.directory.clone();
        tokio::spawn(async move {
            while progress.changed().await.is_ok() {
                let cursor = match progress.borrow_and_update().clone() {
                    rx_host::publication::StatusView::Published(c)
                    | rx_host::publication::StatusView::Idle(c) => Some(c),
                    _ => None,
                };
                if let Some(cursor) = cursor {
                    let bytes = serde_json::to_vec(&cursor).expect("cursor JSON");
                    let _ = std::fs::write(directory.join("publisher.tmp"), bytes);
                    let _ = std::fs::rename(
                        directory.join("publisher.tmp"),
                        directory.join("publisher.json"),
                    );
                }
            }
        });
        let (keep, stop) = tokio::sync::watch::channel(false);
        publisher_lifetime = Some(keep);
        tokio::spawn(async move {
            if let Err(error) = publisher.run(stop).await {
                eprintln!("fixture publisher stopped: {error}");
            }
        });
    }
    adapter
        .serve(
            listener,
            TlsMaterial {
                server_certificate_pem: std::fs::read(config.server_certificate)?,
                server_key_pem: std::fs::read(config.server_key)?,
                client_ca_pem: std::fs::read(config.client_ca)?,
            },
            std::future::pending(),
        )
        .await?;
    drop(publisher_lifetime);
    Ok(())
}
