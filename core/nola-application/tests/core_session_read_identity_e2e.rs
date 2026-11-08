// SPDX-License-Identifier: Apache-2.0
//! Canonical conversation keys and Core session identities are separate facts.

#[path = "support/model_identity.rs"]
mod fixture_model_identity;
use std::collections::BTreeMap;

use nola_application::{
    begin_assistant_turn, execute_application_command, load_conversation_projection_v1,
    load_conversation_projection_window_v1, load_session_projection,
    reduce_and_commit_active_submission, ApplicationCommand, BeginAssistantTurn,
    ReduceAndCommitActiveSubmission,
};
use nola_agent_engine::agent::host::*;
use nola_agent_engine::agent::protocol::*;
use nola_domain::MessagePartView;
use nola_store::NolaStore;

const CONVERSATION: &str = "canonical-conversation";
const CORE_SESSION: &str = "actual-core-session";

fn config() -> AgentCoreConfig {
    AgentCoreConfig {
        turn_identity: fixture_model_identity::identity(),
        session_id: Some(SessionId::from_string(CORE_SESSION.into())),
        model: Some("fixture-model".into()),
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("fixture-host".into()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities: BTreeMap::from([(
                "model.stream".into(),
                HostCapabilityDescriptor {
                    id: "model.stream".into(),
                    title: "model.stream".into(),
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
                },
            )]),
        }),
        ..AgentCoreConfig::default()
    }
}

fn begin(store: &mut NolaStore) -> nola_application::AcceptedAssistantTurn {
    begin_assistant_turn(
        store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: CONVERSATION.into(),
            conversation_title: "Separate identities".into(),
            user_message_id: "user".into(),
            user_part_id: "user:body".into(),
            user_text: "Continue the task".into(),
            user_content_json: None,
            assistant_message_id: "visible-assistant".into(),
            context_checkpoint_id: None,
            core_config: config(),
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(
                    ItemId::from_string("user".into()),
                    "Continue the task",
                )],
                turn_context_items: vec![],
                turn_context_platform: Default::default(),
                turn_context_components: vec![],
                submission_id: Some("submission".into()),
                submitted_at_epoch_ms: Some(1000),
                delivery: AgentInputDelivery::CurrentTurn,
            },
            created_at_ms: 1000,
        },
    )
    .unwrap()
}

fn start_skill(store: &mut NolaStore, begun: &nola_application::AcceptedAssistantTurn) {
    let envelope = serde_json::from_value(serde_json::json!({
        "operation_id":"start-skill",
        "command":{
            "type":"start", "run_id":"skill-run",
            "owner":{"session_id":CORE_SESSION,"parent_turn_id":begun.begin.run_id},
            "descriptor":{
                "skill_id":"skill", "version":"1", "display_name":"Active skill",
                "instructions_ref":format!("skill#sha256={}", "a".repeat(64)),
                "instructions_digest":"a".repeat(64)
            },
            "mode":"inline",
            "budget":{"max_model_turns":100,"max_tool_calls":100,"max_elapsed_millis":null}
        }
    }))
    .unwrap();
    reduce_and_commit_active_submission(
        store,
        ReduceAndCommitActiveSubmission {
            conversation_id: CONVERSATION.into(),
            run_id: begun.begin.run_id.clone(),
            invocation_config_digest: begun.invocation_config_digest.clone(),
            transition_id: "skill-start-transition".into(),
            submission: AgentSubmission::SkillRuntime { envelope },
            committed_at_ms: 1100,
        },
    )
    .unwrap();
}

fn assert_reads(store: &mut NolaStore, _run: &str) -> nola_domain::MessageView {
    let local = load_conversation_projection_v1(store, CONVERSATION).unwrap();
    assert_eq!(local.conversation_id.0, CONVERSATION);
    assert_eq!(local.messages.len(), 2);
    assert!(local.messages[1].parts.iter().any(|part| {
        matches!(part, MessagePartView::Skill { run_id, .. } if run_id == "skill-run")
    }));
    let window =
        load_conversation_projection_window_v1(store, CONVERSATION, None, 1, 32_768).unwrap();
    assert_eq!(window.projection.messages[0], local.messages[1]);
    let context: nola_domain::ContextUsageView = serde_json::from_value(
        execute_application_command(
            store,
            ApplicationCommand::LoadConversationContextUsageV1 {
                conversation_id: CONVERSATION.into(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    // Both endpoints consume the same exact-Run committed control fact.
    assert_eq!(context.used_tokens, local.context_usage.used_tokens);
    assert_eq!(
        context.context_window_tokens,
        local.context_usage.context_window_tokens
    );
    assert_eq!(
        context.prompt_token_limit,
        local.context_usage.prompt_token_limit
    );
    assert_eq!(context.estimated, local.context_usage.estimated);
    assert_eq!(
        context.measured_at_revision,
        local.context_usage.measured_at_revision
    );
    let session = load_session_projection(store, CONVERSATION)
        .unwrap()
        .unwrap();
    assert_eq!(session.conversation_id, CONVERSATION);
    assert_eq!(session.skill_runtime_projection.session_id, CORE_SESSION);
    assert_eq!(
        session.skill_runtime_projection.runs["skill-run"]
            .owner
            .session_id,
        CORE_SESSION
    );
    let permissions = execute_application_command(
        store,
        ApplicationCommand::LoadConversationPermissionProjectionV1 {
            conversation_id: CONVERSATION.into(),
            run_id: _run.into(),
            scope: nola_application::PermissionProjectionReadScope::CurrentRun,
        },
    )
    .unwrap();
    assert!(permissions.is_object());
    local.messages[1].clone()
}

#[test]
fn production_begin_distinct_core_session_reads_after_cold_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("local.db");
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin(&mut store);
    assert_eq!(begun.execution.control.session_id.as_str(), CORE_SESSION);
    assert_eq!(begun.execution.configuration.session_id.as_ref().unwrap().as_str(), CORE_SESSION);
    start_skill(&mut store, &begun);
    let before = assert_reads(&mut store, &begun.begin.run_id);
    drop(store);
    let mut reopened = NolaStore::open(&database).unwrap();
    let after = assert_reads(&mut reopened, &begun.begin.run_id);
    assert_eq!(before, after);
}

#[test]
fn owned_checkpoint_rejects_foreign_root_state_or_skill_session() {
    for foreign in ["root", "state", "skill"] {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("foreign.db");
        let mut store = NolaStore::open(&database).unwrap();
        let begun = begin(&mut store);
        start_skill(&mut store, &begun);
        let usage_before = execute_application_command(
            &mut store,
            ApplicationCommand::LoadConversationContextUsageV1 {
                conversation_id: CONVERSATION.into(),
            },
        )
        .unwrap();
        let stored = store.load_checkpoint(CONVERSATION).unwrap().unwrap();
        let mut checkpoint: serde_json::Value =
            serde_json::from_str(&stored.checkpoint_json).unwrap();
        match foreign {
            "root" => checkpoint["session_id"] = serde_json::json!("foreign-core"),
            "state" => checkpoint["state"]["session_id"] = serde_json::json!("foreign-core"),
            "skill" => {
                checkpoint["state"]["skill_runtime"]["runs"]["skill-run"]["owner"]["session_id"] =
                    serde_json::json!("foreign-core")
            }
            _ => unreachable!(),
        }
        // Isolated corruption is deliberate: the canonical key stays owned by
        // this conversation, so dropping identity checks would accept it.
        let inspect = rusqlite::Connection::open(&database).unwrap();
        inspect
            .execute(
                "UPDATE core_checkpoint_bodies SET checkpoint_json=?1 WHERE conversation_id=?2",
                rusqlite::params![checkpoint.to_string(), CONVERSATION],
            )
            .unwrap();
        assert!(load_conversation_projection_v1(&store, CONVERSATION).is_err());
        assert!(load_session_projection(&store, CONVERSATION).is_err());
        assert_eq!(
            execute_application_command(
                &mut store,
                ApplicationCommand::LoadConversationContextUsageV1 {
                    conversation_id: CONVERSATION.into()
                }
            )
            .unwrap(),
            usage_before,
            "committed usage does not decode the independently corrupt execution checkpoint"
        );
        if foreign != "skill" {
            assert!(execute_application_command(
                &mut store,
                ApplicationCommand::LoadConversationPermissionProjectionV1 {
                    conversation_id: CONVERSATION.into(),
                    run_id: begun.begin.run_id.clone(),
                    scope: nola_application::PermissionProjectionReadScope::CurrentRun,
                },
            )
            .is_err());
        }
    }
}
