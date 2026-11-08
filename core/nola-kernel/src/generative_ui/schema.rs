// SPDX-License-Identifier: Apache-2.0
//! Pinned schemas resolved entirely in memory; model-supplied URLs are never fetched.
use jsonschema::Validator;
use nola_domain::generative_ui::UiError;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

pub const SUPPORTED_COMPONENTS: &[&str] = &["Text", "Icon", "Row", "Column", "List", "Card", "Tabs", "Modal", "Divider", "Button", "TextField", "CheckBox", "ChoicePicker", "Slider", "DateTimeInput"];
pub const SUPPORTED_FUNCTIONS: &[&str] = &["required", "regex", "length", "numeric", "and", "or", "not"];
pub const NOLA_CATALOG_ID: &str = "https://nola.app/a2ui/v0.9.1/catalog.json";
const SERVER: &str = include_str!("../../../../third_party/a2ui/spec/specification/v0_9_1/json/server_to_client.json");
const CLIENT: &str = include_str!("../../../../third_party/a2ui/spec/specification/v0_9_1/json/client_to_server.json");
const COMMON: &str = include_str!("../../../../third_party/a2ui/spec/specification/v0_9_1/json/common_types.json");
const CATALOG: &str = include_str!("../../../../third_party/a2ui/spec/specification/v0_9_1/catalogs/basic/catalog.json");

// Register the entire pinned reference graph before compilation. Nested catalog
// references may be resolved after the validator registry has been populated,
// when the jsonschema library deliberately disallows further retrieval.
fn build_validator(schema: &Value) -> Result<Validator, String> {
    let mut options = jsonschema::draft202012::options();
    for (uri, source) in [
        ("https://a2ui.org/specification/v0_9/common_types.json", COMMON),
        ("https://a2ui.org/specification/v0_9/catalog.json", CATALOG),
        ("https://a2ui.org/specification/v0_9/catalogs/basic/catalog.json", CATALOG),
    ] {
        let contents: Value = serde_json::from_str(source).map_err(|error| error.to_string())?;
        let resource = jsonschema::Resource::from_contents(contents).map_err(|error| error.to_string())?;
        options = options.with_resource(uri, resource);
    }
    options.build(schema).map_err(|error| error.to_string())
}

static MESSAGE_VALIDATOR: OnceLock<Result<Validator, String>> = OnceLock::new();
static CLIENT_VALIDATOR: OnceLock<Result<Validator, String>> = OnceLock::new();

pub fn validate_client_message(value: &Value) -> Result<(), UiError> {
    if value.get("version").and_then(Value::as_str) != Some(nola_domain::generative_ui::A2UI_WIRE_VERSION) {
        return Err(UiError::UnsupportedProtocol);
    }
    let validator = CLIENT_VALIDATOR.get_or_init(|| {
        let schema: Value = serde_json::from_str(CLIENT).map_err(|error| error.to_string())?;
        build_validator(&schema)
    });
    match validator {
        Ok(validator) if validator.is_valid(value) => Ok(()),
        _ => Err(UiError::InvalidMessage),
    }
}
pub fn validate_message(value: &Value) -> Result<(), UiError> {
    let validator = MESSAGE_VALIDATOR.get_or_init(|| {
        let schema: Value = serde_json::from_str(SERVER).map_err(|error| error.to_string())?;
        build_validator(&schema)
    });
    match validator {
        Ok(validator) if validator.is_valid(value) => Ok(()),
        _ => Err(UiError::InvalidMessage),
    }
}

pub fn catalog_digest() -> String {
    static DIGEST: OnceLock<String> = OnceLock::new();
    DIGEST.get_or_init(compute_catalog_digest).clone()
}

fn compute_catalog_digest() -> String {
    let mut digest = Sha256::new();
    digest.update(NOLA_CATALOG_ID.as_bytes());
    digest.update(CATALOG.as_bytes());
    digest.update(COMMON.as_bytes());
    digest.update(b"nola:event-actions-only:summary-budget:");
    digest.update((nola_domain::generative_ui::UI_MAX_SUMMARY_BYTES as u64).to_be_bytes());
    // The contract changes when our admitted subset changes even if the
    // pinned upstream document has not changed.
    for name in SUPPORTED_COMPONENTS.iter().chain(SUPPORTED_FUNCTIONS.iter()) {
        digest.update((name.len() as u64).to_be_bytes());
        digest.update(name.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

pub fn catalog_schema() -> Result<Value, UiError> {
    let mut catalog: Value = serde_json::from_str(CATALOG).map_err(|_| UiError::InvalidMessage)?;
    catalog["catalogId"] = Value::String(NOLA_CATALOG_ID.into());
    let common: Value = serde_json::from_str(COMMON).map_err(|_| UiError::InvalidMessage)?;
    let mut event_action = common.pointer("/$defs/Action/oneOf/0").ok_or(UiError::InvalidMessage)?.clone();
    *event_action.pointer_mut("/properties/event/properties/context/additionalProperties/$ref")
        .ok_or(UiError::InvalidMessage)? = Value::String("https://a2ui.org/specification/v0_9/common_types.json#/$defs/DynamicValue".into());
    // Function implementations are separately negotiated. Until the trusted
    // native-effect capability is wired, Button advertises events only.
    let action = catalog.pointer_mut("/components/Button/allOf/3/properties/action")
        .ok_or(UiError::InvalidMessage)?;
    *action = event_action;
    catalog.get_mut("components").and_then(Value::as_object_mut)
        .ok_or(UiError::InvalidMessage)?
        .retain(|name, _| SUPPORTED_COMPONENTS.contains(&name.as_str()));
    catalog.get_mut("functions").and_then(Value::as_object_mut)
        .ok_or(UiError::InvalidMessage)?
        .retain(|name, _| SUPPORTED_FUNCTIONS.contains(&name.as_str()));
    for (definition, namespace, supported) in [
        ("anyComponent", "components", SUPPORTED_COMPONENTS),
        ("anyFunction", "functions", SUPPORTED_FUNCTIONS),
    ] {
        let alternatives = catalog.get_mut("$defs").and_then(|defs| defs.get_mut(definition))
            .and_then(|schema| schema.get_mut("oneOf")).and_then(Value::as_array_mut)
            .ok_or(UiError::InvalidMessage)?;
        let prefix = format!("#/{namespace}/");
        alternatives.retain(|alternative| alternative.get("$ref").and_then(Value::as_str)
            .and_then(|reference| reference.strip_prefix(&prefix))
            .is_some_and(|name| supported.contains(&name)));
        if alternatives.len() != supported.len() { return Err(UiError::InvalidMessage); }
    }
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admitted_catalog_has_no_dangling_local_references() {
        let catalog = catalog_schema().unwrap();
        let mut pending = vec![&catalog];
        while let Some(value) = pending.pop() {
            match value {
                Value::Object(object) => {
                    if let Some(reference) = object.get("$ref").and_then(Value::as_str)
                        .and_then(|reference| reference.strip_prefix('#')) {
                        assert!(catalog.pointer(reference).is_some(), "dangling reference: {reference}");
                    }
                    pending.extend(object.values());
                }
                Value::Array(array) => pending.extend(array),
                _ => {}
            }
        }
        assert_eq!(catalog["components"].as_object().unwrap().len(), SUPPORTED_COMPONENTS.len());
        assert_eq!(catalog["functions"].as_object().unwrap().len(), SUPPORTED_FUNCTIONS.len());
    }
}

#[cfg(test)]
mod action_catalog_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn catalog_advertises_event_actions_and_parser_rejects_unconnected_native_functions() {
        let catalog = catalog_schema().unwrap();
        let action = catalog.pointer("/components/Button/allOf/3/properties/action").unwrap();
        assert_eq!(action["required"], json!(["event"]));
        assert!(action["properties"].get("functionCall").is_none());
        assert!(catalog["functions"].get("openUrl").is_none());
        let native = json!({"version":"v0.9.1", "updateComponents": {"surfaceId":"form", "components":[
            {"id":"root","component":"Button","child":"label","action":{"functionCall":{"call":"openUrl","args":{"url":"https://example.com"}}}},
            {"id":"label","component":"Text","text":"Open"}
        ]}});
        assert!(super::super::parse_a2ui_message(&serde_json::to_vec(&native).unwrap()).is_err());
        let mut event = native;
        event["updateComponents"]["components"][0]["action"] = json!({"event":{"name":"open_requested","context":{"url":"https://example.com"}}});
        let parsed = super::super::parse_a2ui_message(&serde_json::to_vec(&event).unwrap());
        if parsed.is_err() {
            let validator = MESSAGE_VALIDATOR.get().unwrap().as_ref().unwrap();
            for error in validator.iter_errors(&event) { eprintln!("{error:?}"); }
        }
        assert!(parsed.is_ok(), "{parsed:?}");
    }
}

/// A self-contained model-visible contract, derived from the same pinned wire
/// and admitted catalog schemas as execution. All references remain local.
pub fn tool_input_schema() -> Result<Value, UiError> {
    fn localize(value: &mut Value, namespace: &str) -> Result<(), UiError> {
        match value {
            Value::Object(fields) => {
                fields.remove("$id");
                fields.remove("$schema");
                if let Some(Value::String(reference)) = fields.get_mut("$ref") {
                    let (document, fragment) = reference.split_once('#').unwrap_or((reference.as_str(), ""));
                    let target = match document {
                        "" => namespace,
                        "catalog.json" | "https://a2ui.org/specification/v0_9/catalog.json"
                        | "https://a2ui.org/specification/v0_9/catalogs/basic/catalog.json" => "catalog",
                        "https://a2ui.org/specification/v0_9/common_types.json" => "common",
                        _ => return Err(UiError::InvalidMessage),
                    };
                    *reference = format!("#/$defs/{target}{fragment}");
                }
                for child in fields.values_mut() { localize(child, namespace)?; }
            }
            Value::Array(values) => for child in values { localize(child, namespace)?; },
            _ => {}
        }
        Ok(())
    }
    let mut message: Value = serde_json::from_str(SERVER).map_err(|_| UiError::InvalidMessage)?;
    for definition in message["$defs"].as_object_mut().ok_or(UiError::InvalidMessage)?.values_mut() {
        definition["properties"]["version"] = serde_json::json!({"const": "v0.9.1"});
    }
    message["$defs"]["CreateSurfaceMessage"]["properties"]["createSurface"]["properties"]["catalogId"] = serde_json::json!({"const":NOLA_CATALOG_ID});
    let mut common: Value = serde_json::from_str(COMMON).map_err(|_| UiError::InvalidMessage)?;
    let mut catalog = catalog_schema()?;
    localize(&mut message, "message")?;
    localize(&mut common, "common")?;
    localize(&mut catalog, "catalog")?;
    let mut input = nola_domain::generative_ui_tool::UiToolInput::input_schema();
    input["$defs"] = serde_json::json!({"message":message,"common":common,"catalog":catalog});
    input["properties"]["messages"]["items"] = serde_json::json!({"$ref":"#/$defs/message"});
    input["examples"] = serde_json::json!([minimal_form_example()]);
    Ok(input)
}

pub fn minimal_form_example() -> Value {
    serde_json::json!({"expected_revision":0,"messages":[
        {"version":"v0.9.1","createSurface":{"surfaceId":"form","catalogId":NOLA_CATALOG_ID}},
        {"version":"v0.9.1","updateComponents":{"surfaceId":"form","components":[
            {"id":"root","component":"Column","children":["name","submit"]},
            {"id":"name","component":"TextField","label":"Name","value":{"path":"/name"}},
            {"id":"submit","component":"Button","child":"submit-label","action":{"event":{"name":"submit","context":{"name":{"path":"/name"}}}}},
            {"id":"submit-label","component":"Text","text":"Submit"}
        ]}},
        {"version":"v0.9.1","updateDataModel":{"surfaceId":"form","value":{"name":""}}}
    ]})
}

#[cfg(test)]
mod model_contract_tests {
    use super::*;
    #[test]
    fn advertised_form_validates_and_projects_and_rejects_observed_wrong_shapes() {
        let schema = tool_input_schema().unwrap();
        let validator = jsonschema::draft202012::options().build(&schema).unwrap();
        let example = minimal_form_example();
        assert!(validator.is_valid(&example));
        let input: nola_domain::generative_ui_tool::UiToolInput = serde_json::from_value(example.clone()).unwrap();
        let identity = nola_domain::generative_ui::UiSurfaceIdentity {
            owner: nola_domain::generative_ui::UiSurfaceOwner {
                authority_id:"host".into(),conversation_id:"conversation".into(),participant_id:"assistant".into(),
                context_id:"conversation".into(),originating_run_id:"run".into(),message_id:"message".into(),part_id:"part".into(),
            }, surface_id:"form".into(),generation:1,
        };
        let state = super::super::state::transition_messages(None, &identity, &input).unwrap();
        let projected = super::super::projection::project_surface(&state).unwrap();
        assert_eq!(projected.snapshot.phase, nola_domain::generative_ui::UiSurfacePhase::Ready);
        assert!(projected.snapshot.nodes.values().any(|node| node.component == "Button"));
        let mut wrong = example.clone(); wrong["messages"][0].as_object_mut().unwrap().remove("version");
        assert!(!validator.is_valid(&wrong));
        let mut wrong = example.clone(); wrong["messages"][0]["version"] = serde_json::json!("v0.9");
        assert!(!validator.is_valid(&wrong));
        let mut wrong = example.clone(); wrong["messages"][0]["createSurface"].as_object_mut().unwrap().remove("catalogId");
        assert!(!validator.is_valid(&wrong));
        let mut wrong = example.clone();
        let button = &mut wrong["messages"][1]["updateComponents"]["components"][2];
        button.as_object_mut().unwrap().remove("child"); button["label"] = serde_json::json!("Submit");
        assert!(!validator.is_valid(&wrong));
        let mut wrong = example;
        wrong["messages"][2]["updateDataModel"].as_object_mut().unwrap().remove("surfaceId");
        wrong["messages"][2]["surfaceId"] = serde_json::json!("form");
        assert!(!validator.is_valid(&wrong));
    }
}
