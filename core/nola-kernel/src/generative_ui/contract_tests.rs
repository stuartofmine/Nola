// SPDX-License-Identifier: Apache-2.0
//! Admission contracts exercise the same complete wire messages used by tools.
use super::*;
use nola_domain::{generative_ui::*, generative_ui_tool::UiToolInput};
use serde_json::json;

fn identity() -> UiSurfaceIdentity {
    UiSurfaceIdentity {
        owner: UiSurfaceOwner { authority_id:"host".into(), conversation_id:"conversation".into(),
            participant_id:"assistant".into(), context_id:"conversation".into(), originating_run_id:"run".into(),
            message_id:"message".into(), part_id:"part".into() },
        surface_id:"form".into(), generation:1,
    }
}
fn input(messages: Vec<Value>, revision: u64) -> UiToolInput { UiToolInput { expected_revision:revision, messages } }
fn form() -> UiSurfaceState {
    state::transition_messages(None, &identity(), &serde_json::from_value(schema::minimal_form_example()).unwrap()).unwrap()
}
fn parse(value: Value) -> Result<A2uiMessage, UiError> { parse_a2ui_message(&serde_json::to_vec(&value).unwrap()) }

#[test]
fn exact_version_complete_json_and_unique_keys_are_required() {
    for version in [json!("v0.9"), json!("0.9.1"), json!("v1.0"), Value::Null] {
        assert_eq!(parse(json!({"version":version,"deleteSurface":{"surfaceId":"form"}})), Err(UiError::UnsupportedProtocol));
    }
    assert!(parse_a2ui_message(br#"{"version":"v0.9.1","deleteSurface":{"surfaceId":"form"}"#).is_err());
    assert!(parse_a2ui_message(br#"{"version":"v0.9.1","version":"v0.9.1","deleteSurface":{"surfaceId":"form"}}"#).is_err());
    assert!(parse_a2ui_message(br#"{"version":"v0.9.1","deleteSurface":{"surfaceId":"form"}}{}"#).is_err());
    assert_eq!(parse_a2ui_message(&vec![b' '; UI_MAX_MESSAGE_BYTES + 1]), Err(UiError::ResourceLimit));
}

#[test]
fn null_value_is_stored_while_missing_value_deletes_and_missing_path_targets_root() {
    let original = form();
    let null = state::transition_messages(Some(&original), &identity(), &input(vec![json!({"version":"v0.9.1",
        "updateDataModel":{"surfaceId":"form","path":"/name","value":null}})], original.revision)).unwrap();
    assert_eq!(null.data.get("name"), Some(&Value::Null));
    let deleted = state::transition_messages(Some(&null), &identity(), &input(vec![json!({"version":"v0.9.1",
        "updateDataModel":{"surfaceId":"form","path":"/name"}})], null.revision)).unwrap();
    assert!(deleted.data.get("name").is_none());
    let root = state::transition_messages(Some(&deleted), &identity(), &input(vec![json!({"version":"v0.9.1",
        "updateDataModel":{"surfaceId":"form","value":{"replacement":true}}})], deleted.revision)).unwrap();
    assert_eq!(root.data, json!({"replacement":true}));
}

#[test]
fn failing_second_message_does_not_publish_the_first_message_or_increment_revision() {
    let original = form();
    let before = serde_json::to_value(&original).unwrap();
    let batch = input(vec![
        json!({"version":"v0.9.1","updateDataModel":{"surfaceId":"form","path":"/name","value":"must rollback"}}),
        json!({"version":"v0.9.1","updateComponents":{"surfaceId":"form","components":[
            {"id":"root","component":"Column","children":["root"]}
        ]}}),
    ], original.revision);
    assert!(state::transition_messages(Some(&original), &identity(), &batch).is_err());
    assert_eq!(serde_json::to_value(&original).unwrap(), before);
    assert_eq!(projection::project_surface(&original).unwrap().snapshot.revision, original.revision);
}

#[test]
fn pending_forward_reference_becomes_ready_then_deleted_generation_rejects_late_update() {
    let mut example = schema::minimal_form_example();
    let messages = example["messages"].as_array_mut().unwrap();
    let created = state::transition_messages(None, &identity(), &input(vec![messages[0].clone()], 0)).unwrap();
    assert_eq!(created.phase, UiSurfacePhase::Pending);
    let ready = state::transition_messages(Some(&created), &identity(), &input(messages[1..].to_vec(), created.revision)).unwrap();
    assert_eq!(ready.phase, UiSurfacePhase::Ready);
    let deleted = state::transition_messages(Some(&ready), &identity(), &input(vec![json!({"version":"v0.9.1",
        "deleteSurface":{"surfaceId":"form"}})], ready.revision)).unwrap();
    assert_eq!(deleted.phase, UiSurfacePhase::Deleted);
    assert!(deleted.components.is_empty());
    assert_eq!(state::transition_messages(Some(&deleted), &identity(), &input(vec![messages[2].clone()], deleted.revision)), Err(UiError::SurfaceClosed));
    assert_eq!(state::transition_messages(Some(&ready), &identity(), &input(vec![messages[2].clone()], 0)), Err(UiError::RevisionConflict));
}

#[test]
fn every_advertised_pure_function_evaluates_and_unknown_functions_do_not_execute() {
    let cases = [
        ("required", json!({"value":"x"}), true), ("regex", json!({"value":"ab","pattern":"^a"}), true),
        ("length", json!({"value":"中日","min":2,"max":2}), true),
        ("numeric", json!({"value":3,"min":2,"max":4}), true),
        ("and", json!({"values":[true,false]}), false), ("or", json!({"values":[false,true]}), true),
        ("not", json!({"value":true}), false),
    ];
    assert_eq!(cases.len(), schema::SUPPORTED_FUNCTIONS.len());
    for (name, args, expected) in cases {
        assert!(schema::SUPPORTED_FUNCTIONS.contains(&name));
        assert_eq!(resolve::Evaluator::new(&json!({})).resolve(&json!({"call":name,"args":args}), "/").unwrap(), json!(expected));
    }
    assert_eq!(resolve::Evaluator::new(&json!({})).resolve(&json!({"call":"openUrl","args":{"url":"https://example.com"}}), "/"), Err(UiError::UnsupportedFunction));
    let mut example = schema::minimal_form_example();
    example["messages"][0]["createSurface"]["catalogId"] = json!("https://untrusted.example/catalog");
    assert_eq!(state::transition_messages(None, &identity(), &serde_json::from_value(example).unwrap()), Err(UiError::UnsupportedCatalog));
}

#[test]
fn pending_and_disconnected_nodes_cannot_defer_unsupported_function_admission() {
    for id in ["root", "not-connected"] {
        let rejected = parse(json!({"version":"v0.9.1","updateComponents":{"surfaceId":"form",
            "components":[{"id":id,"component":"Text","text":{
                "call":"formatString","args":{"value":"hidden"},"returnType":"string"
            }}]}}));
        assert_eq!(rejected, Err(UiError::UnsupportedFunction));
    }
}
