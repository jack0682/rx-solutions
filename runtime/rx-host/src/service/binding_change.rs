//! Passive comparison of a P proposal with two pinned startup configurations.
use super::{
    AdapterFactory, Builtin, Result,
    config::{Backend, Loaded},
};
use rx_domain::{canonical, types::*};
use rx_process_contract::host_binding_plan::Plan;
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Serialize)]
pub struct Inspection {
    pub schema: Name,
    pub plan_digest: Digest,
    pub host: Name,
    pub current_installation_identity: Digest,
    pub proposed_installation_identity: Digest,
    pub proposed_bindings_digest: Digest,
    pub software_matches: bool,
    pub issues: Vec<String>,
    pub runtime_provider_available: bool,
    pub installation_changed: bool,
    pub activation_authorized: bool,
    pub native_processes_started: Counter,
}
pub fn inspect(plan: &Plan, current: &Loaded, proposed: &Loaded) -> Result<Inspection> {
    plan.validate()?;
    let host = &current.config.host;
    if plan.installation != current.config.installation
        || proposed.config.installation != plan.installation
        || &proposed.config.host != host
    {
        return Err("Host/installation differs from binding plan".into());
    }
    let target = plan
        .hosts
        .get(host)
        .ok_or("Host not included in binding plan")?;
    let mut issues = vec![];
    // A binding inspection cannot smuggle a change of storage, identity, TLS, release or network settings.
    let mut permitted = current.config.clone();
    permitted.bindings = proposed.config.bindings.clone();
    permitted.backend = proposed.config.backend.clone();
    if canonical::bytes(&permitted)? != canonical::bytes(&proposed.config)? {
        issues.push("Startup changes extend beyond bindings/backend; a separate deployment plan is required".into());
    }
    let required_cells: BTreeSet<_> = target
        .other_affected_cells
        .iter()
        .chain(std::iter::once(&plan.cell))
        .collect();
    if current
        .bindings
        .iter()
        .map(|b| &b.cell)
        .collect::<BTreeSet<_>>()
        != required_cells
        || proposed
            .bindings
            .iter()
            .map(|b| &b.cell)
            .collect::<BTreeSet<_>>()
            != required_cells
    {
        issues.push("The complete affected Host cell cohort must be present".into());
    }
    for next in &proposed.bindings {
        let Some(old) = current.bindings.iter().find(|b| b.cell == next.cell) else {
            issues.push("Adding a Host cell requires a separate ownership/cohort plan".into());
            continue;
        };
        if next.cell != plan.cell {
            if canonical::bytes(next)? != canonical::bytes(old)? {
                issues.push(
                    "An additional affected cell changes without its own approved target".into(),
                );
            }
            continue;
        }
        let mut expected = old.clone();
        expected.definition = plan.definition.clone();
        expected.envelope = plan.envelope.clone();
        expected.allowed_intents = target.required_intents.clone();
        expected.scope_ids = plan.scopes.iter().cloned().collect();
        expected.condition_ids = target.required_conditions.iter().cloned().collect();
        // Normalization permits order differences without changing qualification/purpose/peer identity.
        let normalize = |mut binding: crate::Binding| -> Result<Vec<u8>> {
            binding.allowed_intents = binding
                .allowed_intents
                .iter()
                .map(|i| i.normalized())
                .collect::<std::result::Result<_, _>>()?;
            binding
                .allowed_intents
                .sort_by_cached_key(|i| i.digest().expect("normalized"));
            binding.scope_ids.sort();
            binding.condition_ids.sort();
            Ok(canonical::bytes(&binding)?)
        };
        if normalize(next.clone())? != normalize(expected)?
            || plan.environment.as_str()
                != match next.environment {
                    crate::Environment::Simulation => "SIMULATION",
                    crate::Environment::Physical => "PHYSICAL",
                }
        {
            issues.push("Proposed binding differs from approved intents/conditions/envelope or changes identity/purpose/qualification".into());
        }
    }
    <Builtin as AdapterFactory<crate::service_clock::SystemClock>>::validate(
        &Builtin,
        &proposed.config.backend,
        &proposed.bindings,
    )?;
    if !target.device_packages.is_empty() {
        if target.device_packages.len() != 1 {
            issues.push("Current release supports one device package per native Host".into());
        }
        match &proposed.config.backend {
            Backend::JtcPackage {
                directory,
                manifest_digest,
                ..
            }
            | Backend::MelsecPackage {
                directory,
                manifest_digest,
                ..
            } => {
                let signature = rx_package::directory::read_relative_file(
                    directory,
                    &rx_package::PackagePath::new("manifest.sig.json")?,
                    4096,
                )?;
                let raw = rx_package::directory::read_relative_file(
                    directory,
                    &rx_package::PackagePath::new("device-catalog.json")?,
                    131_072,
                )?;
                let declaration: rx_process_contract::device_catalog::Catalog =
                    canonical::decode_json(&raw)?;
                declaration.validate()?;
                let catalog = canonical::bytes(&declaration)?;
                if target.device_packages.iter().all(|p| {
                    p.manifest != *manifest_digest
                        || p.signature != rx_package::content_digest(&signature)
                        || p.catalog.sha256 != rx_package::content_digest(&catalog)
                        || p.catalog.size_bytes.0 != catalog.len() as u64
                        || p.catalog.schema_id != declaration.schema
                }) {
                    issues.push(
                        "Selected signed device package/catalog differs from the process proposal"
                            .into(),
                    );
                }
            }
            _ => issues
                .push("Approved device source requires a signed native package backend".into()),
        }
    } else if canonical::bytes(&current.config.backend)?
        != canonical::bytes(&proposed.config.backend)?
    {
        issues.push("Backend changed without a device source in the approved plan".into());
    }
    Ok(Inspection {
        schema: Name::new("rx.host-binding-inspection.v1")?,
        plan_digest: plan.digest()?,
        host: host.clone(),
        current_installation_identity: current.identity,
        proposed_installation_identity: proposed.identity,
        proposed_bindings_digest: proposed.config.bindings.sha256,
        software_matches: issues.is_empty(),
        issues,
        runtime_provider_available: match &proposed.config.backend {
            Backend::JtcPackage { .. } => false,
            Backend::ValidatedDriver { .. } => {
                crate::dynamixel::profile::validate(&proposed.config.backend, &proposed.bindings)
                    .is_ok()
                    && crate::dynamixel::profile::executable_pin(std::path::Path::new("/opt/rx"))
                        .is_ok()
            }
            _ => true,
        },
        installation_changed: false,
        activation_authorized: false,
        native_processes_started: Counter(0),
    })
}
