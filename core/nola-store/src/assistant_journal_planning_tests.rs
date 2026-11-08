// SPDX-License-Identifier: Apache-2.0
//! Persisted changed-item ownership and order allocation contracts.
use super::*;
use nola_domain::assistant_journal::AssistantJournalIdentity;
use nola_domain::assistant_journal_operations::{JournalNumber, JournalSection};
use nola_domain::{AssistantTurnBinding, PartMutation, TransitionMessageInput};
use nola_store_port::assistant_journal::{AssistantJournalRepository, AssistantSourceCommitStamp};
use nola_store_port::assistant_ownership::AssistantOwnershipFacts;
#[path = "../tests/support/terminal_sources.rs"]
mod terminal_sources;

#[test]
fn core_turn_resolution_never_guesses_latest_run_and_rejects_cross_run_ambiguity() {
    let directory=tempfile::tempdir().unwrap();let mut store=NolaStore::open(directory.path().join("core-turn-owner.sqlite")).unwrap();
    store.begin_turn(&admission("old-run","old-core-turn",1)).unwrap();
    terminal_sources::settle_fixture(&mut store,"conversation","old-run","old-core-turn","old-run-user");
    let observed = store.load_conversation_message_window("conversation", None, 1, 1).unwrap().unwrap();
    let mut next = admission("new-run","new-core-turn",3);
    next.expected_configuration_revision = Some(observed.conversation.configuration_revision);
    next.expected_parent_message_id = observed.conversation.messages.last().map(|message| message.id.clone());
    store.begin_turn(&next).unwrap();
    let old=store.assistant_owner_for_core_turn("conversation","old-core-turn").unwrap().unwrap();
    assert_eq!(old.run_id,"old-run");assert_eq!(old.assistant_message_id,"old-run-assistant");
    assert!(store.assistant_owner_for_core_turn("conversation","unknown-turn").unwrap().is_none());
    assert!(store.assistant_owner_for_core_turn("foreign","old-core-turn").unwrap().is_none());
    // A different actual admission may reuse a Turn ID. Preserve the evidence
    // and reject ambiguity rather than choosing the newest matching Run.
    terminal_sources::settle_fixture(&mut store,"conversation","new-run","new-core-turn","new-run-user");
    let observed = store.load_conversation_message_window("conversation", None, 1, 1).unwrap().unwrap();
    let mut collision = admission("collision-run","old-core-turn",5);
    collision.expected_configuration_revision = Some(observed.conversation.configuration_revision);
    collision.expected_parent_message_id = observed.conversation.messages.last().map(|message| message.id.clone());
    store.begin_turn(&collision).unwrap();
    assert!(store.assistant_owner_for_core_turn("conversation","old-core-turn").is_err());
    assert_eq!(store.assistant_owner_for_turn("conversation","old-run","old-core-turn").unwrap().unwrap(),old);
}

fn part_binding(turn: &str, message: &str, part: &str) -> AssistantTurnBinding {
    AssistantTurnBinding::Part {
        turn_id: turn.into(),
        assistant_message_id: message.into(),
        part_id: part.into(),
    }
}
fn admission(run: &str, turn: &str, revision: i64) -> BeginTurnInput {
    let message = format!("{run}-assistant");
    let part = format!("{run}-part");
    BeginTurnInput {
        user_part_kind: "text".into(),
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        initial_events: Vec::new(),
        conversation_id: "conversation".into(),
        conversation_title: "Journal planning".into(),
        user_message_id: format!("{run}-user"),
        user_part_id: format!("{run}-user-part"),
        user_text: "继续 / continue".into(),
        user_content_json: None,
        additional_user_parts: vec![],
        assistant_message_id: message.clone(),
        assistant_part_id: part.clone(),
        initial_assistant_part_mutations: vec![],
        assistant_turn_bindings: vec![
            AssistantTurnBinding::Admission {
                turn_id: turn.into(),
                assistant_message_id: message.clone(),
            },
            part_binding(turn, &message, &part),
        ],
        run_id: run.into(),
        context_checkpoint_id: None,
        checkpoint_revision: revision,
        checkpoint_event_cursor: 0,
        checkpoint_json: "{}".into(),
        invocation_config_json: "{}".into(),
        invocation_config_digest: "fixture".into(),
        outbox_effects: vec![],
        model_attempt_mutations: vec![],
        delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![],
        memory_usage_entries: vec![],
        created_at_ms: revision,
    }
}
fn transition(run: &str, revision: i64) -> CommitTransitionInput {
    CommitTransitionInput {
        transition_id: format!("{run}-transition-{revision}"),
        run_id: run.into(),
        expected_checkpoint_revision: revision - 1,
        checkpoint_revision: revision,
        checkpoint_event_cursor: 0,
        checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline { checkpoint_json: "{}".into() },
        submission_reduction_json: None, host_event: None,
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
fn append_message(input: &mut CommitTransitionInput, message: &str, part: &str, turn: &str) {
    input.ensured_messages.push(TransitionMessageInput {
        id: message.into(),
        run_id: Some(input.run_id.clone()),
        role: "assistant".into(),
        status: "streaming".into(),
        created_at_ms: input.committed_at_ms,
        completed_at_ms: None,
        initial_parts: vec![],
    });
    input
        .part_message_owners
        .insert(part.into(), message.into());
    input.part_mutations.push(PartMutation::Upsert {
        part_id: part.into(),
        kind: "text".into(),
        ordinal: 0,
        state: "completed".into(),
        content_text: "actual text".into(),
        content_json: Some("{\"turn_id\":\"forged-presentation-turn\"}".into()),
        content_digest: None,
        created_at_ms: input.committed_at_ms,
        updated_at_ms: input.committed_at_ms,
    });
    input
        .assistant_turn_bindings
        .push(part_binding(turn, message, part));
}
fn identity(store: &NolaStore, run: &str, message: &str) -> AssistantJournalIdentity {
    AssistantJournalIdentity {
        source_store_epoch: store.assistant_source_epoch().unwrap(),
        conversation_id: "conversation".into(),
        run_id: run.into(),
        assistant_message_id: message.into(),
    }
}
fn internal(store: &NolaStore, run: &str, revision: i64) -> BeginInternalTurnInput {
    let target = store
        .internal_assistant_target("conversation")
        .unwrap()
        .unwrap();
    BeginInternalTurnInput {
        initial_events: Vec::new(),
        conversation_id: "conversation".into(),
        assistant_message_id: target.assistant_message_id.clone(),
        expected_previous_run_id: target.run_id,
        expected_checkpoint_revision: store.checkpoint_revision("conversation").unwrap(),
        assistant_part_id: "not-yet-materialized".into(),
        initial_assistant_part_mutations: vec![],
        assistant_turn_bindings: vec![AssistantTurnBinding::Admission {
            turn_id: "internal-core-turn".into(),
            assistant_message_id: target.assistant_message_id,
        }],
        run_id: run.into(),
        context_checkpoint_id: None,
        checkpoint_revision: revision,
        checkpoint_event_cursor: 0,
        checkpoint_json: "{}".into(),
        invocation_config_json: "{}".into(),
        invocation_config_digest: "fixture".into(),
        outbox_effects: vec![],
        model_attempt_mutations: vec![],
        delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![],
        memory_usage_entries: vec![],
        created_at_ms: revision,
    }
}

#[test]
fn exact_persisted_ownership_survives_reopen_without_body_or_run_guessing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("owners.db");
    let mut store = NolaStore::open(&path).unwrap();
    store
        .begin_turn(&admission("run", "actual-core-turn", 1))
        .unwrap();
    // Deliberately invalid presentation bodies must not affect keyed ownership.
    store
        .connection
        .execute(
            "UPDATE message_parts SET content_json='not-json' WHERE id='run-part'",
            [],
        )
        .unwrap();
    // The derived delivery-witness trigger now rejects malformed checkpoint
    // JSON at write time. Keep that guard, then install valid JSON which is
    // not a Core checkpoint: keyed ownership must not deserialize it or infer
    // identity from the forged presentation Turn.
    assert!(store
        .connection
        .execute("UPDATE core_checkpoint_bodies SET checkpoint_json='not-json'", [])
        .is_err());
    store
        .connection
        .execute(
            "UPDATE core_checkpoint_bodies SET checkpoint_json=?1",
            [r#"{"turn_id":"forged-presentation-turn","state":"not-a-core-state"}"#],
        )
        .unwrap();
    drop(store);
    let store = NolaStore::open(&path).unwrap();
    let owner = store
        .assistant_owner_for_turn("conversation", "run", "actual-core-turn")
        .unwrap()
        .unwrap();
    assert_eq!(owner.assistant_message_id, "run-assistant");
    assert!(store
        .assistant_owner_for_turn("conversation", "run", "run")
        .unwrap()
        .is_none());
    assert!(store
        .assistant_owner_for_turn("conversation", "run", "forged-presentation-turn")
        .unwrap()
        .is_none());
    let part = store
        .assistant_owner_for_part("conversation", "run", "run-part")
        .unwrap()
        .unwrap();
    assert_eq!(part.turn_id.as_deref(), Some("actual-core-turn"));
    assert_eq!(part.owner, owner);
    assert!(store
        .assistant_owner_for_part("conversation", "run", "run-user-part")
        .is_err());
    assert!(store
        .assistant_owner_for_turn("foreign", "run", "actual-core-turn")
        .is_err());
    assert!(store
        .assistant_owner_for_part("conversation", "missing-run", "run-part")
        .unwrap()
        .is_none());
}

#[test]
fn admission_priority_and_ambiguous_successors_preserve_real_membership() {
    let mut store = NolaStore::open(":memory:").unwrap();
    store
        .begin_turn(&admission("run", "admitted-turn", 1))
        .unwrap();
    let mut input = transition("run", 2);
    append_message(
        &mut input,
        "successor-a",
        "successor-part-a",
        "admitted-turn",
    );
    store.commit_transition(&input).unwrap();
    assert_eq!(
        store
            .assistant_owner_for_turn("conversation", "run", "admitted-turn")
            .unwrap()
            .unwrap()
            .assistant_message_id,
        "run-assistant"
    );
    assert_eq!(
        store
            .assistant_owner_for_part("conversation", "run", "successor-part-a")
            .unwrap()
            .unwrap()
            .owner
            .assistant_message_id,
        "successor-a"
    );
    let mut input = transition("run", 3);
    append_message(
        &mut input,
        "successor-b",
        "successor-part-b",
        "shared-non-admission-turn",
    );
    append_message(
        &mut input,
        "successor-c",
        "successor-part-c",
        "shared-non-admission-turn",
    );
    assert!(store.commit_transition(&input).unwrap().applied);
    assert!(matches!(
        store.assistant_owner_for_turn("conversation", "run", "shared-non-admission-turn"),
        Err(StoreError::Conflict(_))
    ));
    for (message, part) in [
        ("successor-b", "successor-part-b"),
        ("successor-c", "successor-part-c"),
    ] {
        assert_eq!(
            store
                .assistant_owner_for_part("conversation", "run", part)
                .unwrap()
                .unwrap()
                .owner
                .assistant_message_id,
            message
        );
    }
    let json = store
        .load_local_presentation_inputs("conversation", "[\"run-assistant\",\"successor-b\"]", None)
        .unwrap()
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let facts: AssistantOwnershipFacts =
        serde_json::from_value(value["ownership"].clone()).unwrap();
    assert!(facts.turns.iter().any(|turn| turn.admission
        && turn.turn_id == "admitted-turn"
        && turn.owner.assistant_message_id == "run-assistant"));
    assert_eq!(
        facts
            .turns
            .iter()
            .filter(|turn| turn.turn_id == "shared-non-admission-turn")
            .count(),
        2
    );
    assert!(facts
        .parts
        .iter()
        .all(|part| part.turn_id.as_deref() != Some("forged-presentation-turn")));
}

#[test]
fn rejected_replayed_and_rolled_back_commits_cannot_mint_turn_bindings() {
    let mut store = NolaStore::open(":memory:").unwrap();
    store
        .begin_turn(&admission("run", "admitted-turn", 1))
        .unwrap();
    let accepted = transition("run", 2);
    store.commit_transition(&accepted).unwrap();
    let mut replay = accepted;
    append_message(&mut replay, "replay-message", "replay-part", "replay-turn");
    assert!(!store.commit_transition(&replay).unwrap().applied);
    assert!(store
        .assistant_owner_for_turn("conversation", "run", "replay-turn")
        .unwrap()
        .is_none());
    replay.transition_id = "stale-transition".into();
    assert!(store.commit_transition(&replay).is_err());
    store.begin_write_unit_of_work().unwrap();
    let mut input = transition("run", 3);
    append_message(
        &mut input,
        "rollback-message",
        "rollback-part",
        "rollback-turn",
    );
    store.commit_transition(&input).unwrap();
    assert!(store
        .assistant_owner_for_turn("conversation", "run", "rollback-turn")
        .unwrap()
        .is_some());
    store.rollback_write_unit_of_work().unwrap();
    assert!(store
        .assistant_owner_for_turn("conversation", "run", "rollback-turn")
        .unwrap()
        .is_none());
    let mut input = transition("run", 3);
    input.assistant_turn_bindings.push(part_binding(
        "forged-turn",
        "run-assistant",
        "missing-part",
    ));
    assert!(store.commit_transition(&input).is_err());
    assert_eq!(store.checkpoint_revision("conversation").unwrap(), Some(2));
    assert!(store
        .assistant_owner_for_turn("conversation", "run", "forged-turn")
        .unwrap()
        .is_none());
}

#[test]
fn internal_admission_requires_exact_selected_message_previous_run_and_checkpoint() {
    let mut store = NolaStore::open(":memory:").unwrap();
    assert!(store
        .internal_assistant_target("conversation")
        .unwrap()
        .is_none());
    store
        .begin_turn(&admission("first-run", "first-turn", 1))
        .unwrap();
    terminal_sources::settle_fixture(&mut store,"conversation","first-run","first-turn","first-run-user");
    let observed = store.load_conversation_message_window("conversation", None, 1, 1).unwrap().unwrap();
    let mut latest = admission("latest-run", "latest-turn", 3);
    latest.expected_configuration_revision = Some(observed.conversation.configuration_revision);
    latest.expected_parent_message_id = observed.conversation.messages.last().map(|message| message.id.clone());
    store.begin_turn(&latest).unwrap();
    terminal_sources::settle_fixture(&mut store,"conversation","latest-run","latest-turn","latest-run-user");
    let target = store
        .internal_assistant_target("conversation")
        .unwrap()
        .unwrap();
    assert_eq!(target.assistant_message_id, "latest-run-assistant");
    let valid = internal(&store, "resume", 5);
    // Every independent mismatch must reject before canonical writes.
    let mut wrong_message = valid.clone();
    wrong_message.assistant_message_id = "first-run-assistant".into();
    assert!(matches!(
        store.begin_internal_turn(&wrong_message),
        Err(StoreError::Conflict(_))
    ));
    let mut wrong_run = valid.clone();
    wrong_run.expected_previous_run_id = "first-run".into();
    assert!(matches!(
        store.begin_internal_turn(&wrong_run),
        Err(StoreError::Conflict(_))
    ));
    let mut wrong_checkpoint = valid.clone();
    wrong_checkpoint.expected_checkpoint_revision = None;
    assert!(matches!(
        store.begin_internal_turn(&wrong_checkpoint),
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(
        store.internal_assistant_target("conversation").unwrap(),
        Some(target)
    );
    assert_eq!(store.checkpoint_revision("conversation").unwrap(), Some(4));
    store.begin_internal_turn(&valid).unwrap();
    assert_eq!(
        store
            .internal_assistant_target("conversation")
            .unwrap()
            .unwrap()
            .run_id,
        "resume"
    );
    assert_eq!(
        store
            .assistant_owner_for_turn("conversation", "resume", "internal-core-turn")
            .unwrap()
            .unwrap()
            .assistant_message_id,
        "latest-run-assistant"
    );
    assert!(store
        .assistant_owner_for_part("conversation", "resume", "not-yet-materialized")
        .unwrap()
        .is_none());
    let mut stale = valid;
    stale.run_id = "competing-resume".into();
    stale.checkpoint_revision = 6;
    assert!(matches!(
        store.begin_internal_turn(&stale),
        Err(StoreError::Conflict(_))
    ));
}

#[test]
fn durable_order_is_exact_owner_section_scoped_idempotent_and_rollback_safe() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("orders.db");
    let mut store = NolaStore::open(&path).unwrap();
    store.begin_write_unit_of_work().unwrap();
    store
        .begin_turn(&admission("run", "actual-turn", 1))
        .unwrap();
    let id = identity(&store, "run", "run-assistant");
    let stamp = AssistantSourceCommitStamp::Admission {
        run_id: "run".into(),
    };
    let first = store
        .reserve_assistant_journal_order(&stamp, &id, JournalSection::Transcript, "journal-part-a")
        .unwrap();
    assert_eq!(first.position, JournalNumber(1));
    assert_eq!(
        store
            .reserve_assistant_journal_order(
                &stamp,
                &id,
                JournalSection::Transcript,
                "journal-part-a"
            )
            .unwrap(),
        first
    );
    assert!(store
        .reserve_assistant_journal_order(&stamp, &id, JournalSection::Tasks, "journal-part-a")
        .is_err());
    assert_eq!(
        store
            .reserve_assistant_journal_order(&stamp, &id, JournalSection::Tasks, "task-a")
            .unwrap()
            .position,
        JournalNumber(1)
    );
    let foreign = AssistantJournalIdentity {
        source_store_epoch: "foreign".into(),
        ..id.clone()
    };
    assert!(store
        .reserve_assistant_journal_order(&stamp, &foreign, JournalSection::Transcript, "foreign")
        .is_err());
    store.commit_write_unit_of_work().unwrap();
    assert!(store
        .reserve_assistant_journal_order(&stamp, &id, JournalSection::Transcript, "no-uow")
        .is_err());
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    store.begin_write_unit_of_work().unwrap();
    assert!(store
        .reserve_assistant_journal_order(
            &stamp,
            &id,
            JournalSection::Transcript,
            "historical-stamp"
        )
        .is_err());
    let input = transition("run", 2);
    store.commit_transition(&input).unwrap();
    let fresh = AssistantSourceCommitStamp::Transition {
        transition_id: input.transition_id,
    };
    assert_eq!(
        store
            .reserve_assistant_journal_order(
                &fresh,
                &id,
                JournalSection::Transcript,
                "journal-part-a"
            )
            .unwrap(),
        first
    );
    assert_eq!(
        store
            .reserve_assistant_journal_order(
                &fresh,
                &id,
                JournalSection::Transcript,
                "journal-part-b"
            )
            .unwrap()
            .position,
        JournalNumber(2)
    );
    store.rollback_write_unit_of_work().unwrap();
    store.begin_write_unit_of_work().unwrap();
    store.commit_transition(&transition("run", 2)).unwrap();
    assert_eq!(
        store
            .reserve_assistant_journal_order(
                &fresh,
                &id,
                JournalSection::Transcript,
                "journal-part-c"
            )
            .unwrap()
            .position,
        JournalNumber(2)
    );
    store.commit_write_unit_of_work().unwrap();
}

#[test]
fn indexed_attempt_successor_rejects_corrupt_scope_and_lineage() {
    use nola_domain::{ModelAttemptMutation, ModelAttemptRecord, ModelAttemptState};
    let mut store = NolaStore::open(":memory:").unwrap();
    let record = |id: &str, predecessor: Option<&str>| ModelAttemptRecord {
        id: id.into(),
        run_id: "run".into(),
        invocation_id: "invocation".into(),
        effect_id: "effect".into(),
        text_part_id: format!("{id}-text"),
        reasoning_part_id: format!("{id}-reasoning"),
        predecessor_id: predecessor.map(str::to_owned),
        state: ModelAttemptState::Superseded,
        provider: None,
        model: None,
        failure: None,
        usage: serde_json::json!({}),
        created_at_ms: 1,
        started_at_ms: None,
        completed_at_ms: Some(1),
        revision: 1,
    };
    let mut input = admission("run", "turn", 1);
    let parent = record("parent", None);
    let child = record("child", Some("parent"));
    input.model_attempt_mutations = vec![
        ModelAttemptMutation {
            expected_revision: None,
            record: parent,
        },
        ModelAttemptMutation {
            expected_revision: None,
            record: child.clone(),
        },
    ];
    store.begin_turn(&input).unwrap();
    assert_eq!(
        store
            .model_attempt_successor("run", "effect", "invocation", "parent")
            .unwrap(),
        Some(child.clone())
    );
    store
        .connection
        .execute(
            "UPDATE run_attempts SET invocation_id='foreign' WHERE id='child'",
            [],
        )
        .unwrap();
    assert!(store
        .model_attempt_successor("run", "effect", "invocation", "parent")
        .is_err());
    store
        .connection
        .execute(
            "UPDATE run_attempts SET invocation_id='invocation' WHERE id='child'",
            [],
        )
        .unwrap();
    let mut forged = child;
    forged.predecessor_id = Some("forged-parent".into());
    store
        .connection
        .execute(
            "UPDATE run_attempts SET record_json=?1 WHERE id='child'",
            [serde_json::to_string(&forged).unwrap()],
        )
        .unwrap();
    assert!(store
        .model_attempt_successor("run", "effect", "invocation", "parent")
        .is_err());
}


#[test]
fn internal_handoff_preserves_typed_historical_read_facts_but_not_write_authority() {
    use nola_domain::{
        ModelAttemptFailure, ModelAttemptMutation, ModelAttemptRecord, ModelAttemptState,
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("read-handoff.db");
    let mut store = NolaStore::open(&path).unwrap();
    let mut input = admission("old-run", "old-core-turn", 1);
    for (index, (kind, id)) in [("task", "old-task"), ("skill", "old-skill")].into_iter().enumerate() {
        input.initial_events.push(nola_store_port::run_creation::InitialCoreRunEvent {
            core_session_id: "different-core-session".into(), core_turn_id: Some("old-core-turn".into()),
            core_cursor: index as u64 + 1, core_revision: 1, event_kind: "assistant_activity_bound".into(),
            payload_json: serde_json::json!({"type":"assistant_activity_bound","turn_id":"old-core-turn",
                "assistant_message_id":"old-run-assistant","activity_kind":kind,"activity_id":id}).to_string(),
        });
        input.assistant_turn_bindings.push(AssistantTurnBinding::Activity {
            turn_id: "old-core-turn".into(), assistant_message_id: "old-run-assistant".into(),
            activity_kind: kind.into(), activity_id: id.into(),
        });
    }
    input.checkpoint_event_cursor = 2;
    input.checkpoint_json = serde_json::json!({
        "session_id":"different-core-session",
        "state":{"session_id":"different-core-session"},
        "revision":1,
        "last_event_cursor":{"sequence":2}
    }).to_string();
    input.model_attempt_mutations.push(ModelAttemptMutation {
        expected_revision: None,
        record: ModelAttemptRecord {
            id: "old-failed-attempt".into(),
            run_id: "old-run".into(),
            effect_id: "old-model-effect".into(),
            invocation_id: "old-model-invocation".into(),
            text_part_id: "old-run-part".into(),
            reasoning_part_id: "old-reasoning-part".into(),
            predecessor_id: None,
            state: ModelAttemptState::Failed,
            provider: None,
            model: None,
            failure: Some(ModelAttemptFailure {
                code: "fixture-failure".into(),
                message: "persisted failure".into(),
                retryable: true,
                http_status: None,
            }),
            usage: serde_json::json!({}),
            created_at_ms: 1,
            started_at_ms: Some(1),
            completed_at_ms: Some(1),
            revision: 1,
        },
    });
    store.begin_turn(&input).unwrap();
    let old_id = identity(&store, "old-run", "old-run-assistant");
    terminal_sources::settle_fixture(&mut store,"conversation","old-run","old-core-turn","old-run-user");
    let mut resume = internal(&store, "resumed-run", 3);
    resume.checkpoint_json = serde_json::json!({
        "session_id":"different-core-session", "state":{"session_id":"different-core-session",
            "conversation":{"projection":{"active_message_segments":{"old-core-turn":"old-run-assistant"},"activity_message_owners":{"task:old-task":"old-run-assistant","skill:old-skill":"old-run-assistant"},"revision":2,"order":[],"parts":{}},"context_window":{"config":{},"used_tokens":0,"usage_origin":"estimated"}},
            "task_projection":{"revision":1,"order":["old-task"],"tasks":{"old-task":{"id":"old-task","owner_session_id":"different-core-session","owner_turn_id":"old-core-turn"}}},
            "skill_runtime":{"revision":1,"catalog":{"revision":1},"runs":{"old-skill":{"run_id":"old-skill","owner":{"session_id":"different-core-session","parent_turn_id":"old-core-turn"}}}}
        },"last_event_cursor":{"sequence":0}
    }).to_string();
    let mut illegal_admission = resume.clone();
    let old_state:String = store.connection.query_row(
        "SELECT state FROM message_parts WHERE id='old-run-part'", [], |row|row.get(0)).unwrap();
    illegal_admission.initial_assistant_part_mutations.push(PartMutation::Metadata {
        part_id:"old-run-part".into(), state:old_state,
        content_json:Some("{\"forged\":true}".into()), updated_at_ms:3,
    });
    illegal_admission.assistant_turn_bindings.push(part_binding(
        "internal-core-turn", "old-run-assistant", "old-run-part"));
    let error = store.begin_internal_turn(&illegal_admission).unwrap_err();
    assert!(error.to_string().contains("Part mutation belongs to another accepted Run"), "{error}");
    assert_eq!(store.connection.query_row::<String,_,_>(
        "SELECT run_id FROM messages WHERE id='old-run-assistant'", [], |row|row.get(0)).unwrap(), "old-run");
    assert!(!store.connection.query_row::<bool,_,_>(
        "SELECT EXISTS(SELECT 1 FROM runs WHERE id='resumed-run')", [], |row|row.get(0)).unwrap(),
        "rejected admission must roll back Run creation and message handoff");
    store.begin_internal_turn(&resume).unwrap();
    let mut next = transition("resumed-run", 4);
    next.checkpoint = nola_store_port::run_effect::CheckpointWrite::Baseline { checkpoint_json: resume.checkpoint_json.clone() };
    next.part_message_owners
        .insert("new-only-part".into(), "old-run-assistant".into());
    next.assistant_turn_bindings.push(part_binding(
        "internal-core-turn",
        "old-run-assistant",
        "new-only-part",
    ));
    next.part_mutations.push(PartMutation::Upsert {
        part_id: "new-only-part".into(),
        kind: "text".into(),
        ordinal: 1,
        state: "completed".into(),
        content_text: "created only after handoff".into(),
        content_json: None,
        content_digest: None,
        created_at_ms: 3,
        updated_at_ms: 3,
    });
    store.commit_transition(&next).unwrap();
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    let json = store
        .load_local_presentation_inputs("conversation", "[\"old-run-assistant\"]", None)
        .unwrap()
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["conversation_id"], "conversation");
    assert_eq!(value["session_id"], "different-core-session");
    let facts: AssistantOwnershipFacts =
        serde_json::from_value(value["ownership"].clone()).unwrap();
    assert!(facts
        .turns
        .iter()
        .any(|turn| turn.turn_id == "old-core-turn"
            && turn.owner.run_id == "old-run"
            && turn.owner.assistant_message_id == "old-run-assistant"));
    assert!(facts.parts.iter().any(|part| part.part_id == "old-run-part"
        && part.owner.run_id == "old-run"
        && part.turn_id.as_deref() == Some("old-core-turn")));
    assert!(facts.parts.iter().all(|part| part.part_id != "old-run-part"
        || part.owner.run_id != "resumed-run"),
        "a handoff cannot assign the predecessor's Part to the current Run");
    assert!(facts
        .parts
        .iter()
        .any(|part| part.part_id == "new-only-part" && part.owner.run_id == "resumed-run"));
    assert!(facts
        .parts
        .iter()
        .all(|part| part.part_id != "new-only-part" || part.owner.run_id != "old-run"));
    assert_eq!(value["tasks"]["order"], serde_json::json!(["old-task"]));
    assert!(value["skill_runs"].get("old-skill").is_some());
    assert_eq!(value["model_attempts"][0]["id"], "old-failed-attempt");
    assert_eq!(value["model_attempts"][0]["run_id"], "old-run");
    // Corrupt isolated Attempt reference cannot borrow a Part created only by
    // the successor Run: no historical binding exists for that exact Part.
    store.connection.execute("UPDATE run_attempts SET record_json=json_set(record_json,'$.text_part_id','new-only-part') WHERE id='old-failed-attempt'", []).unwrap();
    let borrowed: serde_json::Value = serde_json::from_str(
        &store
            .load_local_presentation_inputs("conversation", "[\"old-run-assistant\"]", None)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        borrowed["model_attempts"][0]["text_part_id"],
        "new-only-part"
    );
    let borrowed_facts: AssistantOwnershipFacts =
        serde_json::from_value(borrowed["ownership"].clone()).unwrap();
    assert!(borrowed_facts
        .parts
        .iter()
        .all(|part| part.part_id != "new-only-part" || part.owner.run_id != "old-run"));
    assert_eq!(store.assistant_owner_for_activity("conversation", "old-run", "task", "old-task")
        .unwrap().unwrap().run_id, "old-run");
    assert!(store.assistant_owner_for_activity("conversation", "resumed-run", "task", "old-task")
        .unwrap().is_none());
    assert_eq!(store
        .assistant_owner_for_turn("conversation", "old-run", "old-core-turn")
        .unwrap().unwrap().run_id, "old-run");
    assert_eq!(store
        .assistant_owner_for_part("conversation", "old-run", "old-run-part")
        .unwrap().unwrap().owner.run_id, "old-run");
    assert!(store
        .assistant_owner_for_part("conversation", "resumed-run", "old-run-part")
        .is_err());
    assert!(store
        .assistant_owner_for_part("conversation", "old-run", "new-only-part")
        .is_err());
    let mut forged = transition("resumed-run", 5);
    forged.assistant_turn_bindings.push(part_binding(
        "internal-core-turn", "old-run-assistant", "old-run-part"));
    assert!(store.commit_transition(&forged).is_err(),
        "current membership cannot rebind a predecessor's Part to the successor");
    assert_eq!(store.connection.query_row::<i64,_,_>(
        "SELECT COUNT(*) FROM assistant_turn_part_bindings WHERE part_id='old-run-part'",
        [], |row|row.get(0)).unwrap(), 1);
    assert!(store.assistant_owner_for_part("conversation", "resumed-run", "old-run-part").is_err());
    let before:(String,String,i64,Option<String>) = store.connection.query_row(
        "SELECT content_text,state,updated_at_ms,content_json FROM message_parts WHERE id='old-run-part'",
        [], |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
    let mutations = [
        PartMutation::Metadata { part_id:"old-run-part".into(), state:before.1.clone(),
            content_json:Some("{\"forged\":true}".into()), updated_at_ms:5 },
        PartMutation::Upsert { part_id:"old-run-part".into(), kind:"text".into(),
            ordinal:1, state:before.1.clone(), content_text:"forged successor body".into(),
            content_json:None, content_digest:None, created_at_ms:1, updated_at_ms:5 },
    ];
    for mutation in mutations {
        let mut forged = transition("resumed-run", 5);
        forged.part_mutations.push(mutation);
        let error = store.commit_transition(&forged).unwrap_err();
        assert!(error.to_string().contains("Part mutation belongs to another accepted Run"), "{error}");
        let after:(String,String,i64,Option<String>) = store.connection.query_row(
            "SELECT content_text,state,updated_at_ms,content_json FROM message_parts WHERE id='old-run-part'",
            [], |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
        assert_eq!(after, before, "rejected successor mutation must preserve the old Part");
        let revision:i64 = store.connection.query_row(
            "SELECT revision FROM core_checkpoints WHERE conversation_id='conversation'",
            [], |row|row.get(0)).unwrap();
        assert_eq!(revision, 4, "rejected mutation must roll back its checkpoint");
    }
    store.begin_write_unit_of_work().unwrap();
    assert!(store
        .reserve_assistant_journal_order(
            &AssistantSourceCommitStamp::Admission {
                run_id: "old-run".into()
            },
            &old_id,
            JournalSection::Transcript,
            "historical-write"
        )
        .is_err());
    store.rollback_write_unit_of_work().unwrap();
    // A historical row without the exact current peer cannot authorize reads.
    store.connection.execute("DELETE FROM canonical_message_run_owners WHERE message_id='old-run-assistant' AND run_id='resumed-run'", []).unwrap();
    assert!(store.assistant_owner_for_turn("conversation", "old-run", "old-core-turn").is_err());
    assert!(store.assistant_owner_for_part("conversation", "old-run", "old-run-part").is_err());
    let value: serde_json::Value = serde_json::from_str(
        &store
            .load_local_presentation_inputs("conversation", "[\"old-run-assistant\"]", None)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert!(value["model_attempts"].as_array().unwrap().is_empty());
    assert!(value["tasks"]["order"].as_array().unwrap().is_empty());
    let facts: AssistantOwnershipFacts =
        serde_json::from_value(value["ownership"].clone()).unwrap();
    assert!(facts
        .turns
        .iter()
        .all(|turn| turn.owner.run_id != "old-run"));
    // Even a same-Message history pair cannot pull a foreign conversation Run.
    let mut foreign = admission("foreign-run", "foreign-turn", 1);
    foreign.conversation_id = "foreign-conversation".into();
    store.begin_turn(&foreign).unwrap();
    store.connection.execute("INSERT INTO canonical_message_run_owners VALUES('conversation','old-run-assistant','resumed-run')", []).unwrap();
    store.connection.execute("INSERT INTO canonical_message_run_owners VALUES('conversation','old-run-assistant','foreign-run')", []).unwrap();
    let value: serde_json::Value = serde_json::from_str(
        &store
            .load_local_presentation_inputs("conversation", "[\"old-run-assistant\"]", None)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let facts: AssistantOwnershipFacts =
        serde_json::from_value(value["ownership"].clone()).unwrap();
    assert!(facts
        .parts
        .iter()
        .all(|part| part.owner.run_id != "foreign-run"));
    assert!(facts
        .turns
        .iter()
        .all(|turn| turn.owner.run_id != "foreign-run"));
}

#[test]
fn segment_effect_and_activity_bindings_reject_missing_core_source_atomically() {
    let directory=tempfile::tempdir().unwrap();
    let mut store=NolaStore::open(directory.path().join("segment-binding.sqlite")).unwrap();
    store.begin_turn(&admission("run","turn",1)).unwrap();
    for binding in [
        AssistantTurnBinding::SegmentStarted {turn_id:"turn".into(),assistant_message_id:"forged".into()},
        AssistantTurnBinding::Effect {turn_id:"turn".into(),assistant_message_id:"run-assistant".into(),effect_id:"forged".into()},
        AssistantTurnBinding::Activity {turn_id:"turn".into(),assistant_message_id:"run-assistant".into(),activity_id:"forged".into(),activity_kind:"task".into()},
    ] {
        store.begin_write_unit_of_work().unwrap();
        let failure=crate::assistant_turn_ownership::apply(&store.connection,"conversation","run",Some("run-assistant"),false,&[binding]);
        assert!(failure.is_err(),"a display identity is not Core boundary evidence");
        store.rollback_write_unit_of_work().unwrap();
    }
    assert!(store.assistant_owner_for_effect("conversation","run","forged").unwrap().is_none());
    assert!(store.assistant_owner_for_activity("conversation","run","task","forged").unwrap().is_none());
    assert_eq!(store.assistant_owner_for_turn("conversation","run","turn").unwrap().unwrap().assistant_message_id,"run-assistant");
}

#[test]
fn execution_trace_history_requires_an_exact_admitted_run_and_preserves_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("trace-execution-ownership.db");
    let mut store = NolaStore::open(&path).unwrap();
    store.begin_turn(&admission("actual-run", "actual-run", 1)).unwrap();
    let trace = |trace_id: &str, run_id: &str, message: &str, turn: &str| AppendTraceJournalEventInput {
        event_id: format!("{trace_id}:1"), trace_id: trace_id.into(), run_id: run_id.into(),
        conversation_id: "conversation".into(), message_id: Some(message.into()), turn_id: turn.into(),
        sequence: 1, kind: "context.prepared".into(), status: "running".into(), visibility: "summary".into(),
        span_id: None, parent_span_id: None, emitted_at: "2026-10-26T00:00:00Z".into(),
        event_json: serde_json::json!({"trace_id":trace_id,"run_id":run_id}).to_string(),
        envelope_json: None, run_status: None, terminal_reason: None, is_terminal: false,
    };
    // Arbitrary identifiers deliberately make prefix-based classification fail.
    store.append_trace_journal_event(&trace("formal", "actual-run", "actual-run-assistant", "actual-run")).unwrap();
    store.append_trace_journal_event(&trace("unbound", "no-admission", "diagnostic-message", "diagnostic-turn")).unwrap();
    drop(store);
    let store = NolaStore::open(&path).unwrap();
    let history = store.trace_journal_runs_for_conversation("conversation").unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].run_id, "actual-run");
    assert_eq!(history[0].status, "running");
    assert!(history[0].finished_at.is_none(), "A read cannot invent completion");
    assert_eq!(store.trace_journal_events_after("unbound", 0).unwrap().len(), 1,
        "Diagnostic journal data remains explicitly readable after cold open");
    assert!(store.trace_journal_runs_for_conversation("other").unwrap().is_empty());
    // The existing exact replay path uses the same canonical owner proof.
    assert!(store.exact_run_trace_replay("conversation", "no-admission", "diagnostic-message", 0, None, 8).is_err());
    assert_eq!(store.exact_run_trace_replay("conversation", "actual-run", "actual-run-assistant", 0, None, 8).unwrap().records.len(), 1);
}

#[test]
fn shared_assistant_root_keeps_each_admitted_run_in_cold_trace_history() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shared-root-trace.db");
    let mut store = NolaStore::open(&path).unwrap();
    store.begin_turn(&admission("first", "first", 1)).unwrap();
    terminal_sources::settle_fixture(&mut store,"conversation","first","first","first-user");
    let mut continuation = internal(&store,"second",3);
    continuation.assistant_turn_bindings = vec![AssistantTurnBinding::Admission {
        turn_id:"second".into(),assistant_message_id:"first-assistant".into(),
    }];
    store.begin_internal_turn(&continuation).unwrap();
    for run in ["first", "second"] {
        store.append_trace_journal_event(&AppendTraceJournalEventInput {
            event_id:format!("trace-{run}:1"),trace_id:format!("trace-{run}"),run_id:run.into(),
            conversation_id:"conversation".into(),message_id:Some("first-assistant".into()),turn_id:run.into(),
            sequence:1,kind:"run.started".into(),status:"running".into(),visibility:"summary".into(),
            span_id:None,parent_span_id:None,emitted_at:"2026-10-27T00:00:00Z".into(),
            event_json:"{}".into(),envelope_json:None,run_status:None,terminal_reason:None,is_terminal:false,
        }).unwrap();
    }
    drop(store);
    let store = NolaStore::open(&path).unwrap();
    assert_eq!(store.trace_journal_runs_for_conversation("conversation").unwrap().len(),2);
    for run in ["first", "second"] {
        assert_eq!(store.exact_run_trace_replay("conversation",run,"first-assistant",0,None,8).unwrap().records.len(),1);
        assert!(store.exact_run_trace_replay("conversation",run,"second:assistant",0,None,8).is_err());
        assert!(store.exact_run_trace_replay("foreign",run,"first-assistant",0,None,8).is_err());
    }
}
