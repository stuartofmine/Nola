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
    validate_message_detailed(value).map_err(|_| UiError::InvalidMessage)
}

/// Validates a single A2UI v0.9.1 server-to-client message against the full
/// pinned Draft 2020-12 schema and returns up to 5 actionable field-level
/// errors plus targeted hints for common LLM schema mistakes.
pub fn validate_message_detailed(value: &Value) -> Result<(), Vec<String>> {
    let validator = MESSAGE_VALIDATOR.get_or_init(|| {
        let schema: Value = serde_json::from_str(SERVER).map_err(|error| error.to_string())?;
        build_validator(&schema)
    });
    match validator {
        Ok(validator) if validator.is_valid(value) => Ok(()),
        Ok(validator) => {
            let mut details = collect_targeted_hints(value);
            for error in validator.iter_errors(value) {
                if details.len() >= 5 {
                    break;
                }
                let path = error.instance_path.to_string();
                let path_label = if path.is_empty() { "/" } else { path.as_str() };
                let message = truncate_diagnostic(&error.to_string(), 220);
                let formatted = format!("{path_label}: {message}");
                if !details.contains(&formatted) {
                    details.push(formatted);
                }
            }
            if details.is_empty() {
                details.push("message failed A2UI v0.9.1 schema validation".into());
            }
            Err(details)
        }
        Err(error) => Err(vec![format!("internal A2UI schema validator error: {error}")]),
    }
}

/// Collects up to 5 concise, deduplicated diagnostics across a `render_ui`
/// message batch so model retries receive concrete field-level feedback.
pub fn diagnose_messages(messages: &[Value]) -> Vec<String> {
    let mut diagnostics = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        let prefix = format!("messages[{index}]");
        for hint in collect_targeted_hints(message) {
            push_unique_bounded(&mut diagnostics, format!("{prefix}: {hint}"), 5);
        }
        if diagnostics.len() >= 5 {
            break;
        }
        if let Err(errors) = validate_message_detailed(message) {
            for error in errors {
                push_unique_bounded(&mut diagnostics, format!("{prefix}: {error}"), 5);
                if diagnostics.len() >= 5 {
                    break;
                }
            }
        }
    }
    diagnostics
}

fn push_unique_bounded(target: &mut Vec<String>, item: String, limit: usize) {
    if target.len() < limit && !target.contains(&item) {
        target.push(item);
    }
}

fn truncate_diagnostic(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let collected: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{collected}…")
    } else {
        collected
    }
}

fn collect_targeted_hints(value: &Value) -> Vec<String> {
    let mut hints = Vec::new();
    let Some(envelope) = value.as_object() else {
        hints.push("message must be a JSON object".into());
        return hints;
    };
    match envelope.get("version").and_then(Value::as_str) {
        Some(version) if version == nola_domain::generative_ui::A2UI_WIRE_VERSION => {}
        Some(other) => push_unique_bounded(
            &mut hints,
            format!("version is \"{other}\", must be \"{}\"", nola_domain::generative_ui::A2UI_WIRE_VERSION),
            5,
        ),
        None => push_unique_bounded(
            &mut hints,
            format!("missing required \"version\": \"{}\"", nola_domain::generative_ui::A2UI_WIRE_VERSION),
            5,
        ),
    }
    if envelope.contains_key("surfaceId") {
        push_unique_bounded(
            &mut hints,
            "\"surfaceId\" belongs inside createSurface/updateComponents/updateDataModel/deleteSurface, not at the message root".into(),
            5,
        );
    }
    let op_count = ["createSurface", "updateComponents", "updateDataModel", "deleteSurface"]
        .iter()
        .filter(|key| envelope.contains_key(**key))
        .count();
    if op_count != 1 {
        push_unique_bounded(
            &mut hints,
            "each message must contain \"version\" and exactly one of createSurface, updateComponents, updateDataModel, or deleteSurface".into(),
            5,
        );
    }
    if let Some(create) = envelope.get("createSurface").and_then(Value::as_object) {
        match create.get("catalogId").and_then(Value::as_str) {
            Some(catalog_id) if catalog_id == NOLA_CATALOG_ID => {}
            Some(other) => push_unique_bounded(
                &mut hints,
                format!("createSurface.catalogId is \"{other}\", must be \"{NOLA_CATALOG_ID}\""),
                5,
            ),
            None => push_unique_bounded(
                &mut hints,
                format!("createSurface requires \"catalogId\": \"{NOLA_CATALOG_ID}\""),
                5,
            ),
        }
    }
    if let Some(components) = envelope
        .get("updateComponents")
        .and_then(|payload| payload.get("components"))
        .and_then(Value::as_array)
    {
        for component in components {
            let Some(obj) = component.as_object() else { continue; };
            let id = obj.get("id").and_then(Value::as_str).unwrap_or("?");
            let kind = obj.get("component").and_then(Value::as_str).unwrap_or("");
            if !kind.is_empty() && !SUPPORTED_COMPONENTS.contains(&kind) {
                push_unique_bounded(
                    &mut hints,
                    format!("component \"{kind}\" (id \"{id}\") is unsupported; use one of: {}", SUPPORTED_COMPONENTS.join(", ")),
                    5,
                );
            }
            if kind == "Button" && (obj.contains_key("label") || !obj.contains_key("child")) {
                push_unique_bounded(
                    &mut hints,
                    format!("Button (id \"{id}\") must use \"child\" referencing a separate Text component ID, not \"label\""),
                    5,
                );
            }
            if kind == "Card" && obj.contains_key("children") {
                push_unique_bounded(
                    &mut hints,
                    format!("Card (id \"{id}\") uses singular \"child\" (string component ID), not \"children\""),
                    5,
                );
            }
            if obj.get("action").is_some_and(|action| action.get("functionCall").is_some()) {
                push_unique_bounded(
                    &mut hints,
                    format!("Button (id \"{id}\") action must use {{\"event\":{{\"name\":\"...\"}}}} rather than functionCall"),
                    5,
                );
            }
        }
    }
    hints
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

/// A compact, flat, provider-friendly JSON Schema for `render_ui` model calls.
/// Runtime execution still validates every message against the full pinned
/// Draft 2020-12 A2UI wire and catalog schemas in `validate_message()`.
pub fn tool_input_schema() -> Result<Value, UiError> {
    let mut input = nola_domain::generative_ui_tool::UiToolInput::input_schema();
    input["properties"]["messages"]["description"] = serde_json::json!(
        "Ordered A2UI v0.9.1 messages for one surface. Each element requires version=\"v0.9.1\" and exactly one operation: createSurface, updateComponents, updateDataModel, or deleteSurface."
    );
    input["properties"]["messages"]["items"] = serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["version"],
        "properties": {
            "version": {
                "type": "string",
                "const": "v0.9.1",
                "description": "Always \"v0.9.1\"."
            },
            "createSurface": {
                "type": "object",
                "additionalProperties": false,
                "required": ["surfaceId", "catalogId"],
                "properties": {
                    "surfaceId": { "type": "string" },
                    "catalogId": { "type": "string", "const": NOLA_CATALOG_ID },
                    "sendDataModel": { "type": "boolean" },
                    "theme": { "type": "object" }
                }
            },
            "updateComponents": {
                "type": "object",
                "additionalProperties": false,
                "required": ["surfaceId", "components"],
                "properties": {
                    "surfaceId": { "type": "string" },
                    "components": {
                        "type": "array",
                        "minItems": 1,
                        "description": "Flat list of UI nodes. Root node MUST have id \"root\".",
                        "items": {
                            "type": "object",
                            "required": ["id", "component"],
                            "properties": {
                                "id": { "type": "string" },
                                "component": { "type": "string", "enum": SUPPORTED_COMPONENTS },
                                "text": { "description": "String or {\"path\":\"/field\"} for Text." },
                                "child": { "type": "string", "description": "Single child component ID (required for Button -> Text node, and Card)." },
                                "children": { "description": "Array of child IDs for Column/Row/List, or template {\"componentId\":\"...\",\"path\":\"/items\"}." },
                                "label": { "description": "Field label for TextField, CheckBox, ChoicePicker, Slider, DateTimeInput (never Button)." },
                                "value": { "description": "Literal or {\"path\":\"/field\"} data binding." },
                                "action": { "type": "object", "description": "Button action: {\"event\":{\"name\":\"...\",\"context\":{...}}}." }
                            }
                        }
                    }
                }
            },
            "updateDataModel": {
                "type": "object",
                "additionalProperties": false,
                "required": ["surfaceId"],
                "properties": {
                    "surfaceId": { "type": "string" },
                    "path": { "type": "string", "description": "JSON Pointer path, default \"/\"." },
                    "value": { "description": "New value to set; omit to delete." }
                }
            },
            "deleteSurface": {
                "type": "object",
                "additionalProperties": false,
                "required": ["surfaceId"],
                "properties": {
                    "surfaceId": { "type": "string" }
                }
            }
        }
    });
    Ok(input)
}

/// Read-only layouts require no artificial form or submit action.
pub fn minimal_card_example() -> Value {
    serde_json::json!({"expected_revision":0,"messages":[
        {"version":"v0.9.1","createSurface":{"surfaceId":"summary","catalogId":NOLA_CATALOG_ID}},
        {"version":"v0.9.1","updateComponents":{"surfaceId":"summary","components":[
            {"id":"root","component":"Column","children":["summary-card"]},
            {"id":"summary-card","component":"Card","child":"summary-text"},
            {"id":"summary-text","component":"Text","text":"A concise, source-supported summary."}
        ]}}
    ]})
}

/// Compact read-only tabbed comparison example.
pub fn minimal_tabs_example() -> Value {
    serde_json::json!({"expected_revision":0,"messages":[
        {"version":"v0.9.1","createSurface":{"surfaceId":"compare","catalogId":NOLA_CATALOG_ID}},
        {"version":"v0.9.1","updateComponents":{"surfaceId":"compare","components":[
            {"id":"root","component":"Tabs","tabs":[
                {"title":"Option A","child":"tab-a"},
                {"title":"Option B","child":"tab-b"}
            ]},
            {"id":"tab-a","component":"Card","child":"a-text"},
            {"id":"a-text","component":"Text","text":"Fastest option with lower cost."},
            {"id":"tab-b","component":"Card","child":"b-text"},
            {"id":"b-text","component":"Text","text":"Higher capacity with flexible schedule."}
        ]}}
    ]})
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
    fn tool_input_schema_is_compact_and_provider_friendly() {
        let schema = tool_input_schema().unwrap();
        let serialized = serde_json::to_string(&schema).unwrap();
        assert!(serialized.len() < 4096, "schema too large: {} bytes", serialized.len());
        for forbidden in ["\"$defs\"", "\"unevaluatedProperties\"", "\"if\":", "\"then\":", "\"not\":"] {
            assert!(!serialized.contains(forbidden), "schema must not contain {forbidden}");
        }
    }

    #[test]
    fn advertised_read_only_card_and_tabs_validate_and_project_without_a_submit_action() {
        let schema = tool_input_schema().unwrap();
        let validator = jsonschema::draft202012::options().build(&schema).unwrap();
        for (surface_id, example) in [("summary", minimal_card_example()), ("compare", minimal_tabs_example())] {
            assert!(validator.is_valid(&example), "compact schema rejected {surface_id}");
            let input: nola_domain::generative_ui_tool::UiToolInput = serde_json::from_value(example).unwrap();
            let identity = nola_domain::generative_ui::UiSurfaceIdentity {
                owner: nola_domain::generative_ui::UiSurfaceOwner { authority_id:"store".into(), conversation_id:"c".into(),
                    participant_id:"agent".into(), context_id:"ctx".into(), originating_run_id:"run".into(),message_id:"message".into(),part_id:"part".into() },
                surface_id:surface_id.into(), generation:1 };
            let state = super::super::state::transition_messages(None,&identity,&input).unwrap();
            let projected = super::super::projection::project_surface(&state).unwrap();
            assert_eq!(projected.snapshot.phase,nola_domain::generative_ui::UiSurfacePhase::Ready);
            assert!(projected.snapshot.nodes.values().any(|node|node.component=="Card"));
            assert!(!projected.snapshot.nodes.values().any(|node|node.component=="Button"));
        }
    }

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
        let button_errors = validate_message_detailed(&wrong["messages"][1]).unwrap_err();
        assert!(button_errors.iter().any(|error| error.contains("Button") && error.contains("\"child\"")));
        let mut wrong = example;
        wrong["messages"][2]["updateDataModel"].as_object_mut().unwrap().remove("surfaceId");
        wrong["messages"][2]["surfaceId"] = serde_json::json!("form");
        assert!(!validator.is_valid(&wrong));
    }

    #[test]
    fn validate_message_detailed_reports_targeted_hints_for_common_llm_mistakes() {
        let card_with_children = serde_json::json!({
            "version": "v0.9.1",
            "updateComponents": {
                "surfaceId": "summary",
                "components": [
                    {"id": "root", "component": "Card", "children": ["text-1"]},
                    {"id": "text-1", "component": "Text", "text": "Hello"}
                ]
            }
        });
        let errors = validate_message_detailed(&card_with_children).unwrap_err();
        assert!(errors.iter().any(|e| e.contains("Card") && e.contains("singular \"child\"")), "{errors:?}");

        let wrong_catalog = serde_json::json!({
            "version": "v0.9.1",
            "createSurface": {"surfaceId": "s", "catalogId": "https://example.com/bad"}
        });
        let diag = diagnose_messages(&[wrong_catalog]);
        assert!(diag.iter().any(|e| e.contains(NOLA_CATALOG_ID)), "{diag:?}");
    }
}
