use rx_domain::types::*;
use rx_ports::{Repository, Transaction};
use rx_storage::SqliteRepository;
use std::sync::{Arc, Mutex};
pub type Hook = Box<dyn FnOnce() -> rx_ports::Result<bool> + Send>;
#[derive(Default)]
pub struct Fault {
    pub key: Option<Name>,
    pub after_staging: Option<Hook>,
    pub fail_after_put: Option<String>,
}
#[derive(Clone)]
pub struct SharedStore {
    pub inner: Arc<Mutex<SqliteRepository>>,
    pub fault: Arc<Mutex<Fault>>,
}
impl SharedStore {
    pub fn new(path: impl AsRef<std::path::Path>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SqliteRepository::open(path).unwrap())),
            fault: Arc::new(Mutex::new(Fault::default())),
        }
    }
    pub fn hook(&self, key: Name, hook: Hook) {
        *self.fault.lock().unwrap() = Fault {
            key: Some(key),
            after_staging: Some(hook),
            fail_after_put: None,
        };
    }
    pub fn fail_after_put(&self, prefix: &str) {
        self.fault.lock().unwrap().fail_after_put = Some(prefix.into());
    }
    pub fn rows(&self) -> Vec<rx_ports::Record> {
        self.inner.lock().unwrap().snapshot().unwrap().1
    }
}
impl Repository for SharedStore {
    fn transact<T>(
        &mut self,
        f: impl FnOnce(&mut dyn Transaction) -> rx_ports::Result<T>,
    ) -> rx_ports::Result<T> {
        let fault = self.fault.clone();
        let mut lose = false;
        let value = self.inner.lock().unwrap().transact(|tx| {
            let value = f(&mut ProbeTransaction {
                inner: tx,
                fault: fault.clone(),
            })?;
            let key = fault.lock().unwrap().key.clone();
            if let Some(key) = key
                && tx.get(&key)?.is_some()
            {
                let hook = fault.lock().unwrap().after_staging.take();
                if let Some(hook) = hook {
                    lose = hook()?;
                }
            }
            Ok(value)
        })?;
        if lose {
            Err(rx_ports::StoreError::Unavailable(
                "replacement test reply lost after commit".into(),
            ))
        } else {
            Ok(value)
        }
    }
    fn pending_outbox_after(
        &mut self,
        a: Option<&Id>,
        l: usize,
    ) -> rx_ports::Result<Vec<rx_ports::OutboxRecord>> {
        self.inner.lock().unwrap().pending_outbox_after(a, l)
    }
    fn control_events_after(
        &mut self,
        a: Counter,
        l: usize,
    ) -> rx_ports::Result<Vec<rx_ports::StoredEvent>> {
        self.inner.lock().unwrap().control_events_after(a, l)
    }
    fn control_snapshot(&mut self) -> rx_ports::Result<(Counter, Vec<rx_ports::Record>)> {
        self.inner.lock().unwrap().control_snapshot()
    }
    fn journal_head(&mut self) -> rx_ports::Result<Counter> {
        self.inner.lock().unwrap().journal_head()
    }
    fn pending_outbox(&mut self, l: usize) -> rx_ports::Result<Vec<rx_ports::OutboxRecord>> {
        self.inner.lock().unwrap().pending_outbox(l)
    }
    fn snapshot(&mut self) -> rx_ports::Result<(Counter, Vec<rx_ports::Record>)> {
        self.inner.lock().unwrap().snapshot()
    }
    fn events_after(
        &mut self,
        a: Counter,
        l: usize,
    ) -> rx_ports::Result<Vec<rx_ports::StoredEvent>> {
        self.inner.lock().unwrap().events_after(a, l)
    }
}

struct ProbeTransaction<'a> {
    inner: &'a mut dyn Transaction,
    fault: Arc<Mutex<Fault>>,
}
impl Transaction for ProbeTransaction<'_> {
    fn append_control(
        &mut self,
        id: &Id,
        e: &rx_ports::Record,
        d: &rx_ports::Document,
    ) -> rx_ports::Result<Counter> {
        self.inner.append_control(id, e, d)
    }
    fn control_head(&mut self) -> rx_ports::Result<Counter> {
        self.inner.control_head()
    }
    fn outbox(&mut self, id: &Id) -> rx_ports::Result<Option<rx_ports::OutboxRecord>> {
        self.inner.outbox(id)
    }
    fn scan(&mut self, p: &str) -> rx_ports::Result<Vec<rx_ports::Record>> {
        self.inner.scan(p)
    }
    fn get(&mut self, k: &Name) -> rx_ports::Result<Option<rx_ports::Record>> {
        self.inner.get(k)
    }
    fn put(
        &mut self,
        k: &Name,
        e: Option<Counter>,
        d: &rx_ports::Document,
    ) -> rx_ports::Result<rx_ports::Record> {
        let row = self.inner.put(k, e, d)?;
        let mut fault = self.fault.lock().unwrap();
        if fault
            .fail_after_put
            .as_ref()
            .is_some_and(|p| k.as_str().starts_with(p))
        {
            fault.fail_after_put = None;
            return Err(rx_ports::StoreError::Unavailable(
                "injected partial staging rollback after new binding put".into(),
            ));
        }
        Ok(row)
    }
    fn lookup(
        &mut self,
        s: &rx_ports::RequestScope,
    ) -> rx_ports::Result<Option<rx_ports::SavedRequest>> {
        self.inner.lookup(s)
    }
    fn remember(
        &mut self,
        s: &rx_ports::RequestScope,
        r: &rx_ports::SavedRequest,
    ) -> rx_ports::Result<()> {
        self.inner.remember(s, r)
    }
    fn append(&mut self, id: &Id, d: &rx_ports::Document) -> rx_ports::Result<Counter> {
        self.inner.append(id, d)
    }
    fn enqueue(&mut self, id: &Id, d: &rx_ports::Document) -> rx_ports::Result<()> {
        self.inner.enqueue(id, d)
    }
    fn transition_outbox(
        &mut self,
        id: &Id,
        e: rx_ports::OutboxState,
        n: rx_ports::OutboxState,
    ) -> rx_ports::Result<()> {
        self.inner.transition_outbox(id, e, n)
    }
}
