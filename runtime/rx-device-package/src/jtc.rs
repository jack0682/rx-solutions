//! ROS position JTC packages use the common authoring/signature pipeline.
use crate::*;
pub use rx_host::ros_jtc::authoring::{ActionSlot, Assembly, Site, Template};
use rx_host::service::jtc_package;
pub fn assemble(t: &Template, s: &Site, r: &Recipe) -> Result<Candidate> {
    if r.schema.as_str() != "rx.device-package-recipe.v1"
        || !r.version.build.is_empty()
        || r.version.to_string().len() > 128
        || r.targets.is_empty()
        || r.targets.len() > 2
        || r.targets.iter().any(|t| {
            t.os != OperatingSystem::Linux
                || t.ros_distribution.as_ref().map(Name::as_str) != Some("jazzy")
        })
    {
        return Err(Error::Invalid(
            "JTC recipe requires Linux/Jazzy target".into(),
        ));
    }
    let a = Assembly {
        schema: name("rx.ros-jtc-assembly.v1"),
        template: t.clone(),
        site: s.clone(),
    }
    .normalized()?;
    let resolved = a.resolve()?;
    let files: BTreeMap<_, _> = [
        (
            path("device-catalog.json"),
            bytes(&jtc_package::catalog(&resolved).map_err(|e| Error::Invalid(e.to_string()))?)?,
        ),
        (path("family.json"), bytes(&resolved.family)?),
        (path("profile.json"), bytes(&resolved.profile)?),
        (path("adapter.json"), bytes(&jtc_package::driver())?),
        (path("authoring/assembly.json"), bytes(&a)?),
        (path("operations.json"), bytes(&resolved.operations)?),
        (
            path("outcomes.json"),
            bytes(&resolved.profile.outcome_table()?)?,
        ),
    ]
    .into();
    if files.values().map(Vec::len).sum::<usize>() > 2 * 1024 * 1024 {
        return Err(Error::Invalid("JTC package exceeds 2 MiB".into()));
    }
    let manifest = Manifest {
        schema: name("rx.package.v2"),
        package: r.package.clone(),
        version: r.version.clone(),
        publisher: r.publisher.clone(),
        contracts: contracts(),
        targets: r.targets.clone(),
        entry: EntryPoint::DeviceReference {
            family: path("family.json"),
            profiles: vec![path("profile.json")],
            adapter: path("adapter.json"),
        },
        permissions: jtc_package::permissions(),
        dependencies: vec![],
        assets: a.required_assets(),
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
            target: r.targets[0].clone(),
            assets: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            max_files: 8,
            max_content_bytes: 2 * 1024 * 1024,
        },
    )?;
    let mut recipe = r.clone();
    recipe.targets.sort();
    Ok(Candidate {
        manifest,
        files,
        recipe,
    })
}
