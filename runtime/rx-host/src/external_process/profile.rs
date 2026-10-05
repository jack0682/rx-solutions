//! Provider-specific pins; approval and operation identity remain execution-v2 facts.
use crate::{HostError, Result};
use rx_domain::{
    canonical,
    intent::{Body, Intent},
    types::*,
};
use rx_process_contract::execution_v2::{self as v2, host_inputs::BoundInput};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

pub const PROFILE_SCHEMA: &str = "rx.external-process-profile.v1";
pub const PROGRAM_SCHEMA: &str = "rx.external-process-program.v1";
pub const PROTOCOL: &str = "rx.external-process-channel.v1";

pub fn invalid(e: impl std::fmt::Display) -> HostError {
    HostError::Invalid(e.to_string())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilePin {
    pub path: PathBuf,
    pub sha256: Digest,
    pub size_bytes: Counter,
}
impl FilePin {
    pub fn verify(&self) -> Result<()> {
        if !self.path.is_absolute()
            || self.size_bytes.0 == 0
            || self.size_bytes.0 > 16 * 1024 * 1024
        {
            return Err(invalid("external program file bound/path"));
        }
        let name = self
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or(HostError::Guard)?;
        let bytes = rx_package::directory::read_relative_file(
            self.path.parent().ok_or(HostError::Guard)?,
            &rx_package::PackagePath::new(name).map_err(invalid)?,
            self.size_bytes.0,
        )
        .map_err(invalid)?;
        if bytes.len() as u64 != self.size_bytes.0
            || rx_package::content_digest(&bytes) != self.sha256
        {
            return Err(invalid("external program closure digest differs"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Program {
    pub schema: Name,
    pub executable: FilePin,
    pub arguments: Vec<String>,
    pub dependencies: Vec<FilePin>,
}
impl Program {
    pub fn validate(&self, verify_files: bool) -> Result<()> {
        if self.schema.as_str() != PROGRAM_SCHEMA
            || self.arguments.len() > 16
            || self.dependencies.len() > 64
            || self
                .arguments
                .iter()
                .any(|s| s.is_empty() || s.len() > 4096 || s.contains('\0'))
        {
            return Err(invalid("external program shape/bounds"));
        }
        let mut paths = std::collections::BTreeSet::new();
        for pin in std::iter::once(&self.executable).chain(&self.dependencies) {
            if !paths.insert(&pin.path)
                || !pin.path.is_absolute()
                || pin.size_bytes.0 == 0
                || pin.size_bytes.0 > 16 * 1024 * 1024
            {
                return Err(invalid("external program closure shape"));
            }
            if verify_files {
                pin.verify()?;
            }
        }
        Ok(())
    }
    pub fn reference(&self) -> Result<ArtifactRef> {
        let raw = canonical::bytes(self).map_err(invalid)?;
        Ok(ArtifactRef {
            schema_id: Name::new(PROGRAM_SCHEMA).map_err(invalid)?,
            sha256: rx_package::content_digest(&raw),
            size_bytes: Counter(raw.len() as u64),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ValueKind {
    Boolean,
    Integer,
    Real,
    Symbol,
    Reals,
}
impl ValueKind {
    pub fn matches(&self, value: &TypedValue) -> bool {
        matches!(
            (self, value),
            (Self::Boolean, TypedValue::Boolean(_))
                | (Self::Integer, TypedValue::Integer(_))
                | (Self::Real, TypedValue::Real(_))
                | (Self::Symbol, TypedValue::Symbol(_))
                | (Self::Reals, TypedValue::Reals(_))
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub schema: Name,
    pub unit: Name,
    pub value_type: ValueKind,
    pub maximum_age_ns: Counter,
    pub maximum_uncertainty_ns: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub schema: Name,
    pub protocol: Name,
    pub program: ArtifactRef,
    pub commands: BTreeMap<Name, v2::NodeContract>,
    pub observations: BTreeMap<Name, Observation>,
    /// Existing boolean condition satisfaction, evaluated by Host from fresh native samples.
    pub conditions: BTreeMap<Name, Name>,
}
impl Profile {
    pub fn digest(&self) -> Result<Digest> {
        Ok(rx_package::content_digest(
            &canonical::bytes(self).map_err(invalid)?,
        ))
    }
    pub fn validate(&self, program: &Program) -> Result<()> {
        program.validate(false)?;
        if self.schema.as_str() != PROFILE_SCHEMA
            || self.protocol.as_str() != PROTOCOL
            || self.program != program.reference()?
            || self.commands.is_empty()
            || self.commands.len() > 16
            || self.observations.is_empty()
            || self.observations.len() > 64
            || self.conditions.len() > 64
        {
            return Err(invalid("external profile identity/bounds"));
        }
        for (name, contract) in &self.commands {
            if name != &contract.primitive {
                return Err(invalid("external command declaration differs"));
            }
        }
        for observation in self.observations.values() {
            if observation.maximum_age_ns.0 == 0
                || observation.maximum_age_ns.0 > 1_000_000_000
                || observation.maximum_uncertainty_ns > observation.maximum_age_ns
            {
                return Err(invalid("external observation freshness bound"));
            }
        }
        for source in self.conditions.values() {
            if !self
                .observations
                .get(source)
                .is_some_and(|o| matches!(o.value_type, ValueKind::Boolean))
            {
                return Err(invalid(
                    "external condition requires declared boolean source",
                ));
            }
        }
        Ok(())
    }
    /// This projection is never an approval check; the Host gate supplies the saved verified input.
    pub fn input(&self, intent: &Intent, input: &BoundInput) -> Result<serde_json::Value> {
        input.binding.validate().map_err(invalid)?;
        let Body::Program(goal) = &intent.body else {
            return Err(HostError::Guard);
        };
        if goal.program != self.program
            || goal.parameter_set != input.binding.selection.parameter
            || intent.profile_digest != self.digest()?
            || intent.digest().map_err(invalid)? != input.binding.selection.intent_digest
        {
            return Err(HostError::Conflict);
        }
        let value: serde_json::Value =
            canonical::decode_json(&input.parameters).map_err(invalid)?;
        let primitive = value["primitive"].as_str().ok_or(HostError::Guard)?;
        let key = Name::new(primitive).map_err(invalid)?;
        if value["schema"] != v2::PARAMETER_SCHEMA || !self.commands.contains_key(&key) {
            return Err(invalid("undeclared external command"));
        }
        Ok(value)
    }
}
