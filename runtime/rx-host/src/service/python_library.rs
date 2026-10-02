//! A bounded, signed library of fixed Python inputs behind one existing Host owner.
use super::{
    Result,
    config::Backend,
    device_package::{Driver, Family},
    python_package,
    python_skill::Registration,
};
use crate::{Binding, Environment, python_skill::Program};
use rx_domain::{
    canonical,
    intent::{Body, Intent},
    types::*,
};
use rx_package::{PackagePath, VerifiedPackage};
use rx_process_contract::{
    device_catalog::{Catalog, Document},
    native_outcome::*,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub input: serde_json::Value,
    pub intent: Intent,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Library {
    pub schema: Name,
    pub installation: Id,
    pub host: Name,
    pub cell: Name,
    pub environment: PathBuf,
    pub environment_digest: Digest,
    pub programs: BTreeMap<Name, Entry>,
}
fn name(v: &str) -> Name {
    Name::new(v).expect("constant")
}
fn path(v: &str) -> PackagePath {
    PackagePath::new(v).expect("constant")
}
impl Library {
    pub fn registrations(&self) -> Vec<Registration> {
        self.programs
            .values()
            .map(|entry| Registration {
                schema: name("rx.python-skill-registration.v1"),
                installation: self.installation.clone(),
                host: self.host.clone(),
                cell: self.cell.clone(),
                environment: self.environment.clone(),
                environment_digest: self.environment_digest,
                input: entry.input.clone(),
                intent: entry.intent.clone(),
            })
            .collect()
    }
    pub fn programs(&self) -> Vec<Program> {
        self.registrations()
            .into_iter()
            .map(Registration::program)
            .collect()
    }
    fn shape(&self) -> Result<()> {
        if self.schema.as_str() != "rx.python-skill-library.v1"
            || !self.environment.is_absolute()
            || self.programs.is_empty()
            || self.programs.len() > 16
            || canonical::bytes(self)?.len() > 1_048_576
        {
            return Err("Python library schema/scope/size differs".into());
        }
        let mut digests = BTreeSet::new();
        for entry in self.programs.values() {
            if !entry.input.is_object() || canonical::bytes(&entry.input)?.len() > 65536 {
                return Err("library input must be a bounded object".into());
            }
            if !digests.insert(entry.intent.digest()?) {
                return Err("duplicate library Intent".into());
            }
        }
        Ok(())
    }
    pub fn profile_digest(&self) -> Result<Digest> {
        let mut original = self.clone();
        for entry in original.programs.values_mut() {
            entry.intent.profile_digest = Digest::from_bytes([0; 32]);
        }
        Ok(canonical::digest(
            "RX-PYTHON-LIBRARY-PROFILE-v1",
            &original,
        )?)
    }
    pub fn assets(&self) -> Result<Vec<ArtifactRef>> {
        let mut assets = BTreeMap::new();
        for entry in self.programs.values() {
            let Body::Program(goal) = &entry.intent.body else {
                return Err("library ProgramGoal required".into());
            };
            for value in [&goal.program, &goal.parameter_set] {
                if assets
                    .insert(value.sha256, value.clone())
                    .as_ref()
                    .is_some_and(|old| old != value)
                {
                    return Err("library artifact identity conflict".into());
                }
            }
        }
        Ok(assets.into_values().collect())
    }
    pub fn validate_bindings(&self, bindings: &[Binding]) -> Result<()> {
        self.shape()?;
        if bindings.len() != 1 || bindings[0].allowed_intents.len() != self.programs.len() {
            return Err("library requires one exact Host binding".into());
        }
        let allowed: BTreeSet<_> = bindings[0]
            .allowed_intents
            .iter()
            .map(Intent::digest)
            .collect::<std::result::Result<_, _>>()?;
        if allowed.len() != self.programs.len() {
            return Err("duplicate bound Intent".into());
        }
        for registration in self.registrations() {
            if !allowed.contains(&registration.intent.digest()?) {
                return Err("library/binding Intent differs".into());
            }
            let mut binding = bindings[0].clone();
            binding.allowed_intents = vec![registration.intent.clone()];
            registration.validate_bindings(&[binding])?;
        }
        Ok(())
    }
}
pub fn driver() -> Driver {
    Driver {
        schema: name("rx.native-driver-reference.v1"),
        implementation: name("rx.python.sdk-library.v1"),
        source_digest: canonical::digest(
            "RX-PYTHON-LIBRARY-DRIVER-v1",
            &(
                python_package::driver().source_digest,
                rx_package::content_digest(include_bytes!("python_library.rs")),
            ),
        )
        .expect("static source"),
    }
}
pub fn documents(
    original: &Library,
    environment: &[u8],
) -> Result<(Library, BTreeMap<PackagePath, Vec<u8>>)> {
    original.shape()?;
    let mut profile = original.clone();
    for (entry, registration) in profile.programs.values_mut().zip(original.registrations()) {
        // Reuse the existing program/environment/input checks; no imports or native I/O.
        let (checked, _) = python_package::documents(&registration, environment)?;
        entry.intent = checked.intent;
        entry.intent.profile_digest = Digest::from_bytes([0; 32]);
    }
    let digest = profile.profile_digest()?;
    for entry in profile.programs.values_mut() {
        entry.intent.profile_digest = digest;
    }
    profile.shape()?;
    let outcomes = NativeOutcomeTable {
        schema: name("rx.native-outcome-table.v1"),
        profile_digest: digest,
        completion_rule: name("rx.python.returned.v1"),
        cases: vec![NativeOutcomeCase {
            status_schema: name("rx.python.returned.v1"),
            statuses: vec![Integer(0)],
            conclusion: NativeConclusion::Succeeded,
        }],
    };
    let operations: BTreeMap<_, _> = profile
        .programs
        .iter()
        .map(|(key, value)| (key.clone(), value.intent.clone()))
        .collect();
    let family = Family {
        schema: name("rx.device-family.v1"),
        family: name("python-sdk"),
        controller_model: name("python-function-library"),
        environment: Environment::Simulation,
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
                &serde_json::json!({"schema":"rx.python-skill-library-assembly.v1","library":original}),
            )?,
        ),
    ]);
    let mut documents = BTreeMap::new();
    for role in ["family", "profile", "adapter", "operations", "outcomes"] {
        let file = format!("{role}.json");
        let bytes = &files[&path(&file)];
        let schema = if role == "operations" {
            name("rx.python-library-operations.v1")
        } else {
            Name::new(
                serde_json::from_slice::<serde_json::Value>(bytes)?["schema"]
                    .as_str()
                    .ok_or("document schema")?,
            )?
        };
        documents.insert(
            name(role),
            Document {
                path: file,
                artifact: ArtifactRef {
                    sha256: rx_package::content_digest(bytes),
                    schema_id: schema,
                    size_bytes: Counter(bytes.len() as u64),
                },
            },
        );
    }
    let targets: BTreeSet<_> = operations.values().map(|v| v.target.clone()).collect();
    if targets.len() != 1 {
        return Err("library requires one declared target".into());
    }
    let catalog = Catalog {
        schema: name("rx.device-operation-catalog.v1"),
        installation: profile.installation.clone(),
        cell: profile.cell.clone(),
        target: targets.into_iter().next().expect("nonempty"),
        environment: rx_process_contract::device_catalog::Environment::Simulation,
        profile_digest: digest,
        condition_ids: BTreeSet::from([name("sim/ready")]),
        operations,
        outcomes: Some(outcomes),
        documents,
    };
    catalog.validate()?;
    files.insert(path("device-catalog.json"), canonical::bytes(&catalog)?);
    Ok((profile, files))
}
pub fn decode(package: &VerifiedPackage) -> Result<Library> {
    let original: serde_json::Value = canonical::decode_json(
        package
            .file(&path("authoring/assembly.json"))
            .ok_or("library assembly missing")?,
    )?;
    if original["schema"] != "rx.python-skill-library-assembly.v1" {
        return Err("library assembly schema differs".into());
    }
    let library: Library = serde_json::from_value(original["library"].clone())?;
    let (profile, files) = documents(
        &library,
        package
            .file(&path("environment.json"))
            .ok_or("library environment missing")?,
    )?;
    python_package::check_package(package, files, profile.assets()?)?;
    Ok(profile)
}
pub fn load(backend: &Backend) -> Result<(Digest, Library)> {
    let Backend::PythonSkillLibraryPackage {
        directory,
        manifest_digest,
        policy,
    } = backend
    else {
        return Err("Python library package required".into());
    };
    let package = python_package::load_package(directory, *manifest_digest, policy)?;
    Ok((package.digest(), decode(&package)?))
}
