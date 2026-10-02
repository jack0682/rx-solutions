//! Reuse the DEVICE_REFERENCE package/signing/review path for prepared Python programs.
use crate::*;
use rx_host::service::{python_package, python_skill::Registration};
pub fn assemble(
    registration: &Registration,
    environment: &[u8],
    recipe: &Recipe,
) -> Result<Candidate> {
    let (profile, files) = python_package::documents(registration, environment)
        .map_err(|e| Error::Invalid(e.to_string()))?;
    let rx_domain::intent::Body::Program(goal) = &profile.intent.body else {
        return Err(Error::Invalid("ProgramGoal required".into()));
    };
    assemble_files(
        environment,
        recipe,
        files,
        vec![goal.program.clone(), goal.parameter_set.clone()],
    )
}
pub fn assemble_library(
    library: &rx_host::service::python_library::Library,
    environment: &[u8],
    recipe: &Recipe,
) -> Result<Candidate> {
    let (profile, files) = rx_host::service::python_library::documents(library, environment)
        .map_err(|e| Error::Invalid(e.to_string()))?;
    let assets = profile
        .assets()
        .map_err(|e| Error::Invalid(e.to_string()))?;
    assemble_files(environment, recipe, files, assets)
}
fn assemble_files(
    environment: &[u8],
    recipe: &Recipe,
    files: BTreeMap<PackagePath, Vec<u8>>,
    assets: Vec<ArtifactRef>,
) -> Result<Candidate> {
    if recipe.schema.as_str() != "rx.device-package-recipe.v1"
        || !recipe.version.build.is_empty()
        || recipe.targets.len() != 1
        || recipe
            .targets
            .iter()
            .any(|t| t.os != OperatingSystem::Linux || t.ros_distribution.is_some())
    {
        return Err(Error::Invalid(
            "Python recipe target/version differs".into(),
        ));
    }
    let metadata: serde_json::Value =
        canonical::decode_json(environment).map_err(|e| Error::Invalid(e.to_string()))?;
    let machine = match recipe.targets[0].architecture {
        Architecture::Arm64 => "aarch64",
        Architecture::Amd64 => "x86_64",
    };
    if metadata["environment"]["python"]["machine"] != machine
        || metadata["environment"]["skill"]["version"] != recipe.version.to_string()
    {
        return Err(Error::Invalid(
            "Python environment architecture or skill version differs from recipe".into(),
        ));
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
                role: name("python-sdk"),
            },
        ],
        dependencies: vec![],
        assets,
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
    Ok(Candidate {
        manifest,
        files,
        recipe: recipe.clone(),
    })
}
