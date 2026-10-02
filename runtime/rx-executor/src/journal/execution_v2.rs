//! Local expectations derived from authenticated P views; never sent as authority.
use super::*;
use rx_process_contract::execution_v2::executor as wire;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Begin {
    pub cell: Name,
    pub run: Id,
    pub ordinal: Counter,
    pub mandate: Id,
    pub expected_budget: Counter,
    pub expected_cell: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submit {
    pub cell: Name,
    pub node: Name,
    pub mandate: Id,
    pub expected_cell: Counter,
    pub expected_run: Counter,
    pub part: wire::PartBinding,
    pub workflow_node: Name,
    pub host: Name,
    pub intent_digest: Digest,
    pub epoch: Counter,
}
impl Begin {
    pub(super) fn validate(&self, scope: &Scope, l: &Logical) -> Result<()> {
        if self.cell != scope.cell
            || self.run != scope.run
            || self.ordinal != l.visit
            || l.node.as_str() != "production/part"
            || self.expected_budget.0 == 0
            || self.expected_cell.0 == 0
        {
            return invalid("v2 Part request scope differs");
        }
        Ok(())
    }
    pub(super) fn semantic_digest(&self) -> Result<Digest> {
        digest(
            "RX-E-BEGIN-PART-v2",
            &(&self.cell, &self.run, self.ordinal, &self.mandate),
        )
    }
}
impl Submit {
    pub(super) fn validate(&self, scope: &Scope, l: &Logical) -> Result<()> {
        self.part.validate().map_err(StoreError::Invalid)?;
        if self.cell != scope.cell
            || self.part.run != scope.run
            || self.part.ordinal != l.visit
            || self.node != l.node
            || self.expected_cell.0 == 0
            || self.expected_run.0 == 0
            || self.epoch.0 == 0
            || !self.part.parameters.contains_key(&self.workflow_node)
        {
            return invalid("v2 node request scope differs");
        }
        Ok(())
    }
    pub(super) fn semantic_digest(&self) -> Result<Digest> {
        digest(
            "RX-E-SUBMIT-v2",
            &(
                &self.cell,
                &self.node,
                &self.mandate,
                &self.part,
                &self.workflow_node,
                &self.host,
                self.intent_digest,
                self.epoch,
            ),
        )
    }
    pub(super) fn reply(&self, a: &wire::Admission) -> Result<()> {
        a.validate().map_err(StoreError::Integrity)?;
        let b = &a.binding;
        let s = &b.selection;
        if b.mandate != self.mandate
            || b.publication != self.part.publication
            || b.policy != self.part.policy
            || b.report != self.part.report
            || a.host != self.host
            || s.run != self.part.run
            || s.part != self.part.part
            || s.ordinal != self.part.ordinal
            || s.slot_ordinal != self.part.slot_ordinal
            || s.slot != self.part.slot
            || s.object != self.part.object
            || s.object_values_digest != self.part.object_values_digest
            || s.candidate != self.part.candidate
            || s.configuration_digest != self.part.configuration.sha256
            || s.node != self.workflow_node
            || self.part.parameters.get(&s.node) != Some(&s.parameter)
            || s.intent_digest != self.intent_digest
            || s.authority_generation != self.epoch
        {
            return invalid("v2 admission differs from pinned Part/node");
        }
        Ok(())
    }
}
pub(super) fn part_reply(b: &Begin, p: &wire::Part) -> Result<()> {
    p.validate().map_err(StoreError::Integrity)?;
    if p.binding.run != b.run
        || p.binding.ordinal != b.ordinal
        || p.state.revision != Counter(1)
        || p.state.disposition != rx_process_contract::execution::PartDisposition::InProgress
    {
        return invalid("v2 Part reply differs");
    }
    Ok(())
}
