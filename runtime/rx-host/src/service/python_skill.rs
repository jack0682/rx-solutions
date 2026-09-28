//! Passive loading of a pinned Python registration. No SDK import occurs here.
use super::{
    Result,
    config::{Backend, PinnedFile},
};
use crate::{
    Binding, Environment,
    python_skill::{Program, ReleasePython},
};
use rx_domain::{canonical, intent::Intent, types::*};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const RELEASE_ROOT: &str = "/opt/rx/python";
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    pub schema: Name,
    pub installation: Id,
    pub host: Name,
    pub cell: Name,
    pub environment: PathBuf,
    pub environment_digest: Digest,
    pub input: serde_json::Value,
    pub intent: Intent,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PythonRelease {
    schema: Name,
    interpreter_sha256: Digest,
}
pub fn load(backend: &Backend) -> Result<(Digest, Registration)> {
    if matches!(backend, Backend::PythonSkillPackage { .. }) {
        return super::python_package::load(backend);
    }
    let Backend::PythonSkillSimulation { registration } = backend else {
        return Err("Python skill registration backend required".into());
    };
    let value: Registration = canonical::decode_json(&registration.read(false)?)?;
    if value.schema.as_str() != "rx.python-skill-registration.v1"
        || !value.environment.is_absolute()
        || value.intent.completion_rule.as_str() != "rx.python.returned.v1"
    {
        return Err("Python registration schema/environment/completion differs".into());
    }
    value.intent.normalized()?;
    Ok((registration.sha256, value))
}
impl Registration {
    pub fn validate_bindings(&self, bindings: &[Binding]) -> Result<()> {
        if bindings.len() != 1
            || bindings[0].environment != Environment::Simulation
            || bindings[0].host != self.host
            || bindings[0].cell != self.cell
            || bindings[0].allowed_intents.len() != 1
            || bindings[0].allowed_intents[0].digest()? != self.intent.digest()?
            || bindings[0].condition_ids != [Name::new("sim/ready")?]
        {
            return Err("Python simulation registration and Host binding differ".into());
        }
        crate::python_skill::validate_program(&self.clone().program())?;
        Ok(())
    }
    pub fn program(self) -> Program {
        Program {
            environment: self.environment,
            environment_digest: self.environment_digest,
            input: self.input,
            intent: self.intent,
        }
    }
}
pub fn release() -> Result<ReleasePython> {
    release_at(Path::new(RELEASE_ROOT))
}
fn release_at(root: &Path) -> Result<ReleasePython> {
    let raw = rx_package::directory::read_relative_file(
        root,
        &rx_package::PackagePath::new("release.json")?,
        65536,
    )?;
    let release: PythonRelease = canonical::decode_json(&raw)?;
    if release.schema.as_str() != "rx.python-host-release.v1" {
        return Err("Python Host release schema differs".into());
    }
    let expected_runner = rx_package::content_digest(include_bytes!(
        "../../../../deployment/local-skills/host_runner.py"
    ));
    let expected_verifier = rx_package::content_digest(include_bytes!(
        "../../../../deployment/local-skills/python_environment.py"
    ));
    for (name, digest) in [
        ("host_runner.py", expected_runner),
        ("python_environment.py", expected_verifier),
    ] {
        PinnedFile {
            path: root.join(name),
            sha256: digest,
        }
        .read(false)?;
    }
    let executable = root.join("python");
    let raw = rx_package::directory::read_relative_file(
        root,
        &rx_package::PackagePath::new("python")?,
        16 * 1024 * 1024,
    )?;
    if rx_package::content_digest(&raw) != release.interpreter_sha256 {
        return Err("Python release interpreter differs".into());
    }
    Ok(ReleasePython {
        executable,
        executable_digest: release.interpreter_sha256,
        runner: root.join("host_runner.py"),
        runner_digest: expected_runner,
        verifier_digest: expected_verifier,
    })
}
