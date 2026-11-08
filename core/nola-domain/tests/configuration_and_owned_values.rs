use nola_domain::configuration_patch::ConversationConfigurationPatch;
use serde_json::json;

#[test]
fn configuration_wire_decodes_once_into_a_validated_owned_patch() {
    let raw=json!({"permissionProfile":"cautious","modelOverrides":{"local:model":{"temperature":0}}}).to_string();
    let patch:ConversationConfigurationPatch=serde_json::from_value(json!(raw)).unwrap();
    assert_eq!(patch.fields()["permissionProfile"],"cautious");
    assert_eq!(serde_json::to_value(&patch).unwrap(),json!(raw));
    for invalid in ["{}",r#"{"permissionProfile":"elevated"}"#,r#"{"unknown":true}"#,
        r#"{"modelOverrides":{"local:model":{"topK":0}}}"#] {
        assert!(serde_json::from_value::<ConversationConfigurationPatch>(json!(invalid)).is_err());
    }
}

#[test]
fn sampling_intent_is_a_typed_per_model_configuration_field() {
    let valid = json!({"modelOverrides":{"configured:fixture":{"samplingOverride":false,"reasoningPreference":"automatic"}}}).to_string();
    let patch: ConversationConfigurationPatch = valid.parse().unwrap();
    assert_eq!(patch.fields()["modelOverrides"]["configured:fixture"]["samplingOverride"], false);
    let invalid = json!({"modelOverrides":{"configured:fixture":{"samplingOverride":"false"}}}).to_string();
    assert!(invalid.parse::<ConversationConfigurationPatch>().is_err());
}

#[test]
fn moved_values_do_not_depend_on_runtime_adapter_or_platform_modules() {
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    assert!(!root.join("nola-legacy-agent-adapter/Cargo.toml").exists());
    for module in ["execution_backend", "data_source", "memory_projection", "configuration_patch"] {
        let source=std::fs::read_to_string(root.join(format!("nola-domain/src/{module}.rs"))).unwrap();
        for forbidden in ["nola_core::", "nola_legacy_agent_adapter::", "Utc::now()", "std::fs::"] {
            assert!(!source.contains(forbidden),"{module} acquired {forbidden}");
        }
    }
}
