//! Release-owned launch recipes. A site plan selects parameters, not executable paths or effect classes.
use crate::{Error, Result, model::*, process::verify};
use rx_domain::{canonical, types::*};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

mod services;
pub use services::{
    ConfigurationPin, Initializer, ServiceConfigurations, ServiceInput, ServiceRole,
    add_guarded_services, validate_service_plan,
};

#[derive(Deserialize)]
struct Inventory {
    schema: String,
    files: BTreeMap<String, Digest>,
    external_files: BTreeMap<String, Digest>,
}
pub fn release_programs(root: &Path) -> Result<BTreeMap<Name, Program>> {
    let bytes = std::fs::read(root.join("manifests/runtime-files.json"))?;
    let inventory: Inventory =
        canonical::decode_json(&bytes).map_err(|e| Error::Invalid(e.to_string()))?;
    if inventory.schema != "rx.solutions-runtime-files.v1" {
        return Err(Error::Invalid("release inventory schema".into()));
    }
    let script = root.join("tools/solutions_status.py");
    let digest = |v: Option<&Digest>| -> Result<Digest> {
        v.copied()
            .ok_or_else(|| Error::Invalid("required release file absent".into()))
    };
    verify(
        &root.join("catalogs/device-support.v1.json"),
        digest(inventory.files.get("catalogs/device-support.v1.json"))?,
    )?;
    let script_hash = digest(inventory.files.get("tools/solutions_status.py"))?;
    let python = PathBuf::from("/usr/bin/python3");
    let python_hash = digest(inventory.external_files.get("/usr/bin/python3"))?;
    verify(&script, script_hash)?;
    verify(&python, python_hash)?;
    let name = |s: &str| Name::new(s).expect("literal name");
    let program = Program {
        functional_readiness: Some(status_readiness()),
        execution_requirements: Some(crate::execution::Requirements(BTreeMap::new())),
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
