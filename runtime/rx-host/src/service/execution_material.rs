//! Installation-owned acquisition of common execution material; never a Python profile field.
use super::{Result, config::PinnedFile};
use rx_domain::types::*;
use rx_process_contract::execution_v2::{self as v2, host_inputs::VerifiedDomain};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactChunks {
    pub reference: ArtifactRef,
    pub parts: Vec<PinnedFile>,
}
impl ArtifactChunks {
    fn read(&self, limit: usize) -> Result<Vec<u8>> {
        if self.parts.is_empty()
            || self.parts.len() > 32
            || self.reference.size_bytes.0 == 0
            || self.reference.size_bytes.0 > limit as u64
        {
            return Err("execution material bounds".into());
        }
        let mut bytes = Vec::new();
        for part in &self.parts {
            let next = part.read(false)?;
            if bytes
                .len()
                .checked_add(next.len())
                .is_none_or(|n| n > limit)
            {
                return Err("execution material exceeds bound".into());
            }
            bytes.extend(next);
        }
        v2::verify_artifact(&bytes, &self.reference, limit)?;
        Ok(bytes)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    pub policy: ArtifactChunks,
    pub inputs: ArtifactChunks,
    pub index: ArtifactChunks,
}
impl Material {
    pub fn verify(&self) -> Result<VerifiedDomain> {
        let policy = self.policy.read(v2::MAX_POLICY_BYTES)?;
        let inputs = self.inputs.read(v2::MAX_DEFINITION_BYTES as usize)?;
        let index = self.index.read(v2::MAX_INDEX_BYTES)?;
        Ok(VerifiedDomain::verify(&policy, &inputs, &index)?)
    }
}
