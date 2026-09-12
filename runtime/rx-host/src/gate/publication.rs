use super::*;
use crate::publication::CONTROL_VIEW;
use crate::publication::{Destination, PublicationAck, PublicationChunk, PublicationCursor};

impl<N: NativeAdapter, C: Clock, H: BoundaryHook> Host<N, C, H> {
    /// Local publisher API, not an RPC. It conveys evidence and cannot enter native submission.
    pub fn publication_chunk(&self, destination: &Destination) -> Result<PublicationChunk> {
        let mut core = self.lock()?;
        check_destination(&core, destination)?;
        let meta = core.store.transact(|tx| {
            decode::<HostMeta>(
                &tx.get(&name("host/meta"))?
                    .ok_or(rx_ports::StoreError::Integrity("Host meta missing".into()))?,
                "rx.host.meta.v1",
            )
        })?;
        let cursor = core
            .store
            .transact(|tx| {
                tx.get(&key(
                    "publisher",
                    (&destination, &meta.evidence_journal, CONTROL_VIEW),
                ))?
                .map(|r| decode::<PublicationCursor>(&r, "rx.host.publisher-cursor.v1"))
                .transpose()
            })?
            .unwrap_or(PublicationCursor {
                journal: meta.evidence_journal.clone(),
                through: Counter(0),
                platform_sequence: Counter(0),
            });
        let tail = core.store.journal_head()?;
        if cursor.journal != meta.evidence_journal || cursor.through > tail {
            return Err(HostError::Invalid(
                "evidence journal no longer covers durable acknowledgment".into(),
            ));
        }
        let first = cursor
            .through
            .increment()
            .map_err(|e| HostError::Invalid(e.to_string()))?;
        let events = core.store.events_after(cursor.through, 128)?;
        let mut records = Vec::new();
        for (index, event) in events.into_iter().enumerate() {
            if event.seq.0 != first.0 + index as u64 {
                return Err(HostError::Invalid("retained evidence gap".into()));
            }
            records.push(
                rx_domain::canonical::decode_json(
                    &rx_domain::canonical::bytes(&event.document.value)
                        .map_err(|e| HostError::Invalid(e.to_string()))?,
                )
                .map_err(|e| HostError::Invalid(e.to_string()))?,
            );
            if event.document.schema.as_str() != "rx.host.evidence.v1" {
                return Err(HostError::Invalid("evidence journal schema".into()));
            }
        }
        Ok(PublicationChunk {
            host: core
                .bindings
                .values()
                .next()
                .expect("nonempty bindings")
                .host
                .clone(),
            boot: core.boot.clone(),
            journal: meta.evidence_journal,
            first,
            tail,
            records,
            cursor,
        })
    }
    pub fn acknowledge_publication(
        &self,
        destination: &Destination,
        ack: PublicationAck,
    ) -> Result<PublicationCursor> {
        let mut core = self.lock()?;
        check_destination(&core, destination)?;
        let tail = core.store.journal_head()?;
        if ack.installation != destination.installation
            || ack.store_generation != destination.store_generation
            || ack.view_id != CONTROL_VIEW
            || ack.through > tail
        {
            return Err(HostError::Invalid(
                "acknowledgment destination/range mismatch".into(),
            ));
        }
        Ok(core.store.transact(|tx| {
            let meta: HostMeta = decode(
                &tx.get(&name("host/meta"))?
                    .ok_or(rx_ports::StoreError::Integrity("Host meta missing".into()))?,
                "rx.host.meta.v1",
            )?;
            if ack.journal != meta.evidence_journal {
                return Err(rx_ports::StoreError::Invalid(
                    "acknowledgment journal mismatch".into(),
                ));
            }
            let k = key(
                "publisher",
                (&destination, &meta.evidence_journal, CONTROL_VIEW),
            );
            let row = tx.get(&k)?;
            if let Some(row) = &row {
                let previous: PublicationCursor = decode(row, "rx.host.publisher-cursor.v1")?;
                if ack.through <= previous.through
                    && ack.platform_sequence <= previous.platform_sequence
                {
                    return Ok(previous);
                }
                if ack.through < previous.through
                    || ack.platform_sequence < previous.platform_sequence
                {
                    return Err(rx_ports::StoreError::Integrity(
                        "acknowledgment cursor regression".into(),
                    ));
                }
            }
            let cursor = PublicationCursor {
                journal: ack.journal,
                through: ack.through,
                platform_sequence: ack.platform_sequence,
            };
            tx.put(
                &k,
                row.map(|r| r.revision),
                &doc("rx.host.publisher-cursor.v1", &cursor)?,
            )?;
            Ok(cursor)
        })?)
    }
}
fn check_destination<N>(core: &Core<N>, destination: &Destination) -> Result<()> {
    if core
        .bindings
        .values()
        .any(|b| b.platform != destination.platform)
    {
        return Err(HostError::Forbidden);
    }
    Ok(())
}
