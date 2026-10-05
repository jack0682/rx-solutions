//! Authoring for the generic external-process provider; no native calls or trust changes.
use crate::*;
use rx_host::service::external_package;

pub fn assemble(source: &external_package::Assembly, recipe: &Recipe) -> Result<Candidate> {
    let (_, _, _, _, files, assets) =
        external_package::documents(source).map_err(|e| Error::Invalid(e.to_string()))?;
    if recipe.schema.as_str() != "rx.device-package-recipe.v1"
        || !recipe.version.build.is_empty()
        || recipe.targets.len() != 1
        || recipe.targets[0].os != OperatingSystem::Linux
        || recipe.targets[0].ros_distribution.is_some()
    {
        return Err(Error::Invalid(
            "external recipe target/version differs".into(),
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
                role: name("external-process"),
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
