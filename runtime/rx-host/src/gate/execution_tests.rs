//! Test authority/clock and a native spy; actual Host ownership and transaction paths.
use super::*;
use crate::simulation::{FileDevice, ManualClock};
use rx_domain::{host_configuration as lc, host_qualification as lq, intent::Body};
use rx_process_contract::execution_v2::{
    self as v2, host_configuration as hc, host_inputs::VerifiedDomain, host_qualification as hq,
};
use std::sync::atomic::{AtomicU64, Ordering};
struct Spy {
    inner: FileDevice<ManualClock>,
    calls: Arc<AtomicU64>,
}
impl NativeAdapter for Spy {
    fn environment(&self) -> Environment {
        self.inner.environment()
    }
    fn protection(&self) -> Arc<dyn LocalProtection> {
        self.inner.protection()
    }
    fn guard(&self, i: &rx_domain::intent::Intent, n: &TimePoint) -> Result<Guard> {
        self.inner.guard(i, n)
    }
    fn guard_with_input(
        &self,
        i: &rx_domain::intent::Intent,
        n: &TimePoint,
        input: Option<&BoundInput>,
    ) -> Result<Guard> {
        assert!(input.is_some());
        self.inner.guard(i, n)
    }
    fn can_handover(&self, r: &[Name]) -> bool {
        self.inner.can_handover(r)
    }
    fn handover_snapshot(&self, r: &[Name]) -> Result<LocalHandover> {
        self.inner.handover_snapshot(r)
    }
    fn submit(&mut self, o: &Id, v: &Id, i: &rx_domain::intent::Intent) -> Result<NativeCapture> {
        self.inner.submit(o, v, i)
    }
    fn submit_with_context(
        &mut self,
        o: &Id,
        v: &Id,
        i: &rx_domain::intent::Intent,
        c: &NativeDispatch,
    ) -> Result<NativeCapture> {
        assert_eq!(c.execution.as_ref().unwrap().binding.operation, *o);
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.submit(o, v, i)
    }
    fn lookup(&mut self, o: &Id, v: &Id) -> Result<Option<NativeCapture>> {
        self.inner.lookup(o, v)
    }
}
fn d(n: u8) -> Digest {
    Digest::from_bytes([n; 32])
}
fn artifact(schema: &str, bytes: &[u8]) -> ArtifactRef {
    ArtifactRef {
        schema_id: name(schema),
        sha256: rx_package::content_digest(bytes),
        size_bytes: Counter(bytes.len() as u64),
    }
}
fn domain() -> VerifiedDomain {
    let inputs: v2::InputClosure = canonical::decode_json(include_bytes!(
        "../../tests/fixtures/execution-v2/inputs.json"
    ))
    .unwrap();
    let mut policy: v2::Policy = canonical::decode_json(include_bytes!(
        "../../tests/fixtures/execution-v2/policy.json"
    ))
    .unwrap();
    policy.compiler_digest = v2::compiler_digest();
    let mut index = v2::ReportIndex {
        schema: name(v2::INDEX_SCHEMA),
        entries: vec![],
    };
    for c in 0..policy.candidates.len() {
        for s in 0..policy.slot_order.len() {
            let m = v2::materialize(&policy, &inputs, c as u8, s as u16).unwrap();
            index.entries.push((c as u8, s as u16, m.report_digest()));
        }
    }
    let index = canonical::bytes(&index).unwrap();
    policy.report_index = artifact(v2::INDEX_SCHEMA, &index);
    VerifiedDomain::verify(
        &canonical::bytes(&policy).unwrap(),
        &canonical::bytes(&inputs).unwrap(),
        &index,
    )
    .unwrap()
}
#[test]
fn host_uses_its_acknowledged_domain_and_rejects_consistently_rehashed_foreign_input() {
    let dir = tempfile::tempdir().unwrap();
    let domain = domain();
    let policy = domain.policy().clone();
    let pref = domain.reference().clone();
    let material = domain.material().clone();
    let clock = ManualClock {
        clock_id: "execution-test".into(),
        ticks: Arc::new(AtomicU64::new(1000)),
    };
    let calls = Arc::new(AtomicU64::new(0));
    let native = Spy {
        inner: FileDevice::open(dir.path().join("device"), clock.clone()).unwrap(),
        calls: calls.clone(),
    };
    let action = policy.templates.values().next().unwrap();
    let cell = name("cell/execution");
    let platform = id();
    let binding = Binding {
        host: action.host.clone(),
        platform: name(platform.as_str()),
        cell: cell.clone(),
        definition: artifact("rx.cell-definition.v1", b"definition"),
        envelope: artifact("rx.operating-envelope.v1", b"envelope"),
        qualification: id(),
        qualification_revision: Counter(1),
        allowed_intents: policy
            .templates
            .values()
            .map(|a| a.intent.clone())
            .collect(),
        scope_ids: vec![name("scope/test")],
        condition_ids: vec![name("sim/ready")],
        environment: Environment::Simulation,
        purposes: [Purpose::Production].into(),
    };
    let host = Host::open(
        dir.path().join("host.db"),
        native,
        clock,
        vec![binding.clone()],
    )
    .unwrap();
    let docs = ["family", "profile", "adapter"]
        .into_iter()
        .map(|role| {
            (
                name(role),
                v2::TemplateDocument {
                    path: format!("{role}.json"),
                    artifact: artifact("test/document", b"document"),
                },
            )
        })
        .collect();
    let catalog = v2::TemplateCatalog {
        schema: name(v2::TEMPLATE_CATALOG_SCHEMA),
        installation: platform,
        cell: cell.clone(),
        environment: rx_process_contract::device_catalog::Environment::Simulation,
        templates: policy
            .templates
            .iter()
            .map(|(n, a)| {
                (
                    n.clone(),
                    v2::TemplateDeclaration {
                        action: a.clone(),
                        contract: policy.node_contracts[n].clone(),
                    },
                )
            })
            .collect(),
        documents: docs,
    };
    let catalog_ref = artifact(
        v2::TEMPLATE_CATALOG_SCHEMA,
        &canonical::bytes(&catalog).unwrap(),
    );
    // Trusted package facts are a unit fixture; signature/loader conformance is tested separately.
    host.install_execution_package(crate::execution_package::Templates {
        manifest: d(60),
        signature: d(61),
        reference: catalog_ref.clone(),
        catalog,
    })
    .unwrap();
    host.install_execution_material(domain).unwrap();
    let caller = Caller {
        peer: binding.platform.clone(),
        session: id(),
    };
    host.bind_platform(caller.clone()).unwrap();
    let pubref = rx_domain::definition::Reference {
        catalog: policy.workflow.catalog.clone(),
        id: id(),
        revision: Counter(1),
        digest: d(62),
    };
    let before = host
        .inspect_process_configuration(&caller)
        .unwrap()
        .snapshot;
    let fence = id();
    let blocks = BTreeSet::from([id()]);
    let scopes: BTreeMap<_, _> = [(name("scope/test"), Counter(2))].into();
    host.fence(
        &caller,
        fence.clone(),
        &cell,
        Counter(2),
        scopes.clone(),
        blocks.clone(),
    )
    .unwrap();
    let intents = policy
        .templates
        .values()
        .map(|a| a.intent.digest().unwrap())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let request = hc::Request {
        schema: name(hc::REQUEST_SCHEMA),
        context: lc::Request {
            schema: name("rx.host-process-configuration-request.v1"),
            id: id(),
            change: id(),
            preparation: Counter(1),
            plan_digest: d(64),
            host: binding.host.clone(),
            expected_host_boot: host.boot_id().unwrap(),
            expected_delivery_journal: before.delivery_journal,
            binding_digest: before.binding_digest,
            cells: vec![lc::CellTarget {
                cell: cell.clone(),
                expected_context: None,
                before_configuration: d(65),
                after_configuration: d(66),
                recipe: artifact(v2::PLAN_SCHEMA, b"plan"),
                definition: binding.definition.sha256,
                envelope: binding.envelope.sha256,
                environment: name("SIMULATION"),
                required_intents: intents.clone(),
                required_conditions: binding.condition_ids.clone(),
                epoch: Counter(2),
                scopes: scopes.clone(),
                fence_request: fence,
            }],
        },
        policies: [(
            cell.clone(),
            hc::CellPolicy {
                publication: pubref.clone(),
                reference: pref.clone(),
                policy: policy.clone(),
                packages: policy
                    .templates
                    .keys()
                    .map(|node| {
                        (
                            node.clone(),
                            hc::Package {
                                manifest: d(60),
                                signature: d(61),
                                catalog: catalog_ref.clone(),
                                template: node.clone(),
                            },
                        )
                    })
                    .collect(),
            },
        )]
        .into(),
    };
    let applied = host
        .accept_execution_configuration(&caller, request.clone())
        .unwrap()
        .receipt
        .unwrap();
    assert_eq!(applied.context.status, lc::Status::AppliedUnqualified);
    let mut bad = request.clone();
    bad.policies.get_mut(&cell).unwrap().publication.digest = d(99);
    assert!(host.accept_execution_configuration(&caller, bad).is_err());
    let fence = id();
    let scopes: BTreeMap<_, _> = [(name("scope/test"), Counter(3))].into();
    host.fence(
        &caller,
        fence.clone(),
        &cell,
        Counter(3),
        scopes.clone(),
        blocks.clone(),
    )
    .unwrap();
    let mut dependencies = BTreeSet::from([
        d(66),
        binding.definition.sha256,
        binding.envelope.sha256,
        pref.sha256,
        policy.report_index.sha256,
        policy.definition_closure.sha256,
        d(60),
        d(61),
        catalog_ref.sha256,
    ]);
    for a in policy.templates.values() {
        let Body::Program(p) = &a.intent.body else {
            panic!("program")
        };
        dependencies.extend([
            p.program.sha256,
            p.parameter_set.sha256,
            a.intent.profile_digest,
            a.intent.site_config_digest,
        ]);
        dependencies.extend(&a.intent.calibration_digests);
    }
    let qid = id();
    let q = hq::Request {
        schema: name(hq::REQUEST_SCHEMA),
        context: lq::Request {
            schema: name("rx.host-qualification-request.v1"),
            id: id(),
            host: binding.host.clone(),
            expected_host_boot: host.boot_id().unwrap(),
            delivery_journal: applied.context.journal.clone(),
            binding_digest: applied.context.request.binding_digest,
            change: request.context.change.clone(),
            review: id(),
            review_revision: Counter(1),
            review_digest: d(70),
            decision_revision: Counter(1),
            policy_digest: d(71),
            application_digest: d(72),
            cells: vec![lq::CellTarget {
                cell: cell.clone(),
                configuration: d(66),
                context_request: request.context.id.clone(),
                context_sequence: applied.context.sequence,
                definition: binding.definition.sha256,
                envelope: binding.envelope.sha256,
                environment: name("SIMULATION"),
                qualification: qid.clone(),
                qualification_revision: Counter(1),
                dependencies: dependencies.into_iter().collect(),
                limitations: artifact("test/limits", b"simulation"),
                allowed_intents: intents,
                purposes: vec![name("PRODUCTION")],
                epoch: Counter(3),
                scopes: scopes.clone(),
                fence_request: fence,
                required_blocks: blocks.iter().cloned().collect(),
            }],
        },
        policies: [(
            cell.clone(),
            hq::PolicyBinding {
                publication: pubref.clone(),
                policy: pref.clone(),
                configuration_request: request.digest().unwrap(),
                configuration_receipt: applied.digest().unwrap(),
            },
        )]
        .into(),
    };
    assert!(
        host.accept_qualification(&caller, q.context.clone())
            .is_err()
    );
    let accepted = host
        .accept_execution_qualification(&caller, q.clone())
        .unwrap()
        .receipt
        .unwrap();
    assert_eq!(accepted.context.status, lq::Status::Accepted);
    host.arm(&caller, id(), &cell, Counter(3), &scopes, &blocks)
        .unwrap();
    let inputs = v2::InputClosure::decode(&material.inputs, &policy).unwrap();
    let generated = v2::materialize(&policy, &inputs, 0, 0).unwrap();
    let node = policy.templates.keys().next().unwrap().clone();
    let action = &generated.actions()[&node];
    let bytes = generated.parameters()[&node].clone();
    let operation = id();
    let mandate = id();
    let selection = v2::Selection {
        schema: name("rx.execution-selection.v2"),
        publication: pubref.id.clone(),
        policy_digest: policy.digest().unwrap(),
        configuration_digest: d(66),
        run: id(),
        part: id(),
        ordinal: Counter(1),
        slot_ordinal: Counter(1),
        object: policy.candidates[0].object_model.clone(),
        object_values_digest: d(74),
        candidate: 0,
        slot: 0,
        report_digest: generated.report_digest(),
        node,
        parameter: artifact(v2::PARAMETER_SCHEMA, &bytes),
        intent_digest: action.intent.digest().unwrap(),
        authority_generation: Counter(3),
    };
    let bound = BoundInput {
        binding: v2::OperationBinding {
            schema: name(v2::OPERATION_SCHEMA),
            operation: operation.clone(),
            mandate: mandate.clone(),
            publication: pubref,
            policy: pref,
            report: artifact("rx.execution-report.v2", generated.report()),
            selection_digest: selection.digest().unwrap(),
            selection,
        },
        parameters: bytes,
    };
    let grant = host
        .acquire_grant(
            &caller,
            id(),
            action.intent.resource_set.clone(),
            Counter(1),
            Counter(60_000_000_000),
        )
        .unwrap();
    let r = Request {
        operation: operation.clone(),
        intent: action.intent.clone(),
        digest: action.intent.digest().unwrap(),
        grant: grant.id.clone(),
        permit: Permit {
            id: id(),
            operation: operation.clone(),
            digest: action.intent.digest().unwrap(),
            cell: cell.clone(),
            epoch: Counter(3),
            scopes,
            envelope: binding.envelope.sha256,
            qualification: qid,
            qualification_revision: Counter(1),
            grant: grant.id,
            host_boot: host.boot_id().unwrap(),
            conditions: [name("sim/ready")].into(),
            expires_at: TimePoint {
                clock_id: "execution-test".into(),
                ticks_ns: Counter(1_000_000_000),
            },
            source_digest: None,
            purpose: Purpose::Production,
            parent: PermitParent::Mandate(mandate),
        },
    };
    assert!(host.prepare(&caller, r.clone()).is_err());
    let mut forged = bound.clone();
    let mut value: serde_json::Value = serde_json::from_slice(&forged.parameters).unwrap();
    value["values"]["value"]["value"]["data"]["range"] = serde_json::json!({"min":21,"max":21});
    forged.parameters = canonical::bytes(&value).unwrap();
    forged.binding.selection.parameter = artifact(v2::PARAMETER_SCHEMA, &forged.parameters);
    let mut attack = r.clone();
    let Body::Program(p) = &mut attack.intent.body else {
        panic!("program")
    };
    p.parameter_set = forged.binding.selection.parameter.clone();
    attack.digest = attack.intent.digest().unwrap();
    attack.permit.digest = attack.digest;
    forged.binding.selection.intent_digest = attack.digest;
    forged.binding.selection_digest = forged.binding.selection.digest().unwrap();
    assert!(host.prepare_execution(&caller, attack, forged).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let prepared = host
        .prepare_execution(&caller, r.clone(), bound.clone())
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(
        host.authorize(&caller, r.clone(), prepared.invocation.as_ref().unwrap())
            .is_err()
    );
    let captured = host
        .authorize_execution(
            &caller,
            r,
            prepared.invocation.as_ref().unwrap(),
            bound.binding.digest().unwrap(),
        )
        .unwrap();
    assert_eq!(captured.state, ReceiptState::ResultCaptured);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        host.execution_binding(&caller, &operation, bound.binding.digest().unwrap())
            .unwrap()
            .parameters,
        bound.parameters
    );
}
