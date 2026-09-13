//! Offline investigation-procedure authoring. No PackageKind, P authority or native execution.
use crate::{Error, Result, directory, trust};
use rx_domain::{canonical, types::*};
use rx_package::{PackagePath, SignatureEnvelope, content_digest, verify_detached_message};
use rx_process_contract::investigation::Procedure;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
};

const MAX_INPUT: u64 = 1_048_576;
fn invalid(error: impl std::fmt::Display) -> Error {
    Error::Invalid(error.to_string())
}
fn bytes(value: &impl Serialize) -> Result<Vec<u8>> {
    canonical::bytes(value).map_err(invalid)
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SigningRequest {
    pub schema: Name,
    pub key: Name,
    pub procedure: ArtifactRef,
    pub message_digest: Digest,
    pub message_hex: String,
}
/// INPUT may use formatting whitespace; the sole output is validated canonical procedure.json.
pub fn assemble(input: &Path, out: &Path) -> Result<ArtifactRef> {
    let procedure: Procedure = trust::read(input)?;
    procedure.validate().map_err(invalid)?;
    let reference = procedure.reference().map_err(invalid)?;
    directory::publish_files(
        BTreeMap::from([(
            PackagePath::new("procedure.json").map_err(invalid)?,
            bytes(&procedure)?,
        )]),
        out,
    )?;
    Ok(reference)
}
/// Detached signing input binds the owning contract's domain, key ID and exact procedure.
pub fn signing_request(procedure: &Path, key: Name, out: &Path) -> Result<SigningRequest> {
    let (procedure, _) = read_canonical(procedure)?;
    let message = procedure.signing_message(&key).map_err(invalid)?;
    let request = SigningRequest {
        schema: Name::new("rx.investigation-signing-request.v1").map_err(invalid)?,
        procedure: procedure.reference().map_err(invalid)?,
        key,
        message_digest: content_digest(&message),
        message_hex: encode_hex(&message),
    };
    write_new(out, &bytes(&request)?)?;
    Ok(request)
}
/// The supplied public key proves signature validity only, never P deployment policy trust.
pub fn seal(
    procedure: &Path,
    signature: &Path,
    public_key_hex: &str,
    out: &Path,
) -> Result<ArtifactRef> {
    let (procedure, original) = read_canonical(procedure)?;
    let signature: SignatureEnvelope = trust::read(signature)?;
    let public_key: Digest =
        serde_json::from_value(serde_json::Value::String(public_key_hex.into()))
            .map_err(invalid)?;
    let message = procedure.signing_message(&signature.key).map_err(invalid)?;
    verify_detached_message(&message, &signature.signature, public_key.as_bytes())?;
    let reference = procedure.reference().map_err(invalid)?;
    if reference.sha256 != content_digest(&original)
        || reference.size_bytes.0 != original.len() as u64
    {
        return Err(invalid(
            "investigation reference differs from the canonical source bytes",
        ));
    }
    directory::publish_files(
        BTreeMap::from([
            (
                PackagePath::new(format!("{}.json", reference.sha256)).map_err(invalid)?,
                original,
            ),
            (
                PackagePath::new(format!("{}.sig.json", reference.sha256)).map_err(invalid)?,
                bytes(&signature)?,
            ),
        ]),
        out,
    )?;
    Ok(reference)
}
fn read_canonical(path: &Path) -> Result<(Procedure, Vec<u8>)> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid("named procedure JSON input required"))?;
    let original = rx_package::directory::read_relative_file(
        parent,
        &PackagePath::new(filename).map_err(invalid)?,
        MAX_INPUT,
    )?;
    let procedure: Procedure = canonical::decode_json(&original).map_err(invalid)?;
    procedure.validate().map_err(invalid)?;
    if bytes(&procedure)? != original {
        return Err(invalid(
            "procedure input must be the exact canonical procedure.json bytes from investigation-assemble",
        ));
    }
    Ok((procedure, original))
}
fn write_new(path: &Path, value: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if !parent.is_dir() {
        return Err(invalid("existing output parent required"));
    }
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(value)?;
    file.sync_all()?;
    File::open(parent)?.sync_all()?;
    Ok(())
}
fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
