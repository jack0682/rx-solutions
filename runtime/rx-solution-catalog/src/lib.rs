//! First-party support declarations. Source observation is never equipment qualification.
use rx_domain::{canonical, types::*};
use rx_package::PackagePath;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const REQUIRED_REPOSITORIES: [&str; 5] = [
    "DynamixelSDK",
    "dynamixel_hardware_interface",
    "open_manipulator",
    "ai_worker",
    "ai_sapiens",
];
pub const REQUIRED_SUPPORT: [&str; 22] = [
    "OM-01",
    "OM-02",
    "OM-03",
    "OM-04",
    "OM-05",
    "OM-06",
    "OM-07",
    "OM-08",
    "OM-09",
    "OM-10",
    "FFW-01",
    "FFW-02-REV2",
    "FFW-02-REV3",
    "FFW-02-REV4",
    "FFW-03",
    "FFW-04",
    "FFW-05",
    "FFW-06",
    "FFW-07",
    "FFW-08",
    "AS-01",
    "AS-02",
];
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
pub struct SupportProfile {
    pub support_id: Name,
    pub model: Name,
    pub role: ControlRole,
    pub declared_update_hz: Counter,
    pub sources: Vec<SourceFile>,
    pub controllers: Vec<ControllerDeclaration>,
    pub evidence_level: EvidenceLevel,
    pub commissioning_inputs: Vec<Name>,
    pub notes: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnPlatformCatalog {
    pub schema: Name,
    pub repositories: Vec<RepositoryPin>,
    pub profiles: Vec<SupportProfile>,
}
#[derive(Debug, thiserror::Error)]
#[error("invalid first-party support catalog: {0}")]
pub struct Error(pub String);
impl OwnPlatformCatalog {
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let catalog: Self = canonical::decode_json(bytes).map_err(|e| Error(e.to_string()))?;
        catalog.validate()?;
        Ok(catalog)
    }
    pub fn validate(&self) -> Result<(), Error> {
        if self.schema.as_str() != "rx.robotis-support.v1" {
            return Err(Error("schema".into()));
        }
        let sources: BTreeMap<_, _> = self
            .repositories
            .iter()
            .map(|r| (&r.repository, r))
            .collect();
        if sources.len() != 5
            || self.repositories.len() != 5
            || REQUIRED_REPOSITORIES
                .iter()
                .any(|name| !sources.keys().any(|id| id.as_str() == *name))
        {
            return Err(Error(
                "all five mandatory repositories must be present exactly once".into(),
            ));
        }
        for source in sources.values() {
            if !git_commit(&source.commit)
                || source.url != format!("https://github.com/ROBOTIS-GIT/{}.git", source.repository)
            {
                return Err(Error("source pin".into()));
            }
        }
        let profiles: BTreeSet<_> = self
            .profiles
            .iter()
            .map(|p| p.support_id.as_str())
            .collect();
        if profiles.len() != 22
            || self.profiles.len() != 22
            || REQUIRED_SUPPORT.iter().any(|id| !profiles.contains(id))
        {
            return Err(Error(
                "mandatory support variants missing, duplicated or combined".into(),
            ));
        }
        for profile in &self.profiles {
            if profile.sources.is_empty()
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

/// Data bundled by this repository; inclusion is mandatory, hardware support remains unqualified.
pub fn builtin_catalog() -> Result<OwnPlatformCatalog, Error> {
    OwnPlatformCatalog::decode(include_bytes!("../../../catalogs/robotis-support.v1.json"))
}
