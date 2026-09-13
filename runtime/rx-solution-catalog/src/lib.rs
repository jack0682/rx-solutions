//! Manufacturer-neutral device declarations. Simulation fixtures and source observation are never equipment qualification.
use rx_domain::{canonical, types::*};
use rx_package::PackagePath;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryPin {
    pub repository: Name,
    pub url: String,
    pub commit: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceFile {
    pub repository: Name,
    pub commit: String,
    pub path: PackagePath,
    pub sha256: Digest,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ControlRole {
    Manipulator,
    Follower,
    Leader,
    MobileBase,
    ImpedanceControl,
    PolicyControl,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EvidenceLevel {
    SourceObserved,
    SimulationFixture,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerDeclaration {
    pub name: String,
    pub plugin: String,
    pub joint_order: Vec<String>,
    pub command_interfaces: InterfaceDeclaration,
    pub state_interfaces: InterfaceDeclaration,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InterfaceDeclaration {
    Names(Vec<String>),
    Gpio(BTreeMap<String, Vec<InterfaceGroup>>),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterfaceGroup {
    pub interfaces: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureFile {
    pub path: PackagePath,
    pub sha256: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupportProfile {
    pub support_id: Name,
    pub model: Name,
    pub role: ControlRole,
    pub declared_update_hz: Counter,
    pub sources: Vec<SourceFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fixture_sources: Vec<FixtureFile>,
    pub controllers: Vec<ControllerDeclaration>,
    pub evidence_level: EvidenceLevel,
    pub commissioning_inputs: Vec<Name>,
    pub notes: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceCatalog {
    pub schema: Name,
    pub repositories: Vec<RepositoryPin>,
    pub profiles: Vec<SupportProfile>,
}
#[derive(Debug, thiserror::Error)]
#[error("invalid device support catalog: {0}")]
pub struct Error(pub String);
impl DeviceCatalog {
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let catalog: Self = canonical::decode_json(bytes).map_err(|e| Error(e.to_string()))?;
        catalog.validate()?;
        Ok(catalog)
    }
    pub fn validate(&self) -> Result<(), Error> {
        if self.schema.as_str() != "rx.device-support.v1" {
            return Err(Error("schema".into()));
        }
        let sources: BTreeMap<_, _> = self
            .repositories
            .iter()
            .map(|r| (&r.repository, r))
            .collect();
        if sources.len() != self.repositories.len() {
            return Err(Error("duplicate source repository".into()));
        }
        for source in sources.values() {
            if !git_commit(&source.commit)
                || !source.url.starts_with("https://")
                || source.url.len() <= "https://".len()
                || source.url.chars().any(char::is_whitespace)
            {
                return Err(Error("source pin".into()));
            }
        }
        let profiles: BTreeSet<_> = self
            .profiles
            .iter()
            .map(|p| p.support_id.as_str())
            .collect();
        if profiles.is_empty() || profiles.len() != self.profiles.len() {
            return Err(Error("missing or duplicate support profile".into()));
        }
        for profile in &self.profiles {
            let provenance_valid = match profile.evidence_level {
                EvidenceLevel::SourceObserved => {
                    !profile.sources.is_empty() && profile.fixture_sources.is_empty()
                }
                EvidenceLevel::SimulationFixture => {
                    profile.sources.is_empty() && !profile.fixture_sources.is_empty()
                }
            };
            if !provenance_valid
                || profile.controllers.is_empty()
                || profile.declared_update_hz.0 == 0
            {
                return Err(Error(format!(
                    "{} lacks source/controller declaration",
                    profile.support_id
                )));
            }
            for required in [
                "native-authority",
                "calibration",
                "startup-effects",
                "handover",
                "completion-evidence",
            ] {
                if !profile
                    .commissioning_inputs
                    .iter()
                    .any(|input| input.as_str() == required)
                {
                    return Err(Error(format!(
                        "{} lacks commissioning input {required}",
                        profile.support_id
                    )));
                }
            }
            let mut seen = BTreeSet::new();
            for file in &profile.sources {
                let pin = sources
                    .get(&file.repository)
                    .ok_or_else(|| Error("unknown source repository".into()))?;
                if pin.commit != file.commit || !seen.insert((&file.repository, &file.path)) {
                    return Err(Error("source revision mismatch or duplicate source".into()));
                }
            }
            if profile
                .fixture_sources
                .iter()
                .map(|f| &f.path)
                .collect::<BTreeSet<_>>()
                .len()
                != profile.fixture_sources.len()
            {
                return Err(Error("duplicate simulation fixture source".into()));
            }
            let mut controllers = BTreeSet::new();
            for controller in &profile.controllers {
                if controller.name.is_empty()
                    || controller.plugin.is_empty()
                    || !controllers.insert(&controller.name)
                    || controller.joint_order.iter().collect::<BTreeSet<_>>().len()
                        != controller.joint_order.len()
                {
                    return Err(Error("invalid controller declaration".into()));
                }
            }
        }
        Ok(())
    }
    pub fn profile(&self, id: &Name) -> Option<&SupportProfile> {
        self.profiles.iter().find(|p| &p.support_id == id)
    }
}
fn git_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Synthetic declarations bundled for software development, never equipment qualification.
pub fn builtin_catalog() -> Result<DeviceCatalog, Error> {
    let catalog =
        DeviceCatalog::decode(include_bytes!("../../../catalogs/device-support.v1.json"))?;
    let fixture_bytes = include_bytes!("../../../catalogs/fixtures/controllers.v1.json");
    let fixture: serde_json::Value =
        canonical::decode_json(fixture_bytes).map_err(|e| Error(e.to_string()))?;
    let digest = rx_package::content_digest(fixture_bytes);
    for profile in &catalog.profiles {
        for source in &profile.fixture_sources {
            if source.path.as_str() != "catalogs/fixtures/controllers.v1.json"
                || source.sha256 != digest
            {
                return Err(Error("bundled simulation fixture identity mismatch".into()));
            }
            let declared = fixture["profiles"]
                .as_array()
                .and_then(|profiles| {
                    profiles
                        .iter()
                        .find(|p| p["support_id"] == profile.support_id.as_str())
                })
                .ok_or_else(|| Error("simulation fixture profile missing".into()))?;
            let actual = serde_json::to_value(profile).map_err(|e| Error(e.to_string()))?;
            for key in ["model", "role", "declared_update_hz", "controllers"] {
                if actual[key] != declared[key] {
                    return Err(Error("simulation fixture declaration differs".into()));
                }
            }
        }
    }
    Ok(catalog)
}
