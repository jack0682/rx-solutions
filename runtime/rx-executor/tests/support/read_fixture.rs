//! Explicit simulation-only read fixture. No native driver or mutation calls.
use rx_domain::{canonical, types::*};
use rx_executor::{Client, Error, PeerPin, TlsEndpoint, clock::Clock};
use serde::Deserialize;
use std::{path::PathBuf, sync::Arc};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    uri: String,
    server_name: String,
    ca: PathBuf,
    certificate: PathBuf,
    key: PathBuf,
    pin: PeerPin,
    run: Id,
    visit: Counter,
    ticks: Counter,
    output: PathBuf,
}
struct FrozenClock(TimePoint);
impl Clock for FrozenClock {
    fn now(&self) -> Result<TimePoint, Error> {
        Ok(self.0.clone())
    }
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config: Config = canonical::decode_json(&std::fs::read(
        std::env::args().nth(1).ok_or("config required")?,
    )?)?;
    if !config.uri.starts_with("https://127.0.0.1:")
        || !(config.pin.clock_id == "test-clock"
            || config.pin.clock_id.starts_with("simulation/")
            || config.pin.clock_id.starts_with("test/"))
    {
        return Err("fixture requires an explicit loopback simulation endpoint and clock".into());
    }
    let endpoint = TlsEndpoint {
        uri: config.uri,
        server_name: config.server_name,
        ca_pem: std::fs::read(config.ca)?,
        certificate_pem: std::fs::read(config.certificate)?,
        private_key_pem: std::fs::read(config.key)?,
    };
    let clock = Arc::new(FrozenClock(TimePoint {
        clock_id: config.pin.clock_id.clone(),
        ticks_ns: config.ticks,
    }));
    let mut client = Client::connect(endpoint, config.pin, clock).await?;
    let restored = client.restore(&config.run).await?;
    let snapshot = client.snapshot(&config.run, config.visit).await?;
    let frame = snapshot.frame(&snapshot.context_identity())?;
    let xml = rx_process::bt_xml::generate(snapshot.process()).map_err(std::io::Error::other)?;
    std::fs::create_dir(&config.output)?;
    for (file, bytes) in [
        ("checkpoint.json", canonical::bytes(&restored)?),
        ("snapshot.json", canonical::bytes(snapshot.data())?),
        ("resolved.json", canonical::bytes(snapshot.process())?),
        ("frame.json", canonical::bytes(&frame)?),
        ("process.bt.xml", xml.into_bytes()),
    ] {
        std::fs::write(config.output.join(file), bytes)?;
    }
    println!("READ_VALIDATED_NO_NATIVE_EFFECT");
    Ok(())
}
