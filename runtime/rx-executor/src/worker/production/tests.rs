use super::*;
use rx_domain::budget::{BudgetUnit, Consumption, RunBudget};
use rx_process_contract::execution::{CHECKPOINT_SCHEMA, CheckpointView, Run, RunSnapshot};

fn id(value: u64) -> Id {
    Id::new(format!("00000000-0000-4000-8000-{value:012}")).unwrap()
}
fn artifact(schema: &str, byte: u8) -> ArtifactRef {
    ArtifactRef {
        sha256: Digest::from_bytes([byte; 32]),
        schema_id: Name::new(schema).unwrap(),
        size_bytes: Counter(1),
    }
}
fn view(dispositions: &[PartDisposition]) -> production::View {
    let run_id = id(1);
    let session = id(2);
    let parts: Vec<_> = dispositions
        .iter()
        .enumerate()
        .map(|(index, disposition)| production::Part {
            id: id(100 + index as u64),
            run: run_id.clone(),
            ordinal: Counter(index as u64 + 1),
            revision: Counter(1),
            disposition: *disposition,
        })
        .collect();
    let mut budget = RunBudget::new(
        BudgetUnit::PartAttempt,
        Counter(dispositions.len().max(1) as u64),
    )
    .unwrap();
    for part in &parts {
        budget
            .consume(Consumption::PartAttempt(part.id.clone()))
            .unwrap();
    }
    let resolved = artifact("rx.resolved-process.v1", 1);
    production::View {
        schema: Name::new(production::SCHEMA).unwrap(),
        installation: id(3),
        store_generation: id(4),
        runtime_boot: id(5),
        sequence: Counter(1),
        caller_session: session.clone(),
        definition: artifact("rx.cell-definition.v1", 2),
        resolved: resolved.clone(),
        cell_revision: Counter(1),
        cell_epoch: Counter(1),
        scope_epochs: [(Name::new("scope/a").unwrap(), Counter(1))].into(),
        checked_at: TimePoint {
            clock_id: "test-clock".into(),
            ticks_ns: Counter(100),
        },
        valid_until: TimePoint {
            clock_id: "test-clock".into(),
            ticks_ns: Counter(200),
        },
        run: RunSnapshot {
            revision: Counter(1),
            run: Run {
                id: run_id.clone(),
                cell: Name::new("cell/a").unwrap(),
                recipe_digest: resolved.sha256,
                envelope_digest: Digest::from_bytes([3; 32]),
                purpose: Some(Purpose::Production),
                state: RunState::Executing,
                budget: Some(budget),
                executor_session: Some(session),
                mandate: Some(id(6)),
                part_ids: parts.iter().map(|p| p.id.clone()).collect(),
                pending_attempt: None,
            },
            checkpoint: CheckpointView {
                run: run_id,
                revision: Counter(1),
                executor_schema: Name::new(CHECKPOINT_SCHEMA).unwrap(),
                payload: artifact(CHECKPOINT_SCHEMA, 4),
                activations: vec![],
            },
        },
        parts,
        admission_allowed: true,
    }
}

#[test]
fn valid_platform_views_with_multiple_active_parts_require_serial_attention() {
    for dispositions in [
        vec![PartDisposition::InProgress, PartDisposition::InProgress],
        vec![
            PartDisposition::ConfirmedCompleted,
            PartDisposition::InProgress,
            PartDisposition::InProgress,
        ],
        vec![
            PartDisposition::InProgress,
            PartDisposition::ConfirmedCompleted,
            PartDisposition::InProgress,
        ],
    ] {
        let data = view(&dispositions);
        // Concurrency is legal in the shared contract. This restriction belongs only to this worker.
        production::validate(&data).unwrap();
        assert!(matches!(
            serial_in_progress(&data.parts),
            Err(MultipleActiveParts)
        ));
    }
}

#[test]
fn serial_selection_preserves_zero_or_one_active_part_without_rewriting_completed_parts() {
    for dispositions in [
        vec![],
        vec![PartDisposition::ConfirmedCompleted],
        vec![
            PartDisposition::ConfirmedCompleted,
            PartDisposition::InProgress,
        ],
    ] {
        let data = view(&dispositions);
        production::validate(&data).unwrap();
        let before = rx_domain::canonical::bytes(&data).unwrap();
        let selected = match serial_in_progress(&data.parts) {
            Ok(selected) => selected,
            Err(_) => panic!("at most one active part must remain supported"),
        };
        assert_eq!(
            selected.map(|p| p.ordinal),
            dispositions
                .iter()
                .position(|d| *d == PartDisposition::InProgress)
                .map(|i| Counter(i as u64 + 1))
        );
        assert_eq!(rx_domain::canonical::bytes(&data).unwrap(), before);
    }
}
