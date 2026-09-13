use rx_domain::types::Name;
use rx_solution_catalog::*;
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
#[test]
fn builtin_is_explicit_simulation_with_independent_joint_sets() {
    let catalog = builtin_catalog().unwrap();
    assert!(catalog.repositories.is_empty());
    assert_eq!(catalog.profiles.len(), 4);
    let arm = catalog.profile(&name("SIM-JTC-6DOF")).unwrap();
    let follower = catalog.profile(&name("SIM-JTC-7DOF")).unwrap();
    assert_eq!(arm.role, ControlRole::Manipulator);
    assert_eq!(follower.role, ControlRole::Follower);
    assert_eq!(arm.controllers[0].joint_order.len(), 6);
    assert_eq!(follower.controllers[0].joint_order.len(), 7);
    assert!(
        catalog
            .profiles
            .iter()
            .all(|p| p.evidence_level == EvidenceLevel::SimulationFixture
                && p.sources.is_empty()
                && !p.fixture_sources.is_empty())
    );
}
#[test]
fn provenance_and_commissioning_cannot_be_erased_or_relabeled() {
    let mut catalog = builtin_catalog().unwrap();
    catalog.profiles[0].fixture_sources.clear();
    assert!(catalog.validate().is_err());
    let mut catalog = builtin_catalog().unwrap();
    catalog.profiles[0].evidence_level = EvidenceLevel::SourceObserved;
    assert!(catalog.validate().is_err());
    let mut catalog = builtin_catalog().unwrap();
    catalog.profiles[0].commissioning_inputs.clear();
    assert!(catalog.validate().is_err());
    let mut catalog = builtin_catalog().unwrap();
    catalog.profiles.push(catalog.profiles[0].clone());
    assert!(catalog.validate().is_err());
}
#[test]
fn external_sources_are_vendor_neutral_and_still_revision_bound() {
    let mut catalog = builtin_catalog().unwrap();
    catalog.profiles.truncate(1);
    catalog.repositories.push(RepositoryPin {
        repository: name("device-driver"),
        url: "https://example.org/devices/driver.git".into(),
        commit: "a".repeat(40),
    });
    let profile = &mut catalog.profiles[0];
    let fixture = profile.fixture_sources.remove(0);
    profile.evidence_level = EvidenceLevel::SourceObserved;
    profile.sources.push(SourceFile {
        repository: name("device-driver"),
        commit: "a".repeat(40),
        path: fixture.path,
        sha256: fixture.sha256,
    });
    assert!(catalog.validate().is_ok());
    catalog.profiles[0].sources[0].commit = "b".repeat(40);
    assert!(catalog.validate().is_err());
    catalog.profiles[0].sources[0].commit = "a".repeat(40);
    catalog.repositories.push(catalog.repositories[0].clone());
    assert!(catalog.validate().is_err());
}
