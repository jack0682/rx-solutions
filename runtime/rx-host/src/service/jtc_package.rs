//! Exact signed JTC content resolution. No executable selection or controller authority from packages.
use super::{Result, device_package::Driver};
use crate::{
    Binding,
    ros_jtc::{
        Profile,
        authoring::{Assembly, Family},
    },
};
use rx_domain::{canonical, intent::Intent, types::*};
use rx_package::{EntryPoint, PackagePath, Permission, VerifiedPackage};
use rx_process_contract::native_outcome::NativeOutcomeTable;
use std::collections::{BTreeMap, BTreeSet};
fn n(v: &str) -> Name {
    Name::new(v).expect("fixed name")
}
fn path(v: &str) -> PackagePath {
    PackagePath::new(v).expect("fixed path")
}
pub fn driver() -> Driver {
    Driver {
        schema: n("rx.native-driver-reference.v1"),
        implementation: n("rx.robotis.position-jtc.v1"),
        source_digest: serde_json::from_value(serde_json::Value::String(
            env!("RX_JTC_DRIVER_SOURCE_SHA256").into(),
        ))
        .expect("build digest"),
    }
}
pub fn permissions() -> Vec<Permission> {
    vec![
        Permission::ArtifactRead,
        Permission::NativeEndpoint {
            role: n("robotis-position-jtc"),
        },
        Permission::ObservationRead {
            schema: n("rx.ros-jtc-reply.v1"),
        },
    ]
}
pub fn catalog(
    resolved: &crate::ros_jtc::authoring::Resolved,
) -> Result<rx_process_contract::device_catalog::Catalog> {
    use rx_process_contract::device_catalog::{Catalog, Document, Environment};
    let profile = &resolved.profile;
    let outcomes = profile.outcome_table()?;
    let sources = [
        (
            "family",
            "family.json",
            "rx.robotis-jtc-family.v1",
            canonical::bytes(&resolved.family)?,
        ),
        (
            "profile",
            "profile.json",
            "rx.robotis-jtc-profile.v1",
            canonical::bytes(profile)?,
        ),
        (
            "adapter",
            "adapter.json",
            "rx.native-driver-reference.v1",
            canonical::bytes(&driver())?,
        ),
        (
            "operations",
            "operations.json",
            "rx.device-operation-map.v1",
            canonical::bytes(&resolved.operations)?,
        ),
        (
            "outcomes",
            "outcomes.json",
            "rx.native-outcome-table.v1",
            canonical::bytes(&outcomes)?,
        ),
    ];
    let c = Catalog {
        schema: n("rx.device-operation-catalog.v1"),
        installation: profile.installation.clone(),
        cell: profile.cell.clone(),
        target: profile.target.clone(),
        environment: match profile.environment {
            crate::Environment::Simulation => Environment::Simulation,
            crate::Environment::Physical => Environment::Physical,
        },
        profile_digest: profile.digest()?,
        condition_ids: profile.conditions.clone(),
        operations: resolved.operations.clone(),
        outcomes: Some(outcomes),
        documents: sources
            .into_iter()
            .map(|(role, path, schema, bytes)| {
                (
                    n(role),
                    Document {
                        path: path.into(),
                        artifact: ArtifactRef {
                            sha256: rx_package::content_digest(&bytes),
                            schema_id: n(schema),
                            size_bytes: Counter(bytes.len() as u64),
                        },
                    },
                )
            })
            .collect(),
    };
    c.validate()?;
    Ok(c)
}
pub struct LoadedDevice {
    pub profile: Profile,
    pub operations: BTreeMap<Name, Intent>,
    pub outcomes: NativeOutcomeTable,
    pub manifest_digest: Digest,
}
impl LoadedDevice {
    pub fn validate_bindings(&self, bindings: &[Binding]) -> Result<()> {
        if bindings.len() != 1 {
            return Err("JTC package requires one exact cell binding".into());
        }
        let b = &bindings[0];
        if b.cell != self.profile.cell
            || b.environment != self.profile.environment
            || b.condition_ids.iter().collect::<BTreeSet<_>>()
                != self.profile.conditions.iter().collect()
            || b.allowed_intents.is_empty()
        {
            return Err("JTC profile/cell/environment/conditions differ".into());
        }
        let allowed = self
            .operations
            .values()
            .map(Intent::digest)
            .collect::<std::result::Result<BTreeSet<_>, _>>()?;
        for i in &b.allowed_intents {
            self.profile.trajectory(i)?;
            if !allowed.contains(&i.digest()?) {
                return Err("JTC intent differs from authored operation".into());
            }
        }
        Ok(())
    }
}
pub fn decode_verified(package: &VerifiedPackage) -> Result<LoadedDevice> {
    let m = package.manifest();
    if !matches!(&m.entry,EntryPoint::DeviceReference {family,profiles,adapter}
        if *family==path("family.json") && profiles==&[path("profile.json")] && *adapter==path("adapter.json"))
        || !matches!(m.files.len(), 6 | 7)
        || m.files.iter().any(|f| f.executable)
        || !m.dependencies.is_empty()
        || m.targets.iter().any(|t| {
            t.os != rx_package::OperatingSystem::Linux
                || t.ros_distribution.as_ref().map(Name::as_str) != Some("jazzy")
        })
        || m.permissions.iter().cloned().collect::<BTreeSet<_>>()
            != permissions().into_iter().collect()
    {
        return Err("JTC manifest/target/permission shape differs".into());
    }
    fn read<T: serde::de::DeserializeOwned>(p: &VerifiedPackage, name: &str) -> Result<T> {
        Ok(canonical::decode_json(
            p.file(&path(name)).ok_or("JTC file absent")?,
        )?)
    }
    let a: Assembly = read(package, "authoring/assembly.json")?;
    let resolved = a.resolve()?;
    if m.files.len() == 7 {
        let declaration: rx_process_contract::device_catalog::Catalog =
            read(package, "device-catalog.json")?;
        if canonical::bytes(&declaration)? != canonical::bytes(&catalog(&resolved)?)? {
            return Err("JTC operation catalog differs from signed assembly".into());
        }
    }
    let family: Family = read(package, "family.json")?;
    let profile: Profile = read(package, "profile.json")?;
    let operations: BTreeMap<Name, Intent> = read(package, "operations.json")?;
    let outcomes: NativeOutcomeTable = read(package, "outcomes.json")?;
    let implementation: Driver = read(package, "adapter.json")?;
    if implementation != driver()
        || canonical::bytes(&family)? != canonical::bytes(&resolved.family)?
        || canonical::bytes(&profile)? != canonical::bytes(&resolved.profile)?
        || canonical::bytes(&operations)? != canonical::bytes(&resolved.operations)?
        || canonical::bytes(&outcomes)? != canonical::bytes(&resolved.profile.outcome_table()?)?
        || m.assets
            .iter()
            .map(|a| (&a.sha256, &a.schema_id, a.size_bytes))
            .collect::<BTreeSet<_>>()
            != a.required_assets()
                .iter()
                .map(|a| (&a.sha256, &a.schema_id, a.size_bytes))
                .collect()
    {
        return Err("JTC signed payload differs from assembly/release/assets".into());
    }
    Ok(LoadedDevice {
        profile,
        operations,
        outcomes,
        manifest_digest: package.digest(),
    })
}
pub fn load(backend: &super::config::Backend) -> Result<LoadedDevice> {
    let super::config::Backend::JtcPackage {
        directory,
        manifest_digest,
        policy,
    } = backend
    else {
        return Err("not a JTC package backend".into());
    };
    if !directory.is_absolute() {
        return Err("absolute JTC package directory required".into());
    }
    let input: rx_package::policy::Policy = canonical::decode_json(&policy.read(false)?)?;
    let arch = match std::env::consts::ARCH {
        "aarch64" => rx_package::Architecture::Arm64,
        "x86_64" => rx_package::Architecture::Amd64,
        _ => return Err("unsupported architecture".into()),
    };
    if input.contracts.base
        != Digest::from_bytes(crate::rpc::base_manifest_hash().try_into().expect("hash"))
        || input.contracts.cell
            != Digest::from_bytes(crate::rpc::cell_manifest_hash().try_into().expect("hash"))
        || input.contracts.package_abi.as_str() != "rx.package-abi.v2"
        || input.target.os != rx_package::OperatingSystem::Linux
        || input.target.architecture != arch
        || input.target.ros_distribution.as_ref().map(Name::as_str) != Some("jazzy")
        || !input.dependencies.is_empty()
        || input.assets.len() > 32
        || input
            .assets
            .iter()
            .any(|a| !a.path.is_absolute() || a.reference.size_bytes.0 > 1_048_576)
    {
        return Err("JTC policy target/contracts/acquisition bounds".into());
    }
    let mut policy = input.load()?;
    policy.max_files = 8;
    policy.max_content_bytes = 2 * 1024 * 1024;
    let package = rx_package::directory::verify_directory(directory, &policy)?;
    if package.digest() != *manifest_digest {
        return Err("selected JTC manifest differs".into());
    }
    decode_verified(&package)
}
