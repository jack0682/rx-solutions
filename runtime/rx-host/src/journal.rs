use crate::model::*;
use rx_domain::{canonical, types::*};
use rx_ports::{Document, Record, Transaction};
use serde::{Serialize, de::DeserializeOwned};

pub fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).expect("generated UUID")
}
pub fn name(s: impl Into<String>) -> Name {
    Name::new(s).expect("internal name")
}
pub fn key(kind: &str, value: impl Serialize) -> Name {
    name(format!(
        "{kind}/{}",
        canonical::digest("RX-HOST-KEY-v1", &value).expect("internal key")
    ))
}
pub fn doc<T: Serialize>(schema: &str, value: &T) -> rx_ports::Result<Document> {
    Ok(Document {
        schema: name(schema),
        value: serde_json::to_value(value)
            .map_err(|e| rx_ports::StoreError::Invalid(e.to_string()))?,
    })
}
pub fn decode<T: DeserializeOwned>(r: &Record, schema: &str) -> rx_ports::Result<T> {
    if r.document.schema.as_str() != schema {
        return Err(rx_ports::StoreError::Integrity("Host record schema".into()));
    }
    canonical::decode_json(
        &canonical::bytes(&r.document.value)
            .map_err(|e| rx_ports::StoreError::Integrity(e.to_string()))?,
    )
    .map_err(|e| rx_ports::StoreError::Integrity(e.to_string()))
}
pub fn record_delivery(
    tx: &mut dyn Transaction,
    record: &mut DeliveryRecord,
    expected: Option<Counter>,
) -> rx_ports::Result<()> {
    let (sequence, _) = next_sequence(tx)?;
    record.journal_seq = sequence;
    tx.put(
        &key("delivery", &record.operation),
        expected,
        &doc("rx.host.delivery.v1", record)?,
    )?;
    tx.put(
        &name(format!("delivery-history/{:020}", record.journal_seq.0)),
        None,
        &doc("rx.host.delivery.v1", record)?,
    )?;
    Ok(())
}
pub fn next_sequence(tx: &mut dyn Transaction) -> rx_ports::Result<(Counter, Id)> {
    let m = tx
        .get(&name("host/meta"))?
        .ok_or(rx_ports::StoreError::Integrity("Host meta missing".into()))?;
    let mut meta: HostMeta = decode(&m, "rx.host.meta.v1")?;
    meta.delivery_seq = meta
        .delivery_seq
        .increment()
        .map_err(|e| rx_ports::StoreError::Integrity(e.to_string()))?;
    tx.put(
        &name("host/meta"),
        Some(m.revision),
        &doc("rx.host.meta.v1", &meta)?,
    )?;
    Ok((meta.delivery_seq, meta.delivery_journal))
}
