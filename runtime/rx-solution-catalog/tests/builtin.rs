use rx_domain::types::Name;
use rx_solution_catalog::*;
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
#[test]
fn all_mandatory_sources_and_separate_model_roles_are_present() {
    let catalog = builtin_catalog().unwrap();
    assert_eq!(catalog.repositories.len(), 5);
    assert_eq!(catalog.profiles.len(), 22);
    assert_eq!(
        catalog.profile(&name("OM-06")).unwrap().role,
        ControlRole::Manipulator
    );
    assert_eq!(
        catalog.profile(&name("OM-08")).unwrap().role,
        ControlRole::Leader
    );
    for id in ["FFW-02-REV2", "FFW-02-REV3", "FFW-02-REV4"] {
        assert!(catalog.profile(&name(id)).is_some());
    }
    let normal = catalog.profile(&name("OM-06")).unwrap();
    let follower = catalog.profile(&name("OM-07")).unwrap();
    assert_eq!(
        normal
            .controllers
            .iter()
            .find(|c| c.name == "arm_controller")
            .unwrap()
            .joint_order
            .len(),
        6
    );
    assert_eq!(
        follower
            .controllers
            .iter()
            .find(|c| c.name == "arm_controller")
            .unwrap()
            .joint_order
            .len(),
        7
    );
    assert!(
        normal
            .controllers
            .iter()
            .any(|c| c.name == "gripper_controller")
    );
    assert!(
        !follower
            .controllers
            .iter()
            .any(|c| c.name == "gripper_controller")
    );
    let gpio = follower
        .controllers
        .iter()
        .find(|c| c.name == "gpio_command_controller")
        .unwrap();
    let InterfaceDeclaration::Gpio(groups) = &gpio.command_interfaces else {
        panic!("grouped GPIO declaration must be preserved")
    };
    assert_eq!(groups["omy_end"][0].interfaces, ["R LED", "G LED", "B LED"]);
    assert!(
        catalog
            .profiles
            .iter()
            .all(|p| p.evidence_level == EvidenceLevel::SourceObserved)
    );
}
#[test]
fn mandatory_inclusion_cannot_be_weakened_by_catalog_edits() {
    let mut catalog = builtin_catalog().unwrap();
    catalog.repositories.pop();
    assert!(catalog.validate().is_err());
    let mut catalog = builtin_catalog().unwrap();
    catalog
        .profiles
        .retain(|p| p.support_id.as_str() != "FFW-02-REV3");
    assert!(catalog.validate().is_err());
    let mut catalog = builtin_catalog().unwrap();
    catalog.profiles[0].sources[0].commit = "f".repeat(40);
    assert!(catalog.validate().is_err());
    let mut catalog = builtin_catalog().unwrap();
    catalog.profiles[0].commissioning_inputs.clear();
    assert!(catalog.validate().is_err());
}
