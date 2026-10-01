use rx_domain::canonical;
use rx_process::*;
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
};
fn main() {
    if let Err(error) = run() {
        eprintln!("rx-process-compile: {error}");
        std::process::exit(1);
    }
}
fn input<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(1_048_577)
        .read_to_end(&mut bytes)?;
    Ok(canonical::decode_json(&bytes)?)
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err(
            "usage: rx-process-compile SOURCE.json BINDINGS.json NEW_OUTPUT_DIRECTORY | --bundle INPUT.json NEW_OUTPUT_DIRECTORY | --workflow INPUT.json NEW_OUTPUT_DIRECTORY".into(),
        );
    }
    let mut extra = BTreeMap::new();
    let (process, provenance) = if args[0] == "--workflow" {
        let bundle: workflow::Input = input(Path::new(&args[1]))?;
        let compiled = workflow::compile(&bundle)?;
        extra.insert(
            "workflow-source.json".to_owned(),
            canonical::bytes(&compiled.source)?,
        );
        extra.insert(
            "workflow-trace.json".to_owned(),
            canonical::bytes(&compiled.trace)?,
        );
        for (digest, bytes) in compiled.assets {
            extra.insert(format!("parameters-{digest}.json"), bytes);
        }
        (
            compiled.process,
            Some(
                serde_json::json!({"resolution":bundle.resolution,"workflow":bundle.report.request.workflow,"slot_index":bundle.report.request.slot_index,"compile_input_digest":canonical::digest("RX-WORKFLOW-COMPILE-INPUT-v1", &bundle)?,"templates_digest":canonical::digest("RX-WORKFLOW-TEMPLATES-v1", &bundle.templates)?}),
            ),
        )
    } else {
        let (source, bindings, provenance) = if args[0] == "--bundle" {
            let bundle: rx_process_contract::compile_input::CompileInput =
                input(Path::new(&args[1]))?;
            let source = bundle.validate()?;
            let mut provenance = serde_json::json!({"draft":bundle.draft,"cell":bundle.cell,"source_revision":bundle.source_revision,"binding_revision":bundle.binding_revision,"source_document_digest":bundle.source_document_digest,"bindings_digest":bundle.bindings_digest,"catalog_digest":bundle.catalog_digest});
            if !bundle.device_sources.is_empty() {
                provenance["device_sources"] = serde_json::to_value(&bundle.device_sources)?;
            }
            (source, bundle.bindings, Some(provenance))
        } else {
            let source: ProcessSource = input(Path::new(&args[0]))?;
            let bindings: BTreeMap<rx_domain::types::Name, ActionBinding> =
                input(Path::new(&args[1]))?;
            (source, bindings, None)
        };
        (compile(&source, bindings)?, provenance)
    };
    let resolved = canonical::bytes(&process)?;
    let xml = rx_process::bt_xml::generate(&process)?;
    let report = serde_json::to_vec_pretty(
        &serde_json::json!({"status":"COMPILED_NOT_QUALIFIED","authoring_input":provenance,"source_digest":process.source_digest,
        "resolved_digest":rx_package::content_digest(&resolved),"bt_xml_digest":rx_package::content_digest(xml.as_bytes()),
        "required_executor_nodes":["RXSequence","RXParallelAll","RXBranch","RXOperation","RXWait","RXIntervention"],
        "limitations":["RX custom BT executor requires the matching runtime and validated P client","P eligibility/checkpoint integration and profile qualification are separate","No device or process is executed by compilation"]}),
    )?;
    let out = Path::new(&args[2]);
    fs::create_dir(out)?;
    extra.insert("resolved.json".into(), resolved);
    extra.insert("process.bt.xml".into(), xml.into_bytes());
    extra.insert("compile-report.json".into(), report);
    for (name, bytes) in extra {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(out.join(name))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    println!("Compiled process artifacts. No device execution or qualification granted.");
    Ok(())
}
