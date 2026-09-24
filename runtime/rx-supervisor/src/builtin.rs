//! Release-owned launch recipes. A site plan selects parameters, not executable paths or effect classes.
use crate::{Error, Result, model::*, process::verify};
use rx_domain::{canonical, types::*};

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

mod services;
pub use services::{
    ConfigurationPin, Initializer, ServiceConfigurations, ServiceInput, ServiceRole,
    add_guarded_services, services_from_release, validate_service_plan,
};

/// Checkpoint diagnostics, never a current permission receipt.
pub fn release_boundary() -> serde_json::Value {
    serde_json::json!({
        "trust": "TRUSTED_INSTALLED_RUST_BINARIES_AND_OS",
        "source_assets": "STATUS_SCRIPT_AND_DEVICE_CATALOG_MATCH_COMPILED_SOURCE_CONTENT",
        "inventory": "CONSISTENCY_INDEX_AUTHENTICATED_BY_COMPILED_DEVELOPMENT_ROOT",
        "authenticated_immutable_provenance": "NOT_ESTABLISHED_FOR_OS_AND_VERIFIER; DEVELOPMENT_RELEASE_CONTENT_AUTHENTICATED_AT_CHECKPOINT",
        "release_key": rx_package::release::root::KEY_ID,
        "product_release_custody_and_rotation": "NOT_ESTABLISHED",
        "whole_state_rollback_or_deletion": "NOT_DETECTED",
        "offline_revocation_freshness": "NOT_ESTABLISHED",
        "interval": "EXPLICIT_CALLER_DRIVEN_CHECKPOINTS",
        "monitor": "NO_TIMER_OR_BACKGROUND_MONITOR",
        "remaining_interval": "TRUSTED_INSTALLATION_STABILITY_BETWEEN_BYTE_CHECK_AND_USE",
        "inspection": "OBSERVATION_ONLY; NOT_DURABLE_ADMISSION_OR_WORK_PERMISSION"
    })
}

pub fn release_metadata(root: &Path, name: &str) -> Result<Vec<u8>> {
    use std::io::Read;
    let path = root.join("manifests").join(name);
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && name != "runtime-files.json" => {
            return Err(rx_package::release::Error::Unsigned.into());
        }
        Err(e) => return Err(rx_package::release::Error::Content(e.to_string()).into()),
    };
    let mut bytes = Vec::new();
    file.take(1_048_577).read_to_end(&mut bytes)?;
    if bytes.len() > 1_048_576 {
        return Err(rx_package::release::Error::Malformed("metadata size".into()).into());
    }
    Ok(bytes)
}
pub fn load_release(root: &Path) -> Result<rx_package::release::VerifiedRelease> {
    verify_release(
        root,
        &release_metadata(root, "release.json")?,
        &release_metadata(root, "revocations.json")?,
    )
}
pub fn verify_release(
    root: &Path,
    release: &[u8],
    revoked: &[u8],
) -> Result<rx_package::release::VerifiedRelease> {
    use std::io::Read;
    Ok(rx_package::release::verify(
        release,
        revoked,
        &release_metadata(root, "runtime-files.json")?,
        |name, external| {
            let path = if external {
                PathBuf::from(name)
            } else {
                root.join(name)
            };
            let file = std::fs::File::open(&path)
                .map_err(|_| rx_package::release::Error::Content(name.into()))?;
            let mut bytes = Vec::new();
            file.take(536_870_913)
                .read_to_end(&mut bytes)
                .map_err(|_| rx_package::release::Error::Content(name.into()))?;
            if bytes.len() > 536_870_912 {
                return Err(rx_package::release::Error::Content(name.into()));
            }
            Ok(bytes)
        },
    )?)
}
pub fn release_programs(root: &Path) -> Result<BTreeMap<Name, Program>> {
    programs_from_release(root, &load_release(root)?)
}
pub fn programs_from_release(
    root: &Path,
    release: &rx_package::release::VerifiedRelease,
) -> Result<BTreeMap<Name, Program>> {
    let inventory = release.inventory();
    let script = root.join("tools/solutions_status.py");
    let digest = |v: Option<&Digest>| -> Result<Digest> {
        v.copied()
            .ok_or_else(|| Error::Invalid("required release file absent".into()))
    };
    // These pins come from reviewed source compiled into the trusted Rust
    // binary, never from the adjacent inventory being checked. The installed
    // Rust binaries and OS remain the trust boundary, not authenticated origin.
    let script_hash = rx_package::content_digest(include_bytes!(
        "../../../native/support/solutions_status.py"
    ));
    let catalog_hash =
        rx_package::content_digest(include_bytes!("../../../catalogs/device-support.v1.json"));
    if digest(inventory.files.get("tools/solutions_status.py"))? != script_hash
        || digest(inventory.files.get("catalogs/device-support.v1.json"))? != catalog_hash
    {
        return Err(Error::Invalid(
            "release/source-pin-mismatch; inventory cannot authorize changed source assets".into(),
        ));
    }
    verify(&root.join("catalogs/device-support.v1.json"), catalog_hash)?;
    let python = PathBuf::from("/usr/bin/python3");
    let python_hash = digest(inventory.external_files.get("/usr/bin/python3"))?;
    verify(&script, script_hash)?;
    verify(&python, python_hash)?;
    let name = |s: &str| Name::new(s).expect("literal name");
    let program = Program {
        functional_readiness: Some(status_readiness()),
        decision_policy: None,
        execution_requirements: Some(crate::execution::Requirements(
            [(
                name("status/address-space"),
                crate::execution::Requirement::UpperBound {
                    resource: crate::execution::Capacity::AddressSpaceBytes,
                    amount: rx_domain::types::Counter(268_435_456),
                },
            )]
            .into(),
        )),
        id: name("rx/status-http"),
        effect: Effect::NonActuating,
        executable: python,
        executable_sha256: python_hash,
        files: [(script.clone(), script_hash)].into_iter().collect(),
        fixed_arguments: vec![script.to_string_lossy().into_owned(), "serve".into()],
        arguments: [
            (
                name("bind"),
                Argument::Choice {
                    flag: "--bind".into(),
                    values: vec!["127.0.0.1".into(), "0.0.0.0".into()],
                },
            ),
            (
                name("port"),
                Argument::Port {
                    flag: "--port".into(),
                },
            ),
        ]
        .into_iter()
        .collect(),
        ready: ReadyProbe::HttpStatus {
            port_parameter: name("port"),
        },
    };
    Ok([(program.id.clone(), program)].into_iter().collect())
}

/// Two intended uses of the same diagnostic publisher, not two invented
/// functions. Provenance follows solutions_status.py: counts are computed at
/// startup; operator_api_delegation is a static release declaration.
fn status_readiness() -> crate::use_assessment::ReadinessContract {
    use crate::use_assessment::*;
    let n = |s| Name::new(s).expect("literal");
    let conditions: BTreeMap<_, _> = [
        (
            n("report/current-instance"),
            ReadinessCondition::InstanceMatches,
        ),
        (
            n("report/schema"),
            ReadinessCondition::Equals {
                field: n("schema"),
                expected: ExpectedValue::Text("rx.solutions-status.v1".into()),
                origin: ReportOrigin::ReleaseDeclaration,
            },
        ),
        (
            n("report/native-package-count"),
            ReadinessCondition::Unsigned {
                field: n("native_packages"),
                origin: ReportOrigin::StartupAuditDerived,
            },
        ),
        (
            n("report/support-profile-count"),
            ReadinessCondition::Unsigned {
                field: n("support_profiles"),
                origin: ReportOrigin::StartupCatalogDerived,
            },
        ),
        (
            n("report/simulation-profile-count"),
            ReadinessCondition::Unsigned {
                field: n("simulation_profiles"),
                origin: ReportOrigin::StartupCatalogDerived,
            },
        ),
    ]
    .into();
    let mut connected = conditions.clone();
    connected.insert(
        n("operator/api-delegation"),
        ReadinessCondition::Equals {
            field: n("operator_api_delegation"),
            expected: ExpectedValue::Text("CONNECTED".into()),
            origin: ReportOrigin::ReleaseDeclaration,
        },
    );
    ReadinessContract(
        [
            (
                n("diagnostics/support-summary"),
                ReadinessProfile {
                    endpoint: StatusEndpoint::SupportSummary,
                    conditions,
                },
            ),
            (
                n("diagnostics/operator-connected"),
                ReadinessProfile {
                    endpoint: StatusEndpoint::SupportSummary,
                    conditions: connected,
                },
            ),
        ]
        .into(),
    )
}
