// SPDX-License-Identifier: Apache-2.0
use nola_store::*;
#[path = "support/terminal_sources.rs"]
mod terminal_sources;

fn turn() -> BeginTurnInput {
    BeginTurnInput {
        user_part_kind: "text".into(),
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        initial_events: Vec::new(),
        initial_assistant_part_mutations: vec![],
        memory_usage_entries: vec![],
        delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![],
        model_attempt_mutations: vec![],
        conversation_id: "c".into(),
        conversation_title: "Continued".into(),
        user_message_id: "new-user".into(),
        user_part_id: "new-user-text".into(),
        user_text: "Continue".into(),
        user_content_json: None,
        additional_user_parts: Vec::new(),
        assistant_message_id: "new-assistant".into(),
        assistant_part_id: "new-assistant-text".into(),
        assistant_turn_bindings: vec![],
        run_id: "new-run".into(),
        context_checkpoint_id: None,
        checkpoint_revision: 1,
        checkpoint_event_cursor: 0,
        checkpoint_json: "{}".into(),
        invocation_config_json: "{}".into(),
        invocation_config_digest: "fixture-config".into(),
        outbox_effects: Vec::new(),
        created_at_ms: 3,
    }
}

fn seed(store: &mut NolaStore) {
    store
        .create_conversation(&CreateConversationInput {
            id: "c".into(),
            title: "c".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    store
        .append_messages(&AppendMessagesInput {
            conversation_id: "c".into(),
            expected_message_count: 0,
            updated_at_ms: 2,
            messages: vec![MessageInput {
                id: "old".into(),
                run_id: None,
                role: "user".into(),
                status: "completed".into(),
                ordinal: 1,
                created_at_ms: 2,
                completed_at_ms: Some(2),
                parts: vec![MessagePartInput {
                    id: "old-text".into(),
                    kind: "text".into(),
                    ordinal: 1,
                    state: "completed".into(),
                    content_text: "history".repeat(20_000),
                    content_json: None,
                    content_digest: None,
                    created_at_ms: 2,
                    updated_at_ms: 2,
                }],
            }],
        })
        .unwrap();
    let mut admission = turn();
    admission.expected_configuration_revision = Some(1);
    admission.expected_parent_message_id = Some("old".into());
    store.begin_turn(&admission).unwrap();
}

fn changes(store: &NolaStore) -> Vec<RemoteSyncChangeInput> {
    store
        .pending_sync_changes(20)
        .unwrap()
        .into_iter()
        .enumerate()
        .map(|(i, item)| RemoteSyncChangeInput {
            cursor: i as i64 + 1,
            operation_id: item.operation_id,
            entity_type: item.entity_type,
            entity_id: item.entity_id,
            revision: item.revision,
            deleted: item.deleted,
            payload_json: item.payload_json,
            created_at_ms: item.created_at_ms,
        })
        .collect()
}

fn page(changes: Vec<RemoteSyncChangeInput>) -> ApplyRemoteSyncPageInput {
    ApplyRemoteSyncPageInput {
        replica_id: "fixture".into(),
        next_cursor: changes.last().unwrap().cursor,
        changes,
        applied_at_ms: 10,
    }
}

#[test]
fn existing_history_turn_restores_after_restart_without_republishing_history() {
    let root = tempfile::tempdir().unwrap();
    let source_path = root.path().join("source.db");
    let mut source = NolaStore::open(&source_path).unwrap();
    seed(&mut source);
    let expected = source.load_conversation("c").unwrap().unwrap();
    drop(source);
    let source = NolaStore::open(&source_path).unwrap();
    let facts = changes(&source);
    assert_eq!(facts.len(), 3);
    assert_eq!(facts[2].entity_type, "conversation_append");
    assert!(facts[2].payload_json.len() < 4096);
    assert!(!facts[2].payload_json.contains("history"));
    let receiver_path = root.path().join("receiver.db");
    let mut receiver = NolaStore::open(&receiver_path).unwrap();
    assert!(receiver
        .apply_remote_sync_page(&page(vec![facts[2].clone()]))
        .is_err());
    assert!(receiver.sync_cursor("fixture").unwrap().is_none());
    receiver
        .apply_remote_sync_page(&page(facts[..2].to_vec()))
        .unwrap();
    drop(receiver);
    let mut receiver = NolaStore::open(&receiver_path).unwrap();
    receiver
        .apply_remote_sync_page(&page(vec![facts[2].clone()]))
        .unwrap();
    assert_eq!(receiver.load_conversation("c").unwrap().unwrap(), expected);
    assert!(
        receiver.active_run("c").unwrap().is_none(),
        "replica must not execute the sender's Run"
    );
    assert!(receiver.pending_sync_changes(20).unwrap().is_empty());
    drop(receiver);
    let mut receiver = NolaStore::open(&receiver_path).unwrap();
    let mut replay = facts[2].clone();
    replay.cursor = 4;
    assert_eq!(
        receiver
            .apply_remote_sync_page(&page(vec![replay]))
            .unwrap()
            .skipped,
        1
    );
    assert_eq!(receiver.load_conversation("c").unwrap().unwrap(), expected);
}

#[test]
fn invalid_turn_ownership_rolls_back_page_and_cursor() {
    let root = tempfile::tempdir().unwrap();
    let mut source = NolaStore::open(root.path().join("source.db")).unwrap();
    seed(&mut source);
    let facts = changes(&source);
    let mut receiver = NolaStore::open(root.path().join("receiver.db")).unwrap();
    for fault in ["run", "placeholder", "kind"] {
        let mut bad = facts.clone();
        let mut payload: serde_json::Value = serde_json::from_str(&bad[2].payload_json).unwrap();
        match fault {
            "run" => payload["input"]["messages"][1]["run_id"] = "another-run".into(),
            "placeholder" => {
                payload["input"]["messages"][1]["parts"][0]["content_text"] = "forged".into()
            }
            _ => {
                payload.as_object_mut().unwrap().remove("kind");
            }
        }
        bad[2].payload_json = payload.to_string();
        assert!(receiver.apply_remote_sync_page(&page(bad)).is_err());
        assert!(receiver.load_conversation("c").unwrap().is_none());
        assert!(receiver.sync_cursor("fixture").unwrap().is_none());
    }
    receiver.apply_remote_sync_page(&page(facts)).unwrap();
    assert_eq!(
        receiver
            .load_conversation("c")
            .unwrap()
            .unwrap()
            .messages
            .len(),
        3
    );
}

#[test]
fn internal_resume_changes_ownership_without_republishing_existing_parts() {
    let root = tempfile::tempdir().unwrap();
    let mut source = NolaStore::open(root.path().join("source.db")).unwrap();
    seed(&mut source);
    let mut finished = CommitTransitionInput {
            memory_usage_entries: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            effect_delivery_attempt_mutations: vec![],
            model_attempt_mutations: vec![],
            transition_id: "finish-first".into(),
            run_id: "new-run".into(),
            expected_checkpoint_revision: 1,
            checkpoint_revision: 2,
            checkpoint_event_cursor: 0,
            checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline { checkpoint_json: "{}".into() },
            submission_reduction_json: None, host_event: None,
            events: Vec::new(),
            ensured_messages: Vec::new(),
            assistant_turn_bindings: vec![],
            part_message_owners: Default::default(),
            part_mutations: vec![PartMutation::Upsert {
                part_id: "new-assistant-text".into(),
                kind: "text".into(),
                ordinal: 1,
                state: "completed".into(),
                content_text: "answer".repeat(20_000),
                content_json: None,
                content_digest: None,
                created_at_ms: 3,
                updated_at_ms: 4,
            }],
            outbox_effects: Vec::new(),
            acknowledged_effect_ids: Vec::new(),
            run_status: None,
            assistant_status: Some("completed".into()),
            assistant_status_message_id: None,
            committed_at_ms: 4,
        };
    terminal_sources::add_sources(&source,&mut finished,"c","new-run","new-user",false);
    source.begin_write_unit_of_work().unwrap();
    source.commit_transition(&finished).unwrap();
    terminal_sources::publish_and_seal(&mut source,&finished,"c");
    source.commit_write_unit_of_work().unwrap();
    let target = source.internal_assistant_target("c").unwrap().unwrap();
    let expected_checkpoint_revision = source.checkpoint_revision("c").unwrap();
    nola_store_port::run_creation::RunCreationRepository::begin_internal_turn(
        &mut source,
        &BeginInternalTurnInput {
            initial_events: Vec::new(),
            initial_assistant_part_mutations: vec![],
            memory_usage_entries: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            model_attempt_mutations: vec![],
            conversation_id: "c".into(),
            assistant_message_id: target.assistant_message_id,
            expected_previous_run_id: target.run_id,
            expected_checkpoint_revision,
            assistant_part_id: "unused-new-part".into(),
            assistant_turn_bindings: vec![],
            run_id: "resumed-run".into(),
            context_checkpoint_id: None,
            checkpoint_revision: 3,
            checkpoint_event_cursor: 0,
            checkpoint_json: "{}".into(),
            invocation_config_json: "{}".into(),
            invocation_config_digest: "fixture-config".into(),
            outbox_effects: Vec::new(),
            created_at_ms: 5,
        },
    )
    .unwrap();
    let expected = source.load_conversation("c").unwrap();
    let facts = changes(&source);
    let last = facts.last().unwrap();
    let completion = &facts[facts.len() - 2];
    assert_eq!(completion.entity_type, "conversation_transition");
    assert!(
        !completion.payload_json.contains("history"),
        "completion must not republish earlier messages"
    );
    assert!(
        !completion.payload_json.contains("checkpoint"),
        "replica must not contain runtime checkpoints"
    );
    assert_eq!(last.entity_type, "conversation_append");
    assert!(last.payload_json.len() < 1024);
    assert!(!last.payload_json.contains("answer"));
    let path = root.path().join("receiver.db");
    let mut receiver = NolaStore::open(&path).unwrap();
    receiver
        .apply_remote_sync_page(&page(facts[..facts.len() - 1].to_vec()))
        .unwrap();
    let before = receiver.load_conversation("c").unwrap();
    let cursor = receiver.sync_cursor("fixture").unwrap().unwrap().cursor;
    let mut bad = last.clone();
    let mut payload: serde_json::Value = serde_json::from_str(&bad.payload_json).unwrap();
    payload["kind"]["previous_run_id"] = "wrong-run".into();
    bad.payload_json = payload.to_string();
    assert!(receiver.apply_remote_sync_page(&page(vec![bad])).is_err());
    assert_eq!(receiver.load_conversation("c").unwrap(), before);
    assert_eq!(
        receiver.sync_cursor("fixture").unwrap().unwrap().cursor,
        cursor
    );
    drop(receiver);
    let mut receiver = NolaStore::open(&path).unwrap();
    receiver
        .apply_remote_sync_page(&page(vec![last.clone()]))
        .unwrap();
    assert_eq!(receiver.load_conversation("c").unwrap(), expected);
    assert!(receiver.active_run("c").unwrap().is_none());
    assert!(receiver.pending_sync_changes(20).unwrap().is_empty());
    let mut replay = last.clone();
    replay.cursor += 1;
    assert_eq!(
        receiver
            .apply_remote_sync_page(&page(vec![replay]))
            .unwrap()
            .skipped,
        1
    );
    assert_eq!(receiver.load_conversation("c").unwrap(), expected);
}

#[test]
fn transition_restores_distinct_part_owners_without_regressing_newer_metadata() {
    use nola_store_port::assistant_journal::AssistantJournalRepository;
    use nola_domain::assistant_journal_operations::{JournalNumber,JournalPartMetadata};
    let root = tempfile::tempdir().unwrap();
    let mut source = NolaStore::open(root.path().join("source.db")).unwrap();
    seed(&mut source);
    let part = |id: &str, kind: &str, text: &str| PartMutation::Upsert {
        part_id: id.into(),
        kind: kind.into(),
        ordinal: 1,
        state: "completed".into(),
        content_text: text.into(),
        content_json: None,
        content_digest: None,
        created_at_ms: 4,
        updated_at_ms: 4,
    };
    let progress = CommitTransitionInput {
            memory_usage_entries: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            effect_delivery_attempt_mutations: vec![],
            model_attempt_mutations: vec![],
            transition_id: "multiple-parts".into(),
            run_id: "new-run".into(),
            expected_checkpoint_revision: 1,
            checkpoint_revision: 2,
            checkpoint_event_cursor: 0,
            checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline { checkpoint_json: "{}".into() },
            submission_reduction_json: None, host_event: None,
            events: Vec::new(),
            ensured_messages: vec![TransitionMessageInput {
                id: "other-assistant".into(),
                run_id: Some("new-run".into()),
                role: "assistant".into(),
                status: "streaming".into(),
                created_at_ms: 4,
                completed_at_ms: None,
                initial_parts: Vec::new(),
            }],
            assistant_turn_bindings: vec![],
            part_message_owners: std::collections::BTreeMap::from([(
                "tool-result".into(),
                "other-assistant".into(),
            )]),
            part_mutations: vec![
                part("reasoning", "reasoning", "Inspect the evidence"),
                part("tool-result", "tool", "Read source.rs: found the answer"),
            ],
            outbox_effects: Vec::new(),
            acknowledged_effect_ids: Vec::new(),
            run_status: None,
            assistant_status: Some("completed".into()),
            assistant_status_message_id: Some("other-assistant".into()),
            committed_at_ms: 4,
        };
    source.begin_write_unit_of_work().unwrap();
    source.commit_transition(&progress).unwrap();
    terminal_sources::publish_completed_tool(&mut source,&progress,"c","other-assistant",
        "tool-result","fixture-read-call","read_source");
    source.commit_write_unit_of_work().unwrap();
    let mut finished = CommitTransitionInput {
            memory_usage_entries: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            effect_delivery_attempt_mutations: vec![],
            model_attempt_mutations: vec![],
            transition_id: "finish-multiple-parts".into(),
            run_id: "new-run".into(),
            expected_checkpoint_revision: 2,
            checkpoint_revision: 3,
            checkpoint_event_cursor: 0,
            checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline { checkpoint_json: "{}".into() },
            submission_reduction_json: None, host_event: None,
            events: Vec::new(),
            ensured_messages: Vec::new(),
            assistant_turn_bindings: vec![],
            part_message_owners: Default::default(),
            part_mutations: vec![part("new-assistant-text", "text", "Final answer")],
            outbox_effects: Vec::new(),
            acknowledged_effect_ids: Vec::new(),
            run_status: None,
            assistant_status: Some("completed".into()),
            assistant_status_message_id: None,
            committed_at_ms: 5,
        };
    terminal_sources::add_sources(&source,&mut finished,"c","new-run","new-user",false);
    source.begin_write_unit_of_work().unwrap();
    source.commit_transition(&finished).unwrap();
    terminal_sources::publish_and_seal(&mut source,&finished,"c");
    source.commit_write_unit_of_work().unwrap();
    let seal=source.local_assistant_run_terminal_seal("c","new-run").unwrap().unwrap();
    assert_eq!(seal.heads_count,JournalNumber(2));
    assert!(matches!(&seal.result.value,nola_domain::run_result::RunResultValue::Incomplete { .. }));
    assert!(source.active_run("c").unwrap().is_none(),"Only the formal sealed Run releases execution ownership");
    let manifest=source.read_local_assistant_run_terminal_manifest(&seal,JournalNumber(0),10,65536).unwrap();
    assert!(manifest.complete);
    let owners=manifest.entries.iter().map(|entry| {
        let nola_domain::assistant_run_terminal::AssistantRunTerminalManifestEntry::Assistant {
            through,state,completed_at_ms,..
        }=entry else {panic!("fixture has no delegated child receipts")};
        assert!(through.sequence>0,"Every admitted assistant must publish its own journal frontier");
        assert_eq!(*state,nola_domain::assistant_run_terminal::AssistantRunTerminalStatus::Completed);
        assert!(*completed_at_ms<=seal.completed_at_ms);
        through.identity.assistant_message_id.clone()
    }).collect::<std::collections::BTreeSet<_>>();
    assert_eq!(owners,std::collections::BTreeSet::from(["new-assistant".into(),"other-assistant".into()]));
    let tool_identity=nola_domain::assistant_journal::AssistantJournalIdentity {
        assistant_message_id:"other-assistant".into(),..seal.identity.clone()
    };
    let tool=source.assistant_journal_part(&tool_identity,"tool-result").unwrap().unwrap();
    assert!(matches!(tool.metadata,JournalPartMetadata::Tool(ref metadata)
        if metadata.tool_call_id=="fixture-read-call" && metadata.state==nola_domain::ActivityState::Completed));
    assert!(source.assistant_journal_part(&seal.identity,"tool-result").unwrap().is_none(),
        "The primary assistant cannot borrow the other assistant's tool Part");
    let mut facts = changes(&source);
    assert_eq!(facts.len(), 5);
    let path = root.path().join("receiver.db");
    let mut receiver = NolaStore::open(&path).unwrap();
    receiver
        .apply_remote_sync_page(&page(facts[..3].to_vec()))
        .unwrap();
    let header = nola_domain::ConversationMetadataReplica::new(
        "c".into(),
        10,
        3,
        "Newer title".into(),
        Some("{\"memoryEnabled\":true}".into()),
        10,
    )
    .unwrap();
    receiver
        .apply_remote_sync_page(&page(vec![RemoteSyncChangeInput {
            cursor: 4,
            operation_id: "newer-metadata".into(),
            entity_type: "conversation_metadata".into(),
            entity_id: "c".into(),
            revision: 10,
            deleted: false,
            payload_json: serde_json::to_string(&header).unwrap(),
            created_at_ms: 10,
        }]))
        .unwrap();
    let mut tail = facts.split_off(3);
    for change in &mut tail {
        change.cursor += 1;
    }
    let mut forged = tail[0].clone();
    let mut payload: serde_json::Value = serde_json::from_str(&forged.payload_json).unwrap();
    payload["parts"][1]["run_id"] = "foreign-run".into();
    forged.payload_json = payload.to_string();
    assert!(receiver
        .apply_remote_sync_page(&page(vec![forged]))
        .is_err());
    assert_eq!(
        receiver
            .load_conversation("c")
            .unwrap()
            .unwrap()
            .messages
            .len(),
        3,
        "failed owner validation must roll back a newly declared message too"
    );
    assert_eq!(receiver.sync_cursor("fixture").unwrap().unwrap().cursor, 4);
    drop(receiver);
    let mut receiver = NolaStore::open(&path).unwrap();
    receiver.apply_remote_sync_page(&page(tail)).unwrap();
    let restored = receiver.load_conversation("c").unwrap().unwrap();
    assert_eq!(
        restored.messages,
        source.load_conversation("c").unwrap().unwrap().messages
    );
    assert_eq!(restored.title, "Newer title");
    assert_eq!(restored.revision, 10);
    assert_eq!(
        restored.settings_json.as_deref(),
        Some("{\"memoryEnabled\":true}")
    );
    assert!(receiver.pending_sync_changes(10).unwrap().is_empty());
}

#[test]
fn first_turn_still_publishes_an_initial_content_base() {
    let root = tempfile::tempdir().unwrap();
    let mut source = NolaStore::open(root.path().join("source.db")).unwrap();
    source.begin_turn(&turn()).unwrap();
    let facts = changes(&source);
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].entity_type, "conversation");
    let mut receiver = NolaStore::open(root.path().join("receiver.db")).unwrap();
    receiver.apply_remote_sync_page(&page(facts)).unwrap();
    assert_eq!(
        receiver.load_conversation("c").unwrap(),
        source.load_conversation("c").unwrap()
    );
}
