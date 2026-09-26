//! Release-owned launch recipes. A site plan selects parameters, not executable paths or effect classes.
use crate::{Error, Result, model::*, process::verify};
use rx_domain::{canonical, types::*};

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

mod ai_sapiens;
mod ai_worker;
mod dhi;
mod open_manipulator;
mod services;
pub use services::{
    ConfigurationPin, Initializer, ServiceConfigurations, ServiceInput, ServiceRole,
    add_guarded_services, services_from_release, validate_service_plan,
};

/// Checkpoint diagnostics, never a current permission receipt.
pub fn release_boundary() -> serde_json::Value {
    let mut boundary = serde_json::json!({
        "trust": "TRUSTED_INSTALLED_RUST_BINARIES_AND_OS",
        "source_assets": "STATUS_SCRIPT_AND_DEVICE_CATALOG_MATCH_COMPILED_SOURCE_CONTENT",
        "inventory": "CONSISTENCY_INDEX_AUTHENTICATED_BY_COMPILED_DEVELOPMENT_ROOT",
        "authenticated_immutable_provenance": "NOT_ESTABLISHED_FOR_OS_AND_VERIFIER; DEVELOPMENT_RELEASE_CONTENT_AUTHENTICATED_AT_CHECKPOINT",
        "release_key": rx_package::release::root::KEY_ID,
        "development_release_signing_custody": rx_package::release::root::DEVELOPMENT_SIGNING_CUSTODY,
        "development_release_previous_root": rx_package::release::root::PREVIOUS_ROOT_STATUS,
        "development_release_dual_root_window": rx_package::release::root::DUAL_ROOT_WINDOW,
        "product_release_custody_and_rotation": "NOT_ESTABLISHED",
        "operating_area_judge": "OPT_IN_DEVELOPMENT_OFFLINE_RULE_JUDGE; PHYSICAL_SAFETY_QUALITY_EQUIPMENT_QUALIFICATION_NOT_GRANTED",
        "operating_areas": "TWO_COMPILED_DEVELOPMENT_AREAS_WITH_DISTINCT_KEYS; SITE_ENROLLMENT_REFUSED; NO_NETWORK_OR_PHYSICAL_AUTHORITY",
        "area_declaration_uniqueness": "COMPILED_CATALOG_ONLY; GENERIC_F6_POLICY_RETAINS_1_TO_8_AUTHORITIES",
        "judge_delivery_order": "COOPERATING_MAILBOX_LOCK_THROUGH_WORK_COMMIT; NONCOOPERATING_PUBLICATION_NOT_ORDERED",
        "dynamixel": "ONE_HOST_OWNED_HELPER; OFFICIAL_SDK_4.1.0_PROTOCOL2_PING; SIMULATED_TRANSPORT_ONLY; REAL_ENDPOINT_REFUSED",
        "dynamixel_physical_qualification": "NOT_PERFORMED",
        "dhi": "OPTIONAL_RELEASE_RECIPE; FRESH_PTY_MODEL_ONLY; ORIGINAL_DHI_COMMAND_AUTHOR; CM_LIFECYCLE; RX_DESCRIPTOR_CUSTODY",
        "dhi_effect_classification": "NONACTUATING_FRESH_PTY_REGISTERED_RELEASE_VERIFIED",
        "dhi_registered_release_admission_verified": true,
        "dhi_registered_release_admission_reason": "SIGNED_RESIDENT_PATH_R1_PRESERVED; FRESH_PTY_ONLY; PHYSICAL_NOT_QUALIFIED",
        "dhi_direct_mechanism_scope": "FRESH_PTY_ONLY; NO_PHYSICAL_ENDPOINT_OR_ADOPTION; PROCESS_EXIT_DOES_NOT_CLEAR_MODEL_RESIDUAL_OR_UNCONFIRMED_STOP",
        "dhi_same_uid_tampering": "OUTSIDE_TRUST_BOUNDARY_CHMOD_PTRACE",
        "dhi_compose_s6_reuse": "NOT_ESTABLISHED",
        "dhi_ros_admin_authority": "NOT_ESTABLISHED",
        "ai_worker": "2.2.7; PINNED_SOURCE; L3_SIMULATION_RECIPE_ONLY; PHYSICAL_START_WITHHELD",
        "ai_worker_l3_owner": "ONE_SERVICE_GENERATION_OWNER; AUTHORITY_TO_CREATE_REPLACEMENT_ROOT_PROCESS_TREE",
        "ai_worker_compose_restart_admission_verified": true,
        "ai_worker_compose_restart_disposition": "BLOCKED_AND_RECORDED_AT_RX_SERVICE_GENERATION_GATE",
        "ai_worker_zed_assets": "NOT_INSTALLED_OR_QUALIFIED; PHYSICAL_PROFILE_REFUSED",
        "ai_worker_rt_authority": "NOT_GRANTED_OR_QUALIFIED; PHYSICAL_PROFILE_REFUSED",
        "open_manipulator_l3_reuse": "VARIANT_REQUIRED; CATALOG_BOUND_SERVICE_IDENTITY; S6_PROCESS_RESTART_BLOCKED_AND_RECORDED",
        "sdk_baseline_complete": false,
        "robotis_bundle_complete": false,
        "work_commit_residuals": ["TTL_CONTINUES_DURING_POST_CUT_IO", "HTTP_OBSERVATION_IS_AS_OF"],
        "whole_state_rollback_or_deletion": "NOT_DETECTED",
        "offline_revocation_freshness": rx_package::release::root::OFFLINE_REVOCATION_FRESHNESS,
        "interval": "EXPLICIT_CALLER_DRIVEN_CHECKPOINTS",
        "monitor": "NO_TIMER_OR_BACKGROUND_MONITOR",
        "remaining_interval": "TRUSTED_INSTALLATION_STABILITY_BETWEEN_BYTE_CHECK_AND_USE",
        "inspection": "OBSERVATION_ONLY; NOT_DURABLE_ADMISSION_OR_WORK_PERMISSION"
    });
    let object = boundary.as_object_mut().expect("release boundary object");
    for (key, value) in [
        (
            "open_manipulator",
            "5.1.2; L3_SIMULATION_ONLY; PHYSICAL_START_WITHHELD",
        ),
        ("open_manipulator_realsense", "NOT_INSTALLED_OR_QUALIFIED"),
        ("open_manipulator_maintenance_handoff", "NOT_ESTABLISHED"),
        (
            "ai_sapiens",
            "0.2.2; ASSET_GATE_SIMULATION_ONLY; PHYSICAL_START_WITHHELD",
        ),
        (
            "ai_sapiens_onnx_runtime",
            "1.23.2_DECLARED_SEPARATELY_FROM_POLICY_ASSETS",
        ),
        (
            "ai_sapiens_policy_assets",
            "FOUR_HASHED_POLICIES; REQUIRED_BEFORE_OUTPUT",
        ),
        (
            "ai_sapiens_residual_control",
            "MISSING_OR_FAILED_POLICY_OUTPUTS_ZERO",
        ),
        (
            "remaining_robotis_product",
            "NONE_UNTOUCHED; BUNDLE_QUALIFICATION_STILL_REQUIRED",
        ),
        (
            "robotis_bundle_completion_gaps",
            "FULL_NATIVE_BUILDS; CROSS_PRODUCT_COMPATIBILITY; PHYSICAL_QUALIFICATION; ASSET_LICENSE_CLOSURE; DEPLOYMENT_RECOVERY",
        ),
    ] {
        object.insert(key.into(), value.into());
    }
    boundary
}

/// Preserve F12's compiled-source refusal before any writable-state access.
/// This is a rejection-only preflight, never release authentication/admission.
pub fn preflight_source_assets(root: &Path) -> Result<()> {
    let inventory: rx_package::release::Inventory =
        canonical::decode_json(&release_metadata(root, "runtime-files.json")?)
            .map_err(|e| rx_package::release::Error::Malformed(e.to_string()))?;
    for (path, bytes) in [
        (
            "tools/solutions_status.py",
            include_bytes!("../../../native/support/solutions_status.py").as_slice(),
        ),
        (
            "catalogs/device-support.v1.json",
            include_bytes!("../../../catalogs/device-support.v1.json").as_slice(),
        ),
    ] {
        let expected = rx_package::content_digest(bytes);
        if inventory.files.get(path) != Some(&expected) {
            return Err(rx_package::release::Error::Content(
                "release/source-pin-mismatch; inventory cannot authorize changed source assets"
                    .into(),
            )
            .into());
        }
        verify(&root.join(path), expected)
            .map_err(|e| rx_package::release::Error::Content(e.to_string()))?;
    }
    dhi::source_pins(root, &inventory)?;
    ai_worker::source_pins(root, &inventory)?;
    ai_sapiens::source_pins(root, &inventory)?;
    open_manipulator::source_pins(root, &inventory)?;
    Ok(())
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
    let mut programs: BTreeMap<Name, Program> = [(program.id.clone(), program)].into();
    if let Some(dhi) = dhi::program(root, release)? {
        programs.insert(dhi.id.clone(), dhi);
    }
    if let Some(ai_worker) = ai_worker::program(root, release)? {
        programs.insert(ai_worker.id.clone(), ai_worker);
    }
    if let Some(ai_sapiens) = ai_sapiens::program(root, release)? {
        programs.insert(ai_sapiens.id.clone(), ai_sapiens);
    }
    if let Some(open_manipulator) = open_manipulator::program(root, release)? {
        programs.insert(open_manipulator.id.clone(), open_manipulator);
    }
    Ok(programs)
}

/// Explicit compiled opt-in. The ordinary release catalog stays unconfigured;
/// site input selects this recipe but cannot provide keys or policy fields.
pub fn development_work_program(
    root: &Path,
    release: &rx_package::release::VerifiedRelease,
) -> Result<Program> {
    development_area_program(root, release, rx_package::operating_area::SUPPORT.program)
}
/// Selection names only an existing compiled declaration. No site key or policy
/// argument exists, and catalog validation precedes authority-map construction.
pub fn development_area_program(
    root: &Path,
    release: &rx_package::release::VerifiedRelease,
    program: &str,
) -> Result<Program> {
    let area = rx_package::operating_area::for_program(program)
        .map_err(|e| Error::Invalid(e.into()))?
        .ok_or_else(|| Error::Invalid("area-catalog/program-not-authored".into()))?;
    let mut work = programs_from_release(root, release)?
        .remove(&Name::new("rx/status-http").expect("literal"))
        .expect("standard status recipe");
    let name = |s: &str| Name::new(s).expect("literal name");
    work.id = name(area.program);
    work.decision_policy = Some(crate::decision::Policy {
        authorities: [(
            name(area.key_id),
            crate::decision::Authority {
                issuer: name(area.issuer),
                public_key: area.public_key,
                operating_area: name(area.area),
                roles: [name(area.role)].into(),
                kinds: [crate::decision::Kind::WorkUse].into(),
                max_ttl_ms: Counter(area.max_ttl_ms),
            },
        )]
        .into(),
    });
    Ok(work)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dhi_registered_release_admission_is_explicitly_verified_and_bounded() {
        let boundary = release_boundary();
        assert_eq!(
            boundary["dhi_effect_classification"],
            "NONACTUATING_FRESH_PTY_REGISTERED_RELEASE_VERIFIED"
        );
        assert_eq!(boundary["dhi_registered_release_admission_verified"], true);
        assert_eq!(
            boundary["dhi_registered_release_admission_reason"],
            "SIGNED_RESIDENT_PATH_R1_PRESERVED; FRESH_PTY_ONLY; PHYSICAL_NOT_QUALIFIED"
        );
        assert_eq!(
            boundary["development_release_signing_custody"],
            "ESTABLISHED_BY_TWO_COPY_RECOVERY_REHEARSAL"
        );
        assert_eq!(
            boundary["development_release_previous_root"],
            "RETIRED_NOT_ACCEPTED"
        );
        assert_eq!(boundary["development_release_dual_root_window"], false);
        assert_eq!(
            boundary["product_release_custody_and_rotation"],
            "NOT_ESTABLISHED"
        );
    }

    #[test]
    fn ai_worker_l3_boundary_is_blocking_not_observation_only() {
        let boundary = release_boundary();
        assert_eq!(
            boundary["ai_worker_compose_restart_admission_verified"],
            true
        );
        assert_eq!(
            boundary["ai_worker_compose_restart_disposition"],
            "BLOCKED_AND_RECORDED_AT_RX_SERVICE_GENERATION_GATE"
        );
        assert_eq!(
            boundary["open_manipulator_l3_reuse"],
            "VARIANT_REQUIRED; CATALOG_BOUND_SERVICE_IDENTITY; S6_PROCESS_RESTART_BLOCKED_AND_RECORDED"
        );
        assert_eq!(boundary["dhi_compose_s6_reuse"], "NOT_ESTABLISHED");
        assert_eq!(boundary["robotis_bundle_complete"], false);
    }

    #[test]
    fn open_manipulator_l3_is_an_explicit_variant_not_unchanged_reuse() {
        let boundary = release_boundary();
        assert_eq!(
            boundary["open_manipulator_l3_reuse"],
            "VARIANT_REQUIRED; CATALOG_BOUND_SERVICE_IDENTITY; S6_PROCESS_RESTART_BLOCKED_AND_RECORDED"
        );
        assert_eq!(
            boundary["open_manipulator_realsense"],
            "NOT_INSTALLED_OR_QUALIFIED"
        );
        assert_eq!(
            boundary["open_manipulator_maintenance_handoff"],
            "NOT_ESTABLISHED"
        );
        assert_eq!(
            boundary["remaining_robotis_product"],
            "NONE_UNTOUCHED; BUNDLE_QUALIFICATION_STILL_REQUIRED"
        );
        assert_eq!(boundary["robotis_bundle_complete"], false);
    }
}
