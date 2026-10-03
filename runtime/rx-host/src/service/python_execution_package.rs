//! Signed Python v2 pins beside a common execution template catalog; no approval logic here.
use super::{
    Result,
    config::Backend,
    device_package::{Driver, Family},
    python_package,
};
use crate::{
    Binding, Environment,
    execution_package::Templates,
    python_execution::{Profile, SCHEMA},
};
use rx_domain::{canonical, intent::Body, types::*};
use rx_package::{PackagePath, VerifiedPackage};
use rx_process_contract::{
    execution_v2::{self as v2, TemplateCatalog, TemplateDocument},
    native_outcome::*,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
fn name(s: &str) -> Name {
    Name::new(s).expect("constant")
}
fn path(s: &str) -> PackagePath {
    PackagePath::new(s).expect("constant")
}
fn reference(schema: &str, bytes: &[u8]) -> ArtifactRef {
    ArtifactRef {
        schema_id: name(schema),
        sha256: rx_package::content_digest(bytes),
        size_bytes: Counter(bytes.len() as u64),
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assembly {
    pub schema: Name,
    pub profile: Profile,
    pub catalog: TemplateCatalog,
}
pub fn driver() -> Driver {
    Driver {
        schema: name("rx.native-driver-reference.v1"),
        implementation: name("rx.python.execution.v2"),
        source_digest: canonical::digest(
            "RX-PYTHON-EXECUTION-DRIVER-v2",
            &(
                python_package::driver().source_digest,
                include_str!("python_execution_package.rs"),
                include_str!("../python_execution.rs"),
            ),
        )
        .expect("source"),
    }
}
pub struct Checked {
    pub profile: Profile,
    pub templates: Templates,
}
impl Checked {
    pub fn validate_bindings(&self, bindings: &[Binding]) -> Result<()> {
        let catalog = self.templates.catalog();
        if bindings.len() != 1
            || bindings[0].environment != Environment::Simulation
            || bindings[0].cell != catalog.cell
            || bindings[0].platform.as_str() != catalog.installation.as_str()
        {
            return Err("Python execution package scope differs".into());
        }
        let expected = catalog
            .templates
            .values()
            .map(|t| {
                if t.action.host != bindings[0].host {
                    return Err("template Host differs".into());
                }
                Ok(t.action.intent.digest()?)
            })
            .collect::<Result<BTreeSet<_>>>()?;
        let actual = bindings[0]
            .allowed_intents
            .iter()
            .map(|i| i.digest())
            .collect::<std::result::Result<BTreeSet<_>, _>>()?;
        if actual != expected || bindings[0].condition_ids != [name("sim/ready")] {
            return Err("Python execution template bindings differ".into());
        }
        self.profile.validate()?;
        Ok(())
    }
}
pub type Documents = (
    Profile,
    TemplateCatalog,
    BTreeMap<PackagePath, Vec<u8>>,
    Vec<ArtifactRef>,
);
pub fn documents(original: &Assembly, environment: &[u8]) -> Result<Documents> {
    if original.schema.as_str() != "rx.python-execution-assembly.v2"
        || environment.len() > 1_048_576
    {
        return Err("Python execution assembly bound/schema".into());
    }
    let profile = &original.profile;
    let metadata: serde_json::Value = canonical::decode_json(environment)?;
    if profile.schema.as_str() != SCHEMA
        || !profile.environment.is_absolute()
        || profile.program != reference("rx.python-environment.v1", environment)
        || metadata["environment_digest"] != profile.environment_digest.to_string()
        || metadata["environment"]["schema"] != "rx.python-environment.v1"
        || metadata["environment"]["path"] != profile.environment.to_string_lossy().as_ref()
        || metadata["environment"]["python"]["platform"] != "linux"
        || rx_package::content_digest(&canonical::bytes(&metadata["environment"])?).to_string()
            != profile.environment_digest.to_string()
    {
        return Err("Python execution environment pins differ".into());
    }
    // Qualification dependencies address the exact profile artifact bytes.
    let profile_digest = rx_package::content_digest(&canonical::bytes(profile)?);
    let family = Family {
        schema: name("rx.device-family.v1"),
        family: name("python-sdk"),
        controller_model: name("python-execution"),
        environment: Environment::Simulation,
    };
    let placeholder = canonical::bytes(&serde_json::json!({}))?;
    let input = reference(v2::PARAMETER_SCHEMA, &placeholder);
    let mut catalog = original.catalog.clone();
    for value in catalog.templates.values_mut() {
        value.action.intent = value.action.intent.normalized()?;
        if value.action.intent.completion_rule.as_str() != "rx.python.returned.v1"
            || value.action.intent.execution_timeout_ms.0 > 60000
        {
            return Err("Python completion/timeout unsupported".into());
        }
        let Body::Program(p) = &mut value.action.intent.body else {
            return Err("Program required".into());
        };
        if p.program != profile.program {
            return Err("template program differs from Python pin".into());
        }
        p.parameter_set = input.clone();
        value.action.intent.profile_digest = profile_digest;
    }
    let outcomes = NativeOutcomeTable {
        schema: name("rx.native-outcome-table.v1"),
        profile_digest,
        completion_rule: name("rx.python.returned.v1"),
        cases: vec![NativeOutcomeCase {
            status_schema: name("rx.python.returned.v1"),
            statuses: vec![Integer(0)],
            conclusion: NativeConclusion::Succeeded,
        }],
    };
    let mut files = BTreeMap::from([
        (path("family.json"), canonical::bytes(&family)?),
        (path("profile.json"), canonical::bytes(profile)?),
        (path("adapter.json"), canonical::bytes(&driver())?),
        (path("environment.json"), environment.to_vec()),
        (path("template-input.json"), placeholder),
        (path("outcomes.json"), canonical::bytes(&outcomes)?),
        (path("authoring/assembly.json"), canonical::bytes(original)?),
    ]);
    catalog.documents = BTreeMap::new();
    let mut assets = vec![profile.program.clone(), input];
    for (role, schema) in [
        ("family", "rx.device-family.v1"),
        ("profile", SCHEMA),
        ("adapter", "rx.native-driver-reference.v1"),
    ] {
        let filename = format!("{role}.json");
        let artifact = reference(schema, &files[&path(&filename)]);
        assets.push(artifact.clone());
        catalog.documents.insert(
            name(role),
            TemplateDocument {
                path: filename,
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
    Ok((profile.clone(), catalog, files, assets))
}
pub fn decode(package: &VerifiedPackage) -> Result<Checked> {
    let original: Assembly = canonical::decode_json(
        package
            .file(&path("authoring/assembly.json"))
            .ok_or("Python execution originals missing")?,
    )?;
    let (profile, _, files, assets) = documents(
        &original,
        package
            .file(&path("environment.json"))
            .ok_or("environment missing")?,
    )?;
    python_package::check_package(package, files, assets)?;
    Ok(Checked {
        profile,
        templates: Templates::verify(package)?,
    })
}
pub fn load(backend: &Backend) -> Result<(Digest, Checked)> {
    let Backend::PythonExecutionPackage {
        directory,
        manifest_digest,
        policy,
    } = backend
    else {
        return Err("Python execution package required".into());
    };
    let package = python_package::load_package(directory, *manifest_digest, policy)?;
    Ok((package.digest(), decode(&package)?))
}
