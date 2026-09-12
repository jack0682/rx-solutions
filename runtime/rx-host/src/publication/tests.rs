//! Transport tests seed an already-negotiated connection. They do not replace the
//! separate real P/Host mTLS Session.Open/Cell.Open restart acceptance test.
use super::*;
use crate::{
    Binding, Environment,
    model::Purpose,
    simulation::{FileDevice, ManualClock},
};
use rx_domain::intent::{Body, Intent, Kind, PredicateGoal};
use std::sync::{
    Mutex,
    atomic::{AtomicU8, AtomicU64, Ordering},
};

fn id(value: u64) -> Id {
    Id::new(format!("00000000-0000-4000-8000-{value:012}")).unwrap()
}
fn name(value: &str) -> Name {
    Name::new(value).unwrap()
}
fn connection(channel: Channel) -> Connection {
    Connection {
        channel,
        session: base::Session {
            session_id: id(10).to_string(),
            ..Default::default()
        },
        probe: true,
        last_probe: None,
        max_batch: 128,
        max_bytes: 1_048_576,
    }
}
fn destination() -> Destination {
    Destination {
        platform: name("platform"),
        installation: id(1),
        store_generation: id(2),
    }
}
fn binding() -> Binding {
    Binding {
        host: name("host/sim"),
        platform: name("platform"),
        cell: name("cell/sim"),
        definition: ArtifactRef {
            sha256: Digest::from_bytes([3; 32]),
            schema_id: name("rx.cell-definition.v1"),
            size_bytes: Counter(1),
        },
        envelope: ArtifactRef {
            sha256: Digest::from_bytes([4; 32]),
            schema_id: name("rx.operating-envelope.v1"),
            size_bytes: Counter(1),
        },
        qualification: id(3),
        qualification_revision: Counter(1),
        allowed_intents: vec![Intent {
            kind: Kind::EnsureState,
            target: name("sim/chuck"),
            profile_digest: Digest::from_bytes([1; 32]),
            site_config_digest: Digest::from_bytes([2; 32]),
            calibration_digests: vec![],
            resource_set: vec![name("sim/controller")],
            execution_timeout_ms: Counter(5000),
            prepare_validity_ms: Counter(1000),
            completion_rule: name("sim/closed"),
            cancel_rule: name("sim/stop"),
            body: Body::Predicate(PredicateGoal {
                predicate_id: name("chuck.closed"),
                target: TypedValue::Boolean(true),
                settle_ms: Counter(0),
            }),
        }],
        scope_ids: vec![name("scope/main")],
        condition_ids: vec![name("sim/ready")],
        environment: Environment::Simulation,
        purposes: [Purpose::Production].into(),
    }
}

#[tokio::test]
async fn local_idle_ticks_do_not_extend_the_one_second_remote_probe_deadline() {
    let channel = Channel::from_static("http://127.0.0.1:1").connect_lazy();
    let mut connection = connection(channel);
    let start = Instant::now();
    assert!(connection.needs_probe(true, start));
    assert!(
        connection.needs_probe(false, start),
        "new connections must establish the prefix before data"
    );
    connection.probe = false;
    connection.last_probe = Some(start);
    for tick in 1..10 {
        assert!(!connection.needs_probe(true, start + Duration::from_millis(tick * 100)));
        assert_eq!(connection.last_probe, Some(start));
    }
    assert!(connection.needs_probe(true, start + IDLE_PROBE_INTERVAL));
    // No accepted ACK occurred: retry delay must not turn the next attempt into an Idle success.
    for elapsed in [1100, 1300, 1700, 2500, 7500] {
        assert!(connection.needs_probe(true, start + Duration::from_millis(elapsed)));
        assert_eq!(connection.last_probe, Some(start));
    }
    assert!(
        !connection.needs_probe(false, start + Duration::from_secs(20)),
        "retained evidence is not delayed by an overdue idle probe"
    );
}

#[derive(Clone)]
struct EvidencePeer {
    destination: Destination,
    calls: Arc<Mutex<Vec<base::EvidenceBatch>>>,
    fault: Arc<AtomicU8>,
}
#[tonic::async_trait]
impl base::evidence_service_server::EvidenceService for EvidencePeer {
    async fn publish(
        &self,
        request: tonic::Request<base::EvidenceBatch>,
    ) -> Result<tonic::Response<base::DurableAck>, Status> {
        let batch = request.into_inner();
        self.calls.lock().unwrap().push(batch.clone());
        let fault = self.fault.load(Ordering::SeqCst);
        match fault {
            1 => return Err(Status::unavailable("test transport interruption")),
            2 => {
                return Err(Status::unauthenticated(
                    "test prior runtime session expired",
                ));
            }
            _ => {}
        }
        Ok(tonic::Response::new(base::DurableAck {
            producer_journal_id: batch.producer_journal_id,
            through_seq: batch.first_seq - 1 + batch.records.len() as u64,
            platform_cursor: Some(base::Cursor {
                installation_id: self.destination.installation.to_string(),
                store_generation: if fault == 3 {
                    id(999).to_string()
                } else {
                    self.destination.store_generation.to_string()
                },
                seq: 10,
                view_id: CONTROL_VIEW.into(),
            }),
        }))
    }
    async fn get(
        &self,
        _: tonic::Request<base::EvidenceRef>,
    ) -> Result<tonic::Response<base::Evidence>, Status> {
        Err(Status::unimplemented("not used by the publisher"))
    }
}

#[tokio::test]
async fn idle_remote_probe_detects_expired_session_without_new_evidence_or_native_effects() {
    let root = tempfile::tempdir().unwrap();
    let clock = ManualClock {
        clock_id: "test/publisher-host-clock".into(),
        ticks: Arc::new(AtomicU64::new(1000)),
    };
    let host = Arc::new(
        Host::open(
            root.path().join("host.db"),
            FileDevice::open(root.path().join("device"), clock.clone()).unwrap(),
            clock,
            vec![binding()],
        )
        .unwrap(),
    );
    let destination = destination();
    let original = host.publication_chunk(&destination).unwrap();
    assert_eq!(original.tail, Counter(0));
    let calls = Arc::new(Mutex::new(vec![]));
    let fault = Arc::new(AtomicU8::new(0));
    let peer = EvidencePeer {
        destination: destination.clone(),
        calls: calls.clone(),
        fault: fault.clone(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(base::evidence_service_server::EvidenceServiceServer::new(
                peer,
            ))
            .serve_with_incoming_shutdown(
                tokio_stream::wrappers::TcpListenerStream::new(listener),
                async {
                    let _ = stopped.await;
                },
            ),
    );
    let channel = Channel::from_shared(format!("http://{address}"))
        .unwrap()
        .connect_timeout(Duration::from_secs(1))
        .timeout(Duration::from_secs(1))
        .connect()
        .await
        .unwrap();
    let mut publisher = Publisher::new(
        host.clone(),
        Configuration {
            destination: destination.clone(),
            release_digest: Digest::from_bytes([8; 32]),
            endpoint: Endpoint {
                uri: format!("https://{address}"),
                server_name: "test.invalid".into(),
                server_ca_pem: vec![],
                client_certificate_pem: vec![],
                client_key_pem: vec![],
            },
        },
    );
    publisher.connection = Some(connection(channel));
    let prefix = publisher.tick().await.unwrap();
    assert_eq!(prefix.through, Counter(0));
    assert_eq!(prefix.journal, original.journal);
    assert_eq!(calls.lock().unwrap().len(), 1);
    let last_probe = publisher.connection.as_ref().unwrap().last_probe;
    for _ in 0..5 {
        assert_eq!(publisher.tick().await.unwrap(), prefix);
    }
    assert_eq!(
        calls.lock().unwrap().len(),
        1,
        "local idle polls must not generate remote traffic"
    );
    assert_eq!(
        publisher.connection.as_ref().unwrap().last_probe,
        last_probe
    );
    publisher.connection.as_mut().unwrap().last_probe = Some(Instant::now() - IDLE_PROBE_INTERVAL);
    assert_eq!(publisher.tick().await.unwrap(), prefix);
    assert_eq!(
        calls.lock().unwrap().len(),
        2,
        "the overdue empty Publish must reach the peer"
    );

    publisher.connection.as_mut().unwrap().last_probe = Some(Instant::now() - IDLE_PROBE_INTERVAL);
    let due = publisher.connection.as_ref().unwrap().last_probe;
    fault.store(1, Ordering::SeqCst);
    for _ in 0..2 {
        assert!(
            matches!(publisher.tick().await, Err(Error::Rpc(status)) if status.code() == Code::Unavailable)
        );
        assert!(
            publisher.connection.is_some(),
            "ordinary transport failure retains the session"
        );
        assert_eq!(publisher.connection.as_ref().unwrap().last_probe, due);
    }
    assert_eq!(
        calls.lock().unwrap().len(),
        4,
        "failure must not reset the retry into a local Idle success"
    );
    fault.store(3, Ordering::SeqCst);
    assert!(
        matches!(publisher.tick().await, Err(Error::Local(_))),
        "a foreign store generation remains blocked"
    );
    assert_eq!(host.publication_chunk(&destination).unwrap().cursor, prefix);
    assert_eq!(publisher.connection.as_ref().unwrap().last_probe, due);

    fault.store(2, Ordering::SeqCst);
    assert!(
        matches!(publisher.tick().await, Err(Error::Rpc(status)) if status.code() == Code::Unauthenticated)
    );
    assert!(
        publisher.connection.is_none(),
        "only authentication expiry starts the original reconnect path"
    );
    let retained = host.publication_chunk(&destination).unwrap();
    assert_eq!(retained.boot, original.boot);
    assert_eq!(retained.journal, original.journal);
    assert_eq!(retained.cursor, prefix);
    assert_eq!(retained.tail, Counter(0));
    assert!(retained.records.is_empty());
    assert_eq!(
        publisher.configuration.destination.installation,
        destination.installation
    );
    assert_eq!(
        publisher.configuration.destination.store_generation,
        destination.store_generation
    );
    assert!(
        FileDevice::effects(root.path().join("device"))
            .unwrap()
            .is_empty()
    );
    for request in calls.lock().unwrap().iter() {
        assert!(request.records.is_empty());
        assert_eq!(request.producer_journal_id, original.journal.as_str());
        assert_eq!(request.first_seq, 1);
        assert_eq!(
            request.context.as_ref().unwrap().session_id,
            id(10).as_str()
        );
        assert!(request.context.as_ref().unwrap().request_key.is_none());
    }
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
