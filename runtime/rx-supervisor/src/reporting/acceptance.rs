//! Authenticated historical registration custody, never permission to launch a process.
use super::*;
use rx_domain::component_transfer::{FreezeRecord, TargetAcceptance};

/// Only a successful authenticated client exchange can construct this value.
/// Stored or caller-supplied JSON is not fresh verification.
pub struct VerifiedAcceptance(TargetAcceptance);
impl VerifiedAcceptance {
    pub fn view(&self) -> &TargetAcceptance {
        &self.0
    }
}
fn verified(
    value: TargetAcceptance,
    peer: &Peer,
    scope: &Scope,
    original: &FreezeRecord,
) -> Result<VerifiedAcceptance> {
    if value.peer != *peer
        || value.scope != scope.id
        || value.component != scope.component
        || scope.reporter_session != peer.id
        || !scope.active
        || scope.component != scope.source_registration
        || value.original != *original
        || original.request.target_installation != peer.installation
    {
        return Err(Error::Invalid("target acceptance context differs"));
    }
    Ok(VerifiedAcceptance(value))
}
impl Client {
    pub async fn registration_acceptance(
        &mut self,
        scope: &VerifiedScope,
        original: &FreezeRecord,
    ) -> Result<VerifiedAcceptance> {
        if scope.0.reporter_session != self.peer.id
            || scope.0.component != scope.0.source_registration
            || original.request.target_installation != self.peer.installation
        {
            return Err(Error::Invalid("acceptance query context differs"));
        }
        let payload = self
            .transport
            .acceptance(wire::ReadAcceptance {
                session_id: self.peer.id.to_string(),
                scope_id: scope.0.id.to_string(),
                freeze_id: original.request.id.to_string(),
                binding_hash: binding_hash(),
            })
            .await?
            .into_inner();
        let value = decode(payload, "rx.registration-target-acceptance.v1")?;
        verified(value, &self.peer, &scope.0, original)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registration::{Binding, CatalogReference, Declaration, FreezeRequest, Registry};
    use rx_domain::component_transfer::{ContentVerification, ProcessOwnership, WorkUse};
    use rx_ports::{
        OutboxRecord, Record, Repository, SealedRepository, StoreError, StoredEvent, Transaction,
    };
    use rx_storage::SqliteRepository;
    use std::sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    };
    fn id() -> Id {
        Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
    }
    fn name(s: &str) -> Name {
        Name::new(s).unwrap()
    }
    struct Fault {
        inner: SqliteRepository,
        mode: Arc<AtomicU8>,
    }
    impl SealedRepository for Fault {
        fn sealed_namespaces(&self) -> rx_ports::Result<Vec<Name>> {
            self.inner.sealed_namespaces()
        }
    }
    impl Repository for Fault {
        fn transact<T>(
            &mut self,
            f: impl FnOnce(&mut dyn Transaction) -> rx_ports::Result<T>,
        ) -> rx_ports::Result<T> {
            let mode = self.mode.swap(0, Ordering::SeqCst);
            let result = self.inner.transact(|tx| {
                let value = f(tx)?;
                if mode == 1 {
                    return Err(StoreError::Unavailable("before commit".into()));
                }
                Ok(value)
            })?;
            if mode == 2 {
                Err(StoreError::Unavailable("after commit".into()))
            } else {
                Ok(result)
            }
        }
        fn pending_outbox_after(
            &mut self,
            a: Option<&Id>,
            l: usize,
        ) -> rx_ports::Result<Vec<OutboxRecord>> {
            self.inner.pending_outbox_after(a, l)
        }
        fn control_events_after(
            &mut self,
            a: Counter,
            l: usize,
        ) -> rx_ports::Result<Vec<StoredEvent>> {
            self.inner.control_events_after(a, l)
        }
        fn control_snapshot(&mut self) -> rx_ports::Result<(Counter, Vec<Record>)> {
            self.inner.control_snapshot()
        }
        fn journal_head(&mut self) -> rx_ports::Result<Counter> {
            self.inner.journal_head()
        }
        fn pending_outbox(&mut self, l: usize) -> rx_ports::Result<Vec<OutboxRecord>> {
            self.inner.pending_outbox(l)
        }
        fn snapshot(&mut self) -> rx_ports::Result<(Counter, Vec<Record>)> {
            self.inner.snapshot()
        }
        fn events_after(&mut self, a: Counter, l: usize) -> rx_ports::Result<Vec<StoredEvent>> {
            self.inner.events_after(a, l)
        }
    }
    #[test]
    fn authenticated_cut_is_atomic_idempotent_and_never_restores_local_authority() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.db");
        let mut registry = Registry::new(SqliteRepository::open(&path).unwrap());
        let declaration = Declaration {
            label: name("test"),
            catalog: CatalogReference {
                program: name("status"),
                digest: Digest::from_bytes([4; 32]),
            },
        };
        let component = registry.register(declaration.clone()).unwrap();
        let original = registry
            .freeze_for_transfer(FreezeRequest {
                id: id(),
                target_installation: id(),
            })
            .unwrap()
            .record()
            .clone();
        let history = registry
            .frozen_history(&original.request.id, Counter(0), 128)
            .unwrap();
        let peer = Peer {
            id: id(),
            principal: name("reporter"),
            peer_boot: id(),
            installation: original.request.target_installation.clone(),
            store_generation: id(),
            runtime_boot: id(),
            authentication_binding: Digest::from_bytes([3; 32]),
        };
        let scope = Scope {
            id: id(),
            component: component.registration.id.clone(),
            component_revision: Counter(1),
            source_registration: component.registration.id.clone(),
            source_revision: Counter(1),
            catalog: declaration.catalog.clone(),
            reporter_session: peer.id.clone(),
            issued_by: name("owner"),
            issued_at: TimePoint {
                clock_id: "p-clock".into(),
                ticks_ns: Counter(3),
            },
            active: true,
            continuation: None,
        };
        let value = TargetAcceptance {
            original: original.clone(),
            receipt_digest: Digest::from_bytes([6; 32]),
            accepted_at: scope.issued_at.clone(),
            component: scope.component.clone(),
            scope: scope.id.clone(),
            peer: peer.clone(),
            process_ownership: ProcessOwnership::NotTransferred,
            content_verification: ContentVerification::NotEstablished,
            work_use_permission: WorkUse::NotEvaluated,
        };
        for mutation in 0..6 {
            let mut forged = value.clone();
            match mutation {
                0 => forged.original.declarations_digest = Digest::from_bytes([9; 32]),
                1 => forged.original.history_head = Counter(999),
                2 => forged.original.request.target_installation = id(),
                3 => forged.peer.runtime_boot = id(),
                4 => forged.scope = id(),
                _ => forged.component = id(),
            }
            assert!(verified(forged, &peer, &scope, &original).is_err());
        }
        let evidence = verified(value.clone(), &peer, &scope, &original).unwrap();
        let mode = Arc::new(AtomicU8::new(1));
        let mut registry = Registry::new(Fault {
            inner: registry.into_repository(),
            mode: mode.clone(),
        });
        assert!(registry.record_platform_acceptance(&evidence).is_err());
        let mut store = registry.into_repository();
        assert!(!store.snapshot().unwrap().1.iter().any(|r| {
            r.key
                .as_str()
                .starts_with("components/platform-acceptance/")
        }));
        let mut registry = Registry::new(store);
        mode.store(2, Ordering::SeqCst);
        assert!(registry.record_platform_acceptance(&evidence).is_err());
        let mut store = registry.into_repository();
        let head = store.transact(|tx| tx.control_head()).unwrap();
        store.inner.close().unwrap();
        let mut registry = Registry::new(SqliteRepository::open_sealed_existing(&path).unwrap());
        assert_eq!(
            registry.record_platform_acceptance(&evidence).unwrap(),
            value
        );
        let mut changed = value.clone();
        changed.receipt_digest = Digest::from_bytes([8; 32]);
        assert!(
            registry
                .record_platform_acceptance(&verified(changed, &peer, &scope, &original).unwrap())
                .is_err()
        );
        let mut stranger = value.clone();
        let mut strangerscope = scope.clone();
        stranger.component = id();
        strangerscope.component = stranger.component.clone();
        strangerscope.source_registration = stranger.component.clone();
        assert!(
            registry
                .record_platform_acceptance(
                    &verified(stranger, &peer, &strangerscope, &original).unwrap()
                )
                .is_err()
        );
        let mut nextpeer = peer.clone();
        nextpeer.id = id();
        nextpeer.peer_boot = id();
        let mut nextscope = scope.clone();
        nextscope.id = id();
        nextscope.reporter_session = nextpeer.id.clone();
        let next = TargetAcceptance {
            peer: nextpeer.clone(),
            scope: nextscope.id.clone(),
            ..value.clone()
        };
        assert_eq!(
            registry
                .record_platform_acceptance(
                    &verified(next, &nextpeer, &nextscope, &original).unwrap()
                )
                .unwrap(),
            value
        );
        assert!(registry.register(declaration.clone()).is_err());
        assert!(
            registry
                .update(&component.registration.id, Counter(1), declaration.clone())
                .is_err()
        );
        assert!(
            registry
                .assign(&Binding {
                    registration: component.registration.id,
                    registration_revision: Counter(1),
                    catalog: declaration.catalog,
                    run: id(),
                    selection: name("status"),
                    instance: id()
                })
                .is_err()
        );
        assert_eq!(
            registry
                .frozen_history(&original.request.id, Counter(0), 128)
                .unwrap(),
            history
        );
        let frozen = serde_json::to_value(registry.frozen_source().unwrap()).unwrap();
        assert_eq!(
            frozen["platform_acceptance"],
            "RECORDED_FROM_AUTHENTICATED_PLATFORM"
        );
        assert_eq!(frozen["process_ownership"], "NOT_TRANSFERRED");
        assert_eq!(
            registry
                .into_repository()
                .transact(|tx| tx.control_head())
                .unwrap(),
            head
        );
    }
}
