// SPDX-License-Identifier: Apache-2.0
//! Original tool sources use the accepted owner, current commit and exact bytes.

use super::*;
use nola_domain::assistant_journal::AssistantJournalIdentity;
use nola_domain::{AssistantTurnBinding, CanonicalPartOwner, PartMutation};
use nola_store_port::assistant_journal::{AssistantJournalRepository, AssistantSourceCommitStamp};
use nola_store_port::tool_source::{
    ToolSourceBinding, ToolSourceInput, ToolSourceKind, ToolSourceRepository,
};

fn admission(conversation: &str, run: &str) -> BeginTurnInput {
    let message = format!("{run}-assistant");
    let turn = format!("{run}-turn");
    let text_part = format!("{run}-text");
    let tool_part = format!("{run}-tool");
    BeginTurnInput {
        user_part_kind: "text".into(),
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        initial_events: vec![],
        conversation_id: conversation.into(),
        conversation_title: "Original tool source".into(),
        user_message_id: format!("{run}-user"),
        user_part_id: format!("{run}-user-part"),
        user_text: "Inspect this isolated fixture".into(),
        user_content_json: None,
        additional_user_parts: vec![],
        assistant_message_id: message.clone(),
        assistant_part_id: text_part.clone(),
        initial_assistant_part_mutations: vec![PartMutation::Upsert {
            part_id: tool_part.clone(),
            kind: "tool".into(),
            ordinal: 2,
            state: "open".into(),
            content_text: "Visible tool presentation, not its original AgentItem".into(),
            content_json: Some("{}".into()),
            content_digest: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        }],
        assistant_turn_bindings: vec![
            AssistantTurnBinding::Admission {
                turn_id: turn.clone(),
                assistant_message_id: message.clone(),
            },
            AssistantTurnBinding::Part {
                turn_id: turn.clone(),
                assistant_message_id: message.clone(),
                part_id: text_part,
            },
            AssistantTurnBinding::Part {
                turn_id: turn,
                assistant_message_id: message,
                part_id: tool_part,
            },
        ],
        run_id: run.into(),
        context_checkpoint_id: None,
        checkpoint_revision: 1,
        checkpoint_event_cursor: 0,
        checkpoint_json: "{}".into(),
        invocation_config_json: "{}".into(),
        invocation_config_digest: "fixture".into(),
        outbox_effects: vec![],
        model_attempt_mutations: vec![],
        delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![],
        memory_usage_entries: vec![],
        created_at_ms: 1,
    }
}

fn transition(run: &str, revision: i64) -> CommitTransitionInput {
    CommitTransitionInput {
        transition_id: format!("{run}-transition-{revision}"),
        run_id: run.into(),
        expected_checkpoint_revision: revision - 1,
        checkpoint_revision: revision,
        checkpoint_event_cursor: 0,
        checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline {
            checkpoint_json: "{}".into(),
        },
        submission_reduction_json: None,
        host_event: None,
        events: vec![],
        ensured_messages: vec![],
        part_message_owners: Default::default(),
        assistant_turn_bindings: vec![],
        part_mutations: vec![],
        outbox_effects: vec![],
        model_attempt_mutations: vec![],
        delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![],
        effect_delivery_attempt_mutations: vec![],
        acknowledged_effect_ids: vec![],
        memory_usage_entries: vec![],
        run_status: None,
        assistant_status: None,
        assistant_status_message_id: None,
        committed_at_ms: revision,
    }
}

fn stage(store: &mut NolaStore, run: &str, revision: i64) -> AssistantSourceCommitStamp {
    let input = transition(run, revision);
    assert!(store.commit_transition(&input).unwrap().applied);
    AssistantSourceCommitStamp::Transition {
        transition_id: input.transition_id,
    }
}

fn identity(store: &NolaStore, conversation: &str, run: &str) -> AssistantJournalIdentity {
    AssistantJournalIdentity {
        source_store_epoch: store.assistant_source_epoch().unwrap(),
        conversation_id: conversation.into(),
        run_id: run.into(),
        assistant_message_id: format!("{run}-assistant"),
    }
}

fn source(
    store: &NolaStore,
    conversation: &str,
    run: &str,
    kind: ToolSourceKind,
) -> ToolSourceInput {
    let owner = store
        .assistant_owner_for_part(conversation, run, &format!("{run}-tool"))
        .unwrap()
        .unwrap();
    let id = match kind {
        ToolSourceKind::ToolCall => format!("{run}-call-item"),
        ToolSourceKind::ToolResult => format!("{run}-result-item"),
    };
    let call_id = format!("{run}-call");
    let body = match kind {
        ToolSourceKind::ToolCall => serde_json::json!({
            "type": "tool_call", "id": id, "call_id": call_id,
            "name": "read_file", "arguments": {"path": "isolated.txt"}
        }),
        ToolSourceKind::ToolResult => serde_json::json!({
            "type": "tool_result", "id": id, "call_id": call_id,
            "name": "read_file", "output": {"text": "original 文本\n"},
            "is_error": false, "full_output_ref": null, "artifacts": []
        }),
    };
    ToolSourceInput {
        source_item_id: id,
        kind,
        call_id,
        name: "read_file".into(),
        owner,
        body_json: serde_json::to_string(&body).unwrap(),
    }
}

fn revision(store: &NolaStore, conversation: &str) -> i64 {
    store
        .connection
        .query_row(
            "SELECT revision FROM conversations WHERE id=?1",
            [conversation],
            |r| r.get(0),
        )
        .unwrap()
}

fn counts(store: &NolaStore) -> (i64, i64, i64) {
    store
        .connection
        .query_row(
            "SELECT (SELECT COUNT(*) FROM canonical_tool_sources),
                    (SELECT COUNT(*) FROM canonical_text_objects),
                    (SELECT COUNT(*) FROM canonical_text_chunks)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
}

fn read_original(store: &NolaStore, binding: &ToolSourceBinding, revision: i64) -> String {
    let mut text = String::new();
    let mut offset = 0;
    loop {
        // Deliberately split multibyte text and persisted chunk boundaries.
        let range = store
            .read_tool_source_range(binding, revision, offset, 4099)
            .unwrap();
        assert_eq!(range.offset_bytes, offset);
        assert_eq!(range.next_offset_bytes - offset, range.text.len() as u64);
        text.push_str(&range.text);
        if range.end_of_content {
            assert_eq!(range.next_offset_bytes, binding.body.total_utf8_bytes());
            break;
        }
        assert!(range.next_offset_bytes > offset);
        offset = range.next_offset_bytes;
    }
    text
}

#[test]
fn tool_source_call_and_result_append_preserves_exact_cold_original_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("originals.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    store.begin_turn(&admission("conversation", "run")).unwrap();
    let identity = identity(&store, "conversation", "run");
    let call = source(&store, "conversation", "run", ToolSourceKind::ToolCall);
    let mut result = source(&store, "conversation", "run", ToolSourceKind::ToolResult);
    let mut body: serde_json::Value = serde_json::from_str(&result.body_json).unwrap();
    body["output"] = serde_json::json!({"text": "原文🙂\n".repeat(12_000)});
    // Whitespace is intentional: a JSON round trip must not replace source bytes.
    result.body_json = format!(" \n{}\n ", serde_json::to_string_pretty(&body).unwrap());

    store.begin_write_unit_of_work().unwrap();
    let stamp = stage(&mut store, "run", 2);
    store.bind_tool_source(&stamp, &identity, &call).unwrap();
    store.bind_tool_source(&stamp, &identity, &call).unwrap();
    assert_eq!(counts(&store).0, 1);
    store.commit_write_unit_of_work().unwrap();
    store.begin_write_unit_of_work().unwrap();
    let stamp = stage(&mut store, "run", 3);
    store.bind_tool_source(&stamp, &identity, &result).unwrap();
    store.commit_write_unit_of_work().unwrap();
    let pinned_revision = revision(&store, "conversation");
    let owner = CanonicalPartOwner::new(
        "conversation".into(),
        "run-assistant".into(),
        "run-tool".into(),
    )
    .unwrap();
    let bindings = store
        .tool_sources_for_part(&owner, pinned_revision)
        .unwrap();
    assert_eq!(bindings.len(), 2);
    assert_eq!(bindings[0].kind, ToolSourceKind::ToolCall);
    assert_eq!(bindings[1].kind, ToolSourceKind::ToolResult);
    assert!(bindings[1].body.chunk_count() > 1);
    for (binding, input) in bindings.iter().zip([&call, &result]) {
        assert_eq!(binding.owner, input.owner);
        assert_eq!(
            binding.body_sha256,
            format!("{:x}", Sha256::digest(input.body_json.as_bytes()))
        );
        assert_eq!(
            read_original(&store, binding, pinned_revision),
            input.body_json
        );
    }
    let presentation: (String, String) = store
        .connection
        .query_row(
            "SELECT content_text,content_json FROM message_parts WHERE id='run-tool'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        presentation,
        (
            "Visible tool presentation, not its original AgentItem".into(),
            "{}".into()
        )
    );
    drop(store);
    let cold = NolaStore::open(&path).unwrap();
    assert_eq!(
        cold.tool_sources_for_part(&owner, pinned_revision).unwrap(),
        bindings
    );
    assert_eq!(
        read_original(&cold, &bindings[0], pinned_revision),
        call.body_json
    );
    assert_eq!(
        read_original(&cold, &bindings[1], pinned_revision),
        result.body_json
    );
}

#[test]
fn tool_source_same_id_cannot_rebind_to_a_different_accepted_owner() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("owners.sqlite")).unwrap();
    store
        .begin_turn(&admission("conversation-a", "run-a"))
        .unwrap();
    store
        .begin_turn(&admission("conversation-b", "run-b"))
        .unwrap();
    let owner_a = identity(&store, "conversation-a", "run-a");
    let owner_b = identity(&store, "conversation-b", "run-b");
    let call_a = source(&store, "conversation-a", "run-a", ToolSourceKind::ToolCall);
    let call_b = source(&store, "conversation-b", "run-b", ToolSourceKind::ToolCall);
    store.begin_write_unit_of_work().unwrap();
    let stamp_a = stage(&mut store, "run-a", 2);
    store.bind_tool_source(&stamp_a, &owner_a, &call_a).unwrap();
    let stamp_b = stage(&mut store, "run-b", 2);
    let before = counts(&store);
    let mut collision = call_b.clone();
    collision.source_item_id = call_a.source_item_id.clone();
    let mut body: serde_json::Value = serde_json::from_str(&collision.body_json).unwrap();
    body["id"] = collision.source_item_id.clone().into();
    collision.body_json = body.to_string();
    assert!(store
        .bind_tool_source(&stamp_b, &owner_b, &collision)
        .is_err());
    assert!(store.bind_tool_source(&stamp_b, &owner_b, &call_a).is_err());
    let mut forged_turn = call_b.clone();
    forged_turn.owner.turn_id = Some("foreign-turn".into());
    assert!(store
        .bind_tool_source(&stamp_b, &owner_b, &forged_turn)
        .is_err());
    assert_eq!(counts(&store), before);
    // The second owner is genuinely valid, not a fixture that rejects every write.
    store.bind_tool_source(&stamp_b, &owner_b, &call_b).unwrap();
    store.commit_write_unit_of_work().unwrap();
    assert!(store
        .tool_source_by_id(
            "conversation-b",
            &call_a.source_item_id,
            revision(&store, "conversation-b")
        )
        .unwrap()
        .is_none());
    let foreign = CanonicalPartOwner::new(
        "conversation-a".into(),
        "run-b-assistant".into(),
        "run-a-tool".into(),
    )
    .unwrap();
    assert!(store
        .tool_sources_for_part(&foreign, revision(&store, "conversation-a"))
        .is_err());
}

#[test]
fn tool_source_forged_body_identity_and_incomplete_current_shape_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("forged.sqlite")).unwrap();
    store.begin_turn(&admission("conversation", "run")).unwrap();
    let identity = identity(&store, "conversation", "run");
    let call = source(&store, "conversation", "run", ToolSourceKind::ToolCall);
    let body: serde_json::Value = serde_json::from_str(&call.body_json).unwrap();
    store.begin_write_unit_of_work().unwrap();
    let stamp = stage(&mut store, "run", 2);
    let before = counts(&store);
    for field in ["type", "id", "call_id", "name"] {
        let mut forged = call.clone();
        let mut value = body.clone();
        value[field] = "foreign".into();
        forged.body_json = value.to_string();
        assert!(
            store.bind_tool_source(&stamp, &identity, &forged).is_err(),
            "{field}"
        );
        assert_eq!(counts(&store), before);
    }
    for value in [
        {
            let mut value = body.clone();
            value.as_object_mut().unwrap().remove("arguments");
            value
        },
        {
            let mut value = body;
            value["tool_name"] = "read_file".into();
            value
        },
    ] {
        let mut forged = call.clone();
        forged.body_json = value.to_string();
        assert!(store.bind_tool_source(&stamp, &identity, &forged).is_err());
        assert_eq!(counts(&store), before);
    }
    let mut malformed = call.clone();
    malformed.body_json = "not-json".into();
    assert!(store
        .bind_tool_source(&stamp, &identity, &malformed)
        .is_err());
    assert_eq!(counts(&store), before);
    store.bind_tool_source(&stamp, &identity, &call).unwrap();
    store.commit_write_unit_of_work().unwrap();
}

#[test]
fn tool_source_result_requires_original_call_and_sources_cannot_change_in_place() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("pair.sqlite")).unwrap();
    store.begin_turn(&admission("conversation", "run")).unwrap();
    let identity = identity(&store, "conversation", "run");
    let call = source(&store, "conversation", "run", ToolSourceKind::ToolCall);
    let result = source(&store, "conversation", "run", ToolSourceKind::ToolResult);
    store.begin_write_unit_of_work().unwrap();
    let stamp = stage(&mut store, "run", 2);
    let empty = counts(&store);
    assert!(store.bind_tool_source(&stamp, &identity, &result).is_err());
    assert_eq!(counts(&store), empty);
    store.bind_tool_source(&stamp, &identity, &call).unwrap();
    let before = counts(&store);
    let mut changed = call.clone();
    changed.body_json.push(' ');
    assert!(store.bind_tool_source(&stamp, &identity, &changed).is_err());
    let mut changed = call.clone();
    changed.source_item_id = "replacement-call-item".into();
    let mut body: serde_json::Value = serde_json::from_str(&changed.body_json).unwrap();
    body["id"] = changed.source_item_id.clone().into();
    changed.body_json = body.to_string();
    assert!(store.bind_tool_source(&stamp, &identity, &changed).is_err());
    for field in ["call_id", "name"] {
        let mut changed = result.clone();
        let mut body: serde_json::Value = serde_json::from_str(&changed.body_json).unwrap();
        body[field] = "different-call-or-capability".into();
        if field == "call_id" {
            changed.call_id = "different-call-or-capability".into();
        } else {
            changed.name = "different-call-or-capability".into();
        }
        changed.body_json = body.to_string();
        assert!(store.bind_tool_source(&stamp, &identity, &changed).is_err());
    }
    assert_eq!(counts(&store), before);
    store.bind_tool_source(&stamp, &identity, &result).unwrap();
    let bound = counts(&store);
    store.bind_tool_source(&stamp, &identity, &result).unwrap();
    assert_eq!(counts(&store), bound);
    assert_eq!(bound.0, 2);
    store.commit_write_unit_of_work().unwrap();
}

#[test]
fn tool_source_reads_and_writes_require_current_revision_and_exact_binding() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("revision.sqlite")).unwrap();
    store.begin_turn(&admission("conversation", "run")).unwrap();
    let identity = identity(&store, "conversation", "run");
    let call = source(&store, "conversation", "run", ToolSourceKind::ToolCall);
    let result = source(&store, "conversation", "run", ToolSourceKind::ToolResult);
    store.begin_write_unit_of_work().unwrap();
    let stamp = stage(&mut store, "run", 2);
    store.bind_tool_source(&stamp, &identity, &call).unwrap();
    store.commit_write_unit_of_work().unwrap();
    let old_revision = revision(&store, "conversation");
    let binding = store
        .tool_source_by_id("conversation", &call.source_item_id, old_revision)
        .unwrap()
        .unwrap();
    assert!(store.bind_tool_source(&stamp, &identity, &result).is_err());
    store.begin_write_unit_of_work().unwrap();
    let next_stamp = stage(&mut store, "run", 3);
    assert!(store.bind_tool_source(&stamp, &identity, &result).is_err());
    store
        .bind_tool_source(&next_stamp, &identity, &result)
        .unwrap();
    store.commit_write_unit_of_work().unwrap();
    let current = revision(&store, "conversation");
    assert!(current > old_revision);
    for stale in [0, old_revision, current + 1] {
        assert!(store
            .tool_source_by_id("conversation", &call.source_item_id, stale)
            .is_err());
        assert!(store
            .read_tool_source_range(&binding, stale, 0, 128)
            .is_err());
    }
    assert_eq!(read_original(&store, &binding, current), call.body_json);
    let mut forged = binding.clone();
    forged.body_sha256 = "0".repeat(64);
    assert!(store
        .read_tool_source_range(&forged, current, 0, 128)
        .is_err());
    let mut forged = binding;
    forged.owner.turn_id = Some("other-turn".into());
    assert!(store
        .read_tool_source_range(&forged, current, 0, 128)
        .is_err());
}

#[test]
fn tool_source_outer_transaction_rollback_removes_sources_bodies_and_source_commit() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rollback.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    store.begin_turn(&admission("conversation", "run")).unwrap();
    let identity = identity(&store, "conversation", "run");
    let call = source(&store, "conversation", "run", ToolSourceKind::ToolCall);
    let result = source(&store, "conversation", "run", ToolSourceKind::ToolResult);
    let before = counts(&store);
    let before_revision = revision(&store, "conversation");
    store.begin_write_unit_of_work().unwrap();
    let stamp = stage(&mut store, "run", 2);
    store.bind_tool_source(&stamp, &identity, &call).unwrap();
    store.bind_tool_source(&stamp, &identity, &result).unwrap();
    assert_eq!(counts(&store).0, 2);
    store.rollback_write_unit_of_work().unwrap();
    assert_eq!(counts(&store), before);
    assert_eq!(revision(&store, "conversation"), before_revision);
    assert_eq!(store.checkpoint_revision("conversation").unwrap(), Some(1));
    assert!(store
        .tool_source_by_id("conversation", &call.source_item_id, before_revision)
        .unwrap()
        .is_none());
    drop(store);
    let mut cold = NolaStore::open(&path).unwrap();
    assert_eq!(counts(&cold), before);
    let cold_identity = AssistantJournalIdentity {
        source_store_epoch: cold.assistant_source_epoch().unwrap(),
        ..identity
    };
    cold.begin_write_unit_of_work().unwrap();
    let stamp = stage(&mut cold, "run", 2);
    cold.bind_tool_source(&stamp, &cold_identity, &call)
        .unwrap();
    cold.bind_tool_source(&stamp, &cold_identity, &result)
        .unwrap();
    cold.commit_write_unit_of_work().unwrap();
    assert_eq!(counts(&cold).0, 2);
}

#[test]
fn tool_source_failed_binding_insert_rolls_back_prepared_body_chunks() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("fault.sqlite")).unwrap();
    store.begin_turn(&admission("conversation", "run")).unwrap();
    let identity = identity(&store, "conversation", "run");
    let call = source(&store, "conversation", "run", ToolSourceKind::ToolCall);
    store
        .connection
        .execute_batch(
            "CREATE TEMP TRIGGER reject_tool_source BEFORE INSERT ON canonical_tool_sources
         BEGIN SELECT RAISE(ABORT,'fixture source insert fault'); END;",
        )
        .unwrap();
    store.begin_write_unit_of_work().unwrap();
    let stamp = stage(&mut store, "run", 2);
    let before = counts(&store);
    assert!(store.bind_tool_source(&stamp, &identity, &call).is_err());
    assert_eq!(counts(&store), before);
    store
        .connection
        .execute_batch("DROP TRIGGER reject_tool_source")
        .unwrap();
    store.bind_tool_source(&stamp, &identity, &call).unwrap();
    store.commit_write_unit_of_work().unwrap();
    let binding = store
        .tool_source_by_id(
            "conversation",
            &call.source_item_id,
            revision(&store, "conversation"),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        read_original(&store, &binding, revision(&store, "conversation")),
        call.body_json
    );
}
