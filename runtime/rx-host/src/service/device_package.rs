//! Reuse signed DEVICE packages. Content/trust validation never supplies physical qualification.
use super::{Result, config::Backend};
use crate::{Binding, melsec::Profile};
use rx_domain::{canonical, types::*};
use rx_package::{Architecture, EntryPoint, OperatingSystem, PackageKind, PackagePath, Permission};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Driver {
    pub schema: Name,
    pub implementation: Name,
    pub source_digest: Digest,
}
pub fn driver() -> Driver {
    Driver {
        schema: Name::new("rx.native-driver-reference.v1").expect("literal"),
        implementation: Name::new("rx.melsec.ensure-state.v1").expect("literal"),
        source_digest: serde_json::from_value(serde_json::Value::String(
            env!("RX_MELSEC_DRIVER_SOURCE_SHA256").into(),
        ))
        .expect("build digest"),
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Family {
    pub schema: Name,
    pub family: Name,
    pub controller_model: Name,
    pub environment: crate::Environment,
}
pub struct LoadedDevice {
    pub profile: Profile,
    pub manifest_digest: Digest,
}
impl LoadedDevice {
    pub fn validate_bindings(&self, bindings: &[Binding]) -> Result<()> {
        if bindings.len() != 1 {
            return Err("MELSEC package requires one exact cell binding".into());
        }
        let b = &bindings[0];
        if b.cell != self.profile.cell
            || b.environment != self.profile.environment()
            || b.condition_ids.iter().collect::<BTreeSet<_>>()
                != self.profile.conditions.keys().collect()
            || b.allowed_intents.is_empty()
        {
            return Err("MELSEC profile/cell/environment/conditions differ".into());
        }
        for intent in &b.allowed_intents {
            self.profile.mapping(intent)?;
        }
        Ok(())
    }
}
pub fn load(backend: &Backend) -> Result<LoadedDevice> {
    let Backend::MelsecPackage {
        directory,
        manifest_digest,
        policy,
    } = backend
    else {
        return Err("not a MELSEC package backend".into());
    };
    if !directory.is_absolute() {
        return Err("absolute device package directory required".into());
    }
    let input: rx_package::policy::Policy = canonical::decode_json(&policy.read(false)?)?;
    let expected_arch = match std::env::consts::ARCH {
        "aarch64" => Architecture::Arm64,
        "x86_64" => Architecture::Amd64,
        _ => return Err("unsupported native architecture".into()),
    };
    if input.contracts.base
        != Digest::from_bytes(crate::rpc::base_manifest_hash().try_into().expect("hash"))
        || input.contracts.cell
            != Digest::from_bytes(crate::rpc::cell_manifest_hash().try_into().expect("hash"))
        || input.contracts.package_abi.as_str() != "rx.package-abi.v2"
        || input.target.os != OperatingSystem::Linux
        || input.target.architecture != expected_arch
        || input.target.ros_distribution.is_some()
        || !input.dependencies.is_empty()
        || input.assets.len() > 32
        || input
            .assets
            .iter()
            .any(|a| !a.path.is_absolute() || a.reference.size_bytes.0 > 1_048_576)
    {
        return Err("device verification policy target/contracts/acquisition bounds".into());
    }
    let mut verification = input.load()?;
    verification.max_files = 8;
    verification.max_content_bytes = 2 * 1024 * 1024;
    let package = rx_package::directory::verify_directory(directory, &verification)?;
    if package.digest() != *manifest_digest {
        return Err("selected device manifest digest differs".into());
    }
    decode_verified(&package)
}
/// Interpret immutable verified bytes without granting qualification or opening an adapter.
/// Host startup additionally checks its pinned policy/current target and selected manifest digest.
pub fn decode_verified(package: &rx_package::VerifiedPackage) -> Result<LoadedDevice> {
    let manifest = package.manifest();
    let EntryPoint::DeviceReference {
        family,
        profiles,
        adapter,
    } = &manifest.entry
    else {
        return Err("DEVICE package required".into());
    };
    if profiles.len() != 1
        || manifest.files.iter().any(|f| f.executable)
        || !manifest.dependencies.is_empty()
        || !matches!(manifest.files.len(), 3 | 4)
    {
        return Err("device package manifest/entry shape differs".into());
    }
    let allowed = [
        Permission::ArtifactRead,
        Permission::NativeEndpoint {
            role: Name::new("melsec-mc3e").expect("literal"),
        },
        Permission::ObservationRead {
            schema: Name::new("rx.melsec.status-image.v1").expect("literal"),
        },
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    if manifest.entry.kind() != PackageKind::Device
        || manifest
            .permissions
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            != allowed
    {
        return Err("device package permissions differ".into());
    }
    fn read<T: serde::de::DeserializeOwned>(
        package: &rx_package::VerifiedPackage,
        path: &PackagePath,
    ) -> Result<T> {
        Ok(canonical::decode_json(
            package.file(path).ok_or("device entry absent")?,
        )?)
    }
    let family: Family = read(package, family)?;
    let implementation: Driver = read(package, adapter)?;
    let profile: Profile = read(package, &profiles[0])?;
    profile.validate()?;
    if manifest.files.len() == 4 {
        let assembly: crate::melsec::authoring::Assembly = read(
            package,
            &PackagePath::new("authoring/assembly.json").expect("literal"),
        )?;
        let (expected_family, expected_profile) = assembly.resolve()?;
        if canonical::bytes(&family)? != canonical::bytes(&expected_family)?
            || canonical::bytes(&profile)? != canonical::bytes(&expected_profile)?
        {
            return Err("signed device payload differs from template/site assembly".into());
        }
        for asset in assembly.required_assets() {
            if !manifest.assets.contains(&asset) {
                return Err("signed assembly asset metadata differs".into());
            }
        }
    }
    if implementation != driver()
        || family.schema.as_str() != "rx.melsec-family.v1"
        || family.family.as_str() != "mitsubishi/melsec-q"
        || family.controller_model.as_str() != "Q03UDVCPU"
        || family.environment != profile.environment()
    {
        return Err("device family/environment/release implementation mismatch".into());
    }
    for (digest, schema) in [
        (
            profile.status.publication_contract,
            "rx.melsec.publication-contract.v1",
        ),
        (profile.status.plc_program, "rx.plc.program-evidence.v1"),
    ] {
        if !manifest
            .assets
            .iter()
            .any(|a| a.sha256 == digest && a.schema_id.as_str() == schema && a.size_bytes.0 > 0)
        {
            return Err("required publication/program source artifact absent".into());
        }
    }
    Ok(LoadedDevice {
        profile,
        manifest_digest: package.digest(),
    })
}
