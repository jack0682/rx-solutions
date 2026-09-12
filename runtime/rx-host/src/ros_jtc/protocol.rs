use crate::{Clock, HostError, Result};
use rx_domain::{canonical, types::*};
use rx_solution_catalog::{ControlRole, InterfaceDeclaration};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub schema: Name,
    pub catalog_sha256: Digest,
    pub support_id: Name,
    pub controller: String,
    pub namespace: String,
    pub controller_manager: String,
    pub domain_id: u32,
    pub timeout_ms: u32,
    pub capacity: u32,
}
pub fn catalog_digest() -> Digest {
    rx_package::content_digest(include_bytes!(
        "../../../../catalogs/robotis-support.v1.json"
    ))
}
fn ros_name(s: &str) -> bool {
    if s == "/" {
        return true;
    }
    s.len() <= 256
        && s.starts_with('/')
        && s[1..].split('/').all(|p| {
            !p.is_empty()
                && (p.as_bytes()[0].is_ascii_alphabetic() || p.starts_with('_'))
                && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
}
impl Configuration {
    pub fn joints(&self) -> Result<Vec<String>> {
        if self.schema.as_str() != "rx.ros-jtc-bridge.v1"
            || self.catalog_sha256 != catalog_digest()
            || !ros_name(&self.namespace)
            || !ros_name(&self.controller_manager)
            || self.domain_id > 232
            || !(1..=1000).contains(&self.timeout_ms)
            || !(1..=512).contains(&self.capacity)
        {
            return Err(HostError::Invalid("ROS bridge configuration".into()));
        }
        let catalog = rx_solution_catalog::builtin_catalog()
            .map_err(|e| HostError::Invalid(e.to_string()))?;
        let p = catalog
            .profiles
            .iter()
            .find(|p| p.support_id == self.support_id)
            .ok_or(HostError::Guard)?;
        if !matches!(
            p.role,
            ControlRole::Manipulator | ControlRole::Follower | ControlRole::MobileBase
        ) {
            return Err(HostError::Guard);
        }
        let c = p
            .controllers
            .iter()
            .find(|c| {
                c.name == self.controller
                    && c.plugin == "joint_trajectory_controller/JointTrajectoryController"
            })
            .ok_or(HostError::Guard)?;
        if !matches!(&c.command_interfaces,InterfaceDeclaration::Names(v) if v==&["position"])
            || c.joint_order.is_empty()
            || c.joint_order.len() > 64
            || c.joint_order.iter().collect::<BTreeSet<_>>().len() != c.joint_order.len()
        {
            return Err(HostError::Guard);
        }
        Ok(c.joint_order.clone())
    }
    pub fn action(&self) -> String {
        format!(
            "{}/{}/follow_joint_trajectory",
            self.namespace.trim_end_matches('/'),
            self.controller
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub positions: Vec<f64>,
    pub velocities: Vec<f64>,
    pub accelerations: Vec<f64>,
    pub time_ns: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Tolerance {
    pub name: String,
    pub position: f64,
    pub velocity: f64,
    pub acceleration: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Goal {
    pub joints: Vec<String>,
    pub points: Vec<Point>,
    pub path_tolerance: Vec<Tolerance>,
    pub goal_tolerance: Vec<Tolerance>,
    pub goal_time_ns: Counter,
}
impl Goal {
    pub fn validate(&self, config: &Configuration) -> Result<()> {
        let joints = config.joints()?;
        let valid = |n: &f64| n.is_finite() && n.abs() <= 1e6;
        if self.joints != joints
            || self.points.is_empty()
            || self.points.len() > 1024
            || self.goal_time_ns.0 == 0
            || self.goal_time_ns.0 > 60_000_000_000
        {
            return Err(HostError::Guard);
        }
        let mut previous = 0;
        for p in &self.points {
            if p.positions.len() != joints.len()
                || (!p.velocities.is_empty() && p.velocities.len() != joints.len())
                || (!p.accelerations.is_empty() && p.accelerations.len() != joints.len())
                || !p
                    .positions
                    .iter()
                    .chain(&p.velocities)
                    .chain(&p.accelerations)
                    .all(valid)
                || p.time_ns.0 <= previous
                || p.time_ns.0 > 3_600_000_000_000
            {
                return Err(HostError::Guard);
            }
            previous = p.time_ns.0;
        }
        for values in [&self.path_tolerance, &self.goal_tolerance] {
            if values.len() != joints.len()
                || values.iter().zip(&joints).any(|(v, j)| {
                    v.name != *j
                        || [v.position, v.velocity, v.acceleration]
                            .iter()
                            .any(|n| !valid(n) || *n <= 0.)
                })
            {
                return Err(HostError::Guard);
            }
        }
        if canonical::bytes(self)
            .map_err(|e| HostError::Invalid(e.to_string()))?
            .len()
            > 524_288
        {
            return Err(HostError::Invalid(
                "trajectory artifact exceeds 512 KiB".into(),
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum State {
    Ready,
    Observed,
    SendRecorded,
    SendUnknown,
    ResultCaptured,
    ResultUnknown,
    CancelResponse,
    CancelRecorded,
    CancelUnknown,
    Rejected,
    RpcUnknown,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    pub schema: Name,
    pub bridge_instance: Id,
    pub sequence: Option<Counter>,
    pub clock_id: String,
    pub ticks_ns: Counter,
    pub state: State,
    pub value: serde_json::Value,
    pub fault: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ready {
    pub support_id: Name,
    pub model: Name,
    pub controller: String,
    pub action: String,
    pub joints: Vec<String>,
    pub source_observed_only: bool,
    pub catalog_sha256: Digest,
    pub controller_generation_known: bool,
}
impl Reply {
    pub fn validate(
        &self,
        instance: Option<&Id>,
        sequence: Option<Counter>,
        clock: &dyn Clock,
    ) -> Result<()> {
        let now = clock.now();
        if !clock.healthy()
            || self.schema.as_str() != "rx.ros-jtc-reply.v1"
            || self.sequence != sequence
            || self.clock_id != now.clock_id
            || self.ticks_ns > now.ticks_ns
            || instance.is_some_and(|i| i != &self.bridge_instance)
            || self.fault.as_ref().is_some_and(|s| s.len() > 8192)
        {
            return Err(HostError::NativeUnknown(
                "bridge reply context/clock differs".into(),
            ));
        }
        Ok(())
    }
    pub fn ready(&self, config: &Configuration, clock: &dyn Clock) -> Result<Ready> {
        self.validate(None, None, clock)?;
        let value: Ready = canonical::decode_json(
            &canonical::bytes(&self.value).map_err(|e| HostError::Invalid(e.to_string()))?,
        )
        .map_err(|e| HostError::Invalid(e.to_string()))?;
        if !matches!(self.state, State::Ready)
            || self.fault.is_some()
            || value.support_id != config.support_id
            || value.controller != config.controller
            || value.action != config.action()
            || value.joints != config.joints()?
            || value.catalog_sha256 != config.catalog_sha256
            || !value.source_observed_only
            || value.controller_generation_known
        {
            return Err(HostError::Guard);
        }
        Ok(value)
    }
}
pub trait Transport: Send {
    fn instance(&self) -> &Id;
    fn exchange(
        &mut self,
        command: &str,
        body: serde_json::Value,
        until: &TimePoint,
    ) -> Result<Reply>;
    /// Initiate passive IPC close and retain/reap the child; never kill or cancel a robot action.
    fn try_close(&mut self) -> Result<bool>;
}
