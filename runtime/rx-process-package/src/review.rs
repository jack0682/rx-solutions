//! Produce review evidence using actual signed bytes and the process compiler. Signing is external.
use crate::{Error, Result};
use rx_domain::{canonical, types::*};
use rx_package::{PackagePath, VerificationPolicy, VerifiedPackage, content_digest};
use rx_process_contract::package_review::{Issue, Report, Request};
use std::collections::BTreeMap;
pub fn validator_digest() -> Digest {
    // Versioned source identity; this is NOT hardware-backed execution attestation.
    content_digest(
        concat!(
            "RX-PROCESS-VERIFIER-v1\0",
            include_str!("review.rs"),
            include_str!("../../../Cargo.lock"),
            include_str!("../../../sdk/source-lock.json"),
            include_str!("candidate.rs"),
            include_str!("../../rx-process/src/compile.rs"),
            include_str!("../../rx-process/src/lib.rs"),
            include_str!("../../../sdk/crates/rx-process-contract/src/model.rs"),
            include_str!("../../../sdk/crates/rx-process-contract/src/source_validation.rs"),
            include_str!("../../../sdk/crates/rx-process-contract/src/validation.rs"),
            include_str!("../../../sdk/crates/rx-process-contract/src/package_review.rs")
        )
        .as_bytes(),
    )
}
pub struct Bundle {
    pub report: Report,
    pub files: BTreeMap<PackagePath, Vec<u8>>,
}
pub fn verify_process(
    request: Request,
    package: &VerifiedPackage,
    policy: &VerificationPolicy,
    policy_file_digest: Digest,
) -> Result<Bundle> {
    request.validate().map_err(Error::Invalid)?;
    if request.package_manifest != package.digest()
        || request.package_signature
            != content_digest(
                &canonical::bytes(package.signature())
                    .map_err(|e| Error::Invalid(e.to_string()))?,
            )
        || request.package_policy_fingerprint != policy.fingerprint()?
    {
        return Err(Error::Invalid(
            "review request/package/policy binding differs".into(),
        ));
    }
    let mut files = BTreeMap::new();
    let mut issues = Vec::new();
    let resolved = match crate::compile_verified(package) {
        Ok(process) => {
            let bytes = canonical::bytes(&process).map_err(|e| Error::Invalid(e.to_string()))?;
            if bytes.len() > 1_048_576 {
                issues.push(Issue {
                    code: Name::new("RESOLVED_SIZE_LIMIT").expect("literal"),
                    location: "resolved".into(),
                    detail: "resolved process exceeds review artifact limit".into(),
                });
                None
            } else {
                let reference = ArtifactRef {
                    sha256: content_digest(&bytes),
                    schema_id: Name::new("rx.resolved-process.v1").expect("literal"),
                    size_bytes: Counter(bytes.len() as u64),
                };
                files.insert(PackagePath::new("resolved.json").expect("literal"), bytes);
                Some(reference)
            }
        }
        Err(error) => {
            issues.push(Issue {
                code: Name::new("PROCESS_COMPILE_FAILED").expect("literal"),
                location: "process".into(),
                detail: error.to_string().chars().take(400).collect(),
            });
            None
        }
    };
    let report = Report {
        schema: Name::new("rx.process-verification-report.v1").expect("literal"),
        request,
        validator_digest: validator_digest(),
        validator_policy_file_digest: policy_file_digest,
        resolved,
        issues,
    };
    report.validate().map_err(Error::Invalid)?;
    files.insert(
        PackagePath::new("verification.json").expect("literal"),
        canonical::bytes(&report).map_err(|e| Error::Invalid(e.to_string()))?,
    );
    Ok(Bundle { report, files })
}
