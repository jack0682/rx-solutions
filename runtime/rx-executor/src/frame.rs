//! Projection of authenticated P data into the C++ BT's bounded read frame.
use crate::{Error, ValidatedSnapshot};
use rx_domain::{
    operation::{Disposition, Integrity, Knowledge, Outcome},
    types::*,
};
use rx_process_contract::frontier::WaitProgress;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub run: Id,
    pub executor_session: Id,
    pub resolved_digest: Digest,
    pub visit: Counter,
    pub epoch: Counter,
}
#[derive(Clone, Debug, Serialize)]
pub struct SourceDeadline {
    pub clock_id: String,
    pub checked_at_ns: Counter,
    pub valid_until_ns: Counter,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WaitState {
    Pending,
    Satisfied,
    TimedOut,
}
#[derive(Clone, Debug, Serialize)]
pub struct Node {
    pub operation_id: Option<Id>,
    pub outcome: Outcome,
    pub unknown: bool,
    pub integrity_valid: bool,
    pub released: bool,
    pub branch: Option<bool>,
    pub decision_id: Option<Id>,
    pub wait: WaitState,
    pub clearance: Option<Id>,
}
impl Default for Node {
    fn default() -> Self {
        Self {
            operation_id: None,
            outcome: Outcome::None,
            unknown: false,
            integrity_valid: true,
            released: false,
            branch: None,
            decision_id: None,
            wait: WaitState::Pending,
            clearance: None,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Packet {
    pub schema: &'static str,
    pub identity: Identity,
    pub revision: Counter,
    pub complete: bool,
    pub current: bool,
    pub submission_authorized: bool,
    pub remaining_validity_ns: Counter,
    pub source: SourceDeadline,
    pub nodes: BTreeMap<Name, Node>,
    pub eligible: BTreeSet<Name>,
}
impl ValidatedSnapshot {
    /// Call only when deliberately creating a new execution context; never auto-adopt a new epoch.
    pub fn context_identity(&self) -> Identity {
        Identity {
            run: self.raw.run.run.id.clone(),
            executor_session: self.raw.caller_session.clone(),
            resolved_digest: self.raw.resolved.sha256,
            visit: self.raw.visit,
            epoch: self.raw.cell_epoch,
        }
    }
    pub fn frame(&self, expected: &Identity) -> Result<Packet, Error> {
        if &self.context_identity() != expected {
            return Err(Error::Invalid(
                "execution context changed; explicit reconciliation required".into(),
            ));
        }
        if !self.is_current() {
            return Err(Error::Expired);
        }
        let now = self.clock.now()?;
        if now.clock_id.len() > 256 {
            return Err(Error::Invalid("clock identity exceeds frame bound".into()));
        }
        if now.clock_id != self.raw.checked_at.clock_id
            || now.ticks_ns < self.raw.checked_at.ticks_ns
        {
            return Err(Error::Invalid("shared clock changed".into()));
        }
        let source_remaining = self
            .raw
            .valid_until
            .ticks_ns
            .0
            .checked_sub(now.ticks_ns.0)
            .ok_or(Error::Expired)?;
        let local_remaining = self
            .deadline
            .saturating_duration_since(Instant::now())
            .as_nanos();
        let remaining = source_remaining.min(u64::try_from(local_remaining).unwrap_or(u64::MAX));
        if remaining == 0 {
            return Err(Error::Expired);
        }
        let mut nodes = BTreeMap::<Name, Node>::new();
        for (node, progress) in &self.raw.progress.operations {
            let operation = &progress.operation;
            nodes.insert(
                node.clone(),
                Node {
                    operation_id: Some(operation.id().clone()),
                    outcome: operation.outcome(),
                    unknown: operation.knowledge() == Knowledge::Unknown
                        || operation.outcome() == Outcome::Unresolved,
                    integrity_valid: operation.integrity() == Integrity::Valid,
                    released: operation.disposition() == Disposition::Released,
                    ..Node::default()
                },
            );
        }
        for (node, choice) in &self.raw.progress.branches {
            nodes.insert(
                node.clone(),
                Node {
                    branch: Some(choice.chosen),
                    decision_id: Some(choice.decision.clone()),
                    ..Node::default()
                },
            );
        }
        for (node, result) in &self.raw.progress.waits {
            let (wait, decision) = match result {
                WaitProgress::Satisfied { decision, .. } => (WaitState::Satisfied, decision),
                WaitProgress::TimedOut { decision } => (WaitState::TimedOut, decision),
            };
            nodes.insert(
                node.clone(),
                Node {
                    wait,
                    decision_id: Some(decision.clone()),
                    ..Node::default()
                },
            );
        }
        for (node, clearance) in &self.raw.progress.cleared_interventions {
            nodes.insert(
                node.clone(),
                Node {
                    clearance: Some(clearance.clone()),
                    ..Node::default()
                },
            );
        }
        let eligible = self
            .frontier
            .operations
            .iter()
            .chain(&self.frontier.decisions)
            .chain(&self.frontier.waits)
            .chain(&self.frontier.handovers)
            .chain(&self.frontier.interventions)
            .cloned()
            .collect();
        Ok(Packet {
            schema: "rx.bt-frame.v1",
            identity: expected.clone(),
            revision: self.raw.sequence,
            complete: true,
            current: true,
            submission_authorized: self.raw.request_admission_allowed,
            remaining_validity_ns: Counter(remaining),
            source: SourceDeadline {
                clock_id: self.raw.checked_at.clock_id.clone(),
                checked_at_ns: self.raw.checked_at.ticks_ns,
                valid_until_ns: self.raw.valid_until.ticks_ns,
            },
            nodes,
            eligible,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RequestKind {
    SubmitOperation,
    ResolveBranch,
    BeginWait,
    RequestIntervention,
    RequestHandover,
    PauseExecutor,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub identity: Identity,
    pub node: Name,
    pub kind: RequestKind,
    pub argument: String,
    pub timeout_ns: Counter,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Clock;
    use rx_domain::canonical;
    use rx_process_contract::{
        ResolvedProcess, execution::ExecutionSnapshot, execution_validation,
    };
    use std::{
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
        time::Duration,
    };
    struct TestClock(AtomicU64);
    impl Clock for TestClock {
        fn now(&self) -> Result<TimePoint, Error> {
            Ok(TimePoint {
                clock_id: "test-clock".into(),
                ticks_ns: Counter(self.0.load(Ordering::SeqCst)),
            })
        }
    }
    fn fixture() -> (ValidatedSnapshot, Arc<TestClock>) {
        let raw: ExecutionSnapshot = canonical::decode_json(include_bytes!(
            "../tests/fixtures/restored-v1/snapshot.json"
        ))
        .unwrap();
        let process: ResolvedProcess = canonical::decode_json(include_bytes!(
            "../tests/fixtures/restored-v1/resolved.json"
        ))
        .unwrap();
        let frontier = execution_validation::validate(&raw, &process).unwrap();
        let clock = Arc::new(TestClock(AtomicU64::new(1000)));
        (
            ValidatedSnapshot {
                raw,
                process: Arc::new(process),
                frontier,
                deadline: Instant::now() + Duration::from_millis(100),
                clock: clock.clone(),
            },
            clock,
        )
    }
    #[test]
    fn restored_p_snapshot_preserves_work_and_does_not_restore_admission() {
        let (snapshot, _) = fixture();
        let packet = snapshot.frame(&snapshot.context_identity()).unwrap();
        assert!(packet.complete && packet.current);
        assert!(!packet.submission_authorized);
        assert_eq!(
            packet.nodes.values().next().unwrap().operation_id.as_ref(),
            Some(
                snapshot
                    .raw
                    .progress
                    .operations
                    .values()
                    .next()
                    .unwrap()
                    .operation
                    .id()
            )
        );
        assert_eq!(packet.revision, snapshot.raw.sequence);
    }
    #[test]
    fn source_clock_expiry_blocks_a_view_even_if_the_local_steady_deadline_has_not_passed() {
        let (snapshot, clock) = fixture();
        clock
            .0
            .store(snapshot.raw.valid_until.ticks_ns.0, Ordering::SeqCst);
        assert!(Instant::now() < snapshot.deadline);
        assert!(!snapshot.is_current());
        assert!(matches!(
            snapshot.frame(&snapshot.context_identity()),
            Err(Error::Expired)
        ));
    }
    #[test]
    fn a_new_epoch_is_not_implicitly_adopted_by_an_existing_context() {
        let (snapshot, _) = fixture();
        let mut expected = snapshot.context_identity();
        expected.epoch = expected.epoch.increment().unwrap();
        assert!(matches!(snapshot.frame(&expected), Err(Error::Invalid(_))));
    }
}
