use rx_domain::types::*;
use rx_executor::{frame::Identity, journal::*};
use rx_ports::*;
use rx_storage::SqliteRepository;
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id(n: u8) -> Id {
    Id::new(format!("00000000-0000-4000-8000-{n:012}")).unwrap()
}
fn scope() -> Scope {
    Scope {
        installation: id(1),
        store_generation: id(2),
        principal: name("executor"),
        release: Digest::from_bytes([3; 32]),
        cell: name("cell/a"),
        definition: Digest::from_bytes([4; 32]),
        run: id(5),
        resolved_digest: Digest::from_bytes([6; 32]),
    }
}
fn identity() -> Identity {
    Identity {
        run: id(5),
        executor_session: id(7),
        resolved_digest: Digest::from_bytes([6; 32]),
        visit: Counter(1),
        epoch: Counter(1),
    }
}
fn logical() -> Logical {
    Logical {
        visit: Counter(1),
        node: name("node/a"),
        stage: Stage::ResolveActivation,
        control: None,
    }
}
fn body(revision: u64) -> Body {
    Body::ResolveActivation {
        run: id(5),
        node: name("node/a"),
        visit: Counter(1),
        expected_run: Counter(revision),
    }
}
fn basis(sequence: u64) -> Basis {
    Basis {
        runtime_boot: id(8),
        sequence: Counter(sequence),
        run_revision: Counter(4),
        cell_revision: Counter(2),
        checked_at: TimePoint {
            clock_id: "test-clock".into(),
            ticks_ns: Counter(sequence),
        },
    }
}
fn response() -> Response {
    Response::Activation(rx_process_contract::execution::ActivationSnapshot {
        id: id(9),
        node: name("node/a"),
        visit: Counter(1),
        slots: vec![],
    })
}
struct FaultRepo {
    inner: SqliteRepository,
    mode: Arc<AtomicU8>,
}
impl Repository for FaultRepo {
    fn transact<T>(&mut self, f: impl FnOnce(&mut dyn Transaction) -> Result<T>) -> Result<T> {
        let mode = self.mode.swap(0, Ordering::SeqCst);
        let result = self.inner.transact(|tx| {
            let value = f(tx)?;
            if mode == 1 {
                Err(StoreError::Unavailable("before commit".into()))
            } else {
                Ok(value)
            }
        })?;
        if mode == 2 {
            Err(StoreError::Unavailable("after commit, reply lost".into()))
        } else {
            Ok(result)
        }
    }
    fn pending_outbox_after(&mut self, a: Option<&Id>, l: usize) -> Result<Vec<OutboxRecord>> {
        self.inner.pending_outbox_after(a, l)
    }
    fn control_events_after(&mut self, a: Counter, l: usize) -> Result<Vec<StoredEvent>> {
        self.inner.control_events_after(a, l)
    }
    fn control_snapshot(&mut self) -> Result<(Counter, Vec<Record>)> {
        self.inner.control_snapshot()
    }
    fn journal_head(&mut self) -> Result<Counter> {
        self.inner.journal_head()
    }
    fn pending_outbox(&mut self, l: usize) -> Result<Vec<OutboxRecord>> {
        self.inner.pending_outbox(l)
    }
    fn snapshot(&mut self) -> Result<(Counter, Vec<Record>)> {
        self.inner.snapshot()
    }
    fn events_after(&mut self, a: Counter, l: usize) -> Result<Vec<StoredEvent>> {
        self.inner.events_after(a, l)
    }
}
#[test]
fn prepared_entered_and_reply_boundaries_survive_rollback_and_lost_commit_response() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("e.db");
    let mode = Arc::new(AtomicU8::new(0));
    let mut journal = Journal::open(
        FaultRepo {
            inner: SqliteRepository::open(&path).unwrap(),
            mode: mode.clone(),
        },
        scope(),
    )
    .unwrap();
    mode.store(1, Ordering::SeqCst);
    assert!(
        journal
            .prepare(logical(), identity(), body(4), basis(1))
            .is_err()
    );
    assert!(journal.get(&logical()).unwrap().is_none());
    let first = journal
        .prepare(logical(), identity(), body(4), basis(1))
        .unwrap();
    assert_eq!(
        journal
            .prepare(logical(), identity(), body(4), basis(20))
            .unwrap()
            .key,
        first.key
    );
    mode.store(1, Ordering::SeqCst);
    assert!(journal.enter(&first.key).is_err());
    assert_eq!(
        journal.get(&logical()).unwrap().unwrap().send,
        SendState::Prepared
    );
    mode.store(2, Ordering::SeqCst);
    assert!(journal.enter(&first.key).is_err());
    drop(journal);
    let mut journal = Journal::open(
        FaultRepo {
            inner: SqliteRepository::open(&path).unwrap(),
            mode: mode.clone(),
        },
        scope(),
    )
    .unwrap();
    let entered = journal.get(&logical()).unwrap().unwrap();
    assert_eq!(entered.key, first.key);
    assert_eq!(entered.send, SendState::EmitEntered);
    assert!(matches!(
        journal.prepare(logical(), identity(), body(5), basis(2)),
        Err(StoreError::KeyConflict)
    ));
    mode.store(2, Ordering::SeqCst);
    assert!(journal.reply(&first.key, response()).is_err());
    drop(journal);
    let mut journal = Journal::open(SqliteRepository::open(&path).unwrap(), scope()).unwrap();
    let saved = journal.get(&logical()).unwrap().unwrap();
    assert!(matches!(saved.resolution, Resolution::Reply { .. }));
    assert!(journal.enter(&saved.key).is_err());
    let mut changed = match response() {
        Response::Activation(v) => v,
        _ => unreachable!(),
    };
    changed.id = id(10);
    assert!(matches!(
        journal.reply(&saved.key, Response::Activation(changed)),
        Err(StoreError::Integrity(_))
    ));
}
#[test]
fn only_a_confirmed_transactional_rejection_allows_new_cas_and_key() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(
        SqliteRepository::open(dir.path().join("e.db")).unwrap(),
        scope(),
    )
    .unwrap();
    let first = journal
        .prepare(logical(), identity(), body(4), basis(1))
        .unwrap();
    journal.enter(&first.key).unwrap();
    assert!(
        journal
            .prepare(logical(), identity(), body(5), basis(2))
            .is_err()
    );
    journal.revision_rejected(&first.key).unwrap();
    let next = journal
        .prepare(logical(), identity(), body(5), basis(2))
        .unwrap();
    assert_ne!(next.key, first.key);
    assert_eq!(next.generation, Counter(2));
    let mut changed_context = identity();
    changed_context.epoch = Counter(2);
    assert!(
        journal
            .prepare(logical(), changed_context, body(5), basis(3))
            .is_err()
    );
    let mut repo = journal.into_repository();
    let records = repo.snapshot().unwrap().1;
    assert_eq!(
        records
            .iter()
            .filter(|r| r.document.schema.as_str() == "rx.executor-request.v1")
            .count(),
        2
    );
}
#[test]
fn p_observation_preserves_network_uncertainty_and_deduplicates_unchanged_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("e.db");
    let mut journal = Journal::open(SqliteRepository::open(&path).unwrap(), scope()).unwrap();
    let entry = journal
        .prepare(logical(), identity(), body(4), basis(1))
        .unwrap();
    journal.enter(&entry.key).unwrap();
    let Response::Activation(value) = response() else {
        unreachable!()
    };
    journal
        .observe(Observation {
            logical: logical(),
            target: ObservedTarget::Activation {
                value: value.clone(),
            },
            basis: basis(2),
        })
        .unwrap();
    journal
        .observe(Observation {
            logical: logical(),
            target: ObservedTarget::Activation {
                value: value.clone(),
            },
            basis: basis(3),
        })
        .unwrap();
    let pending = journal.get(&logical()).unwrap().unwrap();
    assert!(matches!(pending.resolution, Resolution::Pending));
    assert_eq!(pending.send, SendState::EmitEntered);
    let mut changed = value;
    changed.id = id(11);
    assert!(
        journal
            .observe(Observation {
                logical: logical(),
                target: ObservedTarget::Activation { value: changed },
                basis: basis(4)
            })
            .is_err()
    );
    let mut repo = journal.into_repository();
    assert_eq!(
        repo.events_after(Counter(0), 128)
            .unwrap()
            .iter()
            .filter(|e| e.document.schema.as_str() == "rx.executor-observed.v1")
            .count(),
        1
    );
    drop(repo);
    let mut wrong = scope();
    wrong.store_generation = id(12);
    assert!(Journal::open(SqliteRepository::open(path).unwrap(), wrong).is_err());
}

#[test]
fn pause_intent_is_scoped_to_executor_session_and_epoch_without_changing_operation_keys() {
    use rx_process_contract::execution::{CHECKPOINT_SCHEMA, CheckpointView, RunState};
    let dir = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(
        SqliteRepository::open(dir.path().join("e.db")).unwrap(),
        scope(),
    )
    .unwrap();
    assert!(
        !serde_json::to_value(logical())
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("control")
    );
    let context = identity();
    let pause = Logical {
        visit: Counter(1),
        node: name("root"),
        stage: Stage::PauseRun,
        control: Some(ControlIdentity {
            session: context.executor_session.clone(),
            epoch: context.epoch,
        }),
    };
    let body = Body::PauseRun {
        run: context.run.clone(),
        root: name("root"),
        expected_run: Counter(4),
    };
    let first = journal
        .prepare(pause.clone(), context.clone(), body.clone(), basis(1))
        .unwrap();
    journal.enter(&first.key).unwrap();
    let response = Response::Run(Box::new(RunResponse {
        run: context.run.clone(),
        revision: Counter(5),
        recipe: scope().resolved_digest,
        state: RunState::Paused,
        executor_session: None,
        checkpoint: CheckpointView {
            run: context.run.clone(),
            revision: Counter(5),
            executor_schema: name(CHECKPOINT_SCHEMA),
            payload: ArtifactRef {
                sha256: Digest::from_bytes([1; 32]),
                schema_id: name(CHECKPOINT_SCHEMA),
                size_bytes: Counter(1),
            },
            activations: vec![],
        },
    }));
    journal.reply(&first.key, response).unwrap();
    let mut next_context = context;
    next_context.epoch = Counter(2);
    assert!(
        journal
            .prepare(pause.clone(), next_context.clone(), body.clone(), basis(2))
            .is_err()
    );
    let mut next = pause;
    next.control.as_mut().unwrap().epoch = Counter(2);
    let second = journal.prepare(next, next_context, body, basis(2)).unwrap();
    assert_ne!(second.key, first.key);
}

#[test]
fn reconciliation_acknowledgment_is_not_a_resource_release_observation() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(
        SqliteRepository::open(dir.path().join("e.db")).unwrap(),
        scope(),
    )
    .unwrap();
    let logical = Logical {
        visit: Counter(1),
        node: name("node/a"),
        stage: Stage::ReconcileOperation,
        control: None,
    };
    let digest = Digest::from_bytes([20; 32]);
    let operation = id(21);
    let entry = journal
        .prepare(
            logical.clone(),
            identity(),
            Body::ReconcileOperation {
                run: scope().run,
                operation: operation.clone(),
                intent_digest: digest,
            },
            basis(1),
        )
        .unwrap();
    journal.enter(&entry.key).unwrap();
    assert!(journal.revision_rejected(&entry.key).is_err());
    journal
        .reply(
            &entry.key,
            Response::ReconciliationAccepted {
                operation: operation.clone(),
                intent_digest: digest,
            },
        )
        .unwrap();
    assert!(matches!(
        journal.get(&logical).unwrap().unwrap().resolution,
        Resolution::Reply { .. }
    ));
    journal
        .observe(Observation {
            logical: logical.clone(),
            target: ObservedTarget::Released {
                operation: operation.clone(),
                intent_digest: digest,
            },
            basis: basis(2),
        })
        .unwrap();
    assert!(
        journal
            .observe(Observation {
                logical,
                target: ObservedTarget::Released {
                    operation: id(22),
                    intent_digest: digest
                },
                basis: basis(3)
            })
            .is_err()
    );
    let mut repo = journal.into_repository();
    let events = repo.events_after(Counter(0), 128).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.document.schema.as_str() == "rx.executor-request-replied.v1")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.document.schema.as_str() == "rx.executor-observed.v1")
            .count(),
        1
    );
}

fn checkpoint_body(digest_byte: u8, chosen: bool) -> Body {
    use rx_process_contract::{checkpoint_change::*, execution::*};
    Body::CommitCheckpoint {
        target: CheckpointTarget {
            run: scope().run.clone(),
            node: name("node/a"),
            visit: Counter(1),
            action: CheckpointAction::ChooseBranch,
        },
        command: Box::new(CommitCheckpoint {
            run: scope().run,
            expected_revision: Counter(4),
            new_checkpoint: CheckpointView {
                run: scope().run,
                revision: Counter(5),
                executor_schema: name(CHECKPOINT_SCHEMA),
                payload: ArtifactRef {
                    sha256: Digest::from_bytes([digest_byte; 32]),
                    schema_id: name(CHECKPOINT_SCHEMA),
                    size_bytes: Counter(1),
                },
                activations: vec![],
            },
        }),
        decision: CheckpointDecision::Branch(rx_process_contract::frontier::BranchChoice {
            decision: id(digest_byte),
            chosen,
            evidence_ids: vec![id(45)],
        }),
    }
}
fn checkpoint_reply(body: &Body) -> Response {
    let Body::CommitCheckpoint { command, .. } = body else {
        panic!("checkpoint")
    };
    Response::Run(Box::new(RunResponse {
        run: scope().run,
        revision: Counter(5),
        recipe: scope().resolved_digest,
        state: rx_process_contract::execution::RunState::Executing,
        executor_session: Some(identity().executor_session),
        checkpoint: command.new_checkpoint.clone(),
    }))
}
#[test]
fn checkpoint_expiry_preserves_old_attempt_and_pending_projection_is_not_an_rpc_reply() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(
        SqliteRepository::open(dir.path().join("e.db")).unwrap(),
        scope(),
    )
    .unwrap();
    let logical = Logical {
        stage: Stage::CheckpointBranch,
        ..logical()
    };
    let first = journal
        .prepare(
            logical.clone(),
            identity(),
            checkpoint_body(30, true),
            basis(1),
        )
        .unwrap();
    journal.enter(&first.key).unwrap();
    assert!(
        journal
            .prepare(
                logical.clone(),
                identity(),
                checkpoint_body(31, false),
                basis(2)
            )
            .is_err()
    );
    assert!(journal.revision_rejected(&first.key).is_err());
    journal
        .checkpoint_rejected(&first.key, CheckpointRejection::Expired)
        .unwrap();
    let second = journal
        .prepare(
            logical.clone(),
            identity(),
            checkpoint_body(31, false),
            basis(2),
        )
        .unwrap();
    assert_ne!(first.key, second.key);
    assert_eq!(second.generation, Counter(2));
    assert!(matches!(
        journal.attempt(&first.key).unwrap().unwrap().resolution,
        Resolution::CheckpointRejected {
            reason: CheckpointRejection::Expired
        }
    ));
    journal.enter(&second.key).unwrap();
    // Another valid P proposal may win the CAS; observing it does not acknowledge our request.
    let Body::CommitCheckpoint { decision, .. } = checkpoint_body(32, true) else {
        panic!("decision")
    };
    let observation = Observation {
        logical: logical.clone(),
        target: ObservedTarget::Checkpoint { decision },
        basis: basis(3),
    };
    journal.observe(observation.clone()).unwrap();
    journal.observe(observation).unwrap();
    assert!(matches!(
        journal.get(&logical).unwrap().unwrap().resolution,
        Resolution::Pending
    ));
    assert!(
        journal
            .reply(&second.key, checkpoint_reply(&second.body))
            .is_err()
    );
    assert!(matches!(
        journal.get(&logical).unwrap().unwrap().resolution,
        Resolution::Pending
    ));
}

#[test]
fn stop_intent_survives_commit_response_loss_and_never_becomes_a_part_request() {
    use rx_executor::lifecycle::*;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("e.db");
    let mode = Arc::new(AtomicU8::new(0));
    let mut journal = Journal::open(
        FaultRepo {
            inner: SqliteRepository::open(&path).unwrap(),
            mode: mode.clone(),
        },
        scope(),
    )
    .unwrap();
    let intent = StopRecord::new(
        scope().run,
        identity().executor_session,
        None,
        StopReason::Requested,
        None,
    );
    mode.store(1, Ordering::SeqCst);
    assert!(journal.ensure_stop(&intent).is_err());
    assert!(journal.stop_record().unwrap().is_none());
    mode.store(2, Ordering::SeqCst);
    assert!(journal.ensure_stop(&intent).is_err());
    drop(journal);
    let mut journal = Journal::open(SqliteRepository::open(&path).unwrap(), scope()).unwrap();
    let saved = journal.stop_record().unwrap().unwrap();
    assert_eq!(saved.record.id, intent.id);
    assert!(saved.record.origin_context.is_none());
    assert!(saved.record.attempts.is_empty());
    let other = StopRecord::new(
        scope().run,
        identity().executor_session,
        None,
        StopReason::PlannerFault,
        None,
    );
    assert_eq!(journal.ensure_stop(&other).unwrap().record.id, intent.id);
}
#[test]
fn stop_attempt_key_is_preserved_and_p_observation_does_not_invent_a_reply() {
    use rx_executor::lifecycle::*;
    let dir = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(
        SqliteRepository::open(dir.path().join("e.db")).unwrap(),
        scope(),
    )
    .unwrap();
    let intent = StopRecord::new(
        scope().run,
        identity().executor_session.clone(),
        Some(identity()),
        StopReason::Requested,
        None,
    );
    let mut saved = journal.ensure_stop(&intent).unwrap();
    saved.record.attempts.push(StopAttempt {
        key: id(51),
        session: identity().executor_session,
        expected_run: Counter(4),
        state: AttemptState::Prepared,
        response: None,
    });
    saved.revision = journal.save_stop(saved.revision, &saved.record).unwrap();
    saved.record.attempts[0].state = AttemptState::Entered;
    saved.revision = journal.save_stop(saved.revision, &saved.record).unwrap();
    let mut wrong = saved.record.clone();
    wrong.attempts[0].key = id(52);
    assert!(journal.save_stop(saved.revision, &wrong).is_err());
    let mut wrong = saved.record.clone();
    wrong.attempts.push(StopAttempt {
        key: id(52),
        ..wrong.attempts[0].clone()
    });
    assert!(journal.save_stop(saved.revision, &wrong).is_err());
    let Response::Run(mut view) = checkpoint_reply(&checkpoint_body(30, true)) else {
        panic!("run response")
    };
    view.state = rx_process_contract::execution::RunState::Paused;
    view.executor_session = None;
    saved.record.phase = StopPhase::PauseObserved;
    saved.record.observation = Some(*view);
    saved.revision = journal.save_stop(saved.revision, &saved.record).unwrap();
    let read = journal.stop_record().unwrap().unwrap();
    assert_eq!(read.record.attempts[0].state, AttemptState::Entered);
    assert!(read.record.attempts[0].response.is_none());
    let mut wrong = read.record;
    wrong.phase = StopPhase::Pending;
    wrong.observation = None;
    assert!(journal.save_stop(read.revision, &wrong).is_err());
}

#[test]
fn material_identity_is_observed_without_rewriting_a_lost_begin_reply() {
    use rx_process_contract::{execution::PartDisposition, production};
    let dir = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(
        SqliteRepository::open(dir.path().join("e.db")).unwrap(),
        scope(),
    )
    .unwrap();
    let logical = Logical {
        visit: Counter(1),
        node: name("production/part"),
        stage: Stage::BeginPart,
        control: None,
    };
    let body = Body::BeginPart {
        cell: scope().cell,
        run: scope().run,
        ordinal: Counter(1),
        mandate: id(60),
        expected_budget: Counter(1),
        expected_cell: Counter(1),
    };
    let entry = journal
        .prepare(logical.clone(), identity(), body.clone(), basis(1))
        .unwrap();
    journal.enter(&entry.key).unwrap();
    let mut changed = body;
    let Body::BeginPart {
        expected_budget, ..
    } = &mut changed
    else {
        panic!("part")
    };
    *expected_budget = Counter(2);
    assert!(
        journal
            .prepare(logical.clone(), identity(), changed, basis(2))
            .is_err()
    );
    let part = production::Part {
        id: id(61),
        run: scope().run,
        ordinal: Counter(1),
        revision: Counter(1),
        disposition: PartDisposition::InProgress,
    };
    journal
        .observe(Observation {
            logical: logical.clone(),
            target: ObservedTarget::Part {
                value: part.clone(),
            },
            basis: basis(2),
        })
        .unwrap();
    assert!(matches!(
        journal.get(&logical).unwrap().unwrap().resolution,
        Resolution::Pending
    ));
    let mut different = part.clone();
    different.id = id(63);
    assert!(
        journal
            .reply(&entry.key, Response::Part(different))
            .is_err()
    );
    let mut done = part.clone();
    done.disposition = PartDisposition::ConfirmedCompleted;
    assert!(
        journal
            .observe(Observation {
                logical: logical.clone(),
                target: ObservedTarget::Part {
                    value: done.clone()
                },
                basis: basis(3)
            })
            .is_err()
    );
    done.revision = Counter(2);
    journal
        .observe(Observation {
            logical: logical.clone(),
            target: ObservedTarget::Part {
                value: done.clone(),
            },
            basis: basis(3),
        })
        .unwrap();
    assert!(matches!(
        journal.get(&logical).unwrap().unwrap().resolution,
        Resolution::Pending
    ));
    let complete = Logical {
        stage: Stage::CompletePart,
        ..logical
    };
    let entry = journal
        .prepare(
            complete,
            identity(),
            Body::CompletePart {
                ordinal: Counter(1),
                command: Box::new(production::CompletePart {
                    cell: scope().cell,
                    run: scope().run,
                    part: part.id,
                    expected_run: Counter(4),
                    expected_part: Counter(1),
                }),
            },
            basis(3),
        )
        .unwrap();
    journal.enter(&entry.key).unwrap();
    let mut wrong = done.clone();
    wrong.id = id(62);
    assert!(journal.reply(&entry.key, Response::Part(wrong)).is_err());
    journal.reply(&entry.key, Response::Part(done)).unwrap();
}
