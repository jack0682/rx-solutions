//! Generated BT.CPP XML. RX nodes require the matching executor plugin; no raw scripts/includes.
use crate::model::*;
pub fn generate(process: &ResolvedProcess) -> Result<String, String> {
    let digest = crate::frontier::resolved_digest(process)?;
    let mut result = format!(
        "<root BTCPP_format=\"4\" main_tree_to_execute=\"RXMain\">\n  <!-- RX resolved process: {digest} -->\n  <BehaviorTree ID=\"RXMain\">\n"
    );
    emit(&process.root, 2, &mut result)?;
    result.push_str("  </BehaviorTree>\n</root>\n");
    Ok(result)
}
fn emit(node: &CompiledNode, depth: usize, output: &mut String) -> Result<(), String> {
    if depth > 68 {
        return Err("XML depth limit".into());
    }
    let pad = "  ".repeat(depth);
    let id = escape(node.id.as_str());
    let (tag, attributes, children): (&str, String, Vec<&CompiledNode>) = match &node.body {
        CompiledBody::Sequence { children } => {
            ("RXSequence", String::new(), children.iter().collect())
        }
        CompiledBody::ParallelAll { children } => {
            ("RXParallelAll", String::new(), children.iter().collect())
        }
        CompiledBody::Branch {
            condition,
            when_true,
            when_false,
        } => (
            "RXBranch",
            format!(" condition_id=\"{}\"", escape(condition.as_str())),
            vec![when_true, when_false],
        ),
        CompiledBody::Operation { binding } => (
            "RXOperation",
            format!(" binding=\"{}\"", escape(binding.as_str())),
            vec![],
        ),
        CompiledBody::Wait {
            condition,
            timeout_ns,
        } => (
            "RXWait",
            format!(
                " condition_id=\"{}\" timeout_ns=\"{}\"",
                escape(condition.as_str()),
                timeout_ns.0
            ),
            vec![],
        ),
        CompiledBody::Intervention { procedure } => (
            "RXIntervention",
            format!(" procedure_digest=\"{}\"", procedure.sha256),
            vec![],
        ),
    };
    if children.is_empty() {
        output.push_str(&format!("{pad}<{tag} node_id=\"{id}\"{attributes}/>\n"));
    } else {
        output.push_str(&format!("{pad}<{tag} node_id=\"{id}\"{attributes}>\n"));
        for child in children {
            emit(child, depth + 1, output)?;
        }
        output.push_str(&format!("{pad}</{tag}>\n"));
    }
    Ok(())
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
