//! Host-owned immutable execution material. Importing material does not acknowledge qualification.
use super::*;
use rx_process_contract::execution_v2::{
    self as v2,
    host_inputs::{DomainMaterial, VerifiedDomain},
};
use serde::{Deserialize, Serialize};
const CHUNK: usize = 64 * 1024;
const DOMAIN: &str = "rx.host.execution-domain.v2";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredArtifact {
    reference: ArtifactRef,
    chunks: u32,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredDomain {
    policy: StoredArtifact,
    inputs: StoredArtifact,
    index: StoredArtifact,
}
fn store_blob(
    tx: &mut dyn rx_ports::Transaction,
    reference: &ArtifactRef,
    bytes: &[u8],
) -> rx_ports::Result<StoredArtifact> {
    for (i, part) in bytes.chunks(CHUNK).enumerate() {
        let k = key("execution-material", (reference.sha256, Counter(i as u64)));
        let data = doc("rx.host.execution-material-chunk.v2", &part)?;
        if let Some(old) = tx.get(&k)? {
            if old.document != data {
                return Err(rx_ports::StoreError::Integrity(
                    "execution material changed".into(),
                ));
            }
        } else {
            tx.put(&k, None, &data)?;
        }
    }
    Ok(StoredArtifact {
        reference: reference.clone(),
        chunks: bytes.len().div_ceil(CHUNK) as u32,
    })
}
fn load_blob(
    tx: &mut dyn rx_ports::Transaction,
    source: &StoredArtifact,
    limit: usize,
) -> rx_ports::Result<Vec<u8>> {
    let size = usize::try_from(source.reference.size_bytes.0)
        .map_err(|_| rx_ports::StoreError::Integrity("material size".into()))?;
    if size == 0 || size > limit || source.chunks as usize != size.div_ceil(CHUNK) {
        return Err(rx_ports::StoreError::Integrity("material bounds".into()));
    }
    let mut bytes = Vec::with_capacity(size);
    for i in 0..source.chunks {
        let row = tx
            .get(&key(
                "execution-material",
                (source.reference.sha256, Counter(u64::from(i))),
            ))?
            .ok_or(rx_ports::StoreError::Integrity(
                "approved material missing".into(),
            ))?;
        let part: Vec<u8> = decode(&row, "rx.host.execution-material-chunk.v2")?;
        if part.len() != CHUNK.min(size - bytes.len()) {
            return Err(rx_ports::StoreError::Integrity(
                "material chunk size".into(),
            ));
        }
        bytes.extend(part);
    }
    v2::verify_artifact(&bytes, &source.reference, limit)
        .map_err(rx_ports::StoreError::Integrity)?;
    Ok(bytes)
}
pub(super) fn domain<N>(
    core: &mut Core<N>,
    reference: &ArtifactRef,
) -> Result<Arc<VerifiedDomain>> {
    if let Some(value) = core.execution_domains.get(&reference.sha256) {
        if value.reference() != reference {
            return Err(HostError::Conflict);
        }
        return Ok(value.clone());
    }
    let material = core.store.transact(|tx| {
        let row = tx.get(&key("execution-domain", reference.sha256))?.ok_or(
            rx_ports::StoreError::Integrity("Host approval material unavailable".into()),
        )?;
        let saved: StoredDomain = decode(&row, DOMAIN)?;
        if saved.policy.reference != *reference {
            return Err(rx_ports::StoreError::Integrity(
                "Host policy reference differs".into(),
            ));
        }
        Ok(DomainMaterial {
            policy: load_blob(tx, &saved.policy, v2::MAX_POLICY_BYTES)?,
            inputs: load_blob(tx, &saved.inputs, v2::MAX_DEFINITION_BYTES as usize)?,
            index: load_blob(tx, &saved.index, v2::MAX_INDEX_BYTES)?,
        })
    })?;
    let checked = Arc::new(
        VerifiedDomain::verify(&material.policy, &material.inputs, &material.index)
            .map_err(invalid)?,
    );
    core.execution_domains
        .insert(reference.sha256, checked.clone());
    Ok(checked)
}
impl<N: NativeAdapter, C: Clock, H: BoundaryHook> Host<N, C, H> {
    /// Trusted installation import, before a platform is bound or admission armed. The supplied
    /// type is constructible only by full domain verification; this still grants no operation.
    pub fn install_execution_material(&self, checked: VerifiedDomain) -> Result<()> {
        let mut core = self.lock()?;
        if core.caller.is_some() || !core.armed.is_empty() {
            return Err(HostError::Guard);
        }
        let reference = checked.reference().clone();
        let material = checked.material();
        core.store.transact(|tx| {
            let saved = StoredDomain {
                policy: store_blob(tx, &reference, &material.policy)?,
                inputs: store_blob(tx, &checked.policy().definition_closure, &material.inputs)?,
                index: store_blob(tx, &checked.policy().report_index, &material.index)?,
            };
            let k = key("execution-domain", reference.sha256);
            let value = doc(DOMAIN, &saved)?;
            if let Some(old) = tx.get(&k)? {
                if old.document != value {
                    return Err(rx_ports::StoreError::Integrity(
                        "immutable domain differs".into(),
                    ));
                }
            } else {
                tx.put(&k, None, &value)?;
            }
            let meta_key = name("host/meta");
            let row = tx
                .get(&meta_key)?
                .ok_or(rx_ports::StoreError::Integrity("Host meta missing".into()))?;
            let mut meta: HostMeta = decode(&row, "rx.host.meta.v1")?;
            meta.execution_reader =
                Some(meta.execution_reader.unwrap_or(Counter(2)).max(Counter(2)));
            tx.put(
                &meta_key,
                Some(row.revision),
                &doc("rx.host.meta.v1", &meta)?,
            )?;
            Ok(())
        })?;
        core.execution_domains
            .insert(reference.sha256, Arc::new(checked));
        Ok(())
    }
}
