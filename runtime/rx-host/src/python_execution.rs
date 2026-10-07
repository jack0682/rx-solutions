//! Python-only program/environment pins. Approval and input semantics belong to execution-v2.
use crate::{HostError, Result, python_skill::Program};
use rx_domain::{
    canonical,
    intent::{Body, Intent},
    types::*,
};
use rx_package::{PackagePath, directory::read_relative_file};
use rx_process_contract::execution_v2::host_inputs::BoundInput;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const SCHEMA: &str = "rx.python-execution-profile.v2";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub schema: Name,
    pub environment: PathBuf,
    pub environment_digest: Digest,
    pub program: ArtifactRef,
}
impl Profile {
    /// Loading is passive; no skill/SDK import and no execution authority are acquired.
    pub fn validate(&self) -> Result<()> {
        if self.schema.as_str() != SCHEMA
            || !self.environment.is_absolute()
            || self.program.schema_id.as_str() != "rx.python-environment.v1"
            || self.program.size_bytes.0 == 0
            || self.program.size_bytes.0 > 1_048_576
        {
            return Err(HostError::Invalid(
                "Python v2 environment/program shape differs".into(),
            ));
        }
        let bytes = read_relative_file(
            &self.environment,
            &PackagePath::new("environment.json").map_err(invalid)?,
            self.program.size_bytes.0,
        )
        .map_err(invalid)?;
        if bytes.len() as u64 != self.program.size_bytes.0
            || rx_package::content_digest(&bytes) != self.program.sha256
        {
            return Err(HostError::Invalid(
                "Python v2 environment program pin differs".into(),
            ));
        }
        let record: serde_json::Value = canonical::decode_json(&bytes).map_err(invalid)?;
        if record["environment_digest"] != self.environment_digest.to_string()
            || record["environment"]["schema"] != "rx.python-environment.v1"
            || record["environment"]["path"] != self.environment.to_string_lossy().as_ref()
            || rx_package::content_digest(
                &canonical::bytes(&record["environment"]).map_err(invalid)?,
            )
            .to_string()
                != self.environment_digest.to_string()
        {
            return Err(HostError::Invalid(
                "Python v2 environment metadata differs".into(),
            ));
        }
        Ok(())
    }
    /// This is a provider projection, not approval. Only the Host gate may call it after
    /// membership/qualification checks and pass the same saved BoundInput at authorization.
    pub fn program(&self, intent: &Intent, input: &BoundInput) -> Result<Program> {
        self.validate()?;
        input.binding.validate().map_err(invalid)?;
        let Body::Program(goal) = &intent.body else {
            return Err(HostError::Guard);
        };
        if intent.completion_rule.as_str() != "rx.python.returned.v1"
            || goal.program != self.program
            || goal.parameter_set != input.binding.selection.parameter
            || intent.digest().map_err(invalid)? != input.binding.selection.intent_digest
        {
            return Err(HostError::Guard);
        }
        let value = canonical::decode_json(&input.parameters).map_err(invalid)?;
        let program = Program {
            environment: self.environment.clone(),
            environment_digest: self.environment_digest,
            input: value,
            intent: intent.clone(),
        };
        crate::python_skill::validate_program(&program)?;
        Ok(program)
    }
}
fn invalid(e: impl std::fmt::Display) -> HostError {
    HostError::Invalid(e.to_string())
}
