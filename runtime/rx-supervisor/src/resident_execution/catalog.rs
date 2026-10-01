use super::*;
use crate::{
    builtin,
    model::{Plan, Process, Program},
    registered::catalog_reference,
    registration::Registry,
};
use rx_domain::component::RegistrationState;
use rx_ports::Repository;
use rx_solution_catalog::DeviceCatalog;
use rx_storage::SqliteRepository;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
/// Actual verified release/catalog checkpoint. Site JSON cannot construct this value.
pub struct Catalog {
    pub(crate) release: Digest,
    pub(crate) programs: BTreeMap<Name, Program>,
    pub(crate) support: DeviceCatalog,
}
impl Catalog {
    pub fn load<R: Repository>(root: &Path, release_store: &mut R) -> Result<Self> {
        builtin::preflight_source_assets(root)?;
        let release_bytes = builtin::release_metadata(root, "release.json")?;
        let revocations = builtin::release_metadata(root, "revocations.json")?;
        rx_package::release::update_revocations(release_store, &revocations)?;
        let verified = builtin::verify_release(root, &release_bytes, &revocations)?;
        let programs = builtin::programs_from_release(root, &verified)?;
        let support = DeviceCatalog::decode(&std::fs::read(
            root.join("catalogs/device-support.v1.json"),
        )?)
        .map_err(invalid)?;
        rx_package::release::admit(release_store, &verified)?;
        Ok(Self {
            release: verified.digest(),
            programs,
            support,
        })
    }
    /// Authoring data only; this serialization cannot be deserialized into a verified catalog.
    pub fn enrollment(&self, registry: Digest) -> Result<data::Enrollment> {
        let programs = self
            .programs
            .iter()
            .map(|(id, p)| {
                Ok((
                    id.clone(),
                    data::CatalogPolicy {
                        digest: catalog_reference(p)?.digest,
                        effect: effect(p.effect),
                    },
                ))
            })
            .collect::<Result<_>>()?;
        Ok(data::Enrollment {
            registry,
            releases: [self.release].into(),
            programs,
        })
    }
}
pub(super) fn effect(value: crate::model::Effect) -> data::Effect {
    match value {
        crate::model::Effect::NonActuating => data::Effect::NonActuating,
        crate::model::Effect::ProtocolGuardedService => data::Effect::ProtocolGuardedService,
        crate::model::Effect::RequiresPlatformAuthority => data::Effect::RequiresPlatformAuthority,
    }
}
/// Captures the actual catalog and plan matched to a current authenticated P assignment.
pub struct Prepared {
    pub(crate) intent: data::Intent,
    pub(crate) offer: data::Preparation,
    pub(crate) plan: Plan,
    pub(crate) catalog: Catalog,
}
impl Prepared {
    pub fn intent(&self) -> &data::Intent {
        &self.intent
    }
    pub fn offer(&self) -> &data::Preparation {
        &self.offer
    }
    pub(super) fn build(
        intent: data::Intent,
        peer: &data::Peer,
        catalog: Catalog,
        registry: &mut Registry<SqliteRepository>,
    ) -> Result<Self> {
        let selections = intent
            .nodes
            .iter()
            .map(|(name, node)| (name.clone(), node.selection.clone()))
            .collect();
        data::Propose {
            supervisor: intent.supervisor.clone(),
            environment: intent.environment,
            profiles: intent.profiles.clone(),
            selections,
        }
        .validate()
        .map_err(invalid)?;
        if intent.supervisor != peer.principal || registry.platform_binding()? != peer.registry {
            return Err(invalid("assignment Supervisor/registry differs"));
        }
        let mut instances = BTreeSet::new();
        let mut programs = BTreeMap::new();
        let mut processes = Vec::new();
        for (selection, node) in &intent.nodes {
            if node.registration.id != node.selection.component
                || node.revision != node.selection.expected_revision
                || node.registration.state != RegistrationState::Accepted
                || !instances.insert(node.instance.clone())
                || node.instance == node.registration.id
                || node.instance == intent.run
            {
                return Err(invalid("assignment identity differs"));
            }
            let p = catalog
                .programs
                .get(&node.registration.declaration.catalog.program)
                .ok_or_else(|| invalid("program is absent from verified release catalog"))?;
            let reference = catalog_reference(p)?;
            if reference != node.registration.declaration.catalog
                || p.execution_requirements.is_none()
            {
                return Err(invalid(
                    "full catalog differs or execution requirements are undeclared",
                ));
            }
            programs.insert(
                selection.clone(),
                data::ProgramVerification {
                    catalog: reference,
                    effect: effect(p.effect),
                },
            );
            processes.push(Process {
                id: selection.clone(),
                program: p.id.clone(),
                parameters: node.selection.parameters.clone(),
                depends_on: node.selection.depends_on.clone(),
                startup_timeout_ms: node.selection.startup_timeout_ms,
                shutdown_timeout_ms: node.selection.shutdown_timeout_ms,
                restart_limit: Counter(0),
                restart_backoff_ms: Counter(100),
            });
        }
        let plan = Plan {
            schema: Name::new("rx.solutions-process-plan.v1").map_err(invalid)?,
            id: intent.run.clone(),
            environment: match intent.environment {
                data::Environment::Simulation => crate::model::Environment::Simulation,
                data::Environment::Physical => crate::model::Environment::Physical,
            },
            profiles: intent.profiles.clone(),
            processes,
        };
        let plan_digest = plan.validate(&catalog.programs, &catalog.support)?;
        let legacy_source = registry.platform_preflight(&intent, peer)?;
        let offer = data::Preparation {
            assignment: intent.id.clone(),
            intent_digest: intent.digest().map_err(invalid)?,
            peer: peer.clone(),
            release: catalog.release,
            plan_digest,
            programs,
            legacy_source,
        };
        Ok(Self {
            intent,
            offer,
            plan,
            catalog,
        })
    }
}
