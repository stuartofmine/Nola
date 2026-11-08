// SPDX-License-Identifier: Apache-2.0
mod support;
use nola_application::{
    begin_assistant_turn, create_conversation_branch, load_conversation_projection_v1,
    BeginAssistantTurn, CreateConversationBranch,
};
use nola_agent_engine::agent::{AgentCoreConfig, AgentItem, AgentSubmission, ItemId, SessionId};
use nola_domain::{CanonicalBranchTarget, MessagePartView, MessageRole};
use nola_store::{
    AppendMessagesInput, CreateConversationInput, MessageInput, MessagePartInput, NolaStore,
};
use tempfile::tempdir;

fn message(id: &str, role: &str, ordinal: i64, text: &str) -> MessageInput {
    MessageInput {
        id: id.into(),
        run_id: None,
        role: role.into(),
        status: "completed".into(),
        ordinal,
        created_at_ms: ordinal,
        completed_at_ms: Some(ordinal),
        parts: vec![MessagePartInput {
            id: format!("{id}:body"),
            kind: "text".into(),
            ordinal: 1,
            state: "completed".into(),
            content_text: text.into(),
            content_json: None,
            content_digest: None,
            created_at_ms: ordinal,
            updated_at_ms: ordinal,
        }],
    }
}
fn intent(source: &str, target: &str, boundary: &str, revision: i64) -> CreateConversationBranch {
    CreateConversationBranch {
        operation_id: format!("branch-{target}"),
        source_conversation_id: source.into(),
        expected_source_revision: revision,
        through_message_id: boundary.into(),
        conversation: CanonicalBranchTarget {
            id: target.into(),
            title: "Branch".into(),
            settings_json: Some(r#"{"owner":"test"}"#.into()),
            created_at_ms: 3,
            updated_at_ms: 3,
        },
        committed_at_ms: 3,
    }
}
fn seed(store: &mut NolaStore, text: &str) {
    store
        .create_conversation(&CreateConversationInput {
            id: "source".into(),
            title: "Source".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    store
        .append_messages(&AppendMessagesInput {
            conversation_id: "source".into(),
            expected_message_count: 0,
            messages: vec![
                message("question", "user", 1, text),
                message("answer", "assistant", 2, "answer"),
            ],
            updated_at_ms: 2,
        })
        .unwrap();
}
fn config(conversation: &str) -> AgentCoreConfig {
    use nola_agent_engine::agent::host::*;
    let descriptor = HostCapabilityDescriptor {
        id: "model.stream".into(),
        title: "Model".into(),
        route: HostCapabilityRoute::Model,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Internal,
        permission_domain: None,
        input_schema: serde_json::json!({"type":"object"}),
        input_normalization: Default::default(),
        output_policy: HostOutputPolicy {
            preview_char_limit: 4000,
            persist_full_output: true,
            allow_artifacts: true,
        },
        supports_streaming: true,
        supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial,
        cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    };
    AgentCoreConfig {
        turn_identity: support::model_identity::identity(),
        session_id: Some(SessionId::from_string(conversation.into())),
        model: Some("test-model".into()),
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("test-host".into()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities: std::collections::BTreeMap::from([("model.stream".into(), descriptor)]),
        }),
        ..Default::default()
    }
}

fn begin(store: &mut NolaStore, conversation: &str) -> nola_application::AcceptedAssistantTurn {
    let frontier = support::admission_frontier::read(store, conversation);
    begin_assistant_turn(
        store,
        BeginAssistantTurn {
            expected_configuration_revision: frontier.0,
            expected_parent_message_id: frontier.1,
            conversation_id: conversation.into(),
            conversation_title: "Branch".into(),
            user_message_id: "next-user".into(),
            user_part_id: "next-user:body".into(),
            user_text: "continue".into(),
            user_content_json: None,
            assistant_message_id: "next-assistant".into(),
            context_checkpoint_id: None,
            core_config: config(conversation),
            submission: AgentSubmission::UserInput {
                delivery: nola_agent_engine::agent::AgentInputDelivery::CurrentTurn,
                items: vec![AgentItem::user_message(
                    ItemId::from_string("next-user".into()),
                    "continue",
                )],
                turn_context_items: Vec::new(),
                turn_context_platform: Default::default(),
                turn_context_components: Vec::new(),
                submission_id: Some("next-turn".into()),
                submitted_at_epoch_ms: Some(4),
            },
            created_at_ms: 4,
        },
    )
    .unwrap()
}

#[test]
fn metadata_branch_preserves_lineage_and_restores_original_context_on_first_turn_after_restart() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("branch.db");
    let mut store = NolaStore::open(&path).unwrap();
    seed(&mut store, "question evidence");
    let input = intent("source", "target", "answer", 2);
    let result =
        create_conversation_branch(&mut store.conversation_write_repository(), input.clone())
            .unwrap();
    assert_eq!(result.message_count, 2);
    assert!(result.created);
    assert!(store.load_checkpoint("target").unwrap().is_none());
    assert!(
        !create_conversation_branch(&mut store.conversation_write_repository(), input.clone())
            .unwrap()
            .created
    );
    let mut changed = input;
    changed.conversation.title = "changed".into();
    assert!(
        create_conversation_branch(&mut store.conversation_write_repository(), changed).is_err()
    );
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    let projection = load_conversation_projection_v1(&store, "target").unwrap();
    assert_eq!(projection.messages.len(), 2);
    assert_eq!(projection.messages[0].role, MessageRole::User);
    assert_eq!(projection.messages[1].role, MessageRole::Assistant);
    assert!(
        matches!(projection.messages[0].parts.as_slice(),[MessagePartView::Text{markdown,..}] if markdown=="question evidence")
    );
    let connection = rusqlite::Connection::open(&path).unwrap();
    let lineage:String=connection.query_row("SELECT intent_json FROM conversation_branch_operations WHERE operation_id='branch-target'",[],|row|row.get(0)).unwrap();
    let lineage: CreateConversationBranch = serde_json::from_str(&lineage).unwrap();
    assert_eq!(lineage.source_conversation_id, "source");
    assert_eq!(lineage.through_message_id, "answer");
    let begun = begin(&mut store, "target");
    let checkpoint = store.load_checkpoint("target").unwrap().unwrap().checkpoint_json;
    assert!(checkpoint.contains("question evidence"));
    assert!(checkpoint.contains("answer"));
    let requests =
        nola_application::load_committed_host_requests(&store, &begun.begin.run_id).unwrap();
    let requests = serde_json::to_string(&requests).unwrap();
    assert!(requests.contains("question evidence"));
    assert!(requests.contains("answer"));
    assert_eq!(
        store
            .load_conversation("target")
            .unwrap()
            .unwrap()
            .messages
            .len(),
        4
    );
}

#[test]
fn ten_million_byte_branch_and_rebranch_use_small_intents_and_bounded_first_turn_context() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("large-branch.db");
    let mut store = NolaStore::open(&path).unwrap();
    let original = "x".repeat(9_999_991) + "TAIL-DONE";
    seed(&mut store, &original);
    let input = intent("source", "large", "question", 2);
    assert!(serde_json::to_vec(&input).unwrap().len() < 1024);
    drop(store);
    let mut runtime = support::open_runtime(&path).unwrap();
    let command = nola_application::ApplicationCommand::CreateConversationBranch { input };
    let received = nola_application::ExecuteReceivedCommand {
        client_id: "branch-client".into(),
        request_id: "large-branch".into(),
        command_type: "create_conversation_branch".into(),
        command_json: serde_json::to_string(&command).unwrap(),
        expected_revision: None,
        received_at: "2026-09-08T00:00:00Z".into(),
    };
    let receipt = runtime
        .execute_received_command(
            received.clone(),
            serde_json::from_str(&received.command_json).unwrap(),
        )
        .unwrap();
    assert!(
        matches!(
            &receipt.outcome,
            nola_application::ApplicationCommandReceiptOutcome::Committed { .. }
        ),
        "{receipt:?}"
    );
    assert!(
        serde_json::to_vec(&receipt.clone().into_protocol_receipt())
            .unwrap()
            .len()
            < 2048
    );
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    assert!(
        runtime
            .execute_received_command(received, command)
            .unwrap()
            .replayed
    );
    drop(runtime);
    let connection = rusqlite::Connection::open(&path).unwrap();
    let event_bytes: i64 = connection
        .query_row(
            "SELECT MAX(length(CAST(payload_json AS BLOB))) FROM projection_events",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        event_bytes < 4096,
        "branch projection crossed bounded invalidation contract: {event_bytes}"
    );

    let mut store = NolaStore::open(&path).unwrap();
    let window = store
        .load_conversation_message_window("large", None, 10, 1024)
        .unwrap()
        .unwrap();
    assert_eq!(window.conversation.messages.len(), 1);
    assert_eq!(window.deferred_text_parts[0].total_utf8_bytes, 10_000_000);
    let message = &window.conversation.messages[0];
    let part = &message.parts[0];
    let range = store
        .read_conversation_text_range("large", &message.id, &part.id, 9_999_991, 32)
        .unwrap();
    assert_eq!(range.text, "TAIL-DONE");
    create_conversation_branch(
        &mut store.conversation_write_repository(),
        intent(
            "large",
            "rebranch",
            &message.id,
            window.conversation.revision,
        ),
    )
    .unwrap();
    let again = store
        .load_conversation_message_window("rebranch", None, 10, 512)
        .unwrap()
        .unwrap();
    assert_ne!(again.conversation.messages[0].id, message.id);
    assert_ne!(again.conversation.messages[0].parts[0].id, part.id);
    let full = store.load_conversation("rebranch").unwrap().unwrap();
    assert_eq!(full.messages[0].parts[0].content_text, original);
    let begun = begin(&mut store, "rebranch");
    let encoded = serde_json::to_string(&begun).unwrap();
    assert!(
        encoded.len() < 512 * 1024,
        "branch first turn response unexpectedly large: {}",
        encoded.len()
    );
    // Admission returns controls/publications, not the reconstructed prompt.
    // The committed model request below must still carry the bounded excerpt.
    let reply: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert!(reply.get("checkpoint").is_none());
    assert!(reply.get("snapshot").is_none());
    assert!(!encoded.contains("bounded context excerpt"));
    assert!(begun.execution.configuration.initial_items.is_empty());
    assert!(begun.execution.control.context_usage.used_tokens > 0);
    let requests =
        nola_application::load_committed_host_requests(&store, &begun.begin.run_id).unwrap();
    let requests = serde_json::to_string(&requests).unwrap();
    assert!(requests.len() < 256 * 1024);
    assert!(requests.contains("bounded context excerpt"));
    assert!(requests.contains("10000000"));
}

#[test]
fn stale_revision_foreign_boundary_and_existing_target_do_not_create_partial_branch() {
    let mut store = NolaStore::open(":memory:").unwrap();
    seed(&mut store, "original");
    for input in [
        intent("source", "bad", "question", 1),
        intent("source", "bad", "foreign", 2),
        intent("source", "source", "question", 2),
    ] {
        assert!(
            create_conversation_branch(&mut store.conversation_write_repository(), input).is_err()
        );
        assert!(store.load_conversation("bad").unwrap().is_none());
    }
    let mut wire = serde_json::to_value(intent("source", "bad", "question", 2)).unwrap();
    wire["messages"] = serde_json::json!([]);
    assert!(serde_json::from_value::<CreateConversationBranch>(wire).is_err());
}

#[test]
fn non_sqlite_branch_helper_preserves_intent_and_commit_failure() {
    use nola_store_port::canonical_branch::{CanonicalBranchRepository, CanonicalBranchResult};
    #[derive(Default)]
    struct Repository {
        seen: Option<CreateConversationBranch>,
        fail: bool,
    }
    impl CanonicalBranchRepository for Repository {
        type Error = String;
        fn create_conversation_branch(
            &mut self,
            intent: &CreateConversationBranch,
        ) -> Result<CanonicalBranchResult, String> {
            self.seen = Some(intent.clone());
            if self.fail {
                return Err("branch dependency publication failed".into());
            }
            Ok(CanonicalBranchResult {
                created: true,
                message_count: 2,
                conversation_revision: 1,
                conversation_id: intent.conversation.id.clone(),
                source_conversation_id: intent.source_conversation_id.clone(),
                through_message_id: intent.through_message_id.clone(),
            })
        }
    }
    let intent = CreateConversationBranch {
        operation_id: "operation".into(),
        source_conversation_id: "source".into(),
        expected_source_revision: 7,
        through_message_id: "boundary".into(),
        conversation: CanonicalBranchTarget {
            id: "target".into(),
            title: "Branch".into(),
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        },
        committed_at_ms: 1,
    };
    let mut repository = Repository::default();
    let result = create_conversation_branch(&mut repository, intent.clone()).unwrap();
    assert_eq!(result.conversation_id, "target");
    assert_eq!(repository.seen, Some(intent.clone()));
    repository.fail = true;
    assert!(create_conversation_branch(&mut repository, intent)
        .unwrap_err()
        .contains("dependency publication failed"));
}
