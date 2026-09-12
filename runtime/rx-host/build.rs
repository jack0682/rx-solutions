use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
};
fn inventory(
    root: &Path,
    folder: &Path,
    files: &mut BTreeSet<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err("SDK symlink is not allowed".into());
        }
        if kind.is_dir() {
            inventory(root, &path, files)?;
        } else if kind.is_file() && path != root.join("source-lock.json") {
            files.insert(
                path.strip_prefix(root)?
                    .to_str()
                    .ok_or("SDK path encoding")?
                    .replace('\\', "/"),
            );
        }
    }
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).join("../../sdk");
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("source-lock.json"))?)?;
    if lock["schema"] != "rx.host-sdk.v1" {
        return Err("unsupported SDK source manifest".into());
    }
    let mut actual = BTreeSet::new();
    inventory(&root, &root, &mut actual)?;
    let expected: BTreeSet<_> = lock["files"]
        .as_object()
        .ok_or("missing SDK inventory")?
        .keys()
        .cloned()
        .collect();
    if actual != expected {
        return Err("SDK inventory mismatch (added or missing source)".into());
    }
    for (path, hash) in lock["files"].as_object().ok_or("missing SDK files")? {
        if PathBuf::from(path).is_absolute() || path.split('/').any(|p| p == "..") {
            return Err("invalid SDK path".into());
        }
        if format!("{:x}", Sha256::digest(fs::read(root.join(path))?))
            != hash.as_str().ok_or("invalid SDK digest")?
        {
            return Err(format!("SDK source hash mismatch: {path}").into());
        }
    }
    println!("cargo:rerun-if-changed={}", root.display());
    let host = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let solutions = host.join("../..").canonicalize()?;
    let mut material = BTreeSet::new();
    for folder in [host.join("src"), solutions.join("drivers/rx-melsec-mc/src")] {
        inventory(&solutions, &folder, &mut material)?;
        println!("cargo:rerun-if-changed={}", folder.display());
    }
    for path in [
        "Cargo.lock",
        "runtime/rx-host/Cargo.toml",
        "runtime/rx-host/build.rs",
        "drivers/rx-melsec-mc/Cargo.toml",
        "sdk/source-lock.json",
    ] {
        material.insert(path.into());
        println!("cargo:rerun-if-changed={}", solutions.join(path).display());
    }
    let mut digest = Sha256::new();
    digest.update(b"RX-HOST-MELSEC-SOURCE-v1\0");
    for path in &material {
        let bytes = fs::read(solutions.join(path))?;
        digest.update((path.len() as u64).to_le_bytes());
        digest.update(path.as_bytes());
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    println!(
        "cargo:rustc-env=RX_MELSEC_DRIVER_SOURCE_SHA256={:x}",
        digest.finalize()
    );
    for folder in [
        solutions.join("native/ros-jtc"),
        solutions.join("runtime/rx-solution-catalog/src"),
    ] {
        inventory(&solutions, &folder, &mut material)?;
        println!("cargo:rerun-if-changed={}", folder.display());
    }
    for path in [
        "catalogs/robotis-support.v1.json",
        "runtime/rx-solution-catalog/Cargo.toml",
        "dependencies/native-stack.lock.json",
        "dependencies/robotis.repos",
    ] {
        material.insert(path.into());
        println!("cargo:rerun-if-changed={}", solutions.join(path).display());
    }
    let mut digest = Sha256::new();
    digest.update(b"RX-HOST-ROBOTIS-JTC-SOURCE-v1\0");
    for path in material {
        let bytes = fs::read(solutions.join(&path))?;
        digest.update((path.len() as u64).to_le_bytes());
        digest.update(path.as_bytes());
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    println!(
        "cargo:rustc-env=RX_JTC_DRIVER_SOURCE_SHA256={:x}",
        digest.finalize()
    );
    Ok(())
}
