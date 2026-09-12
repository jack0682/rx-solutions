//! Bounded volatile suggestions. Mutation identity and uncertainty remain in the durable journal.
use crate::{
    Error,
    frame::{Identity, Request, RequestKind},
    worker::Outcome,
};
use rx_domain::types::{Counter, Name};
use rx_process_contract::checkpoint_change::CheckpointAction;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopState {
    Running,
    PausePending,
    PauseObserved,
    Attention,
}
#[derive(Clone)]
struct Pending {
    request: Request,
    due: Instant,
    delay: Duration,
    in_flight: bool,
}
pub struct PendingRequests {
    identity: Identity,
    root: Name,
    items: VecDeque<Pending>,
    stop: StopState,
}
impl PendingRequests {
    pub fn new(identity: Identity, root: Name) -> Self {
        Self {
            identity,
            root,
            items: VecDeque::new(),
            stop: StopState::Running,
        }
    }
    pub fn stop_state(&self) -> StopState {
        self.stop
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn accept(&mut self, requests: Vec<Request>, now: Instant) -> Result<(), Error> {
        if requests.len() > 32 {
            return invalid("request batch limit");
        }
        // Validate the complete packet before accepting any suggestion.
        let mut candidate = self.items.clone();
        for request in &requests {
            if request.identity != self.identity {
                return invalid("pending request context differs");
            }
            if request.kind == RequestKind::PauseExecutor
                && (request.node != self.root
                    || !request.argument.is_empty()
                    || request.timeout_ns != Counter(0))
            {
                return invalid("pause request shape differs");
            }
            if let Some(old) = candidate.iter().find(|p| same_key(&p.request, request)) {
                if !same(&old.request, request) {
                    return invalid("queued request meaning changed");
                }
            } else {
                candidate.push_back(Pending {
                    request: request.clone(),
                    due: now,
                    delay: Duration::ZERO,
                    in_flight: false,
                });
            }
        }
        if requests
            .iter()
            .any(|r| r.kind == RequestKind::PauseExecutor)
        {
            self.request_pause(now);
            return Ok(());
        }
        if self.stop != StopState::Running {
            return Ok(());
        }
        if candidate.len() > 32 {
            return invalid("pending request capacity exceeded");
        }
        self.items = candidate;
        Ok(())
    }
    pub fn request_pause(&mut self, now: Instant) {
        if matches!(self.stop, StopState::PauseObserved | StopState::Attention) {
            return;
        }
        self.stop = StopState::PausePending;
        self.items
            .retain(|p| p.request.kind == RequestKind::PauseExecutor);
        if self.items.is_empty() {
            self.items.push_front(Pending {
                request: Request {
                    identity: self.identity.clone(),
                    node: self.root.clone(),
                    kind: RequestKind::PauseExecutor,
                    argument: String::new(),
                    timeout_ns: Counter(0),
                },
                due: now,
                delay: Duration::ZERO,
                in_flight: false,
            });
        }
    }
    pub fn next(&mut self, now: Instant) -> Option<Request> {
        let any_in_flight = self.items.iter().any(|p| p.in_flight);
        let index = self.items.iter().position(|p| {
            !p.in_flight
                && p.due <= now
                && (p.request.kind == RequestKind::PauseExecutor || !any_in_flight)
        })?;
        let value = self.items.get_mut(index)?;
        value.in_flight = true;
        Some(value.request.clone())
    }
    pub fn complete(
        &mut self,
        request: &Request,
        result: Result<Outcome, Error>,
        now: Instant,
    ) -> Result<(), Error> {
        let Some(index) = self
            .items
            .iter()
            .position(|p| same_key(&p.request, request))
        else {
            if self.stop != StopState::Running && request.kind != RequestKind::PauseExecutor {
                return Ok(());
            }
            return invalid("completion without queued request");
        };
        if !same(&self.items[index].request, request) || !self.items[index].in_flight {
            return invalid("completion does not match in-flight request");
        }
        use RequestKind as K;
        let mut retry = Some(Duration::from_millis(20));
        let mut fatal = false;
        match result {
            Ok(Outcome::Admitted(_) | Outcome::ObservedOperation(_))
                if request.kind == K::SubmitOperation =>
            {
                retry = None
            }
            Ok(Outcome::ResourcesReleased(_)) if request.kind == K::RequestHandover => retry = None,
            Ok(Outcome::ReconciliationPending(_)) if request.kind == K::RequestHandover => {
                retry = Some(Duration::from_millis(100))
            }
            Ok(
                Outcome::CheckpointCommitted { node, action }
                | Outcome::CheckpointObserved { node, action },
            ) if node == request.node => match (request.kind, action) {
                (K::ResolveBranch, CheckpointAction::ChooseBranch)
                | (K::BeginWait, CheckpointAction::CheckWait) => retry = None,
                (K::BeginWait, CheckpointAction::StartWait) => {}
                _ => return invalid("checkpoint outcome/request kind differs"),
            },
            Ok(Outcome::Paused { run, .. })
                if request.kind == K::PauseExecutor && run == self.identity.run =>
            {
                self.items.clear();
                self.stop = StopState::PauseObserved;
                return Ok(());
            }
            Ok(Outcome::WaitingCondition | Outcome::RefreshRequired) | Err(Error::Expired) => {}
            Ok(Outcome::WaitingForAuthority) => retry = Some(Duration::from_millis(100)),
            Err(Error::Rpc(status))
                if matches!(
                    status.code(),
                    tonic::Code::Unavailable
                        | tonic::Code::DeadlineExceeded
                        | tonic::Code::Cancelled
                        | tonic::Code::Internal
                        | tonic::Code::Unknown
                        | tonic::Code::ResourceExhausted
                ) =>
            {
                retry = Some(
                    (self.items[index].delay * 2)
                        .max(Duration::from_millis(100))
                        .min(Duration::from_secs(5)),
                );
            }
            Ok(Outcome::ContextChanged | Outcome::Attention(_) | Outcome::Unsupported(_))
            | Err(_) => fatal = true,
            _ => return invalid("worker outcome/request kind differs"),
        }
        if fatal {
            if request.kind == K::PauseExecutor {
                self.items.clear();
                self.stop = StopState::Attention;
            } else {
                self.items.remove(index);
                self.request_pause(now);
            }
        } else if let Some(delay) = retry {
            let mut value = self.items.remove(index).expect("known pending request");
            value.due = now + delay;
            value.delay = delay;
            value.in_flight = false;
            self.items.push_back(value);
        } else {
            self.items.remove(index);
        }
        Ok(())
    }
}
fn same_key(a: &Request, b: &Request) -> bool {
    a.node == b.node && a.kind == b.kind
}
fn same(a: &Request, b: &Request) -> bool {
    same_key(a, b)
        && a.identity == b.identity
        && a.argument == b.argument
        && a.timeout_ns == b.timeout_ns
}
fn invalid<T>(why: &str) -> Result<T, Error> {
    Err(Error::Invalid(why.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rx_domain::types::{Digest, Id};
    fn identity() -> Identity {
        Identity {
            run: Id::new("00000000-0000-4000-8000-000000000001").unwrap(),
            executor_session: Id::new("00000000-0000-4000-8000-000000000002").unwrap(),
            resolved_digest: Digest::from_bytes([1; 32]),
            visit: Counter(1),
            epoch: Counter(1),
        }
    }
    fn request(node: &str, kind: RequestKind) -> Request {
        Request {
            identity: identity(),
            node: Name::new(node).unwrap(),
            kind,
            argument: "ready".into(),
            timeout_ns: Counter(1000),
        }
    }
    #[test]
    fn wait_start_does_not_drop_the_only_bt_request_and_due_requests_are_fair() {
        let now = Instant::now();
        let mut queue = PendingRequests::new(identity(), Name::new("root").unwrap());
        let wait = request("wait", RequestKind::BeginWait);
        let op = request("op", RequestKind::SubmitOperation);
        queue.accept(vec![wait.clone(), op.clone()], now).unwrap();
        let first = queue.next(now).unwrap();
        assert_eq!(first.node, wait.node);
        queue
            .complete(
                &first,
                Ok(Outcome::CheckpointCommitted {
                    node: wait.node.clone(),
                    action: CheckpointAction::StartWait,
                }),
                now,
            )
            .unwrap();
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.next(now).unwrap().node, op.node);
        queue
            .complete(&op, Ok(Outcome::ObservedOperation(identity().run)), now)
            .unwrap();
        assert!(queue.next(now).is_none());
        let next = queue.next(now + Duration::from_millis(21)).unwrap();
        queue
            .complete(
                &next,
                Ok(Outcome::CheckpointObserved {
                    node: wait.node,
                    action: CheckpointAction::CheckWait,
                }),
                now,
            )
            .unwrap();
        assert!(queue.is_empty());
    }
    #[test]
    fn full_queue_and_inflight_normal_work_do_not_block_pause_and_meaning_cannot_change() {
        let now = Instant::now();
        let mut queue = PendingRequests::new(identity(), Name::new("root").unwrap());
        queue
            .accept(
                (0..32)
                    .map(|i| request(&format!("node/{i}"), RequestKind::SubmitOperation))
                    .collect(),
                now,
            )
            .unwrap();
        let first = queue.next(now).unwrap();
        let mut changed = first.clone();
        changed.argument = "other".into();
        assert!(queue.accept(vec![changed], now).is_err());
        assert_eq!(queue.len(), 32);
        queue.request_pause(now);
        let pause = queue.next(now).unwrap();
        assert_eq!(pause.kind, RequestKind::PauseExecutor);
        assert_eq!(queue.len(), 1);
        queue
            .complete(
                &first,
                Err(Error::Rpc(tonic::Status::unavailable("lost reply"))),
                now,
            )
            .unwrap();
        queue
            .complete(
                &pause,
                Err(Error::Rpc(tonic::Status::unavailable("offline"))),
                now,
            )
            .unwrap();
        assert!(queue.next(now).is_none());
        let pause = queue.next(now + Duration::from_secs(1)).unwrap();
        queue
            .complete(&pause, Ok(Outcome::ContextChanged), now)
            .unwrap();
        assert_eq!(queue.stop_state(), StopState::Attention);
        assert!(queue.is_empty());
    }
}
