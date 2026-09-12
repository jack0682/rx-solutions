use crate::{Environment, HostError, Result};
use rx_domain::{
    canonical,
    intent::{Body, Intent, Kind},
    types::*,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Meaning supplied by a separately reviewed PLC publication program, not by MC itself.
/// The PLC must publish the entire 9-word image atomically to MC readers:
/// epoch:u64, publication_sequence:u64, flags:u16 (little-word-first).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusImage {
    pub first_d: u32,
    pub publication_contract: Digest,
    pub plc_program: Digest,
    pub source_max_age_ms: u32,
    pub read_budget_ms: u32,
    pub guard_validity_ms: u32,
    pub valid_bit: u8,
    pub ready_bit: u8,
    pub queue_empty_bit: u8,
    pub control_bit: u8,
    pub support_bit: u8,
    pub drop_allowed_bit: u8,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PredicateMapping {
    pub predicate: Name,
    pub command_m: u32,
    /// The PLC applies the declared debounce/settle rule before publishing this bit.
    pub completion_bit: u8,
    pub settle_ms: Counter,
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
    pub transport: rx_melsec_mc::Configuration,
    pub status: StatusImage,
    pub conditions: BTreeMap<Name, u8>,
    pub sources: BTreeMap<Name, u8>,
    pub predicates: Vec<PredicateMapping>,
}
impl Profile {
    pub fn validate(&self) -> Result<()> {
        self.transport
            .validate()
            .map_err(|e| HostError::Invalid(e.to_string()))?;
        let bits = [
            self.status.valid_bit,
            self.status.ready_bit,
            self.status.queue_empty_bit,
            self.status.control_bit,
            self.status.support_bit,
            self.status.drop_allowed_bit,
        ];
        let last = self.status.first_d.checked_add(8).ok_or(HostError::Guard)?;
        if self.schema.as_str() != "rx.melsec-predicate-profile.v1"
            || self.resources.is_empty()
            || self.resources.len() > 16
            || self.resources.iter().collect::<BTreeSet<_>>().len() != self.resources.len()
            || self.calibrations.iter().collect::<BTreeSet<_>>().len() != self.calibrations.len()
            || self.conditions.is_empty()
            || self.conditions.len() > 16
            || self.sources.len() > 16
            || self.predicates.is_empty()
            || self.predicates.len() > 10
            || bits.iter().any(|b| *b >= 16)
            || bits.into_iter().collect::<BTreeSet<_>>().len() != 6
            || self
                .conditions
                .values()
                .chain(self.sources.values())
                .any(|b| *b >= 16)
            || !(1..=50).contains(&self.status.source_max_age_ms)
            || !(1..=50).contains(&self.status.read_budget_ms)
            || !(1..=50).contains(&self.status.guard_validity_ms)
            || self.transport.exchange_timeout_ms > self.status.read_budget_ms
            || !self
                .transport
                .access
                .read_d
                .iter()
                .any(|r| r.first <= self.status.first_d && last <= r.last)
        {
            return Err(HostError::Invalid(
                "MELSEC profile shape/status bounds".into(),
            ));
        }
        let mut ids = BTreeSet::new();
        let mut writes = BTreeSet::new();
        let mut completion = BTreeSet::new();
        for p in &self.predicates {
            if p.completion_bit >= 16
                || bits.contains(&p.completion_bit)
                || !ids.insert(&p.predicate)
                || !writes.insert(p.command_m)
                || !completion.insert(p.completion_bit)
                || !self.transport.access.write_m.contains(&p.command_m)
                || p.settle_ms.0 > 60_000
            {
                return Err(HostError::Invalid("MELSEC predicate mapping".into()));
            }
        }
        if writes != self.transport.access.write_m {
            return Err(HostError::Invalid("unmapped native write address".into()));
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<Digest> {
        self.validate()?;
        canonical::digest("RX-MELSEC-PROFILE-v1", self)
            .map_err(|e| HostError::Invalid(e.to_string()))
    }
    pub fn environment(&self) -> Environment {
        match self.transport.environment {
            rx_melsec_mc::Environment::Simulation => Environment::Simulation,
            rx_melsec_mc::Environment::Physical => Environment::Physical,
        }
    }
    pub(crate) fn mapping(&self, intent: &Intent) -> Result<(&PredicateMapping, bool)> {
        let normalized = intent
            .normalized()
            .map_err(|e| HostError::Invalid(e.to_string()))?;
        let mut resources = self.resources.clone();
        resources.sort();
        let mut calibrations = self.calibrations.clone();
        calibrations.sort();
        let Body::Predicate(goal) = &intent.body else {
            return Err(HostError::Guard);
        };
        let TypedValue::Boolean(target) = goal.target else {
            return Err(HostError::Guard);
        };
        if intent.kind != Kind::EnsureState
            || intent.target != self.target
            || intent.profile_digest != self.digest()?
            || intent.site_config_digest != self.site_config
            || normalized.resource_set != resources
            || normalized.calibration_digests != calibrations
            || intent.completion_rule.as_str() != "rx.melsec.debounced-predicate.v1"
            || intent.cancel_rule.as_str() != "rx.melsec.no-native-cancel.v1"
        {
            return Err(HostError::Guard);
        }
        let mapping = self
            .predicates
            .iter()
            .find(|p| p.predicate == goal.predicate_id && p.settle_ms == goal.settle_ms)
            .ok_or(HostError::Guard)?;
        Ok((mapping, target))
    }
    pub(super) fn resources_match(&self, resources: &[Name]) -> bool {
        resources.len() == self.resources.len()
            && resources.iter().collect::<BTreeSet<_>>() == self.resources.iter().collect()
    }
}
