// SPDX-License-Identifier: Apache-2.0
//! Canonical context ownership and rollback through the Application command boundary.
mod support;
use nola_application::{ApplicationCommand, ContextCommand, CreateConversation};
use serde_json::{json, Value};
use support::ApplicationRuntime;

fn commit(
    conversation: &str,
    checkpoint: &str,
    component: &str,
    title: &str,
) -> ApplicationCommand {
    ApplicationCommand::Context { command: serde_json::from_value(json!({
        "type": "commit_checkpoint", "input": {
            "checkpoint": {
                "id": checkpoint, "conversation_id": conversation, "run_id": null,
                "model_profile_id": "fixture-model", "policy_version": "v2",
                "covered_through_message_id": null, "covered_through_sequence": null,
                "summary_part_id": null, "retained_tail_start_message_id": null,
                "token_usage": {"estimated": 100, "measured": null, "window": 64000, "prompt_limit": 60000},
                "payload_digest": "fixture", "created_at_ms": 2
            },
            "components": [{
                "id": component, "checkpoint_id": checkpoint, "kind": "data_source",
                "source_id": "fixture-source", "source_uri": null, "title": title, "mime_type": "text/plain",
                "authorization": "authorized", "observed_at_ms": 1, "maximum_age_minutes": 60,
                "freshness": "fresh", "content_reference": {"type": "inline_preview", "text": title, "truncated": false},
                "content_digest": "fixture", "estimated_tokens": 100,
                "provenance": {"provider_id": "fixture", "collection_method": "test", "source_revision": null, "disclosed_to_model": true}
            }]
        }
    })).unwrap() }
}

fn load(runtime: &mut ApplicationRuntime, id: &str) -> Value {
    runtime
        .execute(ApplicationCommand::Context {
            command: ContextCommand::LoadCheckpoint {
                checkpoint_id: id.into(),
            },
        })
        .unwrap()
}

fn runtime(path: &std::path::Path) -> ApplicationRuntime {
    let mut runtime = support::open_runtime(path).unwrap();
    for id in ["a", "b"] {
        runtime
            .execute(ApplicationCommand::CreateConversation {
                input: CreateConversation {
                    id: id.into(),
                    title: id.into(),
                    settings_json: None,
                    created_at_ms: 1,
                    updated_at_ms: 1,
                },
            })
            .unwrap();
    }
    runtime
}

#[test]
fn foreign_checkpoint_identity_cannot_replace_components_even_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("context.db");
    let mut runtime = runtime(&path);
    let original = runtime
        .execute(commit("a", "checkpoint-a", "component-a", "original"))
        .unwrap();
    let error = runtime
        .execute(commit("b", "checkpoint-a", "component-b", "foreign"))
        .unwrap_err();
    assert!(error.contains("another conversation"), "{error}");
    assert_eq!(load(&mut runtime, "checkpoint-a"), original);
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    assert_eq!(load(&mut runtime, "checkpoint-a"), original);
    let own = runtime
        .execute(commit("b", "checkpoint-b", "component-b", "own"))
        .unwrap();
    assert_eq!(own["checkpoint"]["conversation_id"], "b");
    assert_eq!(load(&mut runtime, "checkpoint-a"), original);
}

#[test]
fn conflicting_component_rolls_back_checkpoint_and_component_deletion() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("context.db");
    let mut runtime = runtime(&path);
    let a = runtime
        .execute(commit("a", "checkpoint-a", "component-a", "original-a"))
        .unwrap();
    let b = runtime
        .execute(commit("b", "checkpoint-b", "component-b", "original-b"))
        .unwrap();
    assert!(runtime
        .execute(commit("a", "checkpoint-a", "component-b", "replacement"))
        .is_err());
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    assert_eq!(load(&mut runtime, "checkpoint-a"), a);
    assert_eq!(load(&mut runtime, "checkpoint-b"), b);
    let replacement = runtime
        .execute(commit(
            "a",
            "checkpoint-a",
            "component-a",
            "valid replacement",
        ))
        .unwrap();
    assert_eq!(replacement["components"][0]["title"], "valid replacement");
    assert_eq!(load(&mut runtime, "checkpoint-b"), b);
}
