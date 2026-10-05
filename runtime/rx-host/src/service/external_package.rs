//! One signed package registry, independent of device names, vendors and task types.
use super::{
    Result,
    config::PinnedFile,
    device_package::{Driver, Family},
};
use crate::{Binding, Environment, execution_package::Templates, external_process::profile::*};
use rx_domain::{canonical, intent::Body, types::*};
use rx_package::{PackagePath, VerifiedPackage};
use rx_process_contract::{
    execution_v2::{self as v2, TemplateCatalog, TemplateDocument},
    native_outcome::*,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

fn name(v: &str) -> Name {
    Name::new(v).expect("constant")
}
fn path(v: &str) -> PackagePath {
    PackagePath::new(v).expect("constant")
}
fn reference(schema: &str, bytes: &[u8]) -> ArtifactRef {
    ArtifactRef {
        schema_id: name(schema),
        sha256: rx_package::content_digest(bytes),
        size_bytes: Counter(bytes.len() as u64),
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    pub directory: PathBuf,
    pub manifest_digest: Digest,
    pub policy: PinnedFile,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub schema: Name,
    pub entries: BTreeMap<Name, Registration>,
}
impl Registry {
    pub fn selected(pin: &PinnedFile, key: &Name) -> Result<Registration> {
        let registry: Self = canonical::decode_json(&pin.read(false)?)?;
        if registry.schema.as_str() != "rx.external-adapter-registry.v1"
            || registry.entries.len() > 64
        {
            return Err("external registry schema/bounds differ".into());
        }
        registry
            .entries
            .get(key)
            .cloned()
            .ok_or_else(|| "UNREGISTERED_ADAPTER".into())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assembly {
    pub schema: Name,
    pub profile: Profile,
    pub program: Program,
    pub catalog: TemplateCatalog,
    pub outcomes: NativeOutcomeTable,
}
pub fn driver() -> Driver {
    Driver {
        schema: name("rx.native-driver-reference.v1"),
        implementation: name("rx.external-process.v1"),
        source_digest: canonical::digest(
            "RX-EXTERNAL-PROCESS-DRIVER-v1",
            &(
                include_str!("external_package.rs"),
                include_str!("../external_process/profile.rs"),
                include_str!("../external_process/mod.rs"),
                include_str!("../external_process/wire.rs"),
                include_str!("../external_process/owned.rs"),
                include_str!("../../../../deployment/external-adapters/rx_external_adapter.py"),
            ),
        )
        .expect("source identity"),
    }
}
pub struct Checked {
    pub profile: Profile,
    pub program: Program,
    pub templates: Templates,
    pub outcomes: NativeOutcomeTable,
}
pub type Documents = (
    Profile,
    Program,
    TemplateCatalog,
    NativeOutcomeTable,
    BTreeMap<PackagePath, Vec<u8>>,
    Vec<ArtifactRef>,
);
pub fn documents(original: &Assembly) -> Result<Documents> {
    if original.schema.as_str() != "rx.external-process-assembly.v1" {
        return Err("external assembly schema differs".into());
    }
    original.profile.validate(&original.program)?;
    let profile = original.profile.clone();
    let profile_digest = profile.digest()?;
    let mut catalog = original.catalog.clone();
    let input = reference(
        v2::PARAMETER_SCHEMA,
        &canonical::bytes(&serde_json::json!({}))?,
    );
    let mut outcomes = original.outcomes.clone();
    outcomes.profile_digest = profile_digest;
    outcomes.validate()?;
    for value in catalog.templates.values_mut() {
        let declared = profile
            .commands
            .get(&value.contract.primitive)
            .ok_or("undeclared template command")?;
        if canonical::bytes(declared)? != canonical::bytes(&value.contract)? {
            return Err("external typed command/template differs".into());
        }
        value.action.intent = value.action.intent.normalized()?;
        if value.action.intent.completion_rule != outcomes.completion_rule
            || value.action.intent.execution_timeout_ms.0 > 60_000
        {
            return Err("external completion/timeout differs".into());
        }
        let Body::Program(goal) = &mut value.action.intent.body else {
            return Err("external Program required".into());
        };
        if goal.program != profile.program {
            return Err("external program pin differs".into());
        }
        goal.parameter_set = input.clone();
        value.action.intent.profile_digest = profile_digest;
    }
    let family = Family {
        schema: name("rx.device-family.v1"),
        family: name("external-process"),
        controller_model: name("declared-adapter"),
        environment: Environment::Simulation,
    };
    let mut files = BTreeMap::from([
        (path("family.json"), canonical::bytes(&family)?),
        (path("profile.json"), canonical::bytes(&profile)?),
        (path("adapter.json"), canonical::bytes(&driver())?),
        (path("program.json"), canonical::bytes(&original.program)?),
        (
            path("template-input.json"),
            canonical::bytes(&serde_json::json!({}))?,
        ),
        (path("outcomes.json"), canonical::bytes(&outcomes)?),
        (path("authoring/assembly.json"), canonical::bytes(original)?),
    ]);
    catalog.documents.clear();
    let mut assets = vec![profile.program.clone(), input];
    for (role, schema) in [
        ("family", "rx.device-family.v1"),
        ("profile", PROFILE_SCHEMA),
        ("adapter", "rx.native-driver-reference.v1"),
    ] {
        let file = format!("{role}.json");
        let artifact = reference(schema, &files[&path(&file)]);
        assets.push(artifact.clone());
        catalog.documents.insert(
            name(role),
            TemplateDocument {
                path: file,
                artifact,
            },
        );
    }
    catalog.validate()?;
    let raw = canonical::bytes(&catalog)?;
    assets.push(reference(v2::TEMPLATE_CATALOG_SCHEMA, &raw));
    assets.push(reference(
        "rx.native-outcome-table.v1",
        &files[&path("outcomes.json")],
    ));
    files.insert(path("execution-template-catalog.json"), raw);
    Ok((
        profile,
        original.program.clone(),
        catalog,
        outcomes,
        files,
        assets,
    ))
}
pub fn decode(package: &VerifiedPackage) -> Result<Checked> {
    let original: Assembly = canonical::decode_json(
        package
            .file(&path("authoring/assembly.json"))
            .ok_or("external originals missing")?,
    )?;
    let (profile, program, _, outcomes, files, mut assets) = documents(&original)?;
    let manifest = package.manifest();
    let entry = rx_package::EntryPoint::DeviceReference {
        family: path("family.json"),
        profiles: vec![path("profile.json")],
        adapter: path("adapter.json"),
    };
    let permissions = BTreeSet::from([
        rx_package::Permission::ArtifactRead,
        rx_package::Permission::NativeEndpoint {
            role: name("external-process"),
        },
    ]);
    if canonical::bytes(&manifest.entry)? != canonical::bytes(&entry)?
        || manifest
            .permissions
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            != permissions
        || !manifest.dependencies.is_empty()
        || manifest.files.len() != files.len()
        || manifest.targets.len() != 1
        || manifest.targets[0].os != rx_package::OperatingSystem::Linux
        || manifest.targets[0].ros_distribution.is_some()
    {
        return Err("external package entry/permissions/target differs".into());
    }
    assets.sort_by_key(|a| a.sha256);
    let mut actual = manifest.assets.clone();
    actual.sort_by_key(|a| a.sha256);
    if canonical::bytes(&assets)? != canonical::bytes(&actual)? {
        return Err("external asset closure differs".into());
    }
    for (file, bytes) in files {
        if package.file(&file) != Some(bytes.as_slice()) {
            return Err("external package differs from signed originals".into());
        }
    }
    Ok(Checked {
        profile,
        program,
        templates: Templates::verify(package)?,
        outcomes,
    })
}
pub fn load(registry: &PinnedFile, key: &Name) -> Result<(Digest, Checked)> {
    let registration = Registry::selected(registry, key)?;
    // Reuse the existing immutable DEVICE_REFERENCE verification and material acquisition.
    let package = super::python_package::load_package(
        &registration.directory,
        registration.manifest_digest,
        &registration.policy,
    )?;
    Ok((package.digest(), decode(&package)?))
}
impl Checked {
    pub fn validate_bindings(&self, bindings: &[Binding]) -> Result<()> {
        let catalog = self.templates.catalog();
        if bindings.len() != 1
            || bindings[0].environment != Environment::Simulation
            || bindings[0].cell != catalog.cell
            || bindings[0].platform.as_str() != catalog.installation.as_str()
        {
            return Err("external binding scope differs".into());
        }
        let expected = catalog
            .templates
            .values()
            .map(|t| {
                if t.action.host != bindings[0].host {
                    return Err("external template Host differs".into());
                }
                Ok(t.action.intent.digest()?)
            })
            .collect::<Result<BTreeSet<_>>>()?;
        let actual = bindings[0]
            .allowed_intents
            .iter()
            .map(|i| i.digest())
            .collect::<std::result::Result<BTreeSet<_>, _>>()?;
        if expected != actual
            || bindings[0].condition_ids.iter().collect::<BTreeSet<_>>()
                != self.profile.conditions.keys().collect::<BTreeSet<_>>()
        {
            return Err("external template/condition binding differs".into());
        }
        self.program.validate(true)?;
        Ok(())
    }
}
