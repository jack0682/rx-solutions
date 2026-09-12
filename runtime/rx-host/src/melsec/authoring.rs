//! Typed reusable device semantics plus exact installation bindings; never connects to equipment.
use super::{PredicateMapping, Profile, StatusImage};
use crate::{HostError, Result, service::device_package::Family};
use rx_domain::{canonical, types::*};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddrV4,
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusSemantics {
    pub publication_contract: ArtifactRef,
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
pub struct PredicateSlot {
    pub predicate: Name,
    pub command_slot: Name,
    pub completion_bit: u8,
    pub settle_ms: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    pub schema: Name,
    pub id: Name,
    pub revision: Counter,
    pub controller_model: Name,
    pub resource_roles: Vec<Name>,
    pub status: StatusSemantics,
    pub conditions: BTreeMap<Name, u8>,
    pub sources: BTreeMap<Name, u8>,
    pub predicates: Vec<PredicateSlot>,
}
impl Template {
    pub fn normalized(&self) -> Result<Self> {
        let status = &self.status;
        let bits = [
            status.valid_bit,
            status.ready_bit,
            status.queue_empty_bit,
            status.control_bit,
            status.support_bit,
            status.drop_allowed_bit,
        ];
        if bits.iter().any(|b| *b >= 16)
            || bits.into_iter().collect::<BTreeSet<_>>().len() != 6
            || !(1..=50).contains(&status.source_max_age_ms)
            || !(1..=50).contains(&status.read_budget_ms)
            || !(1..=50).contains(&status.guard_validity_ms)
            || self.conditions.is_empty()
            || self.conditions.len() > 16
            || self.sources.len() > 16
            || self
                .conditions
                .values()
                .chain(self.sources.values())
                .any(|b| *b >= 16)
            || self.predicates.iter().any(|p| {
                p.completion_bit >= 16 || bits.contains(&p.completion_bit) || p.settle_ms.0 > 60_000
            })
            || self
                .predicates
                .iter()
                .map(|p| p.completion_bit)
                .collect::<BTreeSet<_>>()
                .len()
                != self.predicates.len()
        {
            return Err(HostError::Invalid(
                "device template observation semantics".into(),
            ));
        }
        if self.schema.as_str() != "rx.melsec-template.v1"
            || self.revision.0 == 0
            || self.controller_model.as_str() != "Q03UDVCPU"
            || self.resource_roles.is_empty()
            || self.resource_roles.len() > 16
            || self.resource_roles.iter().collect::<BTreeSet<_>>().len()
                != self.resource_roles.len()
            || self.predicates.is_empty()
            || self.predicates.len() > 10
            || self
                .predicates
                .iter()
                .map(|p| &p.predicate)
                .collect::<BTreeSet<_>>()
                .len()
                != self.predicates.len()
            || self
                .predicates
                .iter()
                .map(|p| &p.command_slot)
                .collect::<BTreeSet<_>>()
                .len()
                != self.predicates.len()
            || self.status.publication_contract.schema_id.as_str()
                != "rx.melsec.publication-contract.v1"
            || !(1..=1_048_576).contains(&self.status.publication_contract.size_bytes.0)
        {
            return Err(HostError::Invalid("device template shape/identity".into()));
        }
        let mut v = self.clone();
        v.resource_roles.sort();
        v.predicates.sort_by(|a, b| a.predicate.cmp(&b.predicate));
        Ok(v)
    }
    pub fn digest(&self) -> Result<Digest> {
        canonical::digest("RX-MELSEC-TEMPLATE-v1", &self.normalized()?)
            .map_err(|e| HostError::Invalid(e.to_string()))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub environment: rx_melsec_mc::Environment,
    pub endpoint: SocketAddrV4,
    pub route: rx_melsec_mc::Route,
    pub monitoring_timer: u16,
    pub connect_timeout_ms: u32,
    pub exchange_timeout_ms: u32,
    pub m_last: u32,
    pub d_last: u32,
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
    pub calibrations: Vec<Digest>,
    pub resources: BTreeMap<Name, Name>,
    pub connection: Connection,
    pub status_first_d: u32,
    pub plc_program: ArtifactRef,
    pub command_addresses: BTreeMap<Name, u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assembly {
    pub schema: Name,
    pub template: Template,
    pub site: Site,
}
impl Assembly {
    pub fn resolve(&self) -> Result<(Family, Profile)> {
        let t = self.template.normalized()?;
        let s = &self.site;
        if self.schema.as_str() != "rx.melsec-assembly.v1"
            || s.schema.as_str() != "rx.melsec-site-binding.v1"
            || s.template_digest != t.digest()?
            || s.resources.keys().collect::<BTreeSet<_>>() != t.resource_roles.iter().collect()
            || s.command_addresses.keys().collect::<BTreeSet<_>>()
                != t.predicates.iter().map(|p| &p.command_slot).collect()
            || s.plc_program.schema_id.as_str() != "rx.plc.program-evidence.v1"
            || !(1..=1_048_576).contains(&s.plc_program.size_bytes.0)
        {
            return Err(HostError::Invalid(
                "site binding/template scope differs".into(),
            ));
        }
        let c = &s.connection;
        let status = &t.status;
        let mut profile = Profile {
            schema: Name::new("rx.melsec-predicate-profile.v1").expect("literal"),
            installation: s.installation.clone(),
            cell: s.cell.clone(),
            target: s.target.clone(),
            site_config: s.site_config,
            calibrations: s.calibrations.clone(),
            resources: s.resources.values().cloned().collect(),
            transport: rx_melsec_mc::Configuration {
                environment: c.environment,
                endpoint: c.endpoint,
                route: c.route,
                monitoring_timer: c.monitoring_timer,
                connect_timeout_ms: c.connect_timeout_ms,
                exchange_timeout_ms: c.exchange_timeout_ms,
                access: rx_melsec_mc::AccessProfile {
                    m_last: c.m_last,
                    d_last: c.d_last,
                    read_m: vec![],
                    read_d: vec![rx_melsec_mc::AddressRange {
                        first: s.status_first_d,
                        last: s.status_first_d.checked_add(8).ok_or(HostError::Guard)?,
                    }],
                    write_m: s.command_addresses.values().copied().collect(),
                },
            },
            status: StatusImage {
                first_d: s.status_first_d,
                publication_contract: status.publication_contract.sha256,
                plc_program: s.plc_program.sha256,
                source_max_age_ms: status.source_max_age_ms,
                read_budget_ms: status.read_budget_ms,
                guard_validity_ms: status.guard_validity_ms,
                valid_bit: status.valid_bit,
                ready_bit: status.ready_bit,
                queue_empty_bit: status.queue_empty_bit,
                control_bit: status.control_bit,
                support_bit: status.support_bit,
                drop_allowed_bit: status.drop_allowed_bit,
            },
            conditions: t.conditions,
            sources: t.sources,
            predicates: t
                .predicates
                .into_iter()
                .map(|p| PredicateMapping {
                    predicate: p.predicate,
                    command_m: s.command_addresses[&p.command_slot],
                    completion_bit: p.completion_bit,
                    settle_ms: p.settle_ms,
                })
                .collect(),
        };
        profile.resources.sort();
        profile.calibrations.sort();
        profile.validate()?;
        let family = Family {
            schema: Name::new("rx.melsec-family.v1").expect("literal"),
            family: Name::new("mitsubishi/melsec-q").expect("literal"),
            controller_model: t.controller_model,
            environment: profile.environment(),
        };
        Ok((family, profile))
    }
    pub fn normalized(&self) -> Result<Self> {
        self.resolve()?;
        let mut a = self.clone();
        a.template = a.template.normalized()?;
        a.site.calibrations.sort();
        Ok(a)
    }
    pub fn required_assets(&self) -> Vec<ArtifactRef> {
        vec![
            self.template.status.publication_contract.clone(),
            self.site.plc_program.clone(),
        ]
    }
}
