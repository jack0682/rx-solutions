use rx_domain::types::*;
use rx_storage::SqliteRepository;
use rx_supervisor::{
    decision::*,
    model::Process,
    registration::{
        Declaration, Registry,
        diagnostic::{Consumer, catalog},
    },
};
fn n(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn policy() -> Policy {
    Policy {
        authorities: [(
            n("test/issuer-key"),
            Authority {
                issuer: n("test/area-issuer"),
                public_key: [7; 32],
                operating_area: n("test/area"),
                roles: [n("snapshot-after-preparation")].into(),
                kinds: [Kind::InitialBinding].into(),
                max_ttl_ms: Counter(1000),
            },
        )]
        .into(),
    }
}
#[test]
fn author_anchors_are_pinned_and_site_data_cannot_override_them() {
    let dir = tempfile::tempdir().unwrap();
    let mut authored = catalog();
    assert!(authored.decision_policy.is_none());
    let legacy = serde_json::to_value(&authored).unwrap();
    assert!(legacy.get("decision_policy").is_none());
    authored.decision_policy = Some(policy());
    let reference = authored.reference().unwrap();
    let mut registry =
        Registry::new(SqliteRepository::open(dir.path().join("consumer.db")).unwrap());
    let id = registry
        .register(Declaration {
            label: n("authored-consumer"),
            catalog: reference.clone(),
        })
        .unwrap()
        .registration
        .id;
    let accepted = Consumer::open(registry, id.clone(), authored.clone()).unwrap();
    let registry = accepted.into_registry();
    authored
        .decision_policy
        .as_mut()
        .unwrap()
        .authorities
        .get_mut(&n("test/issuer-key"))
        .unwrap()
        .public_key = [9; 32];
    assert_ne!(authored.reference().unwrap(), reference);
    assert!(Consumer::open(registry, id, authored).is_err());
    let site = serde_json::json!({"id":"x","program":"rx/status-http","parameters":{},"depends_on":[],"startup_timeout_ms":"1000","shutdown_timeout_ms":"1000","restart_limit":"0","restart_backoff_ms":"100","decision_policy":policy()});
    assert!(serde_json::from_value::<Process>(site).is_err());
}

#[test]
fn f6_authority_count_is_one_through_eight_without_g4_uniqueness_restrictions() {
    let authority = policy().authorities.into_values().next().unwrap();
    for count in [0, 1, 8, 9] {
        let mut authored = catalog();
        authored.decision_policy = Some(Policy {
            // Generic F6 intentionally permits multiple keys for the same area/issuer.
            authorities: (0..count)
                .map(|i| (n(&format!("test/key-{i}")), authority.clone()))
                .collect(),
        });
        let result = authored.reference();
        assert_eq!(result.is_ok(), (1..=8).contains(&count));
        println!("F6-count-{count}: {result:?}");
    }
}
