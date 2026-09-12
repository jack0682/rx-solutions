//! Pure decision cuts plus real SQLite attachment boundaries. End-to-end transport
//! and planner lifecycle coverage belongs to the resident service fixture.
use super::*;
use crate::{
    assignment_journal::{Identity, Phase as AttachmentPhase, ServiceScope},
    journal::RunResponse,
    lifecycle::{StopRecord, VersionedStop},
    service::PlannerCleanup,
};
use rx_domain::{
    budget::{BudgetUnit, Consumption, RunBudget},
    types::*,
};
use rx_process_contract::{
    assignment::{self, AttemptStatus, Candidate, Cardinality, PendingAttempt},
    execution::{
        CHECKPOINT_SCHEMA, CheckpointView, PartDisposition, Purpose, Run, RunSnapshot, RunState,
    },
    production,
};

fn id(v: u64) -> Id {
    Id::new(format!("00000000-0000-4000-8000-{v:012}")).unwrap()
}
fn n(v: &str) -> Name {
    Name::new(v).unwrap()
}
fn digest(v: u8) -> Digest {
    Digest::from_bytes([v; 32])
}
fn artifact(schema: &str, v: u8) -> ArtifactRef {
    ArtifactRef {
        schema_id: n(schema),
        sha256: digest(v),
        size_bytes: Counter(1),
    }
}
fn time(v: u64) -> TimePoint {
    TimePoint {
        clock_id: "test/cell-service".into(),
        ticks_ns: Counter(v),
    }
}
fn identity() -> Identity {
    Identity {
        journal: id(1),
        scope: ServiceScope {
            installation: id(2),
            store_generation: id(3),
            principal: n("executor/a"),
            release: digest(4),
            cell: n("cell/a"),
            definition: digest(5),
        },
    }
}
fn view(run: u64) -> production::View {
    let resolved = artifact("rx.resolved-process.v1", 6);
    production::View {
        schema: n(production::SCHEMA),
        installation: id(2),
        store_generation: id(3),
        runtime_boot: id(7),
        sequence: Counter(10),
        caller_session: id(8),
        definition: artifact("rx.cell-definition.v1", 5),
        resolved: resolved.clone(),
        cell_revision: Counter(3),
        cell_epoch: Counter(2),
        scope_epochs: [(n("scope/a"), Counter(1))].into(),
        checked_at: time(200),
        valid_until: time(300),
        run: RunSnapshot {
            revision: Counter(4),
            run: Run {
                id: id(run),
                cell: n("cell/a"),
                recipe_digest: resolved.sha256,
                envelope_digest: digest(9),
                purpose: Some(Purpose::Production),
                state: RunState::Executing,
                budget: Some(RunBudget::new(BudgetUnit::PartAttempt, Counter(1)).unwrap()),
                executor_session: Some(id(8)),
                mandate: Some(id(11)),
                part_ids: vec![],
                pending_attempt: None,
            },
            checkpoint: CheckpointView {
                run: id(run),
                revision: Counter(4),
                executor_schema: n(CHECKPOINT_SCHEMA),
                payload: artifact(CHECKPOINT_SCHEMA, 12),
                activations: vec![],
            },
        },
        parts: vec![],
        admission_allowed: true,
    }
}
fn discovery(v: &production::View) -> assignment::View {
    assignment::View {
        schema: n(assignment::SCHEMA),
        installation: v.installation.clone(),
        store_generation: v.store_generation.clone(),
        runtime_boot: v.runtime_boot.clone(),
        sequence: Counter(9),
        caller_session: v.caller_session.clone(),
        cell: v.run.run.cell.clone(),
        executor: n("executor/a"),
        definition: v.definition.clone(),
        cell_revision: v.cell_revision,
        cell_epoch: v.cell_epoch,
        scope_epochs: v.scope_epochs.clone(),
        checked_at: time(100),
        valid_until: time(300),
        cardinality: Cardinality::Single,
        candidates: vec![Candidate {
            run: v.run.run.id.clone(),
            revision: v.run.revision,
            state: v.run.run.state,
            purpose: v.run.run.purpose,
            definition: v.definition.clone(),
            resolved: v.resolved.clone(),
            executor_session: v.run.run.executor_session.clone(),
            mandate: v.run.run.mandate.clone(),
            pending_attempt: None,
            configuration_current: true,
        }],
    }
}
fn preparation(v: &production::View) -> Preparation {
    policy::prepare(&identity(), &discovery(v), v).unwrap()
}
fn completed(mut v: production::View) -> production::View {
    let part = id(500);
    v.run
        .run
        .budget
        .as_mut()
        .unwrap()
        .consume(Consumption::PartAttempt(part.clone()))
        .unwrap();
    v.parts.push(production::Part {
        id: part.clone(),
        run: v.run.run.id.clone(),
        ordinal: Counter(1),
        revision: Counter(2),
        disposition: PartDisposition::ConfirmedCompleted,
    });
    v.run.run.part_ids.push(part);
    v.run.run.state = RunState::Completed;
    v.run.revision = Counter(6);
    v.run.checkpoint.revision = Counter(6);
    v.sequence = Counter(12);
    v.checked_at = time(210);
    v.admission_allowed = false;
    production::validate(&v).unwrap();
    v
}
fn report(v: &production::View) -> service::Report {
    let mut stop = StopRecord::new(
        v.run.run.id.clone(),
        v.caller_session.clone(),
        None,
        StopReason::Completed,
        Some(time(205)),
    );
    stop.phase = StopPhase::PauseObserved;
    stop.observation = Some(RunResponse {
        run: v.run.run.id.clone(),
        revision: v.run.revision,
        state: v.run.run.state,
        recipe: v.resolved.sha256,
        executor_session: v.run.run.executor_session.clone(),
        checkpoint: v.run.checkpoint.clone(),
    });
    service::Report {
        phase: stop.phase,
        stop_id: stop.id.clone(),
        durability_fault: None,
        last_error: None,
        stop,
    }
}
fn saved(report: &service::Report) -> VersionedStop {
    VersionedStop {
        revision: Counter(2),
        record: report.stop.clone(),
    }
}

#[test]
fn observation_shutdown_preserves_preparing_attached_and_known_arming_as_attention() {
    let mut status = Status {
        phase: Phase::Idle,
        session: id(8),
        run: None,
        attachment: None,
        completed_runs: Counter(0),
        active: None,
        detail: None,
    };
    policy::observation_shutdown(&mut status);
    assert_eq!(status.phase, Phase::Stopped);
    assert!(status.run.is_none() && status.attachment.is_none() && status.active.is_none());
    for attached in [false, true] {
        status.phase = if attached { Phase::Idle } else { Phase::Arming };
        status.run = Some(id(100));
        status.attachment = attached.then(|| id(200));
        policy::observation_shutdown(&mut status);
        assert_eq!(status.phase, Phase::Attention);
        assert_eq!(status.run, Some(id(100)));
        assert_eq!(status.attachment, attached.then(|| id(200)));
        assert!(status.active.is_none());
        assert!(status.detail.is_some());
    }
}

#[test]
fn none_and_arming_are_observation_only_and_ambiguity_has_no_winner() {
    let v = view(100);
    let mut d = discovery(&v);
    d.cardinality = Cardinality::None;
    d.candidates.clear();
    assert!(matches!(
        policy::discover(&d).unwrap(),
        policy::Discovery::Idle
    ));
    assert!(policy::prepare(&identity(), &d, &v).is_err());
    d = discovery(&v);
    d.candidates[0].state = RunState::Prepared;
    d.candidates[0].pending_attempt = Some(PendingAttempt {
        id: id(20),
        status: AttemptStatus::Arming,
        executor_session: d.caller_session.clone(),
        valid_until: time(300),
    });
    assignment::validate(&d).unwrap();
    assert!(
        matches!(policy::discover(&d).unwrap(), policy::Discovery::Arming(run) if run == id(100))
    );
    assert!(policy::prepare(&identity(), &d, &v).is_err());
    d = discovery(&v);
    d.cardinality = Cardinality::Ambiguous;
    let mut b = d.candidates[0].clone();
    b.run = id(101);
    b.revision = Counter(999);
    d.candidates.push(b);
    assignment::validate(&d).unwrap();
    assert!(policy::discover(&d).is_err());
    d.candidates.reverse();
    assert!(policy::discover(&d).is_err());
}

#[test]
fn discovery_does_not_hide_old_owner_expired_attempt_or_restricted_work() {
    for state in [RunState::Paused, RunState::RecoveryRequired] {
        let mut d = discovery(&view(100));
        d.candidates[0].state = state;
        assert!(policy::discover(&d).is_err());
    }
    let mut d = discovery(&view(100));
    d.candidates[0].executor_session = Some(id(99));
    assert!(policy::discover(&d).is_err());
    d = discovery(&view(100));
    d.candidates[0].configuration_current = false;
    assert!(policy::discover(&d).is_err());
    d = discovery(&view(100));
    d.candidates[0].state = RunState::Prepared;
    d.candidates[0].pending_attempt = Some(PendingAttempt {
        id: id(20),
        status: AttemptStatus::Pending,
        executor_session: d.caller_session.clone(),
        valid_until: time(100),
    });
    assert!(policy::discover(&d).is_err());
}

#[test]
fn actual_production_cut_must_revalidate_discovery_before_any_reservation() {
    let v = view(100);
    let d = discovery(&v);
    let root = tempfile::tempdir().unwrap();
    let mut journal =
        AssignmentJournal::initialize_file(&root.path().join("service.sqlite3"), identity())
            .unwrap();
    for change in 0..10 {
        let mut next = v.clone();
        match change {
            0 => next.cell_epoch = Counter(3),
            1 => next.caller_session = id(99),
            2 => next.runtime_boot = id(99),
            3 => next.resolved.sha256 = digest(99),
            4 => next.run.run.state = RunState::Paused,
            5 => next.admission_allowed = false,
            6 => next.run.run.executor_session = Some(id(99)),
            7 => next.run.run.mandate = Some(id(99)),
            8 => next.run.run.purpose = Some(Purpose::Setup),
            _ => {
                next.scope_epochs.insert(n("scope/a"), Counter(2));
            }
        }
        assert!(
            policy::prepare(&identity(), &d, &next).is_err(),
            "changed cut {change}"
        );
        assert!(journal.current().unwrap().is_none());
    }
}

#[test]
fn preparing_recovery_keeps_original_ids_and_cannot_switch_to_new_candidate() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("service.sqlite3");
    let p = preparation(&view(100));
    let mut journal = AssignmentJournal::initialize_file(&path, identity()).unwrap();
    journal.prepare(p.clone()).unwrap();
    let run = journal.initialize_run_file(root.path(), &p.id).unwrap();
    drop(run);
    drop(journal);
    let mut journal = AssignmentJournal::open_file_required(&path, identity()).unwrap();
    let Some(RecoveredFile::Existing { attachment, run }) =
        journal.recover_current_file(root.path()).unwrap()
    else {
        panic!("required-open Preparing")
    };
    assert_eq!(attachment.value.phase, AttachmentPhase::Preparing);
    policy::same_preparation(&p, &attachment.value.preparation).unwrap();
    policy::check_existing(&p, &view(100)).unwrap();
    assert!(policy::check_existing(&p, &view(101)).is_err());
    assert!(journal.prepare(preparation(&view(101))).is_err());
    let (attached, _) = journal.attach(*run).unwrap();
    assert_eq!(attached.value.phase, AttachmentPhase::Attached);
    policy::same_preparation(&p, &attached.value.preparation).unwrap();
}

#[test]
fn missing_file_after_creation_entered_never_becomes_fresh_initialization() {
    let root = tempfile::tempdir().unwrap();
    let p = preparation(&view(100));
    let mut journal =
        AssignmentJournal::initialize_file(&root.path().join("service.sqlite3"), identity())
            .unwrap();
    journal.prepare(p.clone()).unwrap();
    drop(journal.initialize_run_file(root.path(), &p.id).unwrap());
    let path = journal.run_file(root.path(), &p.id).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(journal.recover_current_file(root.path()).is_err());
    assert!(journal.initialize_run_file(root.path(), &p.id).is_err());
    assert!(!path.exists());
    policy::same_preparation(&p, &journal.current().unwrap().unwrap().value.preparation).unwrap();
}

#[test]
fn historical_session_runtime_and_paused_attachment_cannot_resume() {
    let p = preparation(&view(100));
    for change in 0..5 {
        let mut v = view(100);
        match change {
            0 => v.caller_session = id(99),
            1 => v.runtime_boot = id(99),
            2 => v.run.run.state = RunState::Paused,
            3 => v.run.run.state = RunState::RecoveryRequired,
            _ => v.cell_epoch = Counter(99),
        }
        assert!(policy::check_existing(&p, &v).is_err());
    }
    // An independent B is absent from the direct A read; changing the unrelated
    // cell revision alone does not invalidate A's exact session/epoch/run binding.
    let mut a = view(100);
    a.cell_revision = Counter(100);
    policy::check_existing(&p, &a).unwrap();
}

#[test]
fn completion_requires_structured_cleanup_exact_stop_and_actual_completed_state() {
    let p = preparation(&view(100));
    let v = completed(view(100));
    let r = report(&v);
    for cleanup in [PlannerCleanup::NotStarted, PlannerCleanup::Confirmed] {
        policy::check_exit(&p, &r, cleanup, Some(&saved(&r))).unwrap();
    }
    assert!(policy::check_exit(&p, &r, PlannerCleanup::Unconfirmed, Some(&saved(&r))).is_err());
    assert!(policy::check_exit(&p, &r, PlannerCleanup::Confirmed, None).is_err());
    for change in 0..7 {
        let mut bad = r.clone();
        match change {
            0 => bad.stop.reason = StopReason::Requested,
            1 => bad.phase = StopPhase::Pending,
            2 => bad.phase = StopPhase::Attention,
            3 => bad.durability_fault = Some("uncertain commit".into()),
            4 => bad.stop.origin_session = id(99),
            5 => bad.stop.observation.as_mut().unwrap().state = RunState::Paused,
            _ => bad.stop_id = id(99),
        }
        assert!(
            policy::check_exit(&p, &bad, PlannerCleanup::Confirmed, Some(&saved(&bad))).is_err()
        );
    }
    let mut other = saved(&r);
    other.record.id = id(99);
    assert!(policy::check_exit(&p, &r, PlannerCleanup::Confirmed, Some(&other)).is_err());
    policy::check_completion(&p, &v).unwrap();
    assert!(policy::check_completion(&p, &view(100)).is_err());
    assert!(policy::check_completion(&p, &completed(view(101))).is_err());
}

#[test]
fn closing_a_preserves_its_stop_and_frees_only_the_next_attachment() {
    let root = tempfile::tempdir().unwrap();
    let mut assignments =
        AssignmentJournal::initialize_file(&root.path().join("service.sqlite3"), identity())
            .unwrap();
    let p = preparation(&view(100));
    assignments.prepare(p.clone()).unwrap();
    let run = assignments.initialize_run_file(root.path(), &p.id).unwrap();
    let (_, mut requests) = assignments.attach(run).unwrap();
    let v = completed(view(100));
    let r = report(&v);
    let pending = StopRecord {
        phase: StopPhase::Pending,
        observation: None,
        ..r.stop.clone()
    };
    let revision = requests.ensure_stop(&pending).unwrap().revision;
    requests.save_stop(revision, &r.stop).unwrap();
    policy::check_exit(
        &p,
        &r,
        PlannerCleanup::Confirmed,
        requests.stop_record().unwrap().as_ref(),
    )
    .unwrap();
    policy::check_completion(&p, &v).unwrap();
    let mut run = assignments
        .open_run_required(&p.id, requests.into_repository())
        .unwrap();
    assignments.close_completed(&mut run, v).unwrap();
    let mut requests = Journal::open(run.into_repository(), p.scope.clone()).unwrap();
    assert_eq!(
        requests.stop_record().unwrap().unwrap().record.id,
        r.stop_id
    );
    let b = preparation(&view(101));
    assignments.prepare(b.clone()).unwrap();
    assert_eq!(
        assignments.get(&p.id).unwrap().unwrap().value.phase,
        AttachmentPhase::Closed
    );
    assert_eq!(
        assignments.current().unwrap().unwrap().value.preparation.id,
        b.id
    );
}
