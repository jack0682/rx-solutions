use ed25519_dalek::{Signer, SigningKey};
use rx_domain::{canonical, types::*};
use rx_package::{SignatureEnvelope, content_digest};
use rx_process_contract::investigation::{Action, Procedure, SCHEMA};
use rx_process_package::investigation;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn name(value: &str) -> Name {
    Name::new(value).unwrap()
}
fn procedure() -> Procedure {
    Procedure {
        schema: name(SCHEMA),
        id: name("test/investigation"),
        revision: Counter(1),
        title: "Review the preserved operation records".into(),
        instructions: vec![
            "Inspect the original operation and retained evidence before any disposition.".into(),
        ],
        cell: name("cell/sim"),
        definition: Digest::from_bytes([3; 32]),
        environment: name("SIMULATION"),
        profiles: [Digest::from_bytes([4; 32])].into(),
        action: Action::AbandonInvestigation,
    }
}
fn canonical_input(root: &Path) -> PathBuf {
    let path = root.join("procedure.json");
    fs::write(&path, canonical::bytes(&procedure()).unwrap()).unwrap();
    path
}
fn signature(root: &Path, procedure: &Procedure, key: &SigningKey, key_id: &Name) -> PathBuf {
    let signature = SignatureEnvelope {
        key: key_id.clone(),
        signature: hex(&key
            .sign(&procedure.signing_message(key_id).unwrap())
            .to_bytes()),
    };
    let path = root.join("signature.json");
    fs::write(&path, canonical::bytes(&signature).unwrap()).unwrap();
    path
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|v| format!("{v:02x}")).collect()
}

#[test]
fn assemble_is_deterministic_unsigned_canonical_content_and_never_a_process_package() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input.json");
    fs::write(&input, serde_json::to_vec_pretty(&procedure()).unwrap()).unwrap();
    let a = root.path().join("candidate-a");
    let b = root.path().join("candidate-b");
    let first = investigation::assemble(&input, &a).unwrap();
    let second = investigation::assemble(&input, &b).unwrap();
    assert_eq!(first, second);
    assert_eq!(first, procedure().reference().unwrap());
    assert_eq!(
        fs::read(a.join("procedure.json")).unwrap(),
        canonical::bytes(&procedure()).unwrap()
    );
    assert_eq!(
        fs::read(a.join("procedure.json")).unwrap(),
        fs::read(b.join("procedure.json")).unwrap()
    );
    assert_eq!(fs::read_dir(&a).unwrap().count(), 1);
    assert!(!a.join("manifest.json").exists() && !a.join("manifest.sig.json").exists());
    assert!(
        investigation::seal(
            &a.join("procedure.json"),
            &a.join("missing.sig.json"),
            &"00".repeat(32),
            &root.path().join("unsigned-cannot-seal")
        )
        .is_err()
    );
    assert!(!root.path().join("unsigned-cannot-seal").exists());
    assert!(investigation::assemble(&input, &a).is_err());
    assert_eq!(
        fs::read(a.join("procedure.json")).unwrap(),
        canonical::bytes(&procedure()).unwrap()
    );
}

#[test]
fn signing_request_uses_the_exact_contract_domain_key_and_reference_without_signing() {
    let root = tempfile::tempdir().unwrap();
    let input = canonical_input(root.path());
    let key = name("test/investigator");
    let output = root.path().join("request.json");
    let request = investigation::signing_request(&input, key.clone(), &output).unwrap();
    let expected = format!(
        "RX-INVESTIGATION-PROCEDURE-SIGNATURE-v1\n{key}\n{}",
        procedure().reference().unwrap().sha256
    );
    assert_eq!(request.message_hex, hex(expected.as_bytes()));
    assert_eq!(request.message_digest, content_digest(expected.as_bytes()));
    assert_eq!(request.procedure, procedure().reference().unwrap());
    let value: serde_json::Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(value["schema"], "rx.investigation-signing-request.v1");
    assert_eq!(value.as_object().unwrap().len(), 5);
    assert!(value.get("signature").is_none());
    let before = fs::read(&output).unwrap();
    assert!(investigation::signing_request(&input, key, &output).is_err());
    assert_eq!(fs::read(&output).unwrap(), before);
}

#[test]
fn actual_ed25519_seal_publishes_exact_canonical_source_and_signature_without_policy_claim() {
    let root = tempfile::tempdir().unwrap();
    let input = canonical_input(root.path());
    let key = SigningKey::from_bytes(&[31; 32]);
    let key_id = name("test/investigator");
    let sig = signature(root.path(), &procedure(), &key, &key_id);
    let output = root.path().join("sealed");
    let public = hex(&key.verifying_key().to_bytes());
    let reference = investigation::seal(&input, &sig, &public, &output).unwrap();
    assert_eq!(
        fs::read(output.join(format!("{}.json", reference.sha256))).unwrap(),
        fs::read(&input).unwrap()
    );
    assert_eq!(
        fs::read(output.join(format!("{}.sig.json", reference.sha256))).unwrap(),
        fs::read(&sig).unwrap()
    );
    assert_eq!(fs::read_dir(&output).unwrap().count(), 2);
    assert!(investigation::seal(&input, &sig, &public, &output).is_err());
}

#[test]
fn tamper_wrong_public_key_key_alias_and_wrong_signing_domain_are_rejected_before_output() {
    let root = tempfile::tempdir().unwrap();
    let input = canonical_input(root.path());
    let key = SigningKey::from_bytes(&[31; 32]);
    let key_id = name("test/investigator");
    let sig = signature(root.path(), &procedure(), &key, &key_id);
    let output = root.path().join("rejected");
    assert!(
        investigation::seal(
            &input,
            &sig,
            &hex(&SigningKey::from_bytes(&[32; 32]).verifying_key().to_bytes()),
            &output
        )
        .is_err()
    );
    let mut changed = procedure();
    changed.instructions[0].push_str(" Changed.");
    fs::write(&input, canonical::bytes(&changed).unwrap()).unwrap();
    assert!(
        investigation::seal(&input, &sig, &hex(&key.verifying_key().to_bytes()), &output).is_err()
    );
    fs::write(&input, canonical::bytes(&procedure()).unwrap()).unwrap();
    let mut envelope: SignatureEnvelope = serde_json::from_slice(&fs::read(&sig).unwrap()).unwrap();
    envelope.key = name("test/alias");
    fs::write(&sig, canonical::bytes(&envelope).unwrap()).unwrap();
    assert!(
        investigation::seal(&input, &sig, &hex(&key.verifying_key().to_bytes()), &output).is_err()
    );
    envelope.key = key_id;
    envelope.signature = hex(&key
        .sign(b"RX-PACKAGE-SIGNATURE-v1\nwrong-domain")
        .to_bytes());
    fs::write(&sig, canonical::bytes(&envelope).unwrap()).unwrap();
    assert!(
        investigation::seal(&input, &sig, &hex(&key.verifying_key().to_bytes()), &output).is_err()
    );
    assert!(!output.exists());
}

#[test]
fn unknown_duplicate_fields_invalid_actions_and_noncanonical_signing_input_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input.json");
    let output = root.path().join("rejected");
    let mut value = serde_json::to_value(procedure()).unwrap();
    value["private_key"] = "not-an-input".into();
    fs::write(&input, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(investigation::assemble(&input, &output).is_err());
    value.as_object_mut().unwrap().remove("private_key");
    value["action"] = "START_RUN".into();
    fs::write(&input, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(investigation::assemble(&input, &output).is_err());
    let original = String::from_utf8(canonical::bytes(&procedure()).unwrap()).unwrap();
    fs::write(&input, format!("{{\"revision\":\"1\",{}", &original[1..])).unwrap();
    assert!(investigation::assemble(&input, &output).is_err());
    fs::write(&input, serde_json::to_vec_pretty(&procedure()).unwrap()).unwrap();
    assert!(
        investigation::signing_request(&input, name("test/key"), &root.path().join("request.json"))
            .is_err()
    );
    assert!(!output.exists());
}

#[test]
fn cli_declares_signature_validity_without_deployment_trust_or_usage_authorization() {
    let root = tempfile::tempdir().unwrap();
    let input = canonical_input(root.path());
    let key = SigningKey::from_bytes(&[31; 32]);
    let sig = signature(root.path(), &procedure(), &key, &name("test/key"));
    let output = Command::new(env!("CARGO_BIN_EXE_rx-process-package"))
        .arg("investigation-seal")
        .arg(&input)
        .arg(&sig)
        .arg(hex(&key.verifying_key().to_bytes()))
        .arg(root.path().join("sealed"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["status"], "SIGNATURE_VERIFIED_NOT_AUTHORIZED");
    assert_eq!(status["signature_verified"], true);
    assert_eq!(status["deployment_policy_verified"], false);
    assert_eq!(status["usage_authorized"], false);
    assert_eq!(status["public_key_check"], "SIGNATURE_VALIDITY_ONLY");
}

#[cfg(unix)]
#[test]
fn symlink_input_and_existing_output_target_are_not_followed_or_overwritten() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let input = canonical_input(root.path());
    let alias = root.path().join("alias.json");
    symlink(&input, &alias).unwrap();
    assert!(
        investigation::signing_request(&alias, name("test/key"), &root.path().join("request.json"))
            .is_err()
    );
    let target = root.path().join("owned.txt");
    fs::write(&target, b"preserve").unwrap();
    let output = root.path().join("request.json");
    symlink(&target, &output).unwrap();
    assert!(investigation::signing_request(&input, name("test/key"), &output).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"preserve");
}
