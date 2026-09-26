//! AI Worker 2.2.7 L3 service-generation ownership, simulation only.
use super::*;

const SOURCES: &[(&str, &[u8])] = &[
    (
        "tools/ai-worker/l3_guard.py",
        include_bytes!("../../../../native/ai-worker/l3_guard.py"),
    ),
    (
        "tools/ai-worker/dependencies.json",
        include_bytes!("../../../../native/ai-worker/dependencies.json"),
    ),
];

pub(super) fn source_pins(root: &Path, inventory: &rx_package::release::Inventory) -> Result<bool> {
    let present = inventory
        .files
        .keys()
        .any(|path| path.starts_with("tools/ai-worker/"));
    if !present {
        return Ok(false);
    }
    for (path, bytes) in SOURCES {
        let expected = rx_package::content_digest(bytes);
        if inventory.files.get(*path) != Some(&expected) {
            return Err(rx_package::release::Error::Content(
                "release/source-pin-mismatch; AI Worker L3 inventory cannot authorize changed source".into(),
            )
            .into());
        }
        verify(&root.join(path), expected)
            .map_err(|error| rx_package::release::Error::Content(error.to_string()))?;
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
    let script = root.join("tools/ai-worker/l3_guard.py");
    let dependencies = root.join("tools/ai-worker/dependencies.json");
    let required =
        |path: &str| -> Result<Digest> {
            inventory.files.get(path).copied().ok_or_else(|| {
                Error::Invalid(format!("AI_WORKER_RELEASE_CONTENT_REQUIRED: {path}"))
            })
        };
    let python = PathBuf::from("/usr/bin/python3");
    let python_hash = inventory
        .external_files
        .get("/usr/bin/python3")
        .copied()
        .ok_or_else(|| Error::Invalid("AI_WORKER_PYTHON_RELEASE_PIN_REQUIRED".into()))?;
    verify(&python, python_hash)?;
    let name = |value| Name::new(value).expect("literal");
    Ok(Some(Program {
        id: name("rx/ai-worker-l3-simulation"),
        effect: Effect::NonActuating,
        executable: python,
        executable_sha256: python_hash,
        files: [
            (script.clone(), required("tools/ai-worker/l3_guard.py")?),
            (dependencies, required("tools/ai-worker/dependencies.json")?),
        ]
        .into(),
        fixed_arguments: vec![
            script.to_string_lossy().into(),
            "serve".into(),
            "--state".into(),
            "/var/lib/rx-solutions/ai-worker-l3".into(),
            "--owner".into(),
            "rx".into(),
        ],
        arguments: [(
            name("port"),
            Argument::Port {
                flag: "--port".into(),
            },
        )]
        .into(),
        ready: ReadyProbe::HttpStatus {
            port_parameter: name("port"),
        },
        execution_requirements: Some(crate::execution::Requirements(BTreeMap::new())),
        functional_readiness: None,
        decision_policy: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_ai_worker_l3_installation_is_not_a_legacy_fallback() {
        let root = tempfile::tempdir().unwrap();
        let mut inventory = rx_package::release::Inventory {
            schema: "rx.solutions-runtime-files.v1".into(),
            files: BTreeMap::new(),
            external_files: BTreeMap::new(),
        };
        assert!(!source_pins(root.path(), &inventory).unwrap());
        inventory.files.insert(
            "tools/ai-worker/l3_guard.py".into(),
            rx_package::content_digest(b"partial"),
        );
        assert!(source_pins(root.path(), &inventory).is_err());
    }
}
