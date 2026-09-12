use super::protocol::{Configuration, Goal};
use crate::{Clock, Environment, HostError, ProtectionIncident, Result, native::LocalProtection};
use rx_domain::{
    canonical,
    intent::{Body, Intent, Kind},
    types::*,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::Arc};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrajectoryAsset {
    pub reference: ArtifactRef,
    pub joint_group: Name,
    pub tool: Digest,
    pub goal: Goal,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub schema: Name,
    pub installation: Id,
    pub cell: Name,
    pub target: Name,
    pub site_config: Digest,
    pub calibrations: Vec<Digest>,
    pub resources: Vec<Name>,
    pub environment: Environment,
    pub bridge: Configuration,
    pub conditions: BTreeSet<Name>,
    pub trajectories: Vec<TrajectoryAsset>,
    pub authority_max_age_ms: Counter,
}
impl Profile {
    pub fn validate(&self) -> Result<()> {
        self.bridge.joints()?;
        if self.schema.as_str() != "rx.robotis-jtc-profile.v1"
            || self.conditions.is_empty()
            || self.conditions.len() > 32
            || self.resources.is_empty()
            || self.resources.len() > 32
            || self.resources.iter().collect::<BTreeSet<_>>().len() != self.resources.len()
            || self.calibrations.iter().collect::<BTreeSet<_>>().len() != self.calibrations.len()
            || self.trajectories.is_empty()
            || self.trajectories.len() > 16
            || self.authority_max_age_ms.0 == 0
            || self.authority_max_age_ms.0 > 100
        {
            return Err(HostError::Invalid("JTC profile shape".into()));
        }
        let mut ids = BTreeSet::new();
        for trajectory in &self.trajectories {
            trajectory.goal.validate(&self.bridge)?;
            let bytes = canonical::bytes(&trajectory.goal)
                .map_err(|e| HostError::Invalid(e.to_string()))?;
            if trajectory.reference.schema_id.as_str() != "rx.ros-jtc.goal.v1"
                || trajectory.reference.sha256 != rx_package::content_digest(&bytes)
                || trajectory.reference.size_bytes.0 != bytes.len() as u64
                || !ids.insert(trajectory.reference.sha256)
            {
                return Err(HostError::Invalid("trajectory artifact identity".into()));
            }
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<Digest> {
        self.validate()?;
        canonical::digest("RX-ROBOTIS-JTC-PROFILE-v1", self)
            .map_err(|e| HostError::Invalid(e.to_string()))
    }
    pub(crate) fn trajectory(&self, intent: &Intent) -> Result<&TrajectoryAsset> {
        let normalized = intent
            .normalized()
            .map_err(|e| HostError::Invalid(e.to_string()))?;
        let mut resources = self.resources.clone();
        resources.sort();
        let mut calibration = self.calibrations.clone();
        calibration.sort();
        let Body::Trajectory(goal) = &intent.body else {
            return Err(HostError::Guard);
        };
        if intent.kind != Kind::FiniteAction
            || intent.target != self.target
            || intent.profile_digest != self.digest()?
            || intent.site_config_digest != self.site_config
            || normalized.resource_set != resources
            || normalized.calibration_digests != calibration
            || intent.completion_rule.as_str() != "rx.ros-jtc.terminal-result.v1"
            || intent.cancel_rule.as_str() != "rx.ros-jtc.exact-cancel.v1"
        {
            return Err(HostError::Guard);
        }
        self.trajectories
            .iter()
            .find(|t| {
                t.reference == goal.trajectory
                    && t.joint_group == goal.joint_group
                    && t.tool == goal.tool_digest
            })
            .ok_or(HostError::Guard)
    }
    pub(super) fn resources_match(&self, resources: &[Name]) -> bool {
        resources.len() == self.resources.len()
            && resources.iter().collect::<BTreeSet<_>>() == self.resources.iter().collect()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoritySnapshot {
    pub controller_session: Id,
    pub observed_at: TimePoint,
    pub uncertainty_ns: Counter,
    pub resources: Vec<Name>,
    pub conditions: BTreeSet<Name>,
    pub exclusive_control: bool,
    pub no_external_goals: bool,
    pub control_available: bool,
    pub support_stable: bool,
    pub client_drop_allowed: bool,
}
impl AuthoritySnapshot {
    pub(super) fn validate(&self, profile: &Profile, clock: &dyn Clock) -> Result<()> {
        let now = clock.now();
        if !clock.healthy()
            || !profile.resources_match(&self.resources)
            || now.age_ns(&self.observed_at).is_none_or(|age| {
                age.saturating_add(self.uncertainty_ns.0)
                    > profile.authority_max_age_ms.0 * 1_000_000
            })
        {
            return Err(HostError::Guard);
        }
        Ok(())
    }
}
/// A release-owned source for actual controller generation/ownership and local support.
/// Neither bridge READY nor a site JSON bool can implement this proof.
pub trait Authority: Send + Sync {
    fn snapshot(&self) -> Result<AuthoritySnapshot>;
    fn protection(&self) -> Arc<dyn LocalProtection>;
}
pub struct UnavailableAuthority;
impl LocalProtection for UnavailableAuthority {
    fn react(&self, _: ProtectionIncident) {}
}
impl Authority for UnavailableAuthority {
    fn snapshot(&self) -> Result<AuthoritySnapshot> {
        Err(HostError::Guard)
    }
    fn protection(&self) -> Arc<dyn LocalProtection> {
        Arc::new(UnavailableAuthority)
    }
}
