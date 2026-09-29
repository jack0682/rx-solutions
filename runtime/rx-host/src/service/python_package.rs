//! Deterministic signed declarations for Python SDK programs. No environment import or I/O.
use super::{
    Result,
    device_package::{Driver, Family},
    python_skill::Registration,
};
use rx_domain::{canonical, intent::Body, types::*};
use rx_package::{PackagePath, VerifiedPackage};
use rx_process_contract::{
    device_catalog::{Catalog, Document, Environment},
    native_outcome::*,
};
use std::collections::{BTreeMap, BTreeSet};
fn name(v: &str) -> Name {
    Name::new(v).expect("constant")
}
fn path(v: &str) -> PackagePath {
    PackagePath::new(v).expect("constant")
}
pub fn driver() -> Driver {
    Driver {
        schema: name("rx.native-driver-reference.v1"),
        implementation: name("rx.python.sdk.v1"),
        source_digest: canonical::digest(
            "RX-PYTHON-SDK-DRIVER-v1",
            &(
                env!("RX_MELSEC_DRIVER_SOURCE_SHA256"),
                rx_package::content_digest(include_bytes!(
                    "../../../../deployment/local-skills/host_runner.py"
                )),
                rx_package::content_digest(include_bytes!(
                    "../../../../deployment/local-skills/python_environment.py"
                )),
            ),
        )
        .expect("static driver"),
    }
}
pub fn documents(
    registration: &Registration,
    environment: &[u8],
) -> Result<(Registration, BTreeMap<PackagePath, Vec<u8>>)> {
    if environment.len() > 1_048_576 {
        return Err("Python manifest exceeds package asset bound".into());
    }
    let mut profile = registration.clone();
    if profile.schema.as_str() != "rx.python-skill-registration.v1"
        || !profile.environment.is_absolute()
        || profile.intent.completion_rule.as_str() != "rx.python.returned.v1"
        || profile.intent.execution_timeout_ms.0 > 60000
    {
        return Err("Python package profile scope differs".into());
    }
    profile.intent = profile.intent.normalized()?;
    let Body::Program(goal) = &profile.intent.body else {
        return Err("Python ProgramGoal required".into());
    };
    let input = canonical::bytes(&profile.input)?;
    if rx_package::content_digest(environment) != goal.program.sha256
        || environment.len() as u64 != goal.program.size_bytes.0
        || rx_package::content_digest(&input) != goal.parameter_set.sha256
        || input.len() as u64 != goal.parameter_set.size_bytes.0
    {
        return Err("Python package program/input identity differs".into());
    }
    let env: serde_json::Value = canonical::decode_json(environment)?;
    if env["environment_digest"] != profile.environment_digest.to_string()
        || env["environment"]["schema"] != "rx.python-environment.v1"
        || env["environment"]["path"] != profile.environment.to_string_lossy().as_ref()
        || env["environment"]["python"]["platform"] != "linux"
    {
        return Err(
            "Python environment metadata/target differs; prepare in the target Linux image".into(),
        );
    }
    if rx_package::content_digest(&canonical::bytes(&env["environment"])?).to_string()
        != profile.environment_digest.to_string()
    {
        return Err("Python environment content digest differs".into());
    }
    profile.intent.profile_digest = Digest::from_bytes([0; 32]);
    profile.intent.profile_digest = canonical::digest("RX-PYTHON-SDK-PROFILE-v1", &profile)?;
    let outcomes = NativeOutcomeTable {
        schema: name("rx.native-outcome-table.v1"),
        profile_digest: profile.intent.profile_digest,
        completion_rule: name("rx.python.returned.v1"),
        cases: vec![NativeOutcomeCase {
            status_schema: name("rx.python.returned.v1"),
            statuses: vec![Integer(0)],
            conclusion: NativeConclusion::Succeeded,
        }],
    };
    let operations = BTreeMap::from([(name("skill/run"), profile.intent.clone())]);
    let family = Family {
        schema: name("rx.device-family.v1"),
        family: name("python-sdk"),
        controller_model: name("python-function"),
        environment: crate::Environment::Simulation,
    };
    let mut files = BTreeMap::from([
        (path("family.json"), canonical::bytes(&family)?),
        (path("profile.json"), canonical::bytes(&profile)?),
        (path("adapter.json"), canonical::bytes(&driver())?),
        (path("operations.json"), canonical::bytes(&operations)?),
        (path("outcomes.json"), canonical::bytes(&outcomes)?),
        (path("environment.json"), environment.to_vec()),
        (
            path("authoring/assembly.json"),
            canonical::bytes(
                &serde_json::json!({"schema":"rx.python-skill-assembly.v1","registration":registration}),
            )?,
        ),
    ]);
    let documents = ["family", "profile", "adapter", "operations", "outcomes"]
        .into_iter()
        .map(|role| {
            let file = format!("{role}.json");
            let bytes = &files[&PackagePath::new(&file).expect("file")];
            let schema = if role == "operations" {
                "rx.python-operations.v1".to_string()
            } else {
                let value: serde_json::Value =
                    serde_json::from_slice(bytes).expect("encoded document");
                value["schema"].as_str().expect("schema").to_string()
            };
            (
                name(role),
                Document {
                    path: file,
                    artifact: ArtifactRef {
                        sha256: rx_package::content_digest(bytes),
                        schema_id: name(&schema),
                        size_bytes: Counter(bytes.len() as u64),
                    },
                },
            )
        })
        .collect();
    let catalog = Catalog {
        schema: name("rx.device-operation-catalog.v1"),
        installation: profile.installation.clone(),
        cell: profile.cell.clone(),
        target: profile.intent.target.clone(),
        environment: Environment::Simulation,
        profile_digest: profile.intent.profile_digest,
        condition_ids: BTreeSet::from([name("sim/ready")]),
        operations,
        outcomes: Some(outcomes),
        documents,
    };
    catalog.validate()?;
    files.insert(path("device-catalog.json"), canonical::bytes(&catalog)?);
    Ok((profile, files))
}
pub fn decode(package: &VerifiedPackage) -> Result<Registration> {
    let original: serde_json::Value = canonical::decode_json(
        package
            .file(&path("authoring/assembly.json"))
            .ok_or("Python assembly absent")?,
    )?;
    if original["schema"] != "rx.python-skill-assembly.v1" {
        return Err("Python assembly schema differs".into());
    }
    let registration: Registration = serde_json::from_value(original["registration"].clone())?;
    let (profile, files) = documents(
        &registration,
        package
            .file(&path("environment.json"))
            .ok_or("Python environment manifest absent")?,
    )?;
    let expected_entry = rx_package::EntryPoint::DeviceReference {
        family: path("family.json"),
        profiles: vec![path("profile.json")],
        adapter: path("adapter.json"),
    };
    if canonical::bytes(&package.manifest().entry)? != canonical::bytes(&expected_entry)? {
        return Err("Python device entry differs".into());
    }
    let expected_permissions = BTreeSet::from([
        rx_package::Permission::ArtifactRead,
        rx_package::Permission::NativeEndpoint {
            role: name("python-sdk"),
        },
    ]);
    if package
        .manifest()
        .permissions
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        != expected_permissions
        || !package.manifest().dependencies.is_empty()
    {
        return Err("Python package permissions/dependencies differ".into());
    }
    let Body::Program(goal) = &profile.intent.body else {
        return Err("Python program absent".into());
    };
    let mut expected_assets = vec![goal.program.clone(), goal.parameter_set.clone()];
    expected_assets.sort_by_key(|a| a.sha256);
    let mut assets = package.manifest().assets.clone();
    assets.sort_by_key(|a| a.sha256);
    if canonical::bytes(&assets)? != canonical::bytes(&expected_assets)? {
        return Err("Python program/input asset declarations differ".into());
    }
    if package.manifest().files.len() != files.len() {
        return Err("Python package has unexpected files".into());
    }
    for (path, bytes) in files {
        if package.file(&path) != Some(bytes.as_slice()) {
            return Err("signed Python package differs from original reassembly".into());
        }
    }
    let metadata: serde_json::Value = canonical::decode_json(
        package
            .file(&path("environment.json"))
            .ok_or("Python environment missing")?,
    )?;
    let target = package
        .manifest()
        .targets
        .first()
        .ok_or("Python package target missing")?;
    let machine = match target.architecture {
        rx_package::Architecture::Arm64 => "aarch64",
        rx_package::Architecture::Amd64 => "x86_64",
    };
    if package.manifest().targets.len() != 1
        || target.os != rx_package::OperatingSystem::Linux
        || target.ros_distribution.is_some()
        || metadata["environment"]["python"]["machine"] != machine
        || metadata["environment"]["skill"]["version"] != package.manifest().version.to_string()
    {
        return Err("signed Python package target/version differs from environment".into());
    }
    Ok(profile)
}

pub fn load(backend: &super::config::Backend) -> Result<(Digest, Registration)> {
    let super::config::Backend::PythonSkillPackage {
        directory,
        manifest_digest,
        policy,
    } = backend
    else {
        return Err("signed Python package backend required".into());
    };
    if !directory.is_absolute() {
        return Err("absolute Python package directory required".into());
    }
    let input: rx_package::policy::Policy = canonical::decode_json(&policy.read(false)?)?;
    let architecture = match std::env::consts::ARCH {
        "aarch64" => rx_package::Architecture::Arm64,
        "x86_64" => rx_package::Architecture::Amd64,
        _ => return Err("unsupported Python Host architecture".into()),
    };
    if input.contracts.base
        != Digest::from_bytes(crate::rpc::base_manifest_hash().try_into().expect("hash"))
        || input.contracts.cell
            != Digest::from_bytes(crate::rpc::cell_manifest_hash().try_into().expect("hash"))
        || input.contracts.package_abi.as_str() != "rx.package-abi.v2"
        || input.target.os != rx_package::OperatingSystem::Linux
        || input.target.architecture != architecture
        || input.target.ros_distribution.is_some()
        || !input.dependencies.is_empty()
        || input.assets.len() > 32
        || input
            .assets
            .iter()
            .any(|a| !a.path.is_absolute() || a.reference.size_bytes.0 > 1_048_576)
    {
        return Err("Python package policy target/contracts/acquisition bounds differ".into());
    }
    let mut verification = input.load()?;
    verification.max_files = 8;
    verification.max_content_bytes = 2 * 1024 * 1024;
    let package = rx_package::directory::verify_directory(directory, &verification)?;
    if package.digest() != *manifest_digest {
        return Err("selected Python package digest differs".into());
    }
    Ok((package.digest(), decode(&package)?))
}
