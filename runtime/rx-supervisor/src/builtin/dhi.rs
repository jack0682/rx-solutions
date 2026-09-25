//! Fresh-PTY investigation only. Registered release admission remains withheld.
use super::*;

const SOURCES: &[(&str, &[u8])] = &[
    (
        "tools/dhi/session.py",
        include_bytes!("../../../../native/dhi/session.py"),
    ),
    (
        "tools/dhi/model.py",
        include_bytes!("../../../../native/dhi/model.py"),
    ),
    (
        "tools/dhi/guardian.c",
        include_bytes!("../../../../native/dhi/guardian.c"),
    ),
    (
        "tools/dhi/dependencies.json",
        include_bytes!("../../../../native/dhi/dependencies.json"),
    ),
    (
        "tools/dhi/endpoint-channels.json",
        include_bytes!("../../../../native/dhi/endpoint-channels.json"),
    ),
];
const GUARDIAN: &str = "bin/rx-dhi-custody";
const MANAGER: &str = "bin/rx-dhi-controller-manager";

pub(super) fn source_pins(root: &Path, inventory: &rx_package::release::Inventory) -> Result<bool> {
    let present = inventory
        .files
        .keys()
        .any(|p| p.starts_with("dhi/") || p.starts_with("tools/dhi/") || p == GUARDIAN);
    if !present {
        return Ok(false); // Older releases have no recipe; selecting it is refused.
    }
    for (path, bytes) in SOURCES {
        let expected = rx_package::content_digest(bytes);
        if inventory.files.get(*path) != Some(&expected) {
            return Err(rx_package::release::Error::Content(
                "release/source-pin-mismatch; DHI inventory cannot authorize changed source".into(),
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
    let required = |key: &str| -> Result<Digest> {
        inventory
            .files
            .get(key)
            .copied()
            .ok_or_else(|| Error::Invalid(format!("DHI_RELEASE_CONTENT_REQUIRED: {key}")))
    };
    let mut files: BTreeMap<PathBuf, Digest> = SOURCES
        .iter()
        .map(|(path, _)| Ok((root.join(path), required(path)?)))
        .collect::<Result<_>>()?;
    files.insert(root.join(GUARDIAN), required(GUARDIAN)?);
    let manager = inventory
        .files
        .get(MANAGER)
        .copied()
        .ok_or_else(|| Error::Invalid("DHI_MANAGER_RELEASE_PIN_REQUIRED".into()))?;
    files.insert(root.join(MANAGER), manager);
    for (path, digest) in &inventory.files {
        if path.starts_with("dhi/") {
            files.insert(root.join(path), *digest);
        }
    }
    for path in [
        "dhi/lib/libdynamixel_hardware_interface.so",
        "dhi/lib/libdynamixel_sdk.so",
        "dhi/share/dynamixel_hardware_interface/param/dxl_model/dynamixel.model",
        "dhi/share/dynamixel_hardware_interface/param/dxl_model/xl430_w250.model",
    ] {
        required(path)?;
    }
    for (path, digest) in &files {
        verify(path, *digest)?;
    }
    let python = PathBuf::from("/usr/bin/python3");
    let executable_sha256 = inventory
        .external_files
        .get("/usr/bin/python3")
        .copied()
        .ok_or_else(|| Error::Invalid("DHI_PYTHON_RELEASE_PIN_REQUIRED".into()))?;
    let name = |value| Name::new(value).expect("literal");
    Ok(Some(Program {
        id: name("rx/dhi-pty-simulation"),
        // Direct P5-P8 evidence exercises the resource gate, but the current
        // development release cannot authenticate this catalog revision after
        // its offline signing key was destroyed. Keep resident launch denied
        // until release signing custody/rotation is established separately.
        effect: Effect::RequiresPlatformAuthority,
        executable: python,
        executable_sha256,
        files,
        fixed_arguments: vec![
            root.join("tools/dhi/session.py").to_string_lossy().into(),
            "serve".into(),
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
        execution_requirements: None,
        functional_readiness: None,
        decision_policy: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(root: &Path) -> rx_package::release::Inventory {
        let mut files = BTreeMap::new();
        for (path, bytes) in SOURCES {
            let destination = root.join(path);
            std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
            std::fs::write(destination, bytes).unwrap();
            files.insert((*path).to_owned(), rx_package::content_digest(bytes));
        }
        rx_package::release::Inventory {
            schema: "rx.solutions-runtime-files.v1".into(),
            files,
            external_files: BTreeMap::new(),
        }
    }

    #[test]
    fn forged_inventory_cannot_authorize_changed_dhi_source() {
        let root = tempfile::tempdir().unwrap();
        let mut inventory = fixture(root.path());
        assert!(source_pins(root.path(), &inventory).unwrap());
        for (path, _) in SOURCES {
            let original = inventory.files[*path];
            inventory
                .files
                .insert((*path).into(), rx_package::content_digest(b"forged source"));
            assert!(
                source_pins(root.path(), &inventory)
                    .unwrap_err()
                    .to_string()
                    .contains("source-pin-mismatch")
            );
            inventory.files.insert((*path).into(), original);
        }
        std::fs::write(root.path().join("tools/dhi/session.py"), b"tampered").unwrap();
        assert!(
            source_pins(root.path(), &inventory)
                .unwrap_err()
                .to_string()
                .contains("release/content-mismatch")
        );
    }

    #[test]
    fn partial_installation_is_not_a_legacy_release_fallback() {
        let root = tempfile::tempdir().unwrap();
        let mut inventory = fixture(root.path());
        inventory.files.clear();
        assert!(!source_pins(root.path(), &inventory).unwrap());
        inventory
            .files
            .insert(GUARDIAN.into(), rx_package::content_digest(b"binary"));
        assert!(source_pins(root.path(), &inventory).is_err());
    }
}
