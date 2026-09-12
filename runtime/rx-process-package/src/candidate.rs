use crate::{Error, Result};
use rx_domain::{canonical, condition::Condition, intent::Body, types::*};
use rx_package::*;
use rx_process_contract::{
    compile_input::CompileInput,
    model::{CompiledBody, CompiledNode, ResolvedProcess},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
pub const SOURCE: &str = "process/source.json";
pub const BINDINGS: &str = "process/bindings.json";
pub const INPUT: &str = "authoring/compile-input.json";
pub const RECIPE: &str = "authoring/package-recipe.json";
pub const CONTEXT: &str = "process/context-requirements.json";
fn name(s: &str) -> Name {
    Name::new(s).expect("literal name")
}
fn path(s: &str) -> PackagePath {
    PackagePath::new(s).expect("literal path")
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub schema: Name,
    pub package: Name,
    pub version: semver::Version,
    pub publisher: Name,
    pub contracts: ContractSet,
    pub targets: Vec<Target>,
    pub dependencies: Vec<Dependency>,
    pub assets: Vec<ArtifactRef>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextRequirements {
    pub schema: Name,
    pub catalog: Digest,
    pub profiles: Vec<Digest>,
    pub site_configurations: Vec<Digest>,
    pub calibrations: Vec<Digest>,
    pub tools: Vec<Digest>,
    pub transitions: Vec<Digest>,
    pub streams: Vec<Digest>,
}
pub struct Candidate {
    manifest: Manifest,
    files: BTreeMap<PackagePath, Vec<u8>>,
}
impl Candidate {
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn files(&self) -> &BTreeMap<PackagePath, Vec<u8>> {
        &self.files
    }
    pub fn digest(&self) -> Result<Digest> {
        Ok(content_digest(&manifest_bytes(&self.manifest)?))
    }
    pub fn signing_message(&self, key: &Name) -> Result<Vec<u8>> {
        Ok(signing_message(&self.manifest, key)?)
    }
    pub fn verify(
        &self,
        signature: &SignatureEnvelope,
        policy: &VerificationPolicy,
    ) -> Result<VerifiedPackage> {
        Ok(verify_package(
            &manifest_bytes(&self.manifest)?,
            &bytes(signature)?,
            self.files.clone(),
            policy,
        )?)
    }
}
fn bytes(value: &impl Serialize) -> Result<Vec<u8>> {
    canonical::bytes(value).map_err(|e| Error::Invalid(e.to_string()))
}
fn condition_schemas(value: &Condition, out: &mut BTreeSet<Name>) {
    match value {
        Condition::All { children } | Condition::Any { children } => {
            for c in children {
                condition_schemas(c, out);
            }
        }
        Condition::Eq { schema, .. }
        | Condition::Range { schema, .. }
        | Condition::SetContains { schema, .. } => {
            out.insert(schema.clone());
        }
    }
}
fn required_from_node(
    node: &CompiledNode,
    process: &ResolvedProcess,
    schemas: &mut BTreeSet<Name>,
    assets: &mut BTreeMap<Digest, ArtifactRef>,
) -> Result<()> {
    match &node.body {
        CompiledBody::Sequence { children } | CompiledBody::ParallelAll { children } => {
            for c in children {
                required_from_node(c, process, schemas, assets)?;
            }
        }
        CompiledBody::Branch {
            condition,
            when_true,
            when_false,
        } => {
            condition_schemas(&process.conditions[condition], schemas);
            required_from_node(when_true, process, schemas, assets)?;
            required_from_node(when_false, process, schemas, assets)?;
        }
        CompiledBody::Wait { condition, .. } => {
            condition_schemas(&process.conditions[condition], schemas)
        }
        CompiledBody::Intervention { procedure } => insert_asset(assets, procedure.clone())?,
        _ => {}
    }
    Ok(())
}
fn insert_asset(assets: &mut BTreeMap<Digest, ArtifactRef>, asset: ArtifactRef) -> Result<()> {
    if assets.get(&asset.sha256).is_some_and(|old| old != &asset) {
        return Err(Error::Invalid(
            "same asset digest with different metadata".into(),
        ));
    }
    assets.insert(asset.sha256, asset);
    Ok(())
}
pub fn assemble(input: &CompileInput, recipe: &Recipe) -> Result<Candidate> {
    if recipe.schema.as_str() != "rx.process-package-recipe.v1"
        || recipe.version.to_string().len() > 128
        || !recipe.version.build.is_empty()
        || recipe.targets.is_empty()
        || recipe.targets.len() > 8
        || recipe.dependencies.len() > 128
        || recipe.assets.len() > 1024
    {
        return Err(Error::Invalid("recipe shape".into()));
    }
    let source = input.validate().map_err(Error::Invalid)?;
    let resolved = rx_process::compile(&source, input.bindings.clone())?;
    if resolved.bindings.len() != input.bindings.len() {
        return Err(Error::Invalid("unused input binding".into()));
    }
    let mut permissions: BTreeSet<Permission> = [Permission::ArtifactRead].into_iter().collect();
    let mut assets = BTreeMap::new();
    let mut schemas = BTreeSet::new();
    required_from_node(&resolved.root, &resolved, &mut schemas, &mut assets)?;
    let (mut profiles, mut sites, mut calibrations, mut tools, mut transitions, mut streams) = (
        BTreeSet::new(),
        BTreeSet::new(),
        BTreeSet::new(),
        BTreeSet::new(),
        BTreeSet::new(),
        BTreeSet::new(),
    );
    for (binding, action) in &resolved.bindings {
        permissions.insert(Permission::OperationSubmit {
            operation: binding.clone(),
        });
        profiles.insert(action.intent.profile_digest);
        sites.insert(action.intent.site_config_digest);
        calibrations.extend(action.intent.calibration_digests.iter().copied());
        match &action.intent.body {
            Body::Trajectory(goal) => {
                insert_asset(&mut assets, goal.trajectory.clone())?;
                tools.insert(goal.tool_digest);
            }
            Body::Program(goal) => {
                insert_asset(&mut assets, goal.program.clone())?;
                insert_asset(&mut assets, goal.parameter_set.clone())?;
            }
            Body::Mode(goal) => {
                transitions.insert(goal.transition_profile);
            }
            Body::Control(goal) => {
                streams.insert(goal.stream_profile);
                schemas.insert(goal.sample_schema.clone());
            }
            _ => {}
        }
    }
    for schema in schemas {
        permissions.insert(Permission::ObservationRead { schema });
    }
    let declared: BTreeMap<_, _> = recipe.assets.iter().map(|a| (a.sha256, a)).collect();
    if declared.len() != recipe.assets.len()
        || assets
            .iter()
            .any(|(digest, asset)| declared.get(digest) != Some(&asset))
    {
        return Err(Error::Invalid(
            "required artifact references are not declared exactly".into(),
        ));
    }
    let context = ContextRequirements {
        schema: name("rx.process-context-requirements.v1"),
        catalog: input.catalog_digest,
        profiles: profiles.into_iter().collect(),
        site_configurations: sites.into_iter().collect(),
        calibrations: calibrations.into_iter().collect(),
        tools: tools.into_iter().collect(),
        transitions: transitions.into_iter().collect(),
        streams: streams.into_iter().collect(),
    };
    let files: BTreeMap<_, _> = [
        (path(SOURCE), bytes(&source)?),
        (path(BINDINGS), bytes(&input.bindings)?),
        (path(INPUT), bytes(input)?),
        (path(RECIPE), bytes(recipe)?),
        (path(CONTEXT), bytes(&context)?),
    ]
    .into_iter()
    .collect();
    if files.values().map(Vec::len).sum::<usize>() > 4 * 1024 * 1024 {
        return Err(Error::Invalid(
            "process package candidate exceeds 4 MiB".into(),
        ));
    }
    let manifest = Manifest {
        schema: name("rx.package.v1"),
        package: recipe.package.clone(),
        version: recipe.version.clone(),
        publisher: recipe.publisher.clone(),
        contracts: recipe.contracts.clone(),
        targets: recipe.targets.clone(),
        entry: EntryPoint::Process {
            source: path(SOURCE),
        },
        permissions: permissions.into_iter().collect(),
        dependencies: recipe.dependencies.clone(),
        assets: recipe.assets.clone(),
        files: files
            .iter()
            .map(|(path, bytes)| FileEntry {
                path: path.clone(),
                sha256: content_digest(bytes),
                size_bytes: Counter(bytes.len() as u64),
                executable: false,
            })
            .collect(),
    };
    // Reuse the package verifier's manifest rules before requesting a signature.
    validate_candidate_manifest(&manifest)?;
    Ok(Candidate { manifest, files })
}
fn validate_candidate_manifest(manifest: &Manifest) -> Result<()> {
    let policy = VerificationPolicy {
        additional_package_abis: Default::default(),
        publishers: BTreeMap::new(),
        contracts: manifest.contracts.clone(),
        target: manifest.targets[0].clone(),
        dependencies: BTreeMap::new(),
        assets: BTreeMap::new(),
        max_files: 32,
        max_content_bytes: 4 * 1024 * 1024,
    };
    validate_manifest(manifest, &policy)?;
    Ok(())
}
/// Rebuild the candidate and compare every signed byte; no path is executed.
pub fn from_files(mut files: BTreeMap<PackagePath, Vec<u8>>) -> Result<Candidate> {
    let manifest = files
        .remove(&path("manifest.json"))
        .ok_or_else(|| Error::Invalid("candidate manifest missing".into()))?;
    if files.contains_key(&path("manifest.sig.json")) {
        return Err(Error::Invalid("candidate already has a signature".into()));
    }
    let input: CompileInput = canonical::decode_json(
        files
            .get(&path(INPUT))
            .ok_or_else(|| Error::Invalid("compile input missing".into()))?,
    )
    .map_err(|e| Error::Invalid(e.to_string()))?;
    let recipe: Recipe = canonical::decode_json(
        files
            .get(&path(RECIPE))
            .ok_or_else(|| Error::Invalid("recipe missing".into()))?,
    )
    .map_err(|e| Error::Invalid(e.to_string()))?;
    let expected = assemble(&input, &recipe)?;
    if manifest != manifest_bytes(expected.manifest())? || files != expected.files {
        return Err(Error::Invalid(
            "candidate differs from its deterministic assembly".into(),
        ));
    }
    Ok(expected)
}
pub fn compile_verified(package: &VerifiedPackage) -> Result<ResolvedProcess> {
    let get = |name: &str| {
        package
            .file(&path(name))
            .ok_or_else(|| Error::Invalid(format!("signed process entry missing: {name}")))
    };
    let input: CompileInput =
        canonical::decode_json(get(INPUT)?).map_err(|e| Error::Invalid(e.to_string()))?;
    let recipe: Recipe =
        canonical::decode_json(get(RECIPE)?).map_err(|e| Error::Invalid(e.to_string()))?;
    let expected = assemble(&input, &recipe)?;
    if manifest_bytes(package.manifest())? != manifest_bytes(expected.manifest())? {
        return Err(Error::Invalid(
            "signed manifest differs from assembly".into(),
        ));
    }
    for (path, bytes) in expected.files {
        if package.file(&path) != Some(bytes.as_slice()) {
            return Err(Error::Invalid("signed bytes differ from assembly".into()));
        }
    }
    let process = rx_process::compile_package(package, input.bindings)?;
    Ok(process)
}
