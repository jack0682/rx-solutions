use super::*;
const FILES: &[(&str, &[u8])] = &[
    (
        "tools/ai-sapiens/asset_gate.py",
        include_bytes!("../../../../native/ai-sapiens/asset_gate.py"),
    ),
    (
        "tools/ai-sapiens/dependencies.json",
        include_bytes!("../../../../native/ai-sapiens/dependencies.json"),
    ),
];
pub(super) fn source_pins(root: &Path, i: &rx_package::release::Inventory) -> Result<bool> {
    let present = i.files.keys().any(|p| p.starts_with("tools/ai-sapiens/"));
    if !present {
        return Ok(false);
    }
    for (path, b) in FILES {
        let d = rx_package::content_digest(b);
        if i.files.get(*path) != Some(&d) {
            return Err(rx_package::release::Error::Content(
                "release/source-pin-mismatch; AI Sapiens asset gate changed".into(),
            )
            .into());
        }
        verify(&root.join(path), d)?;
    }
    Ok(true)
}
pub(super) fn program(
    root: &Path,
    r: &rx_package::release::VerifiedRelease,
) -> Result<Option<Program>> {
    let i = r.inventory();
    if !source_pins(root, i)? {
        return Ok(None);
    }
    let py = PathBuf::from("/usr/bin/python3");
    let ph = *i
        .external_files
        .get("/usr/bin/python3")
        .ok_or_else(|| Error::Invalid("AI_SAPIENS_PYTHON_PIN_REQUIRED".into()))?;
    verify(&py, ph)?;
    let script = root.join("tools/ai-sapiens/asset_gate.py");
    let n = |v| Name::new(v).expect("literal");
    Ok(Some(Program {
        id: n("rx/ai-sapiens-asset-simulation"),
        effect: Effect::NonActuating,
        executable: py,
        executable_sha256: ph,
        files: FILES
            .iter()
            .map(|(p, _)| (root.join(p), *i.files.get(*p).unwrap()))
            .collect(),
        fixed_arguments: vec![
            script.to_string_lossy().into(),
            "probe".into(),
            "--asset-root".into(),
            "/opt/rx/ai-sapiens/assets".into(),
        ],
        arguments: BTreeMap::new(),
        ready: ReadyProbe::AliveOnly,
        execution_requirements: Some(crate::execution::Requirements(BTreeMap::new())),
        functional_readiness: None,
        decision_policy: None,
    }))
}
