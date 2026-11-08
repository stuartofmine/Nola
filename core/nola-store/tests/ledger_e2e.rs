#[path = "support/remote_submission.rs"]
mod remote_submission_fixture;
use remote_submission_fixture::RemoteSubmissionFixture;
use nola_store::{
    AppendMessagesInput, ApplyConversationProjectionInput, BeginMaintenanceRunInput,
    AdmittedReplicaTurnInput, BeginTurnInput, CommitCheckpointProjectionInput,
    CommitContextCheckpointInput, CommitTransitionInput,
    ContextCheckpointInput, ContextComponentInput, CreateConversationBranchInput,
    CreateConversationInput, HostEventInput, MessageInput, MessagePartInput,
    MessageProjectionMutation, NolaStore, OutboxEffectInput, PartMutation, PermissionGrantScope,
    RunEventInput, SaveCheckpointInput, SkillSourceKind, StoreError,
    StoredPermissionDecision, TransitionMessageInput,
    UpsertPermissionGrantInput, UpsertSkillInput, STORE_SCHEMA_VERSION,
};
use sha2::Digest;

fn test_skill(id: &str, source: SkillSourceKind, version: &str) -> UpsertSkillInput {
    UpsertSkillInput {
        id: id.into(),
        source,
        version: version.into(),
        display_name: "Test Skill".into(),
        summary: "Exercises the canonical Skill ledger".into(),
        category: "testing".into(),
        instructions_ref: format!("nola-skill://{id}/{version}"),
        instructions_digest: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
            .into(),
        instructions_text: "hello".into(),
        required_tools_json: Some(r#"[{"provider":"builtin","tool_name":"read_file"}]"#.into()),
        default_mode: Some("inline".into()),
        enabled: true,
        created_at_ms: 10,
        updated_at_ms: 10,
    }
}

#[test]
fn committed_input_merges_attachment_into_existing_user_message() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("attachment-transition.db");
    let mut store = NolaStore::open(&database).unwrap();
    store.begin_turn(&begin_input()).unwrap();

    let mut transition = delta_transition(
        "attachment-transition",
        "attachment-host-event",
        1,
        1,
        1,
        2,
        "",
        "ok",
    );
    transition.ensured_messages = vec![TransitionMessageInput {
        id: "message-user-1".to_string(),
        run_id: Some("run-1".to_string()),
        role: "user".to_string(),
        status: "completed".to_string(),
        created_at_ms: 1_001,
        completed_at_ms: Some(1_001),
        initial_parts: vec![
            MessagePartInput {
                id: "part-user-1".to_string(),
                kind: "text".to_string(),
                ordinal: 1,
                state: "completed".to_string(),
                content_text: "Explain the architecture".to_string(),
                content_json: None,
                content_digest: None,
                created_at_ms: 1_001,
                updated_at_ms: 1_001,
            },
            MessagePartInput {
                id: "message-user-1:attachment:1".to_string(),
                kind: "attachment".to_string(),
                ordinal: 2,
                state: "completed".to_string(),
                content_text: "diagram.png".to_string(),
                content_json: Some(
                    r#"{"type":"artifact","id":"message-user-1:attachment:1","ownership":{"message_id":"message-user-1","turn_id":"run-1"},"artifact":{"uri":"file:///tmp/diagram.png","name":"diagram.png","mime_type":"image/png","operation":"attach"}}"#.to_string(),
                ),
                content_digest: None,
                created_at_ms: 1_001,
                updated_at_ms: 1_001,
            },
        ],
    }];

    assert!(store.commit_transition(&transition).unwrap().applied);
    let conversation = store.load_conversation("conversation-1").unwrap().unwrap();
    let user = conversation
        .messages
        .iter()
        .find(|message| message.id == "message-user-1")
        .unwrap();
    assert_eq!(user.parts.len(), 2);
    assert_eq!(user.parts[1].kind, "attachment");
    assert_eq!(user.parts[1].content_text, "diagram.png");
}

#[test]
fn maintenance_run_commits_without_manufacturing_conversation_messages() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("maintenance-run.db");
    let mut store = NolaStore::open(&database).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "maintenance-conversation".into(),
            title: "Maintenance ownership".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    let mut attempt = nola_domain::ModelAttemptRecord {
        id: "maintenance-attempt".into(),
        run_id: "maintenance-run-1".into(),
        invocation_id: "invocation".into(),
        effect_id: "maintenance-effect-1".into(),
        text_part_id: "maintenance-text-1".into(),
        reasoning_part_id: "maintenance-reasoning-1".into(),
        predecessor_id: None,
        state: nola_domain::ModelAttemptState::Prepared,
        provider: None,
        model: None,
        failure: None,
        usage: serde_json::json!({}),
        created_at_ms: 2,
        started_at_ms: None,
        completed_at_ms: None,
        revision: 1,
    };
    nola_store_port::run_creation::RunCreationRepository::begin_maintenance_run(&mut store, &BeginMaintenanceRunInput {
        initial_events: Vec::new(),
        memory_usage_entries: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            model_attempt_mutations: vec![nola_domain::ModelAttemptMutation {expected_revision:None,record:attempt.clone()}],
            conversation_id: "maintenance-conversation".into(),
            run_id: "maintenance-run-1".into(),
            context_checkpoint_id: None,
            checkpoint_revision: 1,
            checkpoint_event_cursor: 1,
            checkpoint_json: r#"{"revision":1}"#.into(),
            invocation_config_json: "{}".into(),
            invocation_config_digest: "maintenance-config-digest".into(),
            outbox_effects: vec![OutboxEffectInput {
            delegated_invocation_id: None,
            model_attempt_id: Some(attempt.id.clone()),
                effect_id: "maintenance-effect-1".into(),
                request_id: "maintenance-request-1".into(),
                effect_kind: "model_stream".into(),
                write: outbox_fixture::model("maintenance-effect-1", "maintenance-request-1", "maintenance-text-1", "maintenance-reasoning-1"),
            }],
            created_at_ms: 2,
        })
        .unwrap();

    let active = store
        .active_run("maintenance-conversation")
        .unwrap()
        .unwrap();
    assert_eq!(active.kind, "maintenance");
    assert!(active.assistant_message_id.is_none());
    assert!(active.assistant_part_id.is_none());
    let owner = store
        .model_request_parts("maintenance-request-1")
        .unwrap()
        .unwrap();
    assert_eq!(owner.run_id, "maintenance-run-1");
    assert_eq!(owner.text_part_id, "maintenance-text-1");
    assert_eq!(owner.reasoning_part_id, "maintenance-reasoning-1");
    assert!(store
        .load_conversation("maintenance-conversation")
        .unwrap()
        .unwrap()
        .messages
        .is_empty());

    attempt.state = nola_domain::ModelAttemptState::Succeeded;
    attempt.completed_at_ms = Some(3);
    attempt.revision = 2;
    store
        .commit_transition(&CommitTransitionInput {
            memory_usage_entries: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            effect_delivery_attempt_mutations: vec![],
            model_attempt_mutations: vec![nola_domain::ModelAttemptMutation {
                expected_revision: Some(1),
                record: attempt,
            }],
            transition_id: "maintenance-transition-1".into(),
            run_id: "maintenance-run-1".into(),
            expected_checkpoint_revision: 1,
            checkpoint_revision: 2,
            checkpoint_event_cursor: 2,
            checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline { checkpoint_json: r#"{"revision":2}"#.into() },
            submission_reduction_json: None, host_event: None,
            events: Vec::new(),
            ensured_messages: Vec::new(),
            assistant_turn_bindings: vec![],
            part_message_owners: std::collections::BTreeMap::new(),
            part_mutations: Vec::new(),
            outbox_effects: Vec::new(),
            acknowledged_effect_ids: vec!["maintenance-effect-1".into()],
            // A successful model attempt alone is not maintenance completion.
            // Application compaction lifecycle coverage owns Run settlement.
            run_status: None,
            assistant_status: None,
            assistant_status_message_id: None,
            committed_at_ms: 3,
        })
        .unwrap();

    assert!(store
        .active_run("maintenance-conversation")
        .unwrap()
        .is_some());
    let invocation = store
        .load_run_invocation_config("maintenance-run-1")
        .unwrap()
        .expect("maintenance Run retains its admitted invocation for recovery");
    assert_eq!(invocation.invocation_config_json, "{}");
    assert_eq!(invocation.invocation_config_digest, "maintenance-config-digest");
    assert!(store
        .load_conversation("maintenance-conversation")
        .unwrap()
        .unwrap()
        .messages
        .is_empty());
}

#[test]
fn skill_versions_and_availability_survive_real_sqlite_reopen() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("skill-ledger.db");
    {
        let mut store = NolaStore::open(&database).unwrap();
        store
            .upsert_skill(&test_skill("builtin-doc", SkillSourceKind::BuiltIn, "1"))
            .unwrap();
        store
            .upsert_skill(&test_skill("user-review", SkillSourceKind::User, "1"))
            .unwrap();
        assert!(store.set_skill_enabled("builtin-doc", false, 20).unwrap());
        assert!(store.delete_user_skill("user-review", 21).unwrap());
        assert!(store.delete_user_skill("builtin-doc", 22).is_err());

        let mut changed_same_version = test_skill("builtin-doc", SkillSourceKind::BuiltIn, "1");
        changed_same_version.instructions_text = "changed".into();
        changed_same_version.instructions_digest =
            "d67e2e944994496c8d5fd1651209efb8c0f391d28c55e1785f4f2f6b16adbc6f".into();
        assert!(store.upsert_skill(&changed_same_version).is_err());
    }

    let mut reopened = NolaStore::open(&database).unwrap();
    let visible = reopened.list_skills(false).unwrap();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, "builtin-doc");
    assert!(!visible[0].enabled);
    let all = reopened.list_skills(true).unwrap();
    assert_eq!(all.len(), 2);
    assert!(all
        .iter()
        .find(|skill| skill.id == "user-review")
        .unwrap()
        .deleted_at_ms
        .is_some());

    let mut next = test_skill("builtin-doc", SkillSourceKind::BuiltIn, "2");
    next.updated_at_ms = 30;
    let updated = reopened.upsert_skill(&next).unwrap();
    assert_eq!(updated.version, "2");
    assert!(!updated.enabled);
}

#[test]
fn permission_grants_are_scoped_revocable_and_survive_reopen() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("permission-ledger.db");
    {
        let mut store = NolaStore::open(&database).unwrap();
        store
            .admit_remote_fixture(&AdmittedReplicaTurnInput {
                user_part_kind: "text".into(),
                source_host_id: "host".into(),
                conversation_id: "conversation-a".into(),
                conversation_title: "A".into(),
                user_message_id: "user-a".into(),
                user_part_id: "user-a:body".into(),
                user_text: "Edit the file".into(),
                user_content_json: None,
                assistant_message_id: "assistant-a".into(),
                assistant_part_id: "assistant-a:body".into(),
                run_id: "run-a".into(),
                context_checkpoint_id: None,
                created_at_ms: 1,
            })
            .unwrap();
        for input in [
            UpsertPermissionGrantInput {
                id: "global-read".into(),
                scope: PermissionGrantScope::UserGlobal,
                conversation_id: None,
                host_id: None,
                capability_pattern: "read_file".into(),
                resource_pattern: "*".into(),
                decision: StoredPermissionDecision::Allow,
                created_at_ms: 2,
                updated_at_ms: 2,
            },
            UpsertPermissionGrantInput {
                id: "host-shell".into(),
                scope: PermissionGrantScope::Host,
                conversation_id: None,
                host_id: Some("mac-local".into()),
                capability_pattern: "bash".into(),
                resource_pattern: r#"{\"command\":\"cargo *\"}"#.into(),
                decision: StoredPermissionDecision::Allow,
                created_at_ms: 3,
                updated_at_ms: 3,
            },
            UpsertPermissionGrantInput {
                id: "conversation-edit".into(),
                scope: PermissionGrantScope::Conversation,
                conversation_id: Some("conversation-a".into()),
                host_id: None,
                capability_pattern: "write_file".into(),
                resource_pattern: r#"{\"path\":\"/workspace/*\"}"#.into(),
                decision: StoredPermissionDecision::Allow,
                created_at_ms: 4,
                updated_at_ms: 4,
            },
        ] {
            store.upsert_permission_grant(&input).unwrap();
        }
        let other_host = store
            .effective_permission_grants(Some("conversation-a"), Some("linux-host"))
            .unwrap();
        assert_eq!(
            other_host
                .iter()
                .map(|grant| grant.id.as_str())
                .collect::<Vec<_>>(),
            vec!["global-read", "conversation-edit"]
        );
    }
    let mut reopened = NolaStore::open(&database).unwrap();
    let effective = reopened
        .effective_permission_grants(Some("conversation-a"), Some("mac-local"))
        .unwrap();
    assert_eq!(
        effective
            .iter()
            .map(|grant| grant.id.as_str())
            .collect::<Vec<_>>(),
        vec!["global-read", "host-shell", "conversation-edit"]
    );
    assert!(reopened
        .revoke_permission_grant("conversation-edit", 5)
        .unwrap());
    assert!(!reopened
        .revoke_permission_grant("conversation-edit", 6)
        .unwrap());
    let effective = reopened
        .effective_permission_grants(Some("conversation-a"), Some("mac-local"))
        .unwrap();
    assert_eq!(
        effective
            .iter()
            .map(|grant| grant.id.as_str())
            .collect::<Vec<_>>(),
        vec!["global-read", "host-shell"]
    );
}

use tempfile::tempdir;

#[test]
fn non_current_database_is_rejected_without_migration_or_mutation() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("unsupported.db");
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE old_conversation(id TEXT PRIMARY KEY);\n\
             INSERT INTO old_conversation(id) VALUES('preserve-unmodified');\n\
             PRAGMA user_version = 99;",
        )
        .unwrap();
    drop(connection);

    let error = NolaStore::open(&database).expect_err("old schemas must not be migrated");
    assert!(error
        .to_string()
        .contains("No automatic reset is performed."));

    let connection = rusqlite::Connection::open(&database).unwrap();
    let row: String = connection
        .query_row("SELECT id FROM old_conversation", [], |row| row.get(0))
        .unwrap();
    assert_eq!(row, "preserve-unmodified");
    let canonical_tables: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='conversations'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(canonical_tables, 0);
}

fn begin_input() -> BeginTurnInput {
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
        conversation_id: "conversation-1".to_string(),
        conversation_title: "Architecture".to_string(),
        user_message_id: "message-user-1".to_string(),
        user_part_id: "part-user-1".to_string(),
        user_text: "Explain the architecture".to_string(),
        user_content_json: None,
        additional_user_parts: Vec::new(),
        assistant_message_id: "message-assistant-1".to_string(),
        assistant_part_id: "part-assistant-1".to_string(),
        assistant_turn_bindings: vec![],
        run_id: "run-1".to_string(),
        context_checkpoint_id: None,
        checkpoint_revision: 1,
        checkpoint_event_cursor: 0,
        checkpoint_json: r#"{"revision":1,"context":"initial"}"#.to_string(),
        invocation_config_json: "{}".to_string(),
        invocation_config_digest: "conversation-config-digest".to_string(),
        outbox_effects: vec![OutboxEffectInput {
            delegated_invocation_id: None,
            model_attempt_id: None,
            effect_id: "effect-model-1".to_string(),
            request_id: "request-model-1".to_string(),
            effect_kind: "model_request".to_string(),
            write: outbox_fixture::model("effect-model-1", "request-model-1", "text", "reasoning"),
        }],
        created_at_ms: 1_000,
    }
}

#[test]
fn transition_rejects_wrong_run_message_and_status_owners_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("ownership.db")).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "conversation-1".into(),
            title: "Ownership".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    store
        .append_messages(&AppendMessagesInput {
            conversation_id: "conversation-1".into(),
            expected_message_count: 0,
            messages: vec![MessageInput {
                id: "historical-assistant".into(),
                run_id: None,
                role: "assistant".into(),
                status: "completed".into(),
                ordinal: 1,
                created_at_ms: 1001,
                completed_at_ms: Some(1001),
                parts: vec![],
            }],
            updated_at_ms: 1001,
        })
        .unwrap();
    let mut admission = begin_input();
    admission.expected_configuration_revision = Some(1);
    admission.expected_parent_message_id = Some("historical-assistant".into());
    store.begin_turn(&admission).unwrap();
    let before = store.load_conversation("conversation-1").unwrap().unwrap();
    for target in ["message-user-1", "historical-assistant", "absent"] {
        let mut input = delta_transition(
            "wrong-status",
            "wrong-status-event",
            1,
            1,
            1,
            2,
            "",
            "must roll back",
        );
        input.assistant_status_message_id = Some(target.into());
        assert!(store.commit_transition(&input).is_err(), "target {target}");
        let after = store.load_conversation("conversation-1").unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(&after).unwrap(),
            serde_json::to_value(&before).unwrap()
        );
    }
    let mut wrong_part =
        delta_transition("wrong-part", "wrong-part-event", 1, 1, 1, 2, "", "blocked");
    wrong_part
        .part_message_owners
        .insert("part-assistant-1".into(), "historical-assistant".into());
    assert!(store.commit_transition(&wrong_part).is_err());
    let mut wrong_declaration = delta_transition(
        "wrong-declaration",
        "wrong-declaration-event",
        1,
        1,
        1,
        2,
        "",
        "blocked",
    );
    wrong_declaration
        .ensured_messages
        .push(TransitionMessageInput {
            id: "historical-assistant".into(),
            run_id: None,
            role: "assistant".into(),
            status: "streaming".into(),
            created_at_ms: 1001,
            completed_at_ms: None,
            initial_parts: vec![],
        });
    assert!(store.commit_transition(&wrong_declaration).is_err());
    // Rejections must not consume the host event/sequence or advance the checkpoint.
    assert!(
        store
            .commit_transition(&delta_transition(
                "valid-after-rejection",
                "wrong-status-event",
                1,
                1,
                1,
                2,
                "",
                "valid"
            ))
            .unwrap()
            .applied
    );
}

#[test]
fn transition_accepts_successor_assistant_owned_by_same_run() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("successor.db")).unwrap();
    store.begin_turn(&begin_input()).unwrap();
    let mut input = delta_transition(
        "successor",
        "successor-event",
        1,
        1,
        1,
        2,
        "",
        "successor body",
    );
    input.ensured_messages.push(TransitionMessageInput {
        id: "successor-assistant".into(),
        run_id: Some("run-1".into()),
        role: "assistant".into(),
        status: "streaming".into(),
        created_at_ms: 1001,
        completed_at_ms: None,
        initial_parts: vec![MessagePartInput {
            id: "successor-part".into(),
            kind: "text".into(),
            ordinal: 1,
            state: "open".into(),
            content_text: String::new(),
            content_json: None,
            content_digest: None,
            created_at_ms: 1001,
            updated_at_ms: 1001,
        }],
    });
    if let PartMutation::AppendText { part_id, .. } = &mut input.part_mutations[0] {
        *part_id = "successor-part".into();
    }
    input
        .part_message_owners
        .insert("successor-part".into(), "successor-assistant".into());
    input.assistant_status_message_id = Some("successor-assistant".into());
    assert!(store.commit_transition(&input).unwrap().applied);
    let conversation = store.load_conversation("conversation-1").unwrap().unwrap();
    let successor = conversation
        .messages
        .iter()
        .find(|m| m.id == "successor-assistant")
        .unwrap();
    assert_eq!(successor.parts[0].content_text, "successor body");
}

fn delta_transition(
    transition_id: &str,
    host_event_id: &str,
    host_sequence: i64,
    event_sequence: i64,
    expected_revision: i64,
    revision: i64,
    expected_text: &str,
    text: &str,
) -> CommitTransitionInput {
    CommitTransitionInput {
        memory_usage_entries: vec![],
        delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![],
        effect_delivery_attempt_mutations: vec![],
        model_attempt_mutations: vec![],
        transition_id: transition_id.to_string(),
        run_id: "run-1".to_string(),
        expected_checkpoint_revision: expected_revision,
        checkpoint_revision: revision,
        checkpoint_event_cursor: event_sequence,
        checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline { checkpoint_json: format!(r#"{{"revision":{revision}}}"#) },
        submission_reduction_json: None, host_event: Some(HostEventInput {
            event_id: host_event_id.to_string(),
            effect_id: "effect-model-1".to_string(),
            request_id: "request-model-1".to_string(),
            sequence: host_sequence,
            payload_json: format!(r#"{{"delta":{}}}"#, serde_json::to_string(text).unwrap()),
            received_at_ms: 1_000 + revision,
            reduction_json: None,
        }),
        events: vec![RunEventInput {
            event_id: format!("domain-event-{event_sequence}"),
            sequence: event_sequence,
            event_kind: "assistant_text_delta".to_string(),
            payload_json: format!(r#"{{"text":{}}}"#, serde_json::to_string(text).unwrap()),
            created_at_ms: 1_000 + revision,
        }],
        ensured_messages: Vec::new(),
        assistant_turn_bindings: vec![],
        part_message_owners: std::collections::BTreeMap::new(),
        part_mutations: vec![PartMutation::AppendText {
            part_id: "part-assistant-1".to_string(),
            expected_content_digest: format!(
                "{:x}",
                sha2::Sha256::digest(expected_text.as_bytes())
            ),
            text: text.to_string(),
            updated_at_ms: 1_000 + revision,
        }],
        outbox_effects: Vec::new(),
        acknowledged_effect_ids: Vec::new(),
        run_status: Some("running".to_string()),
        assistant_status: Some("streaming".to_string()),
        assistant_status_message_id: None,
        committed_at_ms: 1_000 + revision,
    }
}

#[test]
fn real_sqlite_reopen_replay_and_compaction_preserve_complete_messages() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("nola.db");

    {
        let mut store = NolaStore::open(&database).unwrap();
        assert_eq!(store.schema_version().unwrap(), STORE_SCHEMA_VERSION);
        let begun = store.begin_turn(&begin_input()).unwrap();
        assert_eq!(begun.user_ordinal, 1);
        assert_eq!(begun.assistant_ordinal, 2);
        assert_eq!(store.pending_effects("run-1").unwrap().len(), 1);

        let first = delta_transition("transition-1", "host-event-1", 1, 1, 1, 2, "", "Hello");
        assert!(store.commit_transition(&first).unwrap().applied);
        assert!(!store.commit_transition(&first).unwrap().applied);
    }

    {
        let mut store = NolaStore::open(&database).unwrap();
        let second = delta_transition(
            "transition-2",
            "host-event-2",
            2,
            2,
            2,
            3,
            "Hello",
            ", world",
        );
        assert!(store.commit_transition(&second).unwrap().applied);

        let duplicate_delivery = CommitTransitionInput {
            memory_usage_entries: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            model_attempt_mutations: vec![],
            transition_id: "transition-replayed-under-new-id".to_string(),
            expected_checkpoint_revision: 3,
            checkpoint_revision: 4,
            ..second.clone()
        };
        let duplicate = store.commit_transition(&duplicate_delivery).unwrap();
        assert!(!duplicate.applied);

        let before_compaction = store.load_conversation("conversation-1").unwrap().unwrap();
        let before_messages = before_compaction.messages.clone();
        store
            .commit_context_checkpoint(&CommitContextCheckpointInput {
                checkpoint: ContextCheckpointInput {
                    id: "context-compacted-1".to_string(),
                    conversation_id: "conversation-1".to_string(),
                    run_id: Some("run-1".to_string()),
                    model_profile_id: "model-default".to_string(),
                    policy_version: "v1".to_string(),
                    covered_through_message_id: Some("assistant-1".to_string()),
                    covered_through_sequence: Some(2),
                    summary_part_id: None,
                    retained_tail_start_message_id: None,
                    estimated_tokens: 12,
                    measured_tokens: Some(11),
                    context_window_tokens: 65_536,
                    prompt_limit_tokens: 61_440,
                    payload_digest: "digest-context-1".to_string(),
                    created_at_ms: 2_000,
                },
                components: vec![ContextComponentInput {
                    id: "context-component-1".to_string(),
                    checkpoint_id: "context-compacted-1".to_string(),
                    kind: "conversation_summary".to_string(),
                    source_id: None,
                    source_uri: None,
                    title: None,
                    mime_type: Some("text/plain".to_string()),
                    authorization: "not_required".to_string(),
                    observed_at_ms: Some(2_000),
                    maximum_age_minutes: None,
                    freshness: "fresh".to_string(),
                    content_reference_json: r#"{"type":"inline_preview","text":"User requested an architecture explanation.","truncated":false}"#.to_string(),
                    content_digest: "digest-context-1".to_string(),
                    estimated_tokens: 12,
                    provenance_json: r#"{"provider_id":"agent-core","collection_method":"compaction","source_revision":null,"disclosed_to_model":true}"#.to_string(),
                }],
            })
            .unwrap();
        assert_eq!(
            store.count_context_checkpoints("conversation-1").unwrap(),
            1
        );
        let checkpoint = store
            .load_context_checkpoint("context-compacted-1")
            .unwrap()
            .unwrap();
        assert_eq!(checkpoint.checkpoint.id, "context-compacted-1");
        assert_eq!(checkpoint.components.len(), 1);
        let configuration_revision = store.load_conversation("conversation-1").unwrap().unwrap().configuration_revision;
        assert!(store
            .patch_conversation_configuration(&nola_store_port::conversation_profile::PatchConversationConfigurationInput {
                conversation_id: "conversation-1".to_string(),
                expected_revision: configuration_revision,
                patch_json: r#"{"memoryEnabledOverride":true}"#.parse().unwrap(),
                updated_at_ms: 2_001,
            })
            .unwrap().is_committed());
        let after_compaction = store.load_conversation("conversation-1").unwrap().unwrap();
        assert_eq!(after_compaction.messages, before_messages);
        assert_eq!(
            after_compaction.settings_json.as_deref(),
            Some(r#"{"memoryEnabledOverride":true}"#)
        );
        assert_eq!(
            after_compaction.messages[1].parts[0].content_text,
            "Hello, world"
        );
        assert_eq!(
            store
                .load_checkpoint("conversation-1")
                .unwrap()
                .unwrap()
                .revision,
            3
        );
        assert!(store.acknowledge_effect("effect-model-1", 2_100).unwrap());
        assert!(store.pending_effects("run-1").unwrap().is_empty());
    }
}


#[test]
fn sequence_gap_rolls_back_the_entire_transition() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("nola.db");
    let mut store = NolaStore::open(&database).unwrap();
    store.begin_turn(&begin_input()).unwrap();

    let gap = delta_transition(
        "transition-gap",
        "host-event-gap",
        1,
        2,
        1,
        2,
        "",
        "lost prefix",
    );
    let error = store.commit_transition(&gap).unwrap_err();
    assert!(matches!(error, StoreError::Conflict(_)));

    let conversation = store.load_conversation("conversation-1").unwrap().unwrap();
    assert_eq!(conversation.messages[1].parts[0].content_text, "");
    assert_eq!(
        store
            .load_checkpoint("conversation-1")
            .unwrap()
            .unwrap()
            .revision,
        1
    );
}

#[test]
fn append_prefix_mismatch_rolls_back_without_touching_the_stream() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("nola.db");
    let mut store = NolaStore::open(&database).unwrap();
    store.begin_turn(&begin_input()).unwrap();
    store
        .commit_transition(&delta_transition(
            "transition-prefix-1",
            "host-event-prefix-1",
            1,
            1,
            1,
            2,
            "",
            "Hello",
        ))
        .unwrap();

    let error = store
        .commit_transition(&delta_transition(
            "transition-prefix-2",
            "host-event-prefix-2",
            2,
            2,
            2,
            3,
            "different prefix",
            ", world",
        ))
        .unwrap_err();
    assert!(matches!(error, StoreError::Conflict(_)));
    drop(store);

    let store = NolaStore::open(&database).unwrap();
    let conversation = store.load_conversation("conversation-1").unwrap().unwrap();
    assert_eq!(conversation.messages[1].parts[0].content_text, "Hello");
    assert_eq!(conversation.messages[1].parts[0].state, "open");
    assert_eq!(
        store
            .load_checkpoint("conversation-1")
            .unwrap()
            .unwrap()
            .revision,
        2
    );
    assert_eq!(store.pending_effects("run-1").unwrap().len(), 1);
}

#[test]
fn conversation_creation_is_idempotent_and_messages_use_append_contract() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("nola.db");
    let mut store = NolaStore::open(&database).unwrap();
    let creation = CreateConversationInput {
        id: "created-conversation".to_string(),
        title: "Created".to_string(),
        revision: 7,
        settings_json: Some(r#"{"model":"qwen","memory":true}"#.to_string()),
        created_at_ms: 90,
        updated_at_ms: 120,
    };

    let first = store.create_conversation(&creation).unwrap();
    assert!(first.created);
    let message = MessageInput {
        id: "created-assistant".to_string(),
        run_id: None,
        role: "assistant".to_string(),
        status: "completed".to_string(),
        ordinal: 1,
        created_at_ms: 100,
        completed_at_ms: Some(120),
        parts: vec![MessagePartInput {
            id: "created-part".to_string(),
            kind: "presentation".to_string(),
            ordinal: 1,
            state: "completed".to_string(),
            content_text: "Rendered answer".to_string(),
            content_json: Some(
                r#"{"tool_calls":[{"name":"read_file","state":"completed"}]}"#.to_string(),
            ),
            content_digest: Some("digest-created".to_string()),
            created_at_ms: 100,
            updated_at_ms: 120,
        }],
    };
    store
        .append_messages(&AppendMessagesInput {
            conversation_id: creation.id.clone(),
            expected_message_count: 0,
            messages: vec![message.clone()],
            updated_at_ms: 120,
        })
        .unwrap();
    let replay = store
        .create_conversation(&CreateConversationInput {
            title: "Must not overwrite".to_string(),
            ..creation.clone()
        })
        .unwrap();
    assert!(!replay.created);

    let loaded = store
        .load_conversation("created-conversation")
        .unwrap()
        .unwrap();
    assert_eq!(loaded.title, "Created");
    assert_eq!(loaded.settings_json.as_deref().map(|value| serde_json::from_str::<serde_json::Value>(value).unwrap()),
        creation.settings_json.as_deref().map(|value| serde_json::from_str::<serde_json::Value>(value).unwrap()));
    assert_eq!(
        loaded.messages[0].parts[0].content_json,
        message.parts[0].content_json
    );
    assert_eq!(store.list_conversations().unwrap()[0].message_count, 1);
    assert!(store
        .rename_conversation("created-conversation", "Renamed", None, 130)
        .unwrap());
    assert_eq!(
        store
            .load_conversation("created-conversation")
            .unwrap()
            .unwrap()
            .title,
        "Renamed"
    );
    assert!(store.delete_conversation("created-conversation").unwrap());
    assert!(store
        .load_conversation("created-conversation")
        .unwrap()
        .is_none());
}

#[test]
fn projection_edit_is_atomic_idempotent_and_preserves_ledger_after_reopen() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("nola.db");
    let message = |id: &str, ordinal: i64, text: &str| MessageInput {
        id: id.to_string(),
        run_id: None,
        role: if ordinal % 2 == 1 {
            "user"
        } else {
            "assistant"
        }
        .to_string(),
        status: "completed".to_string(),
        ordinal,
        created_at_ms: ordinal,
        completed_at_ms: Some(ordinal),
        parts: vec![MessagePartInput {
            id: format!("{id}:body"),
            kind: "text".to_string(),
            ordinal: 1,
            state: "completed".to_string(),
            content_text: text.to_string(),
            content_json: None,
            content_digest: None,
            created_at_ms: ordinal,
            updated_at_ms: ordinal,
        }],
    };
    {
        let mut store = NolaStore::open(&database).unwrap();
        store
            .create_conversation(&CreateConversationInput {
                id: "projection-conversation".to_string(),
                title: "Projection".to_string(),
                revision: 1,
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 3,
            })
            .unwrap();
        store
            .append_messages(&AppendMessagesInput {
                conversation_id: "projection-conversation".to_string(),
                expected_message_count: 0,
                messages: vec![
                    message("user-1", 1, "First"),
                    message("assistant-1", 2, "Old answer"),
                    message("user-2", 3, "Old follow-up"),
                ],
                updated_at_ms: 3,
            })
            .unwrap();
        let input = ApplyConversationProjectionInput {
            operation_id: "edit-operation-1".to_string(),
            conversation_id: "projection-conversation".to_string(),
            expected_revision: 2,
            operation_kind: "edit".to_string(),
            parent_message_id: Some("user-1".to_string()),
            mutations: vec![
                MessageProjectionMutation {
                    event_id: "hide-assistant-1".to_string(),
                    message_id: "assistant-1".to_string(),
                    visible: false,
                    reason: "superseded_by_edit".to_string(),
                },
                MessageProjectionMutation {
                    event_id: "hide-user-2".to_string(),
                    message_id: "user-2".to_string(),
                    visible: false,
                    reason: "after_projection_head".to_string(),
                },
            ],
            appended_messages: vec![message("user-2-replacement", 4, "Revised follow-up")],
            created_at_ms: 10,
        };
        let first = store.apply_conversation_projection(&input).unwrap();
        assert!(first.applied);
        assert_eq!(first.conversation_revision, 3);
        assert_eq!(first.ledger_message_count, 4);
        let replay = store.apply_conversation_projection(&input).unwrap();
        assert!(!replay.applied);
        assert_eq!(replay.ledger_message_count, 4);
    }

    let reopened = NolaStore::open(&database).unwrap();
    assert_eq!(reopened.schema_version().unwrap(), STORE_SCHEMA_VERSION);
    let projected = reopened
        .load_conversation("projection-conversation")
        .unwrap()
        .unwrap();
    assert_eq!(
        projected
            .messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>(),
        vec!["user-1", "user-2-replacement"]
    );
    assert_eq!(reopened.list_conversations().unwrap()[0].message_count, 4);
}

#[test]
fn checkpoint_and_projection_commit_atomically_and_preserve_hidden_history_after_reopen() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("atomic-projection.db");
    let message = |id: &str, ordinal: i64, role: &str| MessageInput {
        id: id.to_string(),
        run_id: None,
        role: role.to_string(),
        status: "completed".to_string(),
        ordinal,
        created_at_ms: ordinal,
        completed_at_ms: Some(ordinal),
        parts: vec![MessagePartInput {
            id: format!("{id}:body"),
            kind: "text".to_string(),
            ordinal: 1,
            state: "completed".to_string(),
            content_text: id.to_string(),
            content_json: None,
            content_digest: None,
            created_at_ms: ordinal,
            updated_at_ms: ordinal,
        }],
    };
    let input = CommitCheckpointProjectionInput {
        checkpoint: SaveCheckpointInput {
            conversation_id: "atomic-projection".to_string(),
            expected_revision: 1,
            revision: 2,
            event_cursor: 9,
            checkpoint_json: r#"{"revision":2,"event_cursor":9}"#.to_string(),
            updated_at_ms: 20,
        },
        projection: ApplyConversationProjectionInput {
            operation_id: "atomic-revert-1".to_string(),
            conversation_id: "atomic-projection".to_string(),
            expected_revision: 2,
            operation_kind: "revert".to_string(),
            parent_message_id: Some("user-1".to_string()),
            mutations: vec![
                MessageProjectionMutation {
                    event_id: "atomic-hide-assistant".to_string(),
                    message_id: "assistant-1".to_string(),
                    visible: false,
                    reason: "reverted".to_string(),
                },
                MessageProjectionMutation {
                    event_id: "atomic-hide-user".to_string(),
                    message_id: "user-2".to_string(),
                    visible: false,
                    reason: "reverted".to_string(),
                },
            ],
            appended_messages: Vec::new(),
            created_at_ms: 20,
        },
    };

    {
        let mut store = NolaStore::open(&database).unwrap();
        store
            .create_conversation(&CreateConversationInput {
                id: "atomic-projection".to_string(),
                title: "Atomic projection".to_string(),
                revision: 1,
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
        store
            .append_messages(&AppendMessagesInput {
                conversation_id: "atomic-projection".to_string(),
                expected_message_count: 0,
                messages: vec![
                    message("user-1", 1, "user"),
                    message("assistant-1", 2, "assistant"),
                    message("user-2", 3, "user"),
                ],
                updated_at_ms: 3,
            })
            .unwrap();
        store
            .save_checkpoint(&SaveCheckpointInput {
                conversation_id: "atomic-projection".to_string(),
                expected_revision: 0,
                revision: 1,
                event_cursor: 3,
                checkpoint_json: r#"{"revision":1,"event_cursor":3}"#.to_string(),
                updated_at_ms: 3,
            })
            .unwrap();

        let committed = store.commit_checkpoint_projection(&input).unwrap();
        assert!(committed.projection.applied);
        assert_eq!(committed.checkpoint.revision, 2);
        assert_eq!(committed.projection.conversation_revision, 3);
        let replayed = store.commit_checkpoint_projection(&input).unwrap();
        assert!(!replayed.projection.applied);

        let mut invalid = input.clone();
        invalid.checkpoint.expected_revision = 2;
        invalid.checkpoint.revision = 3;
        invalid.projection.operation_id = "atomic-revert-invalid".to_string();
        invalid.projection.expected_revision = 3;
        invalid.projection.mutations[0].message_id = "foreign-message".to_string();
        assert!(store.commit_checkpoint_projection(&invalid).is_err());
        assert_eq!(
            store
                .load_checkpoint("atomic-projection")
                .unwrap()
                .unwrap()
                .revision,
            2
        );
        assert_eq!(
            store
                .load_conversation("atomic-projection")
                .unwrap()
                .unwrap()
                .revision,
            3
        );
    }

    let reopened = NolaStore::open(&database).unwrap();
    let checkpoint = reopened
        .load_checkpoint("atomic-projection")
        .unwrap()
        .unwrap();
    assert_eq!(checkpoint.revision, 2);
    assert_eq!(checkpoint.event_cursor, 9);
    let projected = reopened
        .load_conversation("atomic-projection")
        .unwrap()
        .unwrap();
    assert_eq!(
        projected
            .messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>(),
        vec!["user-1"]
    );
    assert_eq!(reopened.list_conversations().unwrap()[0].message_count, 3);
}

#[test]
fn branch_creation_rolls_back_target_and_lineage_on_mid_transaction_failure() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("branch-rollback.db");
    let mut store = NolaStore::open(&database).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "existing".to_string(),
            title: "Existing".to_string(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    let colliding_message = MessageInput {
        id: "global-message-id".to_string(),
        run_id: None,
        role: "user".to_string(),
        status: "completed".to_string(),
        ordinal: 1,
        created_at_ms: 1,
        completed_at_ms: Some(1),
        parts: vec![MessagePartInput {
            id: "global-part-id".to_string(),
            kind: "text".to_string(),
            ordinal: 1,
            state: "completed".to_string(),
            content_text: "existing".to_string(),
            content_json: None,
            content_digest: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        }],
    };
    store
        .append_messages(&AppendMessagesInput {
            conversation_id: "existing".to_string(),
            expected_message_count: 0,
            messages: vec![colliding_message.clone()],
            updated_at_ms: 2,
        })
        .unwrap();

    let fault = rusqlite::Connection::open(&database).unwrap();
    fault.execute_batch("CREATE TRIGGER branch_failure BEFORE INSERT ON messages WHEN NEW.conversation_id='rolled-back-branch' BEGIN SELECT RAISE(ABORT,'injected branch failure'); END;").unwrap();
    let error = store
        .create_conversation_branch(&CreateConversationBranchInput {
            operation_id: "branch-rollback".into(),
            source_conversation_id: "existing".into(),
            expected_source_revision: 2,
            through_message_id: "global-message-id".into(),
            conversation: nola_domain::CanonicalBranchTarget {
                id: "rolled-back-branch".into(),
                title: "Branch".into(),
                settings_json: None,
                created_at_ms: 3,
                updated_at_ms: 3,
            },
            committed_at_ms: 3,
        })
        .unwrap_err();
    assert!(matches!(error, StoreError::Sqlite(_)));
    assert!(store
        .load_conversation("rolled-back-branch")
        .unwrap()
        .is_none());
    assert!(store
        .load_checkpoint("rolled-back-branch")
        .unwrap()
        .is_none());
    let operations: i64 = fault
        .query_row(
            "SELECT COUNT(*) FROM conversation_branch_operations",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(operations, 0);
    assert_eq!(store.list_conversations().unwrap().len(), 1);
}

#[test]
fn run_invocation_config_is_run_scoped_and_survives_cold_reopen() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("run-invocation-config.db");
    {
        let mut store = NolaStore::open(&database).unwrap();
        let mut input = begin_input();
        input.invocation_config_json = r#"{"model":"model-a","temperature":0.2}"#.into();
        input.invocation_config_digest = "digest-model-a".into();
        store.begin_turn(&input).unwrap();

        let active = store.active_run("conversation-1").unwrap().unwrap();
        assert_eq!(
            active.invocation_config_digest.as_deref(),
            Some("digest-model-a")
        );
    }

    let reopened = NolaStore::open(&database).unwrap();
    let invocation = reopened
        .load_run_invocation_config("run-1")
        .unwrap()
        .unwrap();
    assert_eq!(invocation.invocation_config_digest, "digest-model-a");
    assert_eq!(
        invocation.invocation_config_json,
        r#"{"model":"model-a","temperature":0.2}"#
    );
    let checkpoint = reopened.load_checkpoint("conversation-1").unwrap().unwrap();
    assert!(!checkpoint.checkpoint_json.contains("model-a"));
}
#[path = "support/outbox.rs"]
mod outbox_fixture;
