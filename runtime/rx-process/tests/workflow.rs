use rx_domain::{canonical, types::*};
use rx_process::{
    CompiledBody,
    workflow::{self, Input},
};
use serde_json::{Value, json};

fn reference() -> Value {
    json!({"catalog":"00000000-0000-4000-8000-000000000001", "id":"00000000-0000-4000-8000-000000000002", "revision":"1", "digest":"11".repeat(32)})
}
fn quantity(data: Value, unit: &str) -> Value {
    json!({"data":data,"unit":unit})
}
fn number(value: f64, unit: &str) -> Value {
    quantity(
        json!({"kind":"NUMBER","range":{"min":value,"max":value}}),
        unit,
    )
}
fn resolved(value: Value) -> Value {
    json!({"value":value,"frame":null,"selected_source":"DEFAULT", "origins":[{"kind":"DEFAULT","reference":null,"path":"fixture","value":value}]})
}
fn artifact() -> Value {
    json!({"schema_id":"test.program.v1","sha256":"22".repeat(32),"size_bytes":"1"})
}
fn fixture() -> Input {
    let mut value: Input = serde_json::from_value(json!({
        "schema":"rx.workflow-compile-input.v1", "process":"test/transfer", "resolution": reference(),
        "report":{
            "schema":"rx.workflow-resolution.v1", "resolver_digest":"33".repeat(32),
            "request":{"workflow":reference(),"contexts":{},"property_sets":[],"overrides":{},"inputs":{},"slot_index":"4"},
            "valid":true,"concrete":true,"status":"RESOLVED_NOT_QUALIFIED","definitions":[],"violations":[],
            "steps":[{"node":"grasp","task":"task/grasp","label":"Grasp","timeout_property":"timeout",
                "properties":{"force":resolved(number(25.0,"N")),"timeout":resolved(number(30.0,"s"))},
                "skills":[{"implementation":resolved(quantity(json!({"kind":"TEXT","value":"test.skill"}),"unitless")),
                    "version":resolved(quantity(json!({"kind":"TEXT","value":"1.0.0"}),"unitless")),
                    "primitive":"grip","parameters":{"force":"force"}}],"declared_capabilities":{},
                "done":{"observation":"gripper/holding","property":reference(),"equals":quantity(json!({"kind":"BOOLEAN","value":true}),"unitless")},
                "on_failure":"STOP","on_unknown":"HOLD_AND_RECONCILE"}]
        },
        "templates":[{"implementation":"test.skill","version":"1.0.0","primitive":"grip","host":"host/simulated",
            "intent":{"kind":"FINITE_ACTION","target":"gripper","profile_digest":"44".repeat(32),"site_config_digest":"55".repeat(32),
                "calibration_digests":[],"resource_set":["gripper"],"execution_timeout_ms":"30000","prepare_validity_ms":"1000",
                "completion_rule":"complete","cancel_rule":"stop","body":{"program":{"program":artifact(),"parameter_set":artifact()}}},
            "parameters":{"force":{"unit":"N","value_type":"NUMBER","frame":null}}}]
    })).unwrap();
    pin(&mut value);
    value
}
fn pin(input: &mut Input) {
    input.resolution.digest =
        canonical::digest("RX-WORKFLOW-RESOLUTION-v1", &input.report).unwrap();
}

#[test]
fn parameters_and_trace_bind_exact_report_slot_and_existing_process_compiler() {
    let input = fixture();
    let first = workflow::compile(&input).unwrap();
    let second = workflow::compile(&input).unwrap();
    assert_eq!(
        canonical::bytes(&first.process).unwrap(),
        canonical::bytes(&second.process).unwrap()
    );
    assert_eq!(first.assets, second.assets);
    assert_eq!(first.trace[0].node.as_str(), "grasp");
    let parameter = &first.trace[0].parameters;
    let bytes = &first.assets[&parameter.sha256];
    assert_eq!(rx_package::content_digest(bytes), parameter.sha256);
    assert_eq!(bytes.len() as u64, parameter.size_bytes.0);
    let payload: workflow::Parameters = canonical::decode_json(bytes).unwrap();
    assert_eq!(payload.resolution, input.resolution);
    assert_eq!(payload.slot_index, Counter(4));
    assert_eq!(
        payload.values[&Name::new("force").unwrap()].value,
        input.report.steps[0].properties[&Name::new("force").unwrap()].value
    );
    let body = &first.process.bindings[&first.trace[0].binding].intent.body;
    let rx_domain::intent::Body::Program(goal) = body else {
        panic!("program required");
    };
    assert_eq!(goal.parameter_set, *parameter);
    let xml = rx_process::bt_xml::generate(&first.process).unwrap();
    assert!(xml.contains("RXOperation"));
    assert!(matches!(
        first.process.root.body,
        CompiledBody::Sequence { .. }
    ));
}

#[test]
fn changed_values_or_slot_change_bound_assets_without_template_edits() {
    let a = fixture();
    let original = workflow::compile(&a).unwrap();
    let mut b = a.clone();
    b.report.request.slot_index = Counter(5);
    pin(&mut b);
    let next = workflow::compile(&b).unwrap();
    assert_ne!(original.trace[0].parameters, next.trace[0].parameters);
    assert_eq!(original.process.source_digest, next.process.source_digest);
    let key = Name::new("force").unwrap();
    b.report.steps[0].properties.get_mut(&key).unwrap().value =
        serde_json::from_value(number(17.5, "N")).unwrap();
    pin(&mut b);
    assert_ne!(
        next.trace[0].parameters,
        workflow::compile(&b).unwrap().trace[0].parameters
    );
}

#[test]
fn forged_reference_bounds_units_frames_and_missing_mappings_are_refused() {
    let original = fixture();
    let mut changed = original.clone();
    changed.report.request.slot_index = Counter(6);
    assert!(workflow::compile(&changed).is_err());
    for field in [
        "range",
        "unit",
        "frame",
        "timeout",
        "mapping",
        "implementation",
        "status",
        "multi-skill",
    ] {
        let mut value = original.clone();
        let force = Name::new("force").unwrap();
        match field {
            "range" => {
                value.report.steps[0]
                    .properties
                    .get_mut(&force)
                    .unwrap()
                    .value = serde_json::from_value(quantity(
                    json!({"kind":"NUMBER","range":{"min":20,"max":40}}),
                    "N",
                ))
                .unwrap()
            }
            "unit" => {
                value.report.steps[0]
                    .properties
                    .get_mut(&force)
                    .unwrap()
                    .value
                    .unit = Name::new("kg").unwrap()
            }
            "frame" => {
                value.report.steps[0]
                    .properties
                    .get_mut(&force)
                    .unwrap()
                    .frame = Some("world".into())
            }
            "timeout" => value.templates[0].intent.execution_timeout_ms = Counter(1),
            "mapping" => {
                value.report.steps[0].skills[0].parameters.clear();
            }
            "implementation" => value.templates[0].version = "2.0.0".into(),
            "status" => value.report.status = "BOUNDED_INPUT_NOT_EXECUTABLE".into(),
            "multi-skill" => {
                let duplicate = value.report.steps[0].skills[0].clone();
                value.report.steps[0].skills.push(duplicate);
            }
            _ => unreachable!(),
        }
        pin(&mut value);
        assert!(workflow::compile(&value).is_err(), "{field}");
    }
}
