//! Reusable controller semantics and installation-specific motion assets; no device access.
use super::{
    Profile, TrajectoryAsset,
    protocol::{Configuration, Goal, catalog_digest},
};
use crate::{Environment, HostError, Result};
use rx_domain::{
    canonical,
    intent::{Body, Intent, Kind, TrajectoryGoal},
    types::*,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
fn n(v: &str) -> Name {
    Name::new(v).expect("fixed name")
}
fn invalid() -> HostError {
    HostError::Invalid("JTC template/site binding differs".into())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionSlot {
    pub id: Name,
    pub joint_group: Name,
    pub tool_role: Name,
    pub execution_timeout_ms: Counter,
    pub prepare_validity_ms: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    pub schema: Name,
    pub id: Name,
    pub revision: Counter,
    pub catalog_sha256: Digest,
    pub support_id: Name,
    pub controller: String,
    pub resource_roles: BTreeSet<Name>,
    pub condition_roles: BTreeSet<Name>,
    pub actions: Vec<ActionSlot>,
    pub authority_max_age_ms: Counter,
}
impl Template {
    pub fn normalized(&self) -> Result<Self> {
        if self.schema.as_str() != "rx.ros-jtc-template.v1"
            || self.revision.0 == 0
            || self.catalog_sha256 != catalog_digest()
            || self.resource_roles.is_empty()
            || self.resource_roles.len() > 32
            || self.condition_roles.is_empty()
            || self.condition_roles.len() > 32
            || self.actions.is_empty()
            || self.actions.len() > 16
            || self
                .actions
                .iter()
                .map(|a| &a.id)
                .collect::<BTreeSet<_>>()
                .len()
                != self.actions.len()
            || !(1..=100).contains(&self.authority_max_age_ms.0)
            || self.actions.iter().any(|a| {
                a.execution_timeout_ms.0 == 0
                    || a.execution_timeout_ms.0 > 3_660_000
                    || a.prepare_validity_ms.0 == 0
                    || a.prepare_validity_ms.0 > 60_000
            })
        {
            return Err(invalid());
        }
        // Validate the selected declaration without discovery or controller startup.
        self.bridge("/".into(), "/controller_manager".into(), 0, 100, 32)
            .joints()?;
        let mut t = self.clone();
        t.actions.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(t)
    }
    pub fn digest(&self) -> Result<Digest> {
        canonical::digest("RX-ROS-JTC-TEMPLATE-v1", &self.normalized()?).map_err(|_| invalid())
    }
    fn bridge(
        &self,
        namespace: String,
        manager: String,
        domain: u32,
        timeout: u32,
        capacity: u32,
    ) -> Configuration {
        Configuration {
            schema: n("rx.ros-jtc-bridge.v1"),
            catalog_sha256: self.catalog_sha256,
            support_id: self.support_id.clone(),
            controller: self.controller.clone(),
            namespace,
            controller_manager: manager,
            domain_id: domain,
            timeout_ms: timeout,
            capacity,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Site {
    pub schema: Name,
    pub template_digest: Digest,
    pub installation: Id,
    pub cell: Name,
    pub target: Name,
    pub site_config: Digest,
    pub environment: Environment,
    pub calibrations: Vec<ArtifactRef>,
    pub tools: BTreeMap<Name, ArtifactRef>,
    pub resources: BTreeMap<Name, Name>,
    pub conditions: BTreeMap<Name, Name>,
    pub namespace: String,
    pub controller_manager: String,
    pub domain_id: u32,
    pub timeout_ms: u32,
    pub capacity: u32,
    pub goals: BTreeMap<Name, Goal>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assembly {
    pub schema: Name,
    pub template: Template,
    pub site: Site,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Family {
    pub schema: Name,
    pub family: Name,
    pub model: Name,
    pub support_id: Name,
    pub environment: Environment,
}
pub struct Resolved {
    pub family: Family,
    pub profile: Profile,
    pub operations: BTreeMap<Name, Intent>,
}
fn artifact_valid(a: &ArtifactRef, schema: &str) -> bool {
    a.schema_id.as_str() == schema && (1..=1_048_576).contains(&a.size_bytes.0)
}
impl Assembly {
    pub fn resolve(&self) -> Result<Resolved> {
        let t = self.template.normalized()?;
        let s = &self.site;
        if self.schema.as_str() != "rx.ros-jtc-assembly.v1"
            || s.schema.as_str() != "rx.ros-jtc-site.v1"
            || s.template_digest != t.digest()?
            || s.resources.keys().cloned().collect::<BTreeSet<_>>() != t.resource_roles
            || s.conditions.keys().cloned().collect::<BTreeSet<_>>() != t.condition_roles
            || s.resources.values().collect::<BTreeSet<_>>().len() != s.resources.len()
            || s.conditions.values().collect::<BTreeSet<_>>().len() != s.conditions.len()
            || s.goals.keys().collect::<BTreeSet<_>>() != t.actions.iter().map(|a| &a.id).collect()
            || s.tools.keys().collect::<BTreeSet<_>>()
                != t.actions.iter().map(|a| &a.tool_role).collect()
            || s.calibrations.is_empty()
            || s.calibrations.len() > 16
            || s.calibrations
                .iter()
                .any(|a| !artifact_valid(a, "rx.robot-calibration.v1"))
            || s.tools
                .values()
                .any(|a| !artifact_valid(a, "rx.tool-definition.v1"))
        {
            return Err(invalid());
        }
        let mut refs = BTreeMap::new();
        for a in s.calibrations.iter().chain(s.tools.values()) {
            if refs.insert(a.sha256, a).is_some_and(|old| old != a) {
                return Err(invalid());
            }
        }
        if refs.len() > 32 {
            return Err(invalid());
        }
        let bridge = t.bridge(
            s.namespace.clone(),
            s.controller_manager.clone(),
            s.domain_id,
            s.timeout_ms,
            s.capacity,
        );
        let mut trajectories = vec![];
        for action in &t.actions {
            let goal = &s.goals[&action.id];
            goal.validate(&bridge)?;
            let duration = goal
                .points
                .last()
                .ok_or_else(invalid)?
                .time_ns
                .0
                .checked_add(goal.goal_time_ns.0)
                .ok_or_else(invalid)?;
            if duration > action.execution_timeout_ms.0 * 1_000_000 {
                return Err(invalid());
            }
            let bytes = canonical::bytes(goal).map_err(|_| invalid())?;
            trajectories.push(TrajectoryAsset {
                reference: ArtifactRef {
                    sha256: rx_package::content_digest(&bytes),
                    schema_id: n("rx.ros-jtc.goal.v1"),
                    size_bytes: Counter(bytes.len() as u64),
                },
                joint_group: action.joint_group.clone(),
                tool: s.tools[&action.tool_role].sha256,
                goal: goal.clone(),
            });
        }
        let mut profile = Profile {
            schema: n("rx.ros-jtc-profile.v1"),
            installation: s.installation.clone(),
            cell: s.cell.clone(),
            target: s.target.clone(),
            site_config: s.site_config,
            calibrations: s.calibrations.iter().map(|a| a.sha256).collect(),
            resources: s.resources.values().cloned().collect(),
            environment: s.environment,
            bridge,
            conditions: s.conditions.values().cloned().collect(),
            trajectories,
            authority_max_age_ms: t.authority_max_age_ms,
        };
        profile.calibrations.sort();
        profile.resources.sort();
        profile.validate()?;
        let digest = profile.digest()?;
        let operations = t
            .actions
            .iter()
            .zip(&profile.trajectories)
            .map(|(a, g)| {
                let intent = Intent {
                    kind: Kind::FiniteAction,
                    target: s.target.clone(),
                    profile_digest: digest,
                    site_config_digest: s.site_config,
                    calibration_digests: profile.calibrations.clone(),
                    resource_set: profile.resources.clone(),
                    execution_timeout_ms: a.execution_timeout_ms,
                    prepare_validity_ms: a.prepare_validity_ms,
                    completion_rule: n("rx.ros-jtc.terminal-result.v1"),
                    cancel_rule: n("rx.ros-jtc.exact-cancel.v1"),
                    body: Body::Trajectory(TrajectoryGoal {
                        trajectory: g.reference.clone(),
                        joint_group: g.joint_group.clone(),
                        tool_digest: g.tool,
                    }),
                };
                (a.id.clone(), intent)
            })
            .collect::<BTreeMap<_, _>>();
        for intent in operations.values() {
            intent.normalized().map_err(|_| invalid())?;
            profile.trajectory(intent)?;
        }
        let catalog = rx_solution_catalog::builtin_catalog().map_err(|_| invalid())?;
        let model = catalog
            .profiles
            .iter()
            .find(|p| p.support_id == t.support_id)
            .ok_or_else(invalid)?
            .model
            .clone();
        Ok(Resolved {
            family: Family {
                schema: n("rx.ros-jtc-family.v1"),
                family: n("ros/jtc"),
                model,
                support_id: t.support_id,
                environment: s.environment,
            },
            profile,
            operations,
        })
    }
    pub fn normalized(&self) -> Result<Self> {
        self.resolve()?;
        let mut a = self.clone();
        a.template = a.template.normalized()?;
        a.site.calibrations.sort_by_key(|v| v.sha256);
        Ok(a)
    }
    pub fn required_assets(&self) -> Vec<ArtifactRef> {
        let assets: BTreeMap<_, _> = self
            .site
            .calibrations
            .iter()
            .chain(self.site.tools.values())
            .map(|a| (a.sha256, a.clone()))
            .collect();
        assets.into_values().collect()
    }
}
