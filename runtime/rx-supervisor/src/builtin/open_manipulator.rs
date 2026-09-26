//! OpenMANIPULATOR 5.1.2 catalog-bound L3 variant, simulation only.
use super::*;

const FILES: &[(&str, &[u8])] = &[
    (
        "tools/ai-worker/l3_guard.py",
        include_bytes!("../../../../native/ai-worker/l3_guard.py"),
    ),
    (
        "tools/open-manipulator/dependencies.json",
        include_bytes!("../../../../native/open-manipulator/dependencies.json"),
    ),
    (
        "tools/open-manipulator/requirements.py",
        include_bytes!("../../../../native/open-manipulator/requirements.py"),
    ),
];

pub(super) fn source_pins(root: &Path, inventory: &rx_package::release::Inventory) -> Result<bool> {
    let present = inventory
        .files
        .keys()
        .any(|p| p.starts_with("tools/open-manipulator/"));
    if !present {
        return Ok(false);
    }
    for (path, bytes) in FILES {
        let expected = rx_package::content_digest(bytes);
        if inventory.files.get(*path) != Some(&expected) {
            return Err(rx_package::release::Error::Content(
                "release/source-pin-mismatch; OpenMANIPULATOR L3 source changed".into(),
            )
            .into());
        }
        verify(&root.join(path), expected)?;
    }
    Ok(true)
}
pub(super) fn program(
    root: &Path,
    release: &rx_package::release::VerifiedRelease,
) -> Result<Option<Program>> {
    let inventory = release.inventory();
    if !source_pins(root, inventory)? {
        return Ok(None);
    }
    let req = |p: &str| {
        inventory.files.get(p).copied().ok_or_else(|| {
            Error::Invalid(format!("OPEN_MANIPULATOR_RELEASE_CONTENT_REQUIRED: {p}"))
        })
    };
    let python = PathBuf::from("/usr/bin/python3");
    let python_hash = *inventory
        .external_files
        .get("/usr/bin/python3")
        .ok_or_else(|| Error::Invalid("OPEN_MANIPULATOR_PYTHON_RELEASE_PIN_REQUIRED".into()))?;
    verify(&python, python_hash)?;
    let script = root.join("tools/ai-worker/l3_guard.py");
    let n = |v| Name::new(v).expect("literal");
    Ok(Some(Program {
        id: n("rx/open-manipulator-l3-simulation"),
        effect: Effect::NonActuating,
        executable: python,
        executable_sha256: python_hash,
        files: [
            (script.clone(), req("tools/ai-worker/l3_guard.py")?),
            (
                root.join("tools/open-manipulator/dependencies.json"),
                req("tools/open-manipulator/dependencies.json")?,
            ),
            (
                root.join("tools/open-manipulator/requirements.py"),
                req("tools/open-manipulator/requirements.py")?,
            ),
        ]
        .into(),
        fixed_arguments: vec![
            script.to_string_lossy().into(),
            "serve".into(),
            "--state".into(),
            "/var/lib/rx-solutions/open-manipulator-l3".into(),
            "--service-generation".into(),
            "open-manipulator/5.1.2/l3-simulation".into(),
            "--owner".into(),
            "rx".into(),
        ],
        arguments: [(
            n("port"),
            Argument::Port {
                flag: "--port".into(),
            },
        )]
        .into(),
        ready: ReadyProbe::HttpStatus {
            port_parameter: n("port"),
        },
        execution_requirements: Some(crate::execution::Requirements(BTreeMap::new())),
        functional_readiness: None,
        decision_policy: None,
    }))
}
