//! Plain gRPC/P protocol fixture, not an enrolled P or native effect claim.
use super::*;
use rx_process_contract::execution_v2 as v2;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
fn n(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn uid(x: u8) -> Id {
    Id::new(format!("00000000-0000-4000-8000-{x:012}")).unwrap()
}
fn artifact(s: &str) -> ArtifactRef {
    ArtifactRef {
        schema_id: n(s),
        sha256: Digest::from_bytes([7; 32]),
        size_bytes: Counter(10),
    }
}
fn reference(x: u8) -> rx_domain::definition::Reference {
    rx_domain::definition::Reference {
        catalog: uid(70),
        id: uid(x),
        revision: Counter(1),
        digest: Digest::from_bytes([8; 32]),
    }
}
fn snapshot() -> snap::Snapshot {
    let mut context: ExecutionSnapshot = decode(include_bytes!(
        "../../tests/fixtures/restored-v1/snapshot.json"
    ))
    .unwrap();
    let mut process: ResolvedProcess = decode(include_bytes!(
        "../../tests/fixtures/restored-v1/resolved.json"
    ))
    .unwrap();
    let child = process.root.clone();
    let child_id = child.id.clone();
    process.root.source.node = n("sequence");
    process.root.id = n(&format!(
        "node/{}",
        canonical::digest(
            "RX-PROCESS-NODE-v1",
            &(&process.process, &process.root.source)
        )
        .unwrap()
    ));
    process.root.body = rx_process_contract::CompiledBody::Sequence {
        children: vec![child],
    };
    let action = process.bindings.get_mut(&n("place")).unwrap();
    action.intent.kind = rx_domain::intent::Kind::FiniteAction;
    action.intent.body = rx_domain::intent::Body::Program(rx_domain::intent::ProgramGoal {
        program: artifact("test.program.v1"),
        parameter_set: artifact(v2::PARAMETER_SCHEMA),
    });
    let plan = v2::Plan {
        schema: n(v2::PLAN_SCHEMA),
        binding: v2::Binding {
            schema: n(v2::BINDING_SCHEMA),
            publication: reference(71),
            policy: artifact(v2::POLICY_SCHEMA),
            nodes: [(child_id, n("pick"))].into(),
        },
        process,
    };
    let part = data::PartBinding {
        schema: n(data::PART_BINDING_SCHEMA),
        run: context.run.run.id.clone(),
        part: context.run.run.part_ids[0].clone(),
        ordinal: Counter(1),
        slot_ordinal: Counter(1),
        slot: 0,
        object: reference(72),
        model: reference(73),
        object_values_digest: Digest::from_bytes([9; 32]),
        candidate: 0,
        publication: plan.binding.publication.clone(),
        policy: plan.binding.policy.clone(),
        configuration: artifact("rx.cell-configuration.v2"),
        report: artifact("rx.execution-report.v2"),
        parameters: [(n("pick"), artifact(v2::PARAMETER_SCHEMA))].into(),
    };
    context.schema = n(snap::CONTEXT_SCHEMA);
    context.resolved = plan.reference().unwrap();
    context.run.run.recipe_digest = context.resolved.sha256;
    context.run.run.state = RunState::Executing;
    context.run.run.executor_session = Some(context.caller_session.clone());
    context.run.checkpoint.activations.clear();
    context.progress.operations.clear();
    context.progress.resolved_digest =
        frontier::resolved_digest(&plan.instantiate(&part).unwrap()).unwrap();
    context.request_admission_allowed = true;
    context.admission_reason = None;
    let value = snap::Snapshot {
        schema: n(snap::SNAPSHOT_SCHEMA),
        configuration: part.configuration.clone(),
        context,
        plan,
        part,
    };
    value.validate().unwrap();
    value
}
fn admission(s: &snap::Snapshot) -> data::Admission {
    let action = &s.plan.instantiate(&s.part).unwrap().bindings[&n("place")];
    let selection = v2::Selection {
        schema: n("rx.execution-selection.v2"),
        publication: s.part.publication.id.clone(),
        policy_digest: Digest::from_bytes([11; 32]),
        configuration_digest: s.part.configuration.sha256,
        run: s.part.run.clone(),
        part: s.part.part.clone(),
        ordinal: s.part.ordinal,
        slot_ordinal: s.part.slot_ordinal,
        object: s.part.object.clone(),
        object_values_digest: s.part.object_values_digest,
        candidate: s.part.candidate,
        slot: s.part.slot,
        report_digest: s.part.report.sha256,
        node: n("pick"),
        parameter: s.part.parameters[&n("pick")].clone(),
        intent_digest: action.intent.digest().unwrap(),
        authority_generation: s.context.cell_epoch,
    };
    data::Admission {
        schema: n(data::ADMISSION_SCHEMA),
        binding: v2::OperationBinding {
            schema: n(v2::OPERATION_SCHEMA),
            operation: uid(80),
            mandate: s.context.run.run.mandate.clone().unwrap(),
            publication: s.part.publication.clone(),
            policy: s.part.policy.clone(),
            report: s.part.report.clone(),
            selection_digest: selection.digest().unwrap(),
            selection: selection.clone(),
        },
        operation: rx_domain::operation::Operation::admitted(uid(80), selection.intent_digest),
        activation: uid(81),
        permit: uid(82),
        host: action.host.clone(),
    }
}
fn encoded<T: serde::Serialize>(schema: &str, v: &T) -> wire2::ExecutionPayload {
    let bytes = canonical::bytes(v).unwrap();
    wire2::ExecutionPayload {
        reference: Some(base::ArtifactRef {
            schema_id: schema.into(),
            sha256: Sha256::digest(&bytes).to_vec(),
            size_bytes: bytes.len() as u64,
        }),
        payload: bytes,
    }
}
#[derive(Clone)]
struct Server {
    saved: Arc<Mutex<Vec<String>>>,
    lose: Arc<AtomicBool>,
    snapshot: snap::Snapshot,
    queries: Arc<std::sync::atomic::AtomicUsize>,
}
#[tonic::async_trait]
impl wire2::execution_control_service_server::ExecutionControlService for Server {
    async fn negotiate(
        &self,
        _: tonic::Request<wire2::NegotiateExecution>,
    ) -> Result<tonic::Response<wire2::ExecutionPayload>, tonic::Status> {
        let c = &self.snapshot.context;
        Ok(tonic::Response::new(encoded(
            data::SESSION_SCHEMA,
            &data::Session {
                schema: n(data::SESSION_SCHEMA),
                installation: c.installation.clone(),
                store_generation: c.store_generation.clone(),
                runtime_boot: c.runtime_boot.clone(),
                principal: n("executor"),
                session: c.caller_session.clone(),
                cell: c.run.run.cell.clone(),
                definition: c.definition.sha256,
                binding: data::binding_hash(),
                declared_at: c.checked_at.clone(),
            },
        )))
    }
    async fn get_snapshot(
        &self,
        r: tonic::Request<wire2::ReadExecutionSnapshot>,
    ) -> Result<tonic::Response<wire2::ExecutionPayload>, tonic::Status> {
        assert_eq!(r.into_inner().binding_hash, data::binding_hash().as_bytes());
        Ok(tonic::Response::new(encoded(
            snap::SNAPSHOT_SCHEMA,
            &self.snapshot,
        )))
    }
    async fn submit_node(
        &self,
        r: tonic::Request<wire2::SubmitExecutionNode>,
    ) -> Result<tonic::Response<wire2::ExecutionPayload>, tonic::Status> {
        let r = r.into_inner();
        assert_eq!(r.run_id, self.snapshot.part.run.as_str());
        assert_eq!(r.part_id, self.snapshot.part.part.as_str());
        assert_eq!(
            r.node,
            self.snapshot
                .plan
                .binding
                .nodes
                .keys()
                .next()
                .unwrap()
                .as_str()
        );
        let key = r.call.unwrap().context.unwrap().request_key.unwrap();
        self.saved.lock().unwrap().push(key);
        if self.lose.swap(false, Ordering::SeqCst) {
            return Err(tonic::Status::unavailable("committed reply lost"));
        }
        Ok(tonic::Response::new(encoded(
            data::ADMISSION_SCHEMA,
            &admission(&self.snapshot),
        )))
    }
    async fn begin_part(
        &self,
        _: tonic::Request<wire2::BeginExecutionPart>,
    ) -> Result<tonic::Response<wire2::ExecutionPayload>, tonic::Status> {
        Err(tonic::Status::unimplemented("not used"))
    }
    async fn get_part(
        &self,
        _: tonic::Request<wire2::ReadExecutionPart>,
    ) -> Result<tonic::Response<wire2::ExecutionPayload>, tonic::Status> {
        Err(tonic::Status::unimplemented("not used"))
    }
    async fn get_artifact(
        &self,
        _: tonic::Request<wire2::ReadExecutionArtifact>,
    ) -> Result<tonic::Response<wire2::ExecutionPayload>, tonic::Status> {
        Err(tonic::Status::unimplemented("not used"))
    }
}
struct Clock(TimePoint);
impl crate::clock::Clock for Clock {
    fn now(&self) -> Result<TimePoint, Error> {
        Ok(self.0.clone())
    }
}
#[tokio::test]
async fn worker_reopens_pending_v2_request_and_reuses_original_key_after_lost_reply() {
    let snapshot = snapshot();
    let c = &snapshot.context;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let saved = Arc::new(Mutex::new(Vec::new()));
    let queries = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server = Server {
        queries: queries.clone(),
        snapshot: snapshot.clone(),
        saved: saved.clone(),
        lose: Arc::new(AtomicBool::new(true)),
    };
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(base::operation_service_server::OperationServiceServer::new(
                server.clone(),
            ))
            .add_service(
                wire2::execution_control_service_server::ExecutionControlServiceServer::new(server),
            )
            .serve_with_shutdown(address, async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let endpoint = Channel::from_shared(format!("http://{address}")).unwrap();
    let mut channel = None;
    for _ in 0..50 {
        match endpoint.clone().connect().await {
            Ok(c) => {
                channel = Some(c);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
        }
    }
    let channel = channel.unwrap();
    let pin = PeerPin {
        principal: n("executor"),
        peer_boot: uid(90),
        installation: c.installation.clone(),
        store_generation: c.store_generation.clone(),
        release: Digest::from_bytes([5; 32]),
        clock_id: c.checked_at.clock_id.clone(),
        cell: c.run.run.cell.clone(),
        definition: c.definition.sha256,
    };
    let scope = crate::journal::Scope {
        installation: pin.installation.clone(),
        store_generation: pin.store_generation.clone(),
        principal: pin.principal.clone(),
        release: pin.release,
        cell: pin.cell.clone(),
        definition: pin.definition,
        run: c.run.run.id.clone(),
        resolved_digest: c.resolved.sha256,
    };
    let client=Client {pin,clock:Arc::new(Clock(c.checked_at.clone())),session:base::Session {session_id:c.caller_session.to_string(),..Default::default()},execution:wire2::execution_control_service_client::ExecutionControlServiceClient::new(channel.clone()),execution_session:None,
        read:wire::executor_read_service_client::ExecutorReadServiceClient::new(channel.clone()),workflow:base::workflow_service_client::WorkflowServiceClient::new(channel.clone()),plans:rx_protocol::executor_plan::executor_plan_service_client::ExecutorPlanServiceClient::new(channel.clone()),cells:cell::cell_service_client::CellServiceClient::new(channel.clone()),operations:base::operation_service_client::OperationServiceClient::new(channel.clone()),production:rx_protocol::production::production_service_client::ProductionServiceClient::new(channel.clone()),assignments:rx_protocol::assignment::executor_assignment_service_client::ExecutorAssignmentServiceClient::new(channel),runtime_boot:None,last_sequence:Counter(0),last_checked:Counter(0),process:None,max_payload:MAX_BYTES};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.db");
    let journal = crate::journal::Journal::open(
        rx_storage::SqliteRepository::open(&path).unwrap(),
        scope.clone(),
    )
    .unwrap();
    let mut worker = crate::worker::Worker::new(client, journal).unwrap();
    worker.negotiate_execution().await.unwrap();
    let mut stale = worker.execution_snapshot(Counter(1)).await.unwrap();
    stale.deadline = Instant::now() - Duration::from_millis(1);
    assert!(matches!(
        worker.handle_execution_snapshot(&stale).await.unwrap(),
        crate::worker::Outcome::RefreshRequired
    ));
    assert!(
        saved.lock().unwrap().is_empty(),
        "expired read cannot submit a new effect"
    );
    let view = worker.execution_snapshot(Counter(1)).await.unwrap();
    assert!(worker.handle_execution_snapshot(&view).await.is_err());
    let (client, journal) = worker.into_parts();
    drop(journal);
    let journal =
        crate::journal::Journal::open(rx_storage::SqliteRepository::open(&path).unwrap(), scope)
            .unwrap();
    let mut worker = crate::worker::Worker::new(client, journal).unwrap();
    let view = worker.execution_snapshot(Counter(1)).await.unwrap();
    assert!(matches!(
        worker.handle_execution_snapshot(&view).await.unwrap(),
        crate::worker::Outcome::Admitted(_)
    ));
    {
        let keys = saved.lock().unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0], keys[1]);
    }
    let mut stale = worker.execution_snapshot(Counter(1)).await.unwrap();
    let mut accepted = admission(&stale.raw);
    accepted
        .operation
        .conclude(rx_domain::operation::Conclusion {
            outcome: rx_domain::operation::Outcome::Succeeded,
            evidence_ids: vec![uid(99)],
        })
        .unwrap();
    let node = stale.raw.plan.binding.nodes.keys().next().unwrap().clone();
    stale.raw.context.run.checkpoint.activations = vec![ActivationSnapshot {
        id: accepted.activation.clone(),
        node: node.clone(),
        visit: Counter(1),
        slots: vec![SlotSnapshot {
            slot: n("main"),
            operation: accepted.operation.id().clone(),
            intent_digest: accepted.operation.intent_digest(),
        }],
    }];
    stale.raw.context.progress.operations.insert(
        node.clone(),
        frontier::OperationProgress {
            intent_digest: accepted.operation.intent_digest(),
            operation: accepted.operation,
        },
    );
    stale.frontier = stale.raw.validate().unwrap();
    stale.deadline = Instant::now() - Duration::from_millis(1);
    assert!(
        matches!(worker.handle_execution_snapshot(&stale).await,Err(Error::Rpc(e)) if e.code()==tonic::Code::Unavailable)
    );
    assert_eq!(
        queries.load(Ordering::SeqCst),
        1,
        "original-operation query must reach P even after execution read expires"
    );
    let entry = worker
        .journal()
        .get(&crate::journal::Logical {
            visit: Counter(1),
            node,
            stage: crate::journal::Stage::ReconcileOperation,
            control: None,
        })
        .unwrap()
        .unwrap();
    assert!(matches!(entry.send, crate::journal::SendState::EmitEntered));
    assert!(matches!(
        entry.resolution,
        crate::journal::Resolution::Pending
    ));
    let _ = stop.send(());
    task.await.unwrap();
}

#[tonic::async_trait]
impl base::operation_service_server::OperationService for Server {
    async fn submit(
        &self,
        _: tonic::Request<base::SubmitOperation>,
    ) -> Result<tonic::Response<base::Receipt>, tonic::Status> {
        Err(tonic::Status::unimplemented("unused"))
    }

    async fn get(
        &self,
        _: tonic::Request<base::OperationRef>,
    ) -> Result<tonic::Response<base::OperationView>, tonic::Status> {
        Err(tonic::Status::unimplemented("unused"))
    }
    async fn lookup(
        &self,
        _: tonic::Request<base::CallContext>,
    ) -> Result<tonic::Response<base::Receipt>, tonic::Status> {
        Err(tonic::Status::unimplemented("unused"))
    }
    async fn request_cancel(
        &self,
        _: tonic::Request<base::CancelRequest>,
    ) -> Result<tonic::Response<base::OperationView>, tonic::Status> {
        Err(tonic::Status::unimplemented("unused"))
    }
    async fn reconcile(
        &self,
        r: tonic::Request<base::OperationRef>,
    ) -> Result<tonic::Response<base::OperationView>, tonic::Status> {
        assert_eq!(r.into_inner().operation_id, uid(80).as_str());
        self.queries.fetch_add(1, Ordering::SeqCst);
        Err(tonic::Status::unavailable("query response lost; no effect"))
    }
}
