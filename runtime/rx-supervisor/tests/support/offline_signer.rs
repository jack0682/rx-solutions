//! Test issuer in a separate OpenSSL process. Fresh ephemeral keys are not
//! production defaults and no private key enters a verification API.
use rx_domain::types::*;
use rx_package::SignatureEnvelope;
use rx_supervisor::decision::*;
use std::{
    path::{Path, PathBuf},
    process::Command,
};
fn n(s: &str) -> Name {
    Name::new(s).unwrap()
}
pub struct ExternalSigner {
    directory: tempfile::TempDir,
    executable: PathBuf,
    public: [u8; 32],
}
impl ExternalSigner {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let executable = PathBuf::from("openssl");
        Self::command(
            &executable,
            &["genpkey", "-algorithm", "ED25519", "-out"],
            &[directory.path().join("issuer.pem")],
        );
        let output = Command::new(&executable)
            .args(["pkey", "-in"])
            .arg(directory.path().join("issuer.pem"))
            .args(["-pubout", "-outform", "DER", "-out"])
            .arg(directory.path().join("public.der"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let der = std::fs::read(directory.path().join("public.der")).unwrap();
        assert_eq!(der.len(), 44);
        assert_eq!(
            &der[..12],
            &[
                0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00
            ]
        );
        Self {
            directory,
            executable,
            public: der[12..].try_into().unwrap(),
        }
    }
    fn command(executable: &Path, args: &[&str], paths: &[PathBuf]) {
        let output = Command::new(executable)
            .args(args)
            .args(paths)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    pub fn key_id(&self) -> Name {
        n(rx_package::operating_area::KEY_ID)
    }
    pub fn policy(&self) -> Policy {
        Policy {
            authorities: [(
                self.key_id(),
                Authority {
                    issuer: n("test/operating-area-issuer"),
                    public_key: self.public,
                    operating_area: n("test/diagnostics"),
                    roles: [
                        n("diagnostics/support-summary"),
                        n("work/support-gap-report"),
                        n("diagnostics/operator-connected"),
                        n("snapshot-after-preparation"),
                        n("snapshot-at-generation"),
                        n("current-report-collection"),
                    ]
                    .into(),
                    kinds: [
                        Kind::WorkUse,
                        Kind::InitialBinding,
                        Kind::ReplacementBinding,
                    ]
                    .into(),
                    max_ttl_ms: Counter(30_000),
                },
            )]
            .into(),
        }
    }
    fn sign(&self, message: &[u8]) -> SignatureEnvelope {
        let id = uuid::Uuid::new_v4().to_string();
        let input = self.directory.path().join(format!("{id}.input"));
        let output = self.directory.path().join(format!("{id}.signature"));
        std::fs::write(&input, message).unwrap();
        let result = Command::new(&self.executable)
            .args(["pkeyutl", "-sign", "-rawin", "-inkey"])
            .arg(self.directory.path().join("issuer.pem"))
            .arg("-in")
            .arg(input)
            .arg("-out")
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let bytes = std::fs::read(output).unwrap();
        assert_eq!(bytes.len(), 64);
        SignatureEnvelope {
            key: self.key_id(),
            signature: bytes.iter().map(|b| format!("{b:02x}")).collect(),
        }
    }
    pub fn decision(&self, claim: Claim) -> SignedDecision {
        let signature = self.sign(&claim.signing_message(&self.key_id()).unwrap());
        SignedDecision { claim, signature }
    }
    pub fn revocation(&self, claim: RevocationClaim) -> SignedRevocation {
        let signature = self.sign(&claim.signing_message(&self.key_id()).unwrap());
        SignedRevocation { claim, signature }
    }
}
