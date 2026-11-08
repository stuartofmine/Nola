// SPDX-License-Identifier: Apache-2.0
use nola_application::{agent_room as rooms, persistent_agent as agents};
use nola_domain::persistent_agent::{AgentAvailability, AgentDefaults};
use nola_store::NolaStore;
use serde_json::json;

fn define(store: &mut NolaStore, id: &str, directory_revision: Option<i64>, language: &str) {
    agents::configure(
        &mut store.product_setting_repository(),
        agents::ConfigureAgent {
            id: id.to_string().try_into().unwrap(),
            expected_directory_revision: directory_revision,
            expected_agent_revision: None,
            name: id.into(),
            description: "Research and analysis".into(),
            defaults: AgentDefaults::try_from(json!({"language":language})).unwrap(),
            availability: AgentAvailability::Active,
            updated_at_ms: 1,
        },
    )
    .unwrap();
}

fn join(store: &mut NolaStore, room: &rooms::RoomSnapshot, agent: &str) -> rooms::RoomSnapshot {
    rooms::join(
        store,
        rooms::JoinAgent {
            room_id: room.room.id().into(),
            expected_room_revision: room.revision,
            agent_id: agent.to_string().try_into().unwrap(),
            expected_agent_revision: 1,
            at_ms: 2,
        },
    )
    .unwrap()
}

#[test]
fn room_submission_selection_and_execution_are_durable_and_not_redispatched() {
    let mut store = NolaStore::open(":memory:").unwrap();
    store
        .upsert_model_profile(&nola_store_port::model::StoredModelProfile {
            id: "model".into(),
            display_name: "Model".into(),
            provider: "fixture".into(),
            model_name: "wire".into(),
            runtime: "http".into(),
            source_json: "{}".into(),
            parameters_json: "{}".into(),
            selected: true,
            enabled: true,
            revision: 1,
            created_at_ms: 1,
            updated_at_ms: 1,
            deleted_at_ms: None,
        })
        .unwrap();
    store.begin_write_unit_of_work().unwrap();
    define(&mut store, "a", None, "zh");
    let room = rooms::create(&mut store.product_setting_repository(), "Shared".into(), 1).unwrap();
    let room = join(&mut store, &room, "a");
    let input = rooms::SubmitRoomMessage {
        room_id: room.room.id().into(),
        submission_id: uuid::Uuid::new_v4().to_string(),
        expected_room_revision: room.revision,
        text: "分析 / analyze".into(),
        participant_id: None,
        at_ms: 3,
        parent_message_id: None,
        conversation_id: None,
    };
    let accepted = rooms::submit(&mut store.product_setting_repository(), input.clone()).unwrap();
    assert_eq!(accepted.state, rooms::RoomSubmissionState::Accepted);
    assert_eq!(
        rooms::submit(&mut store.product_setting_repository(), input.clone()).unwrap(),
        accepted
    );
    let mut conflict = input.clone();
    conflict.text = "different".into();
    assert!(rooms::submit(&mut store.product_setting_repository(), conflict).is_err());
    assert!(rooms::select_recipient(
        &mut store.product_setting_repository(),
        &input.room_id,
        &input.submission_id,
        1,
        "foreign",
        4
    )
    .is_err());
    let selected = rooms::select_recipient(
        &mut store.product_setting_repository(),
        &input.room_id,
        &input.submission_id,
        1,
        &room.room.participants()[0].id,
        4,
    )
    .unwrap();
    assert!(rooms::claim_execution(
        &mut store,
        &input.room_id,
        &input.submission_id,
        selected.revision,
        5
    )
    .is_err());
    let intent = rooms::RoomExecutionIntent {
        execution_id: uuid::Uuid::new_v4().to_string(),
        envelope: rooms::RoomExecutionEnvelope::Native {
            platform: "android".into(),
            conversation_configuration_revision: 1,
            model_profile_id: "model".into(),
            model_profile_revision: 1,
            expected_parent_message_id: None,
            text: input.text.clone(),
        },
    };
    let selected = rooms::prepare_execution(
        &mut store,
        &input.room_id,
        &input.submission_id,
        selected.revision,
        intent.clone(),
        5,
    )
    .unwrap();
    assert_eq!(
        rooms::execution_intent(
            &store.product_setting_repository(),
            &input.room_id,
            &input.submission_id
        )
        .unwrap(),
        Some(intent.clone())
    );
    assert_eq!(
        rooms::prepare_execution(
            &mut store,
            &input.room_id,
            &input.submission_id,
            selected.revision,
            intent.clone(),
            5
        )
        .unwrap(),
        selected
    );
    let mut changed = intent.clone();
    changed.execution_id = uuid::Uuid::new_v4().to_string();
    assert!(rooms::prepare_execution(
        &mut store,
        &input.room_id,
        &input.submission_id,
        selected.revision,
        changed,
        5
    )
    .is_err());
    let claim = rooms::claim_execution(
        &mut store,
        &input.room_id,
        &input.submission_id,
        selected.revision,
        5,
    )
    .unwrap();
    assert!(claim.dispatch_permitted);
    assert_eq!(
        claim.submission.state,
        rooms::RoomSubmissionState::Executing
    );
    let replay = rooms::claim_execution(
        &mut store,
        &input.room_id,
        &input.submission_id,
        selected.revision,
        6,
    )
    .unwrap();
    assert!(
        replay.dispatch_permitted,
        "No exact native Run exists: retry keeps the same durable execution identity"
    );
    assert_eq!(
        claim.submission.execution_id,
        replay.submission.execution_id
    );
    assert!(rooms::observe_execution(
        &mut store,
        &input.room_id,
        &input.submission_id,
        "unknown-run",
        7
    )
    .is_err());
    store.commit_write_unit_of_work().unwrap();
    let saved =
        rooms::submissions(&store.product_setting_repository(), &input.room_id, 10, 0).unwrap();
    assert_eq!(saved, vec![claim.submission]);
    store.begin_write_unit_of_work().unwrap();
    let rejected =
        rooms::reconcile_execution(&mut store, &input.room_id, &input.submission_id, 8).unwrap();
    assert_eq!(rejected.state, rooms::RoomSubmissionState::Executing);
    assert!(rejected.failure_code.is_none(), "Observation cannot fail a durable pre-Run execution intent; exact retry retains its owner");
    assert_eq!(
        rooms::execution_intent(
            &store.product_setting_repository(),
            &input.room_id,
            &input.submission_id
        )
        .unwrap(),
        Some(intent)
    );
    store.commit_write_unit_of_work().unwrap();
}

#[test]
fn cancelling_prepared_message_prevents_late_dispatch_and_preserves_intent() {
    let mut store = NolaStore::open(":memory:").unwrap();
    store.begin_write_unit_of_work().unwrap();
    define(&mut store, "a", None, "zh");
    let room = rooms::create(&mut store.product_setting_repository(), "Team".into(), 1).unwrap();
    let room = join(&mut store, &room, "a");
    let input = rooms::SubmitRoomMessage {
        room_id: room.room.id().into(),
        submission_id: uuid::Uuid::new_v4().to_string(),
        expected_room_revision: room.revision,
        text: "继续 / continue".into(),
        participant_id: Some(room.room.participants()[0].id.clone()),
        at_ms: 3,
        parent_message_id: None,
        conversation_id: None,
    };
    let selected = rooms::submit(&mut store.product_setting_repository(), input.clone()).unwrap();
    let intent = rooms::RoomExecutionIntent {
        execution_id: uuid::Uuid::new_v4().to_string(),
        envelope: rooms::RoomExecutionEnvelope::Native {
            platform: "macos".into(),
            conversation_configuration_revision: 1,
            model_profile_id: "model".into(),
            model_profile_revision: 1,
            expected_parent_message_id: None,
            text: input.text.clone(),
        },
    };
    let mut invalid = intent.clone();
    if let rooms::RoomExecutionEnvelope::Native { text, .. } = &mut invalid.envelope {
        *text = "foreign message".into();
    }
    assert!(rooms::prepare_execution(
        &mut store,
        &input.room_id,
        &input.submission_id,
        selected.revision,
        invalid,
        4
    )
    .is_err());
    assert!(rooms::execution_intent(
        &store.product_setting_repository(),
        &input.room_id,
        &input.submission_id
    )
    .unwrap()
    .is_none());
    assert_eq!(
        rooms::submissions(&store.product_setting_repository(), &input.room_id, 1, 0).unwrap(),
        vec![selected.clone()]
    );
    let prepared = rooms::prepare_execution(
        &mut store,
        &input.room_id,
        &input.submission_id,
        selected.revision,
        intent.clone(),
        4,
    )
    .unwrap();
    let cancelled = rooms::cancel_pending(
        &mut store.product_setting_repository(),
        &input.room_id,
        &input.submission_id,
        prepared.revision,
        5,
    )
    .unwrap();
    assert_eq!(cancelled.state, rooms::RoomSubmissionState::Cancelled);
    store.commit_write_unit_of_work().unwrap();
    store.begin_write_unit_of_work().unwrap();
    let late_claim = rooms::claim_execution(
        &mut store,
        &input.room_id,
        &input.submission_id,
        prepared.revision,
        6,
    )
    .unwrap();
    assert!(!late_claim.dispatch_permitted);
    assert_eq!(late_claim.submission, cancelled);
    assert_eq!(
        rooms::prepare_execution(
            &mut store,
            &input.room_id,
            &input.submission_id,
            prepared.revision,
            intent.clone(),
            7
        )
        .unwrap(),
        cancelled
    );
    assert_eq!(
        rooms::execution_intent(
            &store.product_setting_repository(),
            &input.room_id,
            &input.submission_id
        )
        .unwrap(),
        Some(intent)
    );
    assert_eq!(
        rooms::reconcile_execution(&mut store, &input.room_id, &input.submission_id, 8).unwrap(),
        cancelled
    );
    store.commit_write_unit_of_work().unwrap();
}

#[test]
fn room_transcript_pages_do_not_move_when_execution_state_changes() {
    let mut store = NolaStore::open(":memory:").unwrap();
    store.begin_write_unit_of_work().unwrap();
    define(&mut store, "a", None, "zh");
    let room = rooms::create(&mut store.product_setting_repository(), "Shared".into(), 1).unwrap();
    let room = join(&mut store, &room, "a");
    let mut inputs = Vec::new();
    for at_ms in [3, 4, 5] {
        let input = rooms::SubmitRoomMessage {
            room_id: room.room.id().into(),
            submission_id: uuid::Uuid::new_v4().to_string(),
            expected_room_revision: room.revision,
            text: "message".into(),
            participant_id: None,
            at_ms,
            parent_message_id: None,
            conversation_id: None,
        };
        rooms::submit(&mut store.product_setting_repository(), input.clone()).unwrap();
        inputs.push(input);
    }
    rooms::select_recipient(
        &mut store.product_setting_repository(),
        &inputs[0].room_id,
        &inputs[0].submission_id,
        1,
        &room.room.participants()[0].id,
        99,
    )
    .unwrap();
    for (offset, input) in inputs.iter().rev().enumerate() {
        let page = rooms::submissions(
            &store.product_setting_repository(),
            room.room.id(),
            1,
            offset,
        )
        .unwrap();
        assert_eq!(page[0].input.submission_id, input.submission_id);
    }
    store.commit_write_unit_of_work().unwrap();
}

#[test]
fn same_room_agents_and_same_agent_in_other_room_have_distinct_contexts() {
    let mut store = NolaStore::open(":memory:").unwrap();
    store.begin_write_unit_of_work().unwrap();
    define(&mut store, "a", None, "zh");
    define(&mut store, "b", Some(1), "en");
    let room = rooms::create(&mut store.product_setting_repository(), "Shared".into(), 1).unwrap();
    let room = join(&mut store, &room, "a");
    let room = join(&mut store, &room, "b");
    let other = rooms::create(&mut store.product_setting_repository(), "Other".into(), 1).unwrap();
    let other = join(&mut store, &other, "a");
    let a = &room.room.participants()[0];
    let b = &room.room.participants()[1];
    let a2 = &other.room.participants()[0];
    let contexts: std::collections::BTreeSet<_> = [a, b, a2]
        .into_iter()
        .map(|p| &p.context_conversation_id)
        .collect();
    assert_eq!(contexts.len(), 3);
    for (member, language) in [(a, "zh"), (b, "en"), (a2, "zh")] {
        let native_id = uuid::Uuid::parse_str(&member.context_conversation_id)
            .unwrap()
            .to_string()
            .to_uppercase();
        assert_eq!(member.context_conversation_id, native_id);
        let raw = store
            .load_conversation_settings(&member.context_conversation_id)
            .unwrap()
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["language"], language);
        assert!(store
            .load_conversation(&member.context_conversation_id)
            .unwrap()
            .unwrap()
            .messages
            .is_empty());
    }
    assert_eq!(room.room.recipient(&b.id).unwrap(), b);
    assert!(room.room.recipient(&a2.id).is_err());
    store.commit_write_unit_of_work().unwrap();
}

#[test]
fn rejected_join_and_deactivation_preserve_contexts() {
    let mut store = NolaStore::open(":memory:").unwrap();
    store.begin_write_unit_of_work().unwrap();
    define(&mut store, "a", None, "zh");
    let room = rooms::create(&mut store.product_setting_repository(), "Shared".into(), 1).unwrap();
    let room = join(&mut store, &room, "a");
    assert!(rooms::join(
        &mut store,
        rooms::JoinAgent {
            room_id: room.room.id().into(),
            expected_room_revision: room.revision,
            agent_id: "a".to_string().try_into().unwrap(),
            expected_agent_revision: 1,
            at_ms: 3
        }
    )
    .is_err());
    assert_eq!(store.list_conversations().unwrap().len(), 1);
    assert_eq!(
        rooms::load(&store.product_setting_repository(), room.room.id())
            .unwrap()
            .unwrap(),
        room
    );
    let member = &room.room.participants()[0];
    let removed = rooms::deactivate(
        &mut store.product_setting_repository(),
        room.room.id(),
        &member.id,
        room.revision,
        4,
    )
    .unwrap();
    assert!(removed.room.recipient(&member.id).is_err());
    assert!(store
        .delete_conversation(&member.context_conversation_id)
        .is_err());
    assert!(store
        .load_conversation_settings(&member.context_conversation_id)
        .unwrap()
        .is_some());
    store.commit_write_unit_of_work().unwrap();
}

#[test]
fn room_and_context_mapping_survive_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("rooms.sqlite");
    let original;
    {
        let mut store = NolaStore::open(&path).unwrap();
        store.begin_write_unit_of_work().unwrap();
        define(&mut store, "a", None, "zh");
        let room =
            rooms::create(&mut store.product_setting_repository(), "Shared".into(), 1).unwrap();
        original = join(&mut store, &room, "a");
        store.commit_write_unit_of_work().unwrap();
    }
    let mut store = NolaStore::open(&path).unwrap();
    assert_eq!(
        rooms::load(&store.product_setting_repository(), original.room.id())
            .unwrap()
            .unwrap(),
        original
    );
    store.begin_write_unit_of_work().unwrap();
    let member = &original.room.participants()[0];
    let settings: serde_json::Value = serde_json::from_str(
        &store
            .load_conversation_settings(&member.context_conversation_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(settings["agentRoomOwner"]["room_id"], original.room.id());
    assert_eq!(settings["agentRoomOwner"]["participant_id"], member.id);
    store.rollback_write_unit_of_work().unwrap();
}

#[test]
fn ordinary_conversation_owns_an_empty_room_durably_without_starting_work() {
    use nola_store::CreateConversationInput;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("conversation-room.sqlite");
    let conversation_id = "conv-20261023195331876-2".to_string();
    let original;
    {
        let mut store = NolaStore::open(&path).unwrap();
        store
            .create_conversation(&CreateConversationInput {
                id: conversation_id.clone(),
                title: "Review the proposal".into(),
                revision: 1,
                settings_json: Some("{}".into()),
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
        store.begin_write_unit_of_work().unwrap();
        original = rooms::ensure_for_conversation(&mut store, conversation_id.clone(), 2).unwrap();
        assert_eq!(original.room.room.title(), "Review the proposal");
        assert!(original.room.room.participants().is_empty());
        assert!(rooms::submissions(
            &store.product_setting_repository(),
            &original.room_id,
            20,
            0
        )
        .unwrap()
        .is_empty());
        let retried =
            rooms::ensure_for_conversation(&mut store, conversation_id.clone(), 3).unwrap();
        assert_eq!(retried.room_id, original.room_id);
        assert_eq!(retried.revision, original.revision);
        store.commit_write_unit_of_work().unwrap();
    }
    let mut reopened = NolaStore::open(&path).unwrap();
    let associated = rooms::for_conversation(&mut reopened, &conversation_id)
        .unwrap()
        .unwrap();
    assert_eq!(associated.room_id, original.room_id);
    assert_eq!(associated.room, original.room);
}

#[test]
fn anchored_room_submission_requires_a_visible_user_message_and_uses_its_room_owner() {
    use nola_store::{
        AppendMessagesInput, CreateConversationInput, MessageInput, MessagePartInput,
    };
    let mut store = NolaStore::open(":memory:").unwrap();
    let conversation_id = uuid::Uuid::new_v4().to_string();
    store
        .create_conversation(&CreateConversationInput {
            id: conversation_id.clone(),
            title: "Chat".into(),
            revision: 1,
            settings_json: Some("{}".into()),
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    store
        .append_messages(&AppendMessagesInput {
            conversation_id: conversation_id.clone(),
            expected_message_count: 0,
            messages: vec![MessageInput {
                id: "canonical-question".into(),
                run_id: None,
                role: "user".into(),
                status: "completed".into(),
                ordinal: 1,
                created_at_ms: 2,
                completed_at_ms: Some(2),
                parts: vec![MessagePartInput {
                    id: "question-body".into(),
                    kind: "text".into(),
                    ordinal: 1,
                    state: "completed".into(),
                    content_text: "Please review".into(),
                    content_json: None,
                    content_digest: None,
                    created_at_ms: 2,
                    updated_at_ms: 2,
                }],
            }],
            updated_at_ms: 2,
        })
        .unwrap();
    store.begin_write_unit_of_work().unwrap();
    define(&mut store, "expert", None, "en");
    let association =
        rooms::ensure_for_conversation(&mut store, conversation_id.clone(), 3).unwrap();
    let room = join(&mut store, &association.room, "expert");
    let input = rooms::SubmitRoomMessage {
        room_id: room.room.id().into(),
        submission_id: uuid::Uuid::new_v4().to_string(),
        expected_room_revision: room.revision,
        text: "Please review".into(),
        participant_id: Some(room.room.participants()[0].id.clone()),
        at_ms: 4,
        parent_message_id: Some("canonical-question".into()),
        conversation_id: Some(conversation_id.clone()),
    };
    let accepted: rooms::RoomSubmission = serde_json::from_value(
        nola_application::execute_application_command(
            &mut store,
            nola_application::ApplicationCommand::AgentRoom {
                command: rooms::AgentRoomCommand::Submit {
                    input: input.clone(),
                },
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        accepted.input.parent_message_id.as_deref(),
        Some("canonical-question")
    );
    assert_eq!(
        accepted.selected_participant_id.as_deref(),
        Some(room.room.participants()[0].id.as_str())
    );
    assert_eq!(
        rooms::submit_for_conversation(&mut store, &conversation_id, input.clone()).unwrap(),
        accepted,
        "retrying the exact anchored input returns its original accepted submission"
    );
    let mut missing = input.clone();
    missing.submission_id = uuid::Uuid::new_v4().to_string();
    missing.parent_message_id = Some("deleted-message".into());
    assert_eq!(
        rooms::submit_for_conversation(&mut store, &conversation_id, missing).unwrap_err(),
        "room_parent_message_not_visible_user_message"
    );
    let mut wrong_room = input.clone();
    wrong_room.submission_id = uuid::Uuid::new_v4().to_string();
    wrong_room.room_id = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        rooms::submit_for_conversation(&mut store, &conversation_id, wrong_room).unwrap_err(),
        "room_conversation_owner_mismatch"
    );
    assert_eq!(
        rooms::submissions(&store.product_setting_repository(), &room.room.id(), 20, 0)
            .unwrap()
            .len(),
        1
    );
    store.commit_write_unit_of_work().unwrap();
}

#[test]
fn room_and_conversation_association_is_one_to_one_and_atomic() {
    use nola_store::CreateConversationInput;
    let mut store = NolaStore::open(":memory:").unwrap();
    let conversation_a = "cli-named-session".to_string();
    let conversation_b = uuid::Uuid::new_v4().to_string();
    for id in [&conversation_a, &conversation_b] {
        store
            .create_conversation(&CreateConversationInput {
                id: id.clone(),
                title: "A conversation".into(),
                revision: 1,
                settings_json: Some("{}".into()),
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
    }
    store.begin_write_unit_of_work().unwrap();
    let first = rooms::create(
        &mut store.product_setting_repository(),
        "Existing room".into(),
        1,
    )
    .unwrap();
    let linked = rooms::attach_conversation(
        &mut store,
        conversation_a.clone(),
        first.room.id().into(),
        2,
    )
    .unwrap();
    assert_eq!(linked.room_id, first.room.id());
    assert!(rooms::attach_conversation(
        &mut store,
        conversation_b.clone(),
        first.room.id().into(),
        3
    )
    .is_err());
    let second = rooms::create(
        &mut store.product_setting_repository(),
        "Another room".into(),
        4,
    )
    .unwrap();
    assert!(rooms::attach_conversation(
        &mut store,
        conversation_a.clone(),
        second.room.id().into(),
        5
    )
    .is_err());
    assert_eq!(
        rooms::for_conversation(&mut store, &conversation_a)
            .unwrap()
            .unwrap()
            .room_id,
        first.room.id()
    );
    assert_eq!(
        rooms::for_conversation(&mut store, &conversation_b).unwrap(),
        None
    );
    store.commit_write_unit_of_work().unwrap();
}

#[test]
fn incomplete_conversation_room_association_fails_closed_after_index_deletion() {
    use nola_store::CreateConversationInput;
    let mut store = NolaStore::open(":memory:").unwrap();
    let conversation_id = uuid::Uuid::new_v4().to_string();
    store
        .create_conversation(&CreateConversationInput {
            id: conversation_id.clone(),
            title: "A conversation".into(),
            revision: 1,
            settings_json: Some("{}".into()),
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    store.begin_write_unit_of_work().unwrap();
    rooms::ensure_for_conversation(&mut store, conversation_id.clone(), 2).unwrap();
    let reverse_key = format!(
        "{}{}",
        rooms::ROOM_CONVERSATION_KEY_PREFIX,
        rooms::for_conversation(&mut store, &conversation_id)
            .unwrap()
            .unwrap()
            .room_id
    );
    store.delete_product_setting(&reverse_key).unwrap();
    assert_eq!(
        rooms::for_conversation(&mut store, &conversation_id).unwrap_err(),
        "conversation Room reverse association missing"
    );
    store.rollback_write_unit_of_work().unwrap();
}

#[test]
fn room_command_queries_and_mutations_have_explicit_admission_semantics() {
    use nola_application::ApplicationCommand;
    for (command, read_only) in [
        (
            json!({"type":"agent_room","command":{"type":"directory"}}),
            true,
        ),
        (
            json!({"type":"agent_room","command":{"type":"read","room_id":"room"}}),
            true,
        ),
        (
            json!({"type":"agent_room","command":{"type":"for_conversation","conversation_id":"c"}}),
            true,
        ),
        (
            json!({"type":"agent_room","command":{"type":"ensure_for_conversation","conversation_id":"c","at_ms":1}}),
            false,
        ),
        (
            json!({"type":"agent_room","command":{"type":"attach_conversation","conversation_id":"c","room_id":"r","at_ms":1}}),
            false,
        ),
        (
            json!({"type":"agent_room","command":{"type":"create","title":"Room","at_ms":1}}),
            false,
        ),
    ] {
        let command: ApplicationCommand = serde_json::from_value(command).unwrap();
        assert_eq!(command.is_read_only_query(), read_only);
    }
}

#[test]
fn room_listing_is_bounded_and_does_not_expose_other_settings() {
    let mut store = NolaStore::open(":memory:").unwrap();
    store.begin_write_unit_of_work().unwrap();
    store
        .put_product_setting("other_setting", "true", 100)
        .unwrap();
    let first = rooms::create(&mut store.product_setting_repository(), "First".into(), 1).unwrap();
    let second =
        rooms::create(&mut store.product_setting_repository(), "Second".into(), 2).unwrap();
    assert_eq!(
        rooms::list(&store.product_setting_repository(), 1, 0).unwrap(),
        vec![second]
    );
    assert_eq!(
        rooms::list(&store.product_setting_repository(), 1, 1).unwrap(),
        vec![first]
    );
    assert!(rooms::list(&store.product_setting_repository(), 0, 0).is_err());
    assert!(rooms::list(&store.product_setting_repository(), 257, 0).is_err());
    store.commit_write_unit_of_work().unwrap();
}

#[test]
fn ui_surfaces_with_same_wire_id_are_isolated_and_removed_members_cannot_submit() {
    use nola_domain::generative_ui::*;
    use nola_store_port::generative_ui::GenerativeUiRepository;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("room-ui.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    store.begin_write_unit_of_work().unwrap();
    define(&mut store, "ui-a", None, "en");
    define(&mut store, "ui-b", Some(1), "en");
    let room = rooms::create(&mut store.product_setting_repository(), "UI room".into(), 1).unwrap();
    let room = join(&mut store, &room, "ui-a");
    let room = join(&mut store, &room, "ui-b");
    store.commit_write_unit_of_work().unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    let mut snapshots = Vec::new();
    for (index, member) in room.room.participants().iter().enumerate() {
        let run = format!("ui-run-{index}");
        let message = format!("ui-message-{index}");
        let part = format!("ui-tool-{index}");
        db.execute("INSERT INTO messages(id,conversation_id,run_id,role,status,ordinal,created_at_ms) VALUES(?1,?2,?3,'assistant','streaming',1,1)",
            rusqlite::params![message, member.context_conversation_id, run]).unwrap();
        db.execute("INSERT INTO runs(id,conversation_id,kind,assistant_message_id,status,next_event_sequence,created_at_ms,updated_at_ms) VALUES(?1,?2,'conversation',?3,'running',1,1,1)",
            rusqlite::params![run, member.context_conversation_id, message]).unwrap();
        db.execute("INSERT INTO message_parts(id,message_id,kind,ordinal,state,created_at_ms,updated_at_ms) VALUES(?1,?2,'tool',0,'open',1,1)",
            rusqlite::params![part, message]).unwrap();
        let identity = UiSurfaceIdentity {
            owner: UiSurfaceOwner {
                authority_id: store.ui_authority_id().unwrap(),
                conversation_id: member.context_conversation_id.clone(),
                context_id: member.context_conversation_id.clone(),
                participant_id: member.id.clone(),
                originating_run_id: run,
                message_id: message,
                part_id: part,
            },
            surface_id: "form".into(),
            generation: 1,
        };
        let input =
            serde_json::from_value(nola_kernel::generative_ui::schema::minimal_form_example())
                .unwrap();
        let state = nola_kernel::generative_ui::state::transition_messages(None, &identity, &input)
            .unwrap();
        let snapshot = nola_kernel::generative_ui::projection::project_surface(&state)
            .unwrap()
            .snapshot;
        store.begin_write_unit_of_work().unwrap();
        store.write_ui_surface(0, &state).unwrap();
        store.write_ui_surface_part(&snapshot, 3).unwrap();
        store.commit_write_unit_of_work().unwrap();
        snapshots.push(snapshot);
    }
    assert_eq!(
        snapshots[0].identity.surface_id,
        snapshots[1].identity.surface_id
    );
    assert_ne!(snapshots[0].identity, snapshots[1].identity);
    for snapshot in &snapshots {
        let listed = nola_application::generative_ui::list_ui_snapshots(
            &store,
            &snapshot.identity.owner.context_id,
        )
        .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].identity, snapshot.identity);
        let history = nola_application::generative_ui::read_ui_revision_snapshot(
            &store,
            &snapshot.identity,
            1,
        )
        .unwrap()
        .unwrap();
        assert_eq!(history.phase, UiSurfacePhase::ReadOnly);
        assert!(history
            .nodes
            .values()
            .all(|node| node.actions.is_empty() && node.bindings.is_empty()));
    }
    let make_input = |snapshot: &UiSnapshot, submission: &str| {
        let button = snapshot
            .nodes
            .values()
            .find(|node| node.component == "Button")
            .unwrap();
        nola_application::generative_ui::SubmitUiInteraction {
            interaction: UiInteraction {
                identity: snapshot.identity.clone(),
                expected_surface_revision: snapshot.revision,
                edits: vec![],
                action: Some(button.actions["action"].clone()),
            },
            submission_id: submission.into(),
            submitted_at_ms: 10,
        }
    };
    // A source-host identity cannot be replayed against another authority.
    let mut forged = make_input(&snapshots[0], "foreign");
    forged.interaction.identity.owner.authority_id = "foreign-host".into();
    assert!(nola_application::generative_ui::submit_ui_interaction(&mut store, forged).is_err());
    store.begin_write_unit_of_work().unwrap();
    rooms::deactivate(
        &mut store.product_setting_repository(),
        room.room.id(),
        &snapshots[0].identity.owner.participant_id,
        room.revision,
        8,
    )
    .unwrap();
    store.commit_write_unit_of_work().unwrap();
    assert!(nola_application::generative_ui::submit_ui_interaction(
        &mut store,
        make_input(&snapshots[0], "removed")
    )
    .is_err());
    assert_eq!(
        nola_application::generative_ui::read_ui_snapshot(&store, &snapshots[0].identity)
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    let accepted = nola_application::generative_ui::submit_ui_interaction(
        &mut store,
        make_input(&snapshots[1], "active"),
    )
    .unwrap();
    assert_eq!(accepted.queued_submission_id.as_deref(), Some("active"));
    assert_eq!(
        db.query_row::<i64, _, _>(
            "SELECT count(*) FROM agent_commands WHERE command_id IN ('foreign','removed')",
            [],
            |row| row.get(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn untitled_conversation_can_attach_experts_before_first_turn() {
    use nola_store::CreateConversationInput;
    let mut store = NolaStore::open(":memory:").unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    store
        .create_conversation(&CreateConversationInput {
            id: id.clone(),
            title: String::new(),
            revision: 1,
            settings_json: Some("{}".into()),
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    store.begin_write_unit_of_work().unwrap();
    let association = rooms::ensure_for_conversation(&mut store, id.clone(), 2).unwrap();
    assert_eq!(association.room.room.title(), "Conversation");
    assert_eq!(
        rooms::ensure_for_conversation(&mut store, id, 3).unwrap(),
        association
    );
    store.commit_write_unit_of_work().unwrap();
}

#[test]
fn routing_success_is_reconciled_after_reopen_without_dispatch() {
    use nola_application::execute_auxiliary_invocation;
    use nola_domain::AuxiliaryInvocationCommand;
    use nola_domain::{AuxiliaryInvocationConfiguration, AuxiliaryInvocationOwner, AuxiliaryInvocationScenario, ModelAttemptState};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("room.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    store.begin_write_unit_of_work().unwrap();
    define(&mut store, "router-member", None, "en");
    let room = rooms::create(&mut store.product_setting_repository(), "Routing".into(), 1).unwrap();
    let room = join(&mut store, &room, "router-member");
    let submission_id = uuid::Uuid::new_v4().to_string();
    let member = room.room.participants()[0].id.clone();
    rooms::submit(&mut store.product_setting_repository(), rooms::SubmitRoomMessage {
        room_id:room.room.id().into(), submission_id:submission_id.clone(), expected_room_revision:room.revision,
        text:"Non-sensitive routing fixture".into(), participant_id:None, at_ms:3, parent_message_id:None, conversation_id:None,
    }).unwrap();
    store.commit_write_unit_of_work().unwrap();
    let receipt = execute_auxiliary_invocation(&mut store, AuxiliaryInvocationCommand::Admit {
        operation_id:submission_id.clone(), owner:AuxiliaryInvocationOwner::Room {room_id:room.room.id().into()},
        scenario:AuxiliaryInvocationScenario::RoomRouting,
        configuration:AuxiliaryInvocationConfiguration {
            provider:"fixture".into(), model:Some("frozen".into()), configuration_revision:1,
            configuration_digest:"a".repeat(64), input_digest:"b".repeat(64), max_attempts:1,
            maximum_duration_ms:10000, max_output_tokens_per_attempt:Some(64), total_output_token_budget:Some(64),
        }, at_ms:4,
    }).unwrap();
    let started = execute_auxiliary_invocation(&mut store, AuxiliaryInvocationCommand::StartAttempt {
        operation_id:submission_id.clone(), attempt_id:"attempt".into(), expected_revision:receipt.operation.revision, at_ms:5,
    }).unwrap();
    let attempt = started.attempt.unwrap();
    execute_auxiliary_invocation(&mut store, AuxiliaryInvocationCommand::RecordReceipt {
        operation_id:submission_id.clone(), attempt_id:attempt.id, expected_attempt_revision:attempt.revision,
        state:ModelAttemptState::Succeeded, usage:json!({"coverage":"unknown","provider_raw":null,"normalized":null}),
        failure:None, output:Some(json!({"participant_id":member})), at_ms:6,
    }).unwrap();
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    let selected = rooms::reconcile_routing(&mut store, room.room.id(), &submission_id, None, 7).unwrap().unwrap();
    assert_eq!(selected.state, rooms::RoomSubmissionState::Selected);
    assert_eq!(rooms::reconcile_routing(&mut store, room.room.id(), &submission_id, None, 8).unwrap(), Some(selected));
}
