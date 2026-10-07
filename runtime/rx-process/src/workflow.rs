//! Lower a pinned design report into the existing process compiler and parameter assets.
//! This is untrusted compilation input, never a publication or operating authority.
use crate::{ActionBinding, Flow, Node, NodeBody, ProcessSource, ResolvedProcess};
use rx_domain::{
    canonical,
    definition::{Reference, ValueType},
    intent::{Body, Intent},
    types::*,
    workflow::{Data, Done, KnownFailure, Quantity, Report, Resolved, UnknownOutcome},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub unit: Name,
    pub value_type: ValueType,
    pub frame: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    pub implementation: String,
    pub version: String,
    pub primitive: Name,
    pub host: Name,
    /// Program identity, resource set and upper timeout bound come from package data.
    pub intent: Intent,
    pub parameters: BTreeMap<Name, Parameter>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub schema: Name,
    pub process: Name,
    pub resolution: Reference,
    pub report: Report,
    pub templates: Vec<Template>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Value {
    pub value: Quantity,
    pub frame: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameters {
    pub schema: Name,
    pub resolution: Reference,
    pub node: Name,
    pub task: Name,
    pub slot_index: Counter,
    pub primitive: Name,
    pub values: BTreeMap<Name, Value>,
    pub done: Done,
    pub on_failure: KnownFailure,
    pub on_unknown: UnknownOutcome,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trace {
    pub binding: Name,
    pub node: Name,
    pub task: Name,
    pub skill_index: usize,
    pub parameters: ArtifactRef,
}
pub struct Compilation {
    pub source: ProcessSource,
    pub process: ResolvedProcess,
    pub assets: BTreeMap<Digest, Vec<u8>>,
    pub trace: Vec<Trace>,
}
fn name(value: impl Into<String>) -> Result<Name, String> {
    Name::new(value).map_err(|e| e.to_string())
}
fn text(value: &Resolved) -> Result<&str, String> {
    value.value.validate()?;
    match &value.value.data {
        Data::Text { value } if !value.is_empty() => Ok(value),
        _ => Err("implementation/version must be concrete nonempty text".into()),
    }
}
fn check(value: &Resolved) -> Result<(), String> {
    value.value.validate()?;
    if !value.value.concrete() || value.origins.is_empty() {
        return Err("concrete value and source provenance required".into());
    }
    Ok(())
}
fn timeout(value: &Resolved) -> Result<Counter, String> {
    check(value)?;
    let Data::Number { range } = &value.value.data else {
        return Err("numeric timeout required".into());
    };
    if value.frame.is_some() {
        return Err("timeout cannot have a coordinate frame".into());
    }
    let amount = range.exact().ok_or("concrete timeout required")?;
    let millis = match value.value.unit.as_str() {
        "s" => amount * 1000.0,
        "ms" => amount,
        _ => return Err("timeout requires explicit s or ms".into()),
    };
    if !millis.is_finite() || millis <= 0.0 || millis.fract() != 0.0 {
        return Err("timeout must be positive whole milliseconds".into());
    }
    Ok(Counter(
        format!("{millis:.0}")
            .parse()
            .map_err(|_| "timeout overflow")?,
    ))
}
pub fn compile(input: &Input) -> Result<Compilation, String> {
    let report = &input.report;
    if input.schema.as_str() != "rx.workflow-compile-input.v1"
        || report.schema.as_str() != "rx.workflow-resolution.v1"
        || input.resolution.catalog != report.request.workflow.catalog
        || input.resolution.revision != Counter(1)
        || canonical::digest("RX-WORKFLOW-RESOLUTION-v1", report).map_err(|e| e.to_string())?
            != input.resolution.digest
        || !report.valid
        || !report.concrete
        || !report.violations.is_empty()
        || report.status != "RESOLVED_NOT_QUALIFIED"
        || report.steps.is_empty()
        || report.steps.len() > 128
        || input.templates.is_empty()
        || input.templates.len() > 128
        || canonical::bytes(input).map_err(|e| e.to_string())?.len() > 1_048_576
    {
        return Err("workflow report reference, status or bounds differ".into());
    }
    report.request.validate_shape()?;
    let mut templates = BTreeMap::new();
    for template in &input.templates {
        template.intent.normalized().map_err(|e| e.to_string())?;
        if !matches!(template.intent.body, Body::Program(_))
            || templates
                .insert(
                    (
                        &template.implementation,
                        &template.version,
                        &template.primitive,
                    ),
                    template,
                )
                .is_some()
        {
            return Err("unique Program templates required".into());
        }
    }
    let mut nodes = vec![];
    let mut children = vec![];
    let mut bindings = BTreeMap::new();
    let mut assets = BTreeMap::new();
    let mut trace = vec![];
    let mut seen = BTreeSet::new();
    for (step_index, step) in report.steps.iter().enumerate() {
        if !seen.insert(&step.node) || step.skills.len() != 1 {
            return Err(
                "unique nodes and exactly one Skill per Task required by this compiler profile"
                    .into(),
            );
        }
        for value in step.properties.values() {
            check(value)?;
        }
        step.done.equals.validate()?;
        if !step.done.equals.concrete() {
            return Err("concrete done contract required".into());
        }
        let duration = timeout(
            step.properties
                .get(&step.timeout_property)
                .ok_or("timeout property missing")?,
        )?;
        for (skill_index, skill) in step.skills.iter().enumerate() {
            let implementation = text(&skill.implementation)?.to_owned();
            let version = text(&skill.version)?.to_owned();
            let template = templates.get(&(&implementation, &version, &skill.primitive))
                .ok_or_else(|| format!("nodes/{}/skills/{skill_index}: no exact package implementation/version/primitive", step.node))?;
            if template.parameters.keys().ne(skill.parameters.keys())
                || duration > template.intent.execution_timeout_ms
            {
                return Err(format!(
                    "nodes/{}: parameters or timeout exceed template",
                    step.node
                ));
            }
            let mut values = BTreeMap::new();
            for (parameter, property) in &skill.parameters {
                let value = step
                    .properties
                    .get(property)
                    .ok_or("mapped property missing")?;
                let contract = &template.parameters[parameter];
                let kind = match value.value.data {
                    Data::Number { .. } => ValueType::Number,
                    Data::Vector { .. } => ValueType::Vector,
                    Data::Text { .. } => ValueType::Text,
                    Data::Boolean { .. } => ValueType::Boolean,
                };
                if kind != contract.value_type
                    || value.value.unit != contract.unit
                    || value.frame != contract.frame
                {
                    return Err(format!(
                        "nodes/{}/properties/{property}: package parameter type/unit/frame differs",
                        step.node
                    ));
                }
                values.insert(
                    parameter.clone(),
                    Value {
                        value: value.value.clone(),
                        frame: value.frame.clone(),
                    },
                );
            }
            let payload = Parameters {
                schema: name("rx.workflow-parameters.v1")?,
                resolution: input.resolution.clone(),
                node: step.node.clone(),
                task: step.task.clone(),
                slot_index: report.request.slot_index,
                primitive: skill.primitive.clone(),
                values,
                done: step.done.clone(),
                on_failure: step.on_failure,
                on_unknown: step.on_unknown,
            };
            let bytes = canonical::bytes(&payload).map_err(|e| e.to_string())?;
            let parameters = ArtifactRef {
                sha256: rx_package::content_digest(&bytes),
                schema_id: payload.schema.clone(),
                size_bytes: Counter(bytes.len() as u64),
            };
            let mut intent = template.intent.clone();
            let Body::Program(goal) = &mut intent.body else {
                unreachable!("checked template");
            };
            goal.parameter_set = parameters.clone();
            intent.execution_timeout_ms = duration;
            let binding = name(format!("task/{step_index}/skill/{skill_index}"))?;
            nodes.push(Node {
                id: binding.clone(),
                body: NodeBody::Operation {
                    binding: binding.clone(),
                },
            });
            children.push(binding.clone());
            bindings.insert(
                binding.clone(),
                ActionBinding {
                    host: template.host.clone(),
                    intent,
                },
            );
            assets.insert(parameters.sha256, bytes);
            trace.push(Trace {
                binding,
                node: step.node.clone(),
                task: step.task.clone(),
                skill_index,
                parameters,
            });
        }
    }
    nodes.push(Node {
        id: name("workflow")?,
        body: NodeBody::Sequence { children },
    });
    let source = ProcessSource {
        schema: name("rx.process-source.v1")?,
        process: input.process.clone(),
        entry: name("main")?,
        flows: vec![Flow {
            id: name("main")?,
            root: name("workflow")?,
            nodes,
        }],
        conditions: BTreeMap::new(),
    };
    let process = crate::compile(&source, bindings).map_err(|e| e.to_string())?;
    Ok(Compilation {
        source,
        process,
        assets,
        trace,
    })
}
