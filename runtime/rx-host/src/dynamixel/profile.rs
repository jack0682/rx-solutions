use crate::{Binding, Environment, HostError, Result, service::config::Backend};
use rx_domain::{
    canonical,
    intent::{Body, Intent, Kind},
    types::*,
};
use std::{fs, io::Read, path::Path};
pub const PROFILE: &str = "rx/dynamixel-protocol2-ping-simulation-v1";
pub const ENDPOINT: &str = "simulation/dynamixel/id-1";
pub const BINARY: &str = "/opt/rx/bin/rx-dynamixel-ping";
pub const COMPLETION: &str = "rx.dynamixel.ping.v1";
fn invalid(e: impl std::fmt::Display) -> HostError {
    HostError::Invalid(e.to_string())
}
pub fn content() -> serde_json::Value {
    serde_json::json!({
        "profile":serde_json::from_str::<serde_json::Value>(include_str!("../../../../native/dynamixel/profile.json")).expect("compiled profile"),
        "program":serde_json::from_str::<serde_json::Value>(include_str!("../../../../native/dynamixel/program.json")).expect("compiled program"),
        "parameters":serde_json::from_str::<serde_json::Value>(include_str!("../../../../native/dynamixel/parameters.json")).expect("compiled parameters")
    })
}
pub fn reference(key: &str) -> ArtifactRef {
    let value = &content()[key];
    let bytes = canonical::bytes(value).expect("literal");
    ArtifactRef {
        sha256: rx_package::content_digest(&bytes),
        size_bytes: Counter(bytes.len() as u64),
        schema_id: Name::new(value["schema"].as_str().expect("schema")).expect("schema name"),
    }
}
pub fn source_digest() -> Digest {
    serde_json::from_value(serde_json::Value::String(
        env!("RX_DYNAMIXEL_SOURCE_SHA256").into(),
    ))
    .expect("build pin")
}
pub fn descriptor() -> serde_json::Value {
    serde_json::json!({"schema":"rx.dynamixel-driver.v1","profile":PROFILE,"source_digest":source_digest(),"endpoint":ENDPOINT,"materials":content(),"intent_contract":{"target":"device/dynamixel-simulation","completion":COMPLETION,"cancel":"rx.dynamixel.no-motion.v1"},"scope":"READ_ONLY_PROTOCOL2_PING_SIMULATED_TRANSPORT","physical_qualification":"NOT_PERFORMED","sdk_baseline_complete":false,"robotis_bundle_complete":false})
}
pub fn validate_intent(intent: &Intent) -> Result<()> {
    let Body::Program(goal) = &intent.body else {
        return Err(invalid("DXL_PING_CONTRACT_REQUIRED"));
    };
    if intent.kind != Kind::FiniteAction
        || intent.target.as_str() != "device/dynamixel-simulation"
        || intent.profile_digest != reference("profile").sha256
        || !intent.calibration_digests.is_empty()
        || intent.completion_rule.as_str() != COMPLETION
        || intent.cancel_rule.as_str() != "rx.dynamixel.no-motion.v1"
        || goal.program != reference("program")
        || goal.parameter_set != reference("parameters")
    {
        return Err(invalid("DXL_PING_CONTRACT_REQUIRED"));
    }
    Ok(())
}
pub fn validate(backend: &Backend, bindings: &[Binding]) -> Result<()> {
    let Backend::ValidatedDriver {
        profile,
        driver_digest,
        endpoint,
    } = backend
    else {
        return Err(invalid("DXL_PROFILE_REQUIRED"));
    };
    if endpoint != ENDPOINT {
        return Err(invalid("DXL_REAL_ENDPOINT_UNSUPPORTED"));
    }
    if profile.as_str() != PROFILE || *driver_digest != source_digest() {
        return Err(invalid("DXL_PROFILE_OR_SOURCE_UNSUPPORTED"));
    }
    if bindings.is_empty()
        || bindings
            .iter()
            .any(|b| b.environment != Environment::Simulation)
    {
        return Err(invalid("DXL_SIMULATION_ONLY"));
    }
    for binding in bindings {
        for intent in &binding.allowed_intents {
            validate_intent(intent)?;
        }
    }
    Ok(())
}
/// Existing G2 compiled development release root authenticates the executable pin.
/// The source descriptor cannot authorize arbitrary binaries or paths by itself.
pub fn executable_pin(root: &Path) -> Result<Digest> {
    let read = |name: &str| -> Result<Vec<u8>> {
        rx_package::directory::read_relative_file(
            root,
            &rx_package::PackagePath::new(format!("manifests/{name}")).map_err(invalid)?,
            1_048_576,
        )
        .map_err(invalid)
    };
    let release = rx_package::release::verify(
        &read("release.json")?,
        &read("revocations.json")?,
        &read("runtime-files.json")?,
        |name, external| {
            let path = if external {
                std::path::PathBuf::from(name)
            } else {
                root.join(name)
            };
            let file = fs::File::open(&path)
                .map_err(|_| rx_package::release::Error::Content(name.into()))?;
            let mut bytes = Vec::new();
            file.take(536_870_913)
                .read_to_end(&mut bytes)
                .map_err(|_| rx_package::release::Error::Content(name.into()))?;
            if bytes.len() > 536_870_912 {
                return Err(rx_package::release::Error::Content(name.into()));
            }
            Ok(bytes)
        },
    )
    .map_err(invalid)?;
    let source = read("dynamixel-driver.json")?;
    let parsed: serde_json::Value = canonical::decode_json(&source).map_err(invalid)?;
    if parsed != descriptor()
        || !release
            .inventory()
            .files
            .contains_key("manifests/dynamixel-driver.json")
    {
        return Err(invalid("DXL_RELEASE_SOURCE_DESCRIPTOR_MISMATCH"));
    }
    release
        .inventory()
        .files
        .get("bin/rx-dynamixel-ping")
        .copied()
        .ok_or_else(|| invalid("DXL_HELPER_NOT_IN_AUTHENTICATED_RELEASE"))
}
