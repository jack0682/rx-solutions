//! Common Host approval records; native provider profiles do not own these semantics.
use super::*;
use rx_process_contract::execution_v2::{host_configuration as config, host_qualification as qual};
use serde::{Deserialize, Serialize};
const CONFIG: &str = "rx.host.execution-configuration.v2";
const POLICY: &str = "rx.host.execution-policy.v2";
const QUALIFICATION: &str = "rx.host.execution-qualification.v2";
const ACCEPTED: &str = "rx.host.execution-accepted.v2";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Configured {
    request: Id,
    receipt: config::Receipt,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Qualified {
    request: Id,
    receipt: qual::Receipt,
}

pub(super) fn check_configuration<N>(core: &mut Core<N>, request: &config::Request) -> Result<()> {
    request.validate().map_err(invalid)?;
    for (cell, p) in &request.policies {
        let domain = execution_material::domain(core, &p.reference)?;
        if canonical::bytes(domain.policy()).map_err(invalid)?
            != canonical::bytes(&p.policy).map_err(invalid)?
        {
            return Err(HostError::Conflict);
        }
        let binding = core.bindings.get(cell).ok_or(HostError::Forbidden)?;
        for (node, required) in &p.packages {
            let local = core
                .execution_packages
                .get(&required.manifest)
                .ok_or(HostError::Guard)?;
            if local.signature != required.signature
                || local.reference != required.catalog
                || local.catalog.cell != *cell
                || local.catalog.installation.as_str() != binding.platform.as_str()
            {
                return Err(HostError::Conflict);
            }
            let declared = local
                .catalog
                .templates
                .get(&required.template)
                .ok_or(HostError::Guard)?;
            if declared.action.host != binding.host
                || canonical::bytes(&declared.action).map_err(invalid)?
                    != canonical::bytes(&p.policy.templates[node]).map_err(invalid)?
                || canonical::bytes(&declared.contract).map_err(invalid)?
                    != canonical::bytes(&p.policy.node_contracts[node]).map_err(invalid)?
            {
                return Err(HostError::Conflict);
            }
        }
    }
    Ok(())
}
pub(super) fn record_configuration(
    tx: &mut dyn rx_ports::Transaction,
    caller: &Caller,
    request: &config::Request,
    receipt: &rx_domain::host_configuration::Receipt,
) -> rx_ports::Result<()> {
    let mut policies = BTreeMap::new();
    if receipt.status == rx_domain::host_configuration::Status::AppliedUnqualified {
        for (cell, p) in &request.policies {
            let target = request
                .context
                .cells
                .iter()
                .find(|v| &v.cell == cell)
                .ok_or(rx_ports::StoreError::Invalid("v2 target absent".into()))?;
            policies.insert(
                cell.clone(),
                config::AppliedPolicy {
                    publication: p.publication.clone(),
                    policy: p.reference.clone(),
                    request: request.context.id.clone(),
                    receipt_sequence: receipt.sequence,
                    configuration: target.after_configuration,
                },
            );
        }
    }
    let outer = config::Receipt {
        schema: name(config::RECEIPT_SCHEMA),
        request: request.clone(),
        request_digest: request.digest().map_err(rx_ports::StoreError::Invalid)?,
        context: receipt.clone(),
        policies,
    };
    outer.validate().map_err(rx_ports::StoreError::Invalid)?;
    tx.put(
        &key(
            "execution-configuration-request",
            (&caller.peer, &request.context.id),
        ),
        None,
        &doc(CONFIG, &outer)?,
    )?;
    for cell in outer.policies.keys() {
        let k = key("execution-policy", cell);
        let old = tx.get(&k)?;
        tx.put(
            &k,
            old.map(|r| r.revision),
            &doc(
                POLICY,
                &Configured {
                    request: request.context.id.clone(),
                    receipt: outer.clone(),
                },
            )?,
        )?;
    }
    Ok(())
}
pub(super) fn configured<N>(core: &mut Core<N>, cell: &Name) -> Result<config::Receipt> {
    let row = core
        .store
        .transact(|tx| tx.get(&key("execution-policy", cell)))?
        .ok_or(HostError::Guard)?;
    Ok(decode::<Configured>(&row, POLICY)?.receipt)
}
pub(super) fn check_qualification<N>(core: &mut Core<N>, request: &qual::Request) -> Result<()> {
    request.validate().map_err(invalid)?;
    for (cell, p) in &request.policies {
        let local = configured(core, cell)?;
        request.matches_configuration(&local).map_err(invalid)?;
        execution_material::domain(core, &p.policy)?;
    }
    Ok(())
}
pub(super) fn record_qualification(
    tx: &mut dyn rx_ports::Transaction,
    caller: &Caller,
    request: &qual::Request,
    receipt: &rx_domain::host_qualification::Receipt,
) -> rx_ports::Result<()> {
    let mut accepted = BTreeMap::new();
    if receipt.status == rx_domain::host_qualification::Status::Accepted {
        for (cell, p) in &request.policies {
            let t = request
                .context
                .cells
                .iter()
                .find(|t| &t.cell == cell)
                .ok_or(rx_ports::StoreError::Invalid(
                    "v2 qualification cell absent".into(),
                ))?;
            accepted.insert(
                cell.clone(),
                qual::AcceptedPolicy {
                    binding: p.clone(),
                    qualification: t.qualification.clone(),
                    qualification_revision: t.qualification_revision,
                    request: request.context.id.clone(),
                    acceptance_sequence: receipt.sequence,
                },
            );
        }
    }
    let outer = qual::Receipt {
        schema: name(qual::RECEIPT_SCHEMA),
        request: request.clone(),
        request_digest: request.digest().map_err(rx_ports::StoreError::Invalid)?,
        context: receipt.clone(),
        policies: accepted,
    };
    outer.validate().map_err(rx_ports::StoreError::Invalid)?;
    tx.put(
        &key(
            "execution-qualification-request",
            (&caller.peer, &request.context.id),
        ),
        None,
        &doc(QUALIFICATION, &outer)?,
    )?;
    for cell in outer.policies.keys() {
        let k = key("execution-accepted", cell);
        let old = tx.get(&k)?;
        tx.put(
            &k,
            old.map(|r| r.revision),
            &doc(
                ACCEPTED,
                &Qualified {
                    request: request.context.id.clone(),
                    receipt: outer.clone(),
                },
            )?,
        )?;
    }
    Ok(())
}

impl<N: NativeAdapter, C: Clock, H: BoundaryHook> Host<N, C, H> {
    pub fn install_execution_package(
        &self,
        package: crate::execution_package::Templates,
    ) -> Result<()> {
        let mut core = self.lock()?;
        if core.caller.is_some() || !core.armed.is_empty() {
            return Err(HostError::Guard);
        }
        let binding = core
            .bindings
            .get(&package.catalog.cell)
            .ok_or(HostError::Forbidden)?;
        if package.catalog.installation.as_str() != binding.platform.as_str() {
            return Err(HostError::Conflict);
        }
        if core.execution_packages.contains_key(&package.manifest) {
            return Err(HostError::Conflict);
        }
        core.execution_packages.insert(package.manifest, package);
        Ok(())
    }
}

pub(super) fn check_configuration_replay<N>(
    core: &mut Core<N>,
    caller: &Caller,
    id: &Id,
    request: Option<&config::Request>,
) -> Result<()> {
    let old = core
        .store
        .transact(|tx| tx.get(&key("execution-configuration-request", (&caller.peer, id))))?;
    match (old, request) {
        (None, None) => Ok(()),
        (Some(row), Some(request)) => {
            let old: config::Receipt = decode(&row, CONFIG)?;
            if old.request_digest != request.digest().map_err(invalid)? {
                Err(HostError::Conflict)
            } else {
                Ok(())
            }
        }
        _ => Err(HostError::Conflict),
    }
}
fn configuration_observation<N>(
    core: &mut Core<N>,
    receipt: Option<config::Receipt>,
) -> Result<config::Observation> {
    let facts = configuration::observation(core, receipt.as_ref().map(|r| r.context.clone()))?;
    let mut policies = BTreeMap::new();
    for cell in &facts.snapshot.cells {
        if let Some(row) = core
            .store
            .transact(|tx| tx.get(&key("execution-policy", &cell.cell)))?
        {
            let record: Configured = decode(&row, POLICY)?;
            if let Some(p) = record.receipt.policies.get(&cell.cell)
                && cell.applied.as_ref().is_some_and(|a| {
                    a.request == p.request
                        && a.receipt_sequence == p.receipt_sequence
                        && a.configuration == p.configuration
                })
            {
                policies.insert(cell.cell.clone(), p.clone());
            }
        }
    }
    let current = facts.context_matches_current_host
        && receipt.as_ref().is_some_and(|r| {
            r.policies
                .iter()
                .all(|(cell, p)| policies.get(cell) == Some(p))
        });
    let v = config::Observation {
        schema: name(config::OBSERVATION_SCHEMA),
        snapshot: facts.snapshot,
        policies,
        receipt,
        context_matches_current_host: current,
        activation_authorized: false,
    };
    v.validate().map_err(invalid)?;
    Ok(v)
}
impl<N: NativeAdapter, C: Clock, H: BoundaryHook> Host<N, C, H> {
    pub fn inspect_execution_configuration(&self, caller: &Caller) -> Result<config::Observation> {
        let mut c = self.lock()?;
        authorized(&c, caller)?;
        configuration_observation(&mut c, None)
    }
    pub fn lookup_execution_configuration(
        &self,
        caller: &Caller,
        id: &Id,
    ) -> Result<config::Observation> {
        let mut c = self.lock()?;
        authorized(&c, caller)?;
        let receipt = c
            .store
            .transact(|tx| tx.get(&key("execution-configuration-request", (&caller.peer, id))))?
            .map(|r| decode(&r, CONFIG))
            .transpose()?;
        configuration_observation(&mut c, receipt)
    }
    pub fn accept_execution_configuration(
        &self,
        caller: &Caller,
        request: config::Request,
    ) -> Result<config::Observation> {
        request.validate().map_err(invalid)?;
        self.accept_configuration_with(caller, request.context.clone(), Some(&request))?;
        self.lookup_execution_configuration(caller, &request.context.id)
    }
}

pub(super) fn check_qualification_replay<N>(
    core: &mut Core<N>,
    caller: &Caller,
    id: &Id,
    request: Option<&qual::Request>,
) -> Result<()> {
    let old = core
        .store
        .transact(|tx| tx.get(&key("execution-qualification-request", (&caller.peer, id))))?;
    match (old, request) {
        (None, None) => Ok(()),
        (Some(row), Some(request)) => {
            let old: qual::Receipt = decode(&row, QUALIFICATION)?;
            if old.request_digest != request.digest().map_err(invalid)? {
                Err(HostError::Conflict)
            } else {
                Ok(())
            }
        }
        _ => Err(HostError::Conflict),
    }
}
fn qualification_observation<N>(
    core: &mut Core<N>,
    receipt: Option<qual::Receipt>,
) -> Result<qual::Observation> {
    let facts = qualification::observation(core, receipt.as_ref().map(|r| r.context.clone()))?;
    let mut configured = BTreeMap::new();
    let mut policies = BTreeMap::new();
    for cell in &facts.snapshot.cells {
        if let Some(row) = core
            .store
            .transact(|tx| tx.get(&key("execution-policy", &cell.cell)))?
        {
            let record: Configured = decode(&row, POLICY)?;
            if let Some(p) = record.receipt.policies.get(&cell.cell)
                && cell.applied.as_ref().is_some_and(|a| {
                    a.request == p.request
                        && a.receipt_sequence == p.receipt_sequence
                        && a.configuration == p.configuration
                })
            {
                configured.insert(
                    cell.cell.clone(),
                    qual::ConfiguredPolicy {
                        context: p.clone(),
                        request_digest: record.receipt.request_digest,
                        receipt_digest: record.receipt.digest().map_err(invalid)?,
                    },
                );
            }
        }
        if let Some(row) = core
            .store
            .transact(|tx| tx.get(&key("execution-accepted", &cell.cell)))?
        {
            let record: Qualified = decode(&row, ACCEPTED)?;
            if let Some(p) = record.receipt.policies.get(&cell.cell)
                && facts.accepted.iter().any(|a| {
                    a.cell == cell.cell
                        && a.request == p.request
                        && a.qualification == p.qualification
                        && a.qualification_revision == p.qualification_revision
                        && a.acceptance_sequence == p.acceptance_sequence
                })
            {
                policies.insert(cell.cell.clone(), p.clone());
            }
        }
    }
    let mut value = qual::Observation {
        schema: name(qual::OBSERVATION_SCHEMA),
        snapshot: facts.snapshot,
        configured,
        accepted: facts.accepted,
        policies,
        receipt,
        receipt_matches_current_host: false,
        activation_authorized: false,
    };
    value.receipt_matches_current_host = value.current();
    value.validate().map_err(invalid)?;
    Ok(value)
}
impl<N: NativeAdapter, C: Clock, H: BoundaryHook> Host<N, C, H> {
    pub fn inspect_execution_qualification(&self, caller: &Caller) -> Result<qual::Observation> {
        let mut c = self.lock()?;
        authorized(&c, caller)?;
        qualification_observation(&mut c, None)
    }
    pub fn lookup_execution_qualification(
        &self,
        caller: &Caller,
        id: &Id,
    ) -> Result<qual::Observation> {
        let mut c = self.lock()?;
        authorized(&c, caller)?;
        let receipt = c
            .store
            .transact(|tx| tx.get(&key("execution-qualification-request", (&caller.peer, id))))?
            .map(|r| decode(&r, QUALIFICATION))
            .transpose()?;
        qualification_observation(&mut c, receipt)
    }
    pub fn accept_execution_qualification(
        &self,
        caller: &Caller,
        request: qual::Request,
    ) -> Result<qual::Observation> {
        request.validate().map_err(invalid)?;
        self.accept_qualification_with(caller, request.context.clone(), Some(&request))?;
        self.lookup_execution_qualification(caller, &request.context.id)
    }
}

const INPUT: &str = "rx.host.execution-input.v2";
pub(super) fn saved_input<N>(core: &mut Core<N>, operation: &Id) -> Result<Option<BoundInput>> {
    core.store
        .transact(|tx| tx.get(&key("execution-input", operation)))?
        .map(|r| decode(&r, INPUT).map_err(Into::into))
        .transpose()
}
pub(super) fn same_input<N>(
    core: &mut Core<N>,
    operation: &Id,
    input: Option<&BoundInput>,
) -> Result<()> {
    let old = saved_input(core, operation)?;
    if canonical::bytes(&old).map_err(invalid)? != canonical::bytes(&input).map_err(invalid)? {
        return Err(HostError::Conflict);
    }
    Ok(())
}
pub(super) fn store_input(
    tx: &mut dyn rx_ports::Transaction,
    operation: &Id,
    input: Option<&BoundInput>,
) -> rx_ports::Result<()> {
    if let Some(input) = input {
        tx.put(
            &key("execution-input", operation),
            None,
            &doc(INPUT, input)?,
        )?;
    }
    Ok(())
}
pub(super) fn approved_intent<N>(
    core: &mut Core<N>,
    request: &Request,
    input: Option<&BoundInput>,
) -> Result<Option<Digest>> {
    let Some(input) = input else {
        let v2 = core.store.transact(|tx| {
            Ok(tx
                .get(&key("execution-policy", &request.permit.cell))?
                .is_some()
                || tx
                    .get(&key("execution-input", &request.operation))?
                    .is_some())
        })?;
        if v2 {
            return Err(HostError::Guard);
        }
        return Ok(None);
    };
    let b = &input.binding;
    b.validate().map_err(invalid)?;
    let PermitParent::Mandate(mandate) = &request.permit.parent else {
        return Err(HostError::Conflict);
    };
    if b.operation != request.operation
        || b.mandate != *mandate
        || b.selection.intent_digest != request.digest
        || b.selection.authority_generation != request.permit.epoch
    {
        return Err(HostError::Conflict);
    }
    let row = core
        .store
        .transact(|tx| tx.get(&key("execution-accepted", &request.permit.cell)))?
        .ok_or(HostError::Guard)?;
    let accepted: Qualified = decode(&row, ACCEPTED)?;
    accepted.receipt.validate().map_err(invalid)?;
    let policy = accepted
        .receipt
        .policies
        .get(&request.permit.cell)
        .ok_or(HostError::Guard)?;
    let configured = configured(core, &request.permit.cell)?;
    accepted
        .receipt
        .request
        .matches_configuration(&configured)
        .map_err(invalid)?;
    let configured_policy = configured
        .policies
        .get(&request.permit.cell)
        .ok_or(HostError::Guard)?;
    if policy.qualification != request.permit.qualification
        || policy.qualification_revision != request.permit.qualification_revision
        || policy.binding.publication != b.publication
        || policy.binding.policy != b.policy
        || configured_policy.configuration != b.selection.configuration_digest
    {
        return Err(HostError::Conflict);
    }
    // The domain is selected from our acknowledged record, not from a caller's purported set.
    let domain = execution_material::domain(core, &policy.binding.policy)?;
    let host = &core
        .bindings
        .get(&request.permit.cell)
        .ok_or(HostError::Forbidden)?
        .host;
    domain
        .verify_operation(b, host, &request.intent, &input.parameters)
        .map_err(invalid)?;
    Ok(Some(
        domain.policy().templates[&b.selection.node]
            .intent
            .digest()
            .map_err(invalid)?,
    ))
}
impl<N: NativeAdapter, C: Clock, H: BoundaryHook> Host<N, C, H> {
    pub fn execution_binding(
        &self,
        caller: &Caller,
        operation: &Id,
        expected: Digest,
    ) -> Result<BoundInput> {
        let mut core = self.lock()?;
        authorized(&core, caller)?;
        let input = saved_input(&mut core, operation)?.ok_or(HostError::NotFound)?;
        if input.binding.operation != *operation
            || input.binding.digest().map_err(invalid)? != expected
        {
            return Err(HostError::Conflict);
        }
        Ok(input)
    }
}
