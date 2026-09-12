//! Deterministic device-package authoring. No private keys, device access, trust installation or qualification.
use rx_domain::{canonical, types::*};
pub use rx_host::melsec::authoring::{Assembly, Site, Template};
use rx_host::service::device_package;
use rx_package::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub mod directory;
pub mod jtc;
pub mod review;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("device package: {0}")]
    Invalid(String),
    #[error(transparent)]
    Package(#[from] rx_package::Error),
    #[error(transparent)]
    Host(#[from] rx_host::HostError),
}
pub type Result<T> = std::result::Result<T, Error>;
fn name(v: &str) -> Name {
    Name::new(v).expect("literal")
}
fn path(v: &str) -> PackagePath {
    PackagePath::new(v).expect("literal")
}
pub fn bytes(value: &impl Serialize) -> Result<Vec<u8>> {
    canonical::bytes(value).map_err(|e| Error::Invalid(e.to_string()))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub schema: Name,
    pub package: Name,
    pub version: semver::Version,
    pub publisher: Name,
    pub targets: Vec<Target>,
}
pub struct Candidate {
    manifest: Manifest,
    files: BTreeMap<PackagePath, Vec<u8>>,
    recipe: Recipe,
}
impl Candidate {
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn files(&self) -> &BTreeMap<PackagePath, Vec<u8>> {
        &self.files
    }
    pub fn recipe(&self) -> &Recipe {
        &self.recipe
    }
    pub fn digest(&self) -> Result<Digest> {
        Ok(content_digest(&manifest_bytes(&self.manifest)?))
    }
    pub fn signing_message(&self, key: &Name) -> Result<Vec<u8>> {
        Ok(rx_package::signing_message(&self.manifest, key)?)
    }
    pub fn verify(
        &self,
        signature: &SignatureEnvelope,
        policy: &VerificationPolicy,
    ) -> Result<VerifiedPackage> {
        let verified = verify_package(
            &manifest_bytes(&self.manifest)?,
            &bytes(signature)?,
            self.files.clone(),
            policy,
        )?;
        decode_verified_any(&verified)?;
        Ok(verified)
    }
}
pub fn contracts() -> ContractSet {
    ContractSet {
        base: Digest::from_bytes(rx_host::rpc::base_manifest_hash().try_into().expect("hash")),
        cell: Digest::from_bytes(rx_host::rpc::cell_manifest_hash().try_into().expect("hash")),
        package_abi: name("rx.package-abi.v2"),
    }
}
pub fn assemble(template: &Template, site: &Site, recipe: &Recipe) -> Result<Candidate> {
    if recipe.schema.as_str() != "rx.device-package-recipe.v1"
        || !recipe.version.build.is_empty()
        || recipe.version.to_string().len() > 128
        || recipe.targets.is_empty()
        || recipe.targets.len() > 2
        || recipe
            .targets
            .iter()
            .any(|t| t.os != OperatingSystem::Linux || t.ros_distribution.is_some())
    {
        return Err(Error::Invalid("device recipe target/version/schema".into()));
    }
    let assembly = Assembly {
        schema: name("rx.melsec-assembly.v1"),
        template: template.clone(),
        site: site.clone(),
    }
    .normalized()?;
    let (family, profile) = assembly.resolve()?;
    let files: BTreeMap<_, _> = [
        (path("family.json"), bytes(&family)?),
        (path("profile.json"), bytes(&profile)?),
        (path("adapter.json"), bytes(&device_package::driver())?),
        (path("authoring/assembly.json"), bytes(&assembly)?),
    ]
    .into();
    if files.values().map(Vec::len).sum::<usize>() > 2 * 1024 * 1024 {
        return Err(Error::Invalid("device content exceeds 2 MiB".into()));
    }
    let manifest = Manifest {
        schema: name("rx.package.v2"),
        package: recipe.package.clone(),
        version: recipe.version.clone(),
        publisher: recipe.publisher.clone(),
        contracts: contracts(),
        targets: recipe.targets.clone(),
        entry: EntryPoint::DeviceReference {
            family: path("family.json"),
            profiles: vec![path("profile.json")],
            adapter: path("adapter.json"),
        },
        permissions: vec![
            Permission::ArtifactRead,
            Permission::NativeEndpoint {
                role: name("melsec-mc3e"),
            },
            Permission::ObservationRead {
                schema: name("rx.melsec.status-image.v1"),
            },
        ],
        dependencies: vec![],
        assets: assembly.required_assets(),
        files: files
            .iter()
            .map(|(path, data)| FileEntry {
                path: path.clone(),
                sha256: content_digest(data),
                size_bytes: Counter(data.len() as u64),
                executable: false,
            })
            .collect(),
    };
    validate_manifest(
        &manifest,
        &VerificationPolicy {
            additional_package_abis: Default::default(),
            publishers: BTreeMap::new(),
            contracts: contracts(),
            target: recipe.targets[0].clone(),
            assets: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            max_files: 8,
            max_content_bytes: 2 * 1024 * 1024,
        },
    )?;
    let mut recipe = recipe.clone();
    recipe.targets.sort();
    Ok(Candidate {
        manifest,
        files,
        recipe,
    })
}
pub fn from_files(mut files: BTreeMap<PackagePath, Vec<u8>>) -> Result<Candidate> {
    let manifest = files
        .remove(&path("manifest.json"))
        .ok_or_else(|| Error::Invalid("candidate manifest absent".into()))?;
    let recipe = files
        .remove(&path("candidate-recipe.json"))
        .ok_or_else(|| Error::Invalid("candidate recipe absent".into()))?;
    let recipe: Recipe =
        canonical::decode_json(&recipe).map_err(|e| Error::Invalid(e.to_string()))?;
    let assembly = files
        .get(&path("authoring/assembly.json"))
        .ok_or_else(|| Error::Invalid("assembly source absent".into()))?;
    let source: serde_json::Value =
        canonical::decode_json(assembly).map_err(|e| Error::Invalid(e.to_string()))?;
    let rebuilt = match source["schema"].as_str() {
        Some("rx.melsec-assembly.v1") => {
            let a: Assembly =
                canonical::decode_json(assembly).map_err(|e| Error::Invalid(e.to_string()))?;
            assemble(&a.template, &a.site, &recipe)?
        }
        Some("rx.robotis-jtc-assembly.v1") => {
            let a: jtc::Assembly =
                canonical::decode_json(assembly).map_err(|e| Error::Invalid(e.to_string()))?;
            jtc::assemble(&a.template, &a.site, &recipe)?
        }
        _ => return Err(Error::Invalid("unsupported device assembly schema".into())),
    };
    if manifest != manifest_bytes(rebuilt.manifest())? || files != *rebuilt.files() {
        return Err(Error::Invalid(
            "candidate differs from deterministic assembly".into(),
        ));
    }
    Ok(rebuilt)
}
pub fn decode_verified(
    package: &VerifiedPackage,
) -> Result<rx_host::service::device_package::LoadedDevice> {
    // No filesystem re-read of payloads: the common verifier owns immutable content bytes.
    device_package::decode_verified(package).map_err(|e| Error::Invalid(e.to_string()))
}
pub enum Device {
    Melsec(rx_host::service::device_package::LoadedDevice),
    Jtc(rx_host::service::jtc_package::LoadedDevice),
}
pub fn decode_verified_any(package: &VerifiedPackage) -> Result<Device> {
    let EntryPoint::DeviceReference { adapter, .. } = &package.manifest().entry else {
        return Err(Error::Invalid("device reference required".into()));
    };
    let driver: device_package::Driver = canonical::decode_json(
        package
            .file(adapter)
            .ok_or_else(|| Error::Invalid("driver absent".into()))?,
    )
    .map_err(|e| Error::Invalid(e.to_string()))?;
    match driver.implementation.as_str() {
        "rx.melsec.ensure-state.v1" => Ok(Device::Melsec(decode_verified(package)?)),
        "rx.robotis.position-jtc.v1" => Ok(Device::Jtc(
            rx_host::service::jtc_package::decode_verified(package)
                .map_err(|e| Error::Invalid(e.to_string()))?,
        )),
        _ => Err(Error::Invalid("unsupported device implementation".into())),
    }
}
pub fn source_policy(input: &rx_package::policy::Policy) -> Result<VerificationPolicy> {
    let expected = contracts();
    if input.contracts.base != expected.base
        || input.contracts.cell != expected.cell
        || input.contracts.package_abi != expected.package_abi
        || input.target.os != OperatingSystem::Linux
        || input
            .target
            .ros_distribution
            .as_ref()
            .is_some_and(|v| v.as_str() != "jazzy")
        || !input.dependencies.is_empty()
        || input.assets.len() > 32
        || input
            .assets
            .iter()
            .any(|a| !a.path.is_absolute() || a.reference.size_bytes.0 > 1_048_576)
    {
        return Err(Error::Invalid(
            "device authoring policy scope/target/assets".into(),
        ));
    }
    Ok(input.load()?)
}
pub fn load_policy(input: &rx_package::policy::Policy) -> Result<VerificationPolicy> {
    let mut policy = source_policy(input)?;
    policy.max_files = 8;
    policy.max_content_bytes = 2 * 1024 * 1024;
    Ok(policy)
}
