//! Run actual package/driver decoders and produce software-only evidence. Signing remains external.
use crate::*;
use rx_process_contract::{
    device_catalog::Catalog,
    device_review::{Check, Report, Request, ResultKind, Scope},
    package_review::Issue,
};
pub fn validator_digest() -> Digest {
    let source = content_digest(
        concat!(
            "RX-DEVICE-VERIFIER-SOURCE-v1\0",
            include_str!("review.rs"),
            include_str!("lib.rs"),
            include_str!("jtc.rs"),
            include_str!("../../../Cargo.lock"),
            include_str!("../../../sdk/source-lock.json")
        )
        .as_bytes(),
    );
    canonical::digest(
        "RX-DEVICE-VERIFIER-v1",
        &(
            source,
            rx_host::service::device_package::driver(),
            rx_host::service::jtc_package::driver(),
        ),
    )
    .expect("fixed validator identity")
}
pub fn verify(
    request: Request,
    package: &VerifiedPackage,
    policy: &VerificationPolicy,
    policy_file_digest: Digest,
) -> Result<Report> {
    request.validate().map_err(Error::Invalid)?;
    if request.package_manifest != package.digest()
        || request.package_signature != content_digest(&bytes(package.signature())?)
        || request.package_policy_fingerprint != policy.fingerprint()?
    {
        return Err(Error::Invalid(
            "device request/package/policy differs".into(),
        ));
    }
    let catalog: Catalog = canonical::decode_json(
        package
            .file(&path("device-catalog.json"))
            .ok_or_else(|| Error::Invalid("common device catalog required".into()))?,
    )
    .map_err(|e| Error::Invalid(e.to_string()))?;
    catalog.validate().map_err(Error::Invalid)?;
    let data = bytes(&catalog)?;
    if request.catalog.sha256 != content_digest(&data)
        || request.catalog.size_bytes.0 != data.len() as u64
        || request.installation != catalog.installation
        || request.cell != catalog.cell
    {
        return Err(Error::Invalid("device request/catalog correlation".into()));
    }
    let mut issues = vec![];
    let decoded = match decode_verified_any(package) {
        Ok(_) => ResultKind::Passed,
        Err(e) => {
            issues.push(Issue {
                code: name("DEVICE_SOURCE_CHECK_FAILED"),
                location: "device/source".into(),
                detail: e.to_string().chars().take(400).collect(),
            });
            ResultKind::Failed
        }
    };
    let report = Report {
        schema: name("rx.device-verification-report.v1"),
        request,
        validator_digest: validator_digest(),
        validator_policy_file_digest: policy_file_digest,
        scope: Scope::DevicePackageSoftware,
        checks: [
            (Check::ContentSignature, ResultKind::Passed),
            (Check::DeviceSourceConsistency, decoded),
            (Check::CatalogRequestBinding, ResultKind::Passed),
        ]
        .into(),
        issues,
    };
    report.validate().map_err(Error::Invalid)?;
    Ok(report)
}
