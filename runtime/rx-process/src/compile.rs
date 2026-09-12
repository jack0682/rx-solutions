use crate::model::*;
use rx_domain::{canonical, types::*};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, thiserror::Error)]
#[error("process compile error at {location}: {reason}")]
pub struct Error {
    pub location: String,
    pub reason: String,
}
fn error(location: impl Into<String>, reason: impl Into<String>) -> Error {
    Error {
        location: location.into(),
        reason: reason.into(),
    }
}

pub fn compile(
    source: &ProcessSource,
    bindings: BTreeMap<Name, ActionBinding>,
) -> Result<ResolvedProcess, Error> {
    let report = rx_process_contract::source_validation::validate(source);
    if let Some(issue) = report.issues.first() {
        return Err(error(&issue.location, &issue.message));
    }
    let flows: BTreeMap<_, _> = source.flows.iter().map(|flow| (&flow.id, flow)).collect();
    for flow in &source.flows {
        for node in &flow.nodes {
            if let NodeBody::Operation { binding } = &node.body
                && !bindings.contains_key(binding)
            {
                return Err(error(node.id.as_str(), "unresolved action binding"));
            }
        }
    }
    let mut normalized_bindings = bindings;
    for (id, binding) in &mut normalized_bindings {
        binding.intent = binding
            .intent
            .normalized()
            .map_err(|e| error(id.as_str(), e.to_string()))?;
    }
    let entry = *flows
        .get(&source.entry)
        .ok_or_else(|| error(source.entry.as_str(), "entry flow missing"))?;
    let mut expander = Expander {
        source,
        flows,
        remaining: 4096,
        ids: BTreeSet::new(),
        visited_flows: BTreeSet::new(),
        used_bindings: BTreeSet::new(),
    };
    let root = expander.expand(entry, &entry.root, &[], &mut vec![], 0)?;
    if expander.visited_flows.len() != source.flows.len() {
        return Err(error("source", "unreachable flow definition"));
    }
    normalized_bindings.retain(|id, _| expander.used_bindings.contains(id));
    resources(&root, &normalized_bindings)?;
    let mut canonical_source = source.clone();
    canonical_source.flows.sort_by(|a, b| a.id.cmp(&b.id));
    for flow in &mut canonical_source.flows {
        flow.nodes.sort_by(|a, b| a.id.cmp(&b.id));
    }
    Ok(ResolvedProcess {
        schema: Name::new("rx.resolved-process.v1").expect("schema"),
        package_digest: None,
        source_digest: rx_package::content_digest(
            &canonical::bytes(&canonical_source).map_err(|e| error("source", e.to_string()))?,
        ),
        process: source.process.clone(),
        root,
        bindings: normalized_bindings,
        conditions: source.conditions.clone(),
    })
}
struct Expander<'a> {
    source: &'a ProcessSource,
    flows: BTreeMap<&'a Name, &'a Flow>,
    remaining: usize,
    ids: BTreeSet<Name>,
    visited_flows: BTreeSet<Name>,
    used_bindings: BTreeSet<Name>,
}
impl Expander<'_> {
    fn expand(
        &mut self,
        flow: &Flow,
        node_id: &Name,
        path: &[String],
        calls: &mut Vec<Name>,
        depth: usize,
    ) -> Result<CompiledNode, Error> {
        if depth > 64 || self.remaining == 0 {
            return Err(error(node_id.as_str(), "expanded plan limit"));
        }
        self.remaining -= 1;
        self.visited_flows.insert(flow.id.clone());
        let node = flow
            .nodes
            .iter()
            .find(|node| &node.id == node_id)
            .ok_or_else(|| error(node_id.as_str(), "node missing"))?;
        let source = SourceLocation {
            flow: flow.id.clone(),
            node: node.id.clone(),
            instantiation: path.to_vec(),
        };
        let digest = canonical::digest("RX-PROCESS-NODE-v1", &(&self.source.process, &source))
            .map_err(|e| error(node_id.as_str(), e.to_string()))?;
        let id = Name::new(format!("node/{digest}"))
            .map_err(|e| error(node_id.as_str(), e.to_string()))?;
        if !self.ids.insert(id.clone()) {
            return Err(error(node_id.as_str(), "expanded identity collision"));
        }
        let body = match &node.body {
            NodeBody::Sequence { children } | NodeBody::ParallelAll { children } => {
                let mut expanded = Vec::new();
                for child in children {
                    expanded.push(self.expand(flow, child, path, calls, depth + 1)?);
                }
                if matches!(node.body, NodeBody::Sequence { .. }) {
                    CompiledBody::Sequence { children: expanded }
                } else {
                    CompiledBody::ParallelAll { children: expanded }
                }
            }
            NodeBody::Branch {
                condition,
                when_true,
                when_false,
            } => CompiledBody::Branch {
                condition: condition.clone(),
                when_true: Box::new(self.expand(flow, when_true, path, calls, depth + 1)?),
                when_false: Box::new(self.expand(flow, when_false, path, calls, depth + 1)?),
            },
            NodeBody::Repeat { count, child } => {
                let mut children = Vec::new();
                for iteration in 0..count.0 {
                    let mut instance = path.to_vec();
                    instance.push(format!("repeat:{}/{}", node.id, iteration));
                    children.push(self.expand(flow, child, &instance, calls, depth + 1)?);
                }
                CompiledBody::Sequence { children }
            }
            NodeBody::Call { flow: callee } => {
                if calls.contains(callee) || callee == &flow.id {
                    return Err(error(node.id.as_str(), "recursive flow call"));
                }
                let called = *self
                    .flows
                    .get(callee)
                    .ok_or_else(|| error(node.id.as_str(), "flow missing"))?;
                let mut instance = path.to_vec();
                instance.push(format!("call:{}/{}", flow.id, node.id));
                calls.push(flow.id.clone());
                let child = self.expand(called, &called.root, &instance, calls, depth + 1)?;
                calls.pop();
                CompiledBody::Sequence {
                    children: vec![child],
                }
            }
            NodeBody::Operation { binding } => {
                self.used_bindings.insert(binding.clone());
                CompiledBody::Operation {
                    binding: binding.clone(),
                }
            }
            NodeBody::Wait {
                condition,
                timeout_ns,
            } => CompiledBody::Wait {
                condition: condition.clone(),
                timeout_ns: *timeout_ns,
            },
            NodeBody::Intervention { procedure } => CompiledBody::Intervention {
                procedure: procedure.clone(),
            },
        };
        Ok(CompiledNode { id, source, body })
    }
}
fn resources(
    node: &CompiledNode,
    bindings: &BTreeMap<Name, ActionBinding>,
) -> Result<BTreeSet<Name>, Error> {
    let mut result = BTreeSet::new();
    match &node.body {
        CompiledBody::Operation { binding } => {
            result.extend(bindings[binding].intent.resource_set.iter().cloned())
        }
        CompiledBody::Sequence { children } | CompiledBody::ParallelAll { children } => {
            for child in children {
                let child_resources = resources(child, bindings)?;
                if matches!(node.body, CompiledBody::ParallelAll { .. })
                    && !result.is_disjoint(&child_resources)
                {
                    return Err(error(
                        node.source.node.as_str(),
                        "parallel branches share a resolved command/support resource",
                    ));
                }
                result.extend(child_resources);
            }
        }
        CompiledBody::Branch {
            when_true,
            when_false,
            ..
        } => {
            result.extend(resources(when_true, bindings)?);
            result.extend(resources(when_false, bindings)?);
        }
        _ => {}
    }
    Ok(result)
}
