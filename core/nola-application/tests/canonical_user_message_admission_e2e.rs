// SPDX-License-Identifier: Apache-2.0
//! Canonical user-only admission is durable, replay-safe, and does not start a Run.
mod support;

use nola_application::{
    ApplicationCommand, ApplicationCommandReceiptOutcome, CreateConversation,
    ExecuteReceivedCommand,
};
use nola_application::agent_room::{AgentRoomCommand, SubmitRoomMessage};
use nola_protocol::{AdmitConversationUserMessage, CommandMetadata};
use serde_json::Value;
use support::ApplicationRuntime;

fn execute(runtime: &mut ApplicationRuntime, request_id: &str, command: ApplicationCommand)
    -> Result<Value, String>
{
    let received = ExecuteReceivedCommand {
        client_id: "chat-client".into(), request_id: request_id.into(),
        command_type: command.wire_command_name().into(),
        command_json: serde_json::to_string(&command).unwrap(), expected_revision: None,
        received_at: "2026-10-23T00:00:00Z".into(),
    };
    let receipt = runtime.execute_received_command(received, command).map_err(|error| error.to_string())?;
    match receipt.outcome {
        ApplicationCommandReceiptOutcome::Committed { result, .. } => Ok(result),
        other => Err(format!("unexpected admission outcome: {other:?}")),
    }
}

fn admission(message_id: &str, text: &str) -> ApplicationCommand {
    ApplicationCommand::AdmitConversationUserMessage { input: AdmitConversationUserMessage {
        conversation_id: "11111111-1111-4111-8111-111111111111".into(), message_id: message_id.into(), text: text.into(), created_at_ms: 10,
    }}
}

fn runtime() -> ApplicationRuntime {
    let mut runtime = support::open_runtime(":memory:").unwrap();
    runtime.execute(ApplicationCommand::CreateConversation { input: CreateConversation {
        id: "11111111-1111-4111-8111-111111111111".into(), title: "Chat".into(), settings_json: None,
        created_at_ms: 1, updated_at_ms: 1,
    }}).unwrap();
    runtime
}

#[test]
fn canonical_message_admission_replays_and_conflicts_without_starting_default_run() {
    let mut runtime = runtime();
    let first = execute(&mut runtime, "stable-request", admission("stable-message", "Ask experts")).unwrap();
    assert_eq!(first["message_id"], "stable-message");
    assert_eq!(first["already_admitted"], false);
    let replay = execute(&mut runtime, "stable-request", admission("stable-message", "Ask experts")).unwrap();
    assert_eq!(replay, first);

    let conflict = runtime.execute_received_command(ExecuteReceivedCommand {
        client_id: "chat-client".into(), request_id: "stable-request".into(),
        command_type: "admit_conversation_user_message".into(),
        command_json: serde_json::to_string(&admission("stable-message", "Changed text")).unwrap(),
        expected_revision: None, received_at: "2026-10-23T00:00:01Z".into(),
    }, admission("stable-message", "Changed text"));
    assert!(conflict.is_err(), "a reused transport request ID cannot change payload");

    let duplicate_key = execute(&mut runtime, "new-transport-request", admission("stable-message", "Ask experts")).unwrap();
    assert_eq!(duplicate_key["already_admitted"], true);
    let changed_message = execute(&mut runtime, "another-request", admission("stable-message", "Changed text"));
    assert!(changed_message.is_err());

    let message: Value = runtime.execute(ApplicationCommand::LoadConversation { conversation_id: "11111111-1111-4111-8111-111111111111".into() }).unwrap();
    assert_eq!(message["messages"].as_array().unwrap().len(), 1);
    assert_eq!(message["messages"][0]["role"], "user");
    assert_eq!(message["messages"][0]["run_id"], Value::Null);
    let active: Value = runtime.execute(ApplicationCommand::ActiveRun { conversation_id: "11111111-1111-4111-8111-111111111111".into() }).unwrap();
    assert!(active.is_null(), "admission alone must not create an assistant Run");
}

#[test]
fn only_an_existing_conversation_can_be_admitted_and_expert_anchor_is_accepted() {
    let mut runtime = runtime();
    assert!(execute(&mut runtime, "foreign-conversation", ApplicationCommand::AdmitConversationUserMessage {
        input: AdmitConversationUserMessage { conversation_id: "missing".into(), message_id: "message-x".into(),
            text: "hello".into(), created_at_ms: 11 },
    }).is_err());
    let receipt = execute(&mut runtime, "admit", admission("message-user", "Research this")).unwrap();
    let association: Value = execute(&mut runtime, "association", ApplicationCommand::AgentRoom {
        command: AgentRoomCommand::EnsureForConversation { conversation_id: "11111111-1111-4111-8111-111111111111".into(), at_ms: 12 },
    }).unwrap();
    let room = association["room"]["room"]["id"].as_str().unwrap();
    execute(&mut runtime, "configure-expert", ApplicationCommand::AgentRoom {
        command: AgentRoomCommand::ConfigureAgent { input: nola_application::persistent_agent::ConfigureAgent {
            id: "expert".to_string().try_into().unwrap(), expected_directory_revision: None,
            expected_agent_revision: None, name: "Expert".into(), description: "Inert test expert".into(),
            defaults: nola_domain::persistent_agent::AgentDefaults::try_from(serde_json::json!({"language":"en"})).unwrap(),
            availability: nola_domain::persistent_agent::AgentAvailability::Active, updated_at_ms: 12,
        } },
    }).unwrap();
    let joined = execute(&mut runtime, "join-expert", ApplicationCommand::AgentRoom {
        command: AgentRoomCommand::Join { input: nola_application::agent_room::JoinAgent {
            room_id: room.into(), expected_room_revision: association["room"]["revision"].as_i64().unwrap(),
            agent_id: "expert".to_string().try_into().unwrap(), expected_agent_revision: 1, at_ms: 12,
        } },
    }).unwrap();
    let room_revision = joined["revision"].as_i64().unwrap();
    let anchored: Value = execute(&mut runtime, "expert-submit", ApplicationCommand::AgentRoom {
        command: AgentRoomCommand::Submit { input: SubmitRoomMessage {
            room_id: room.into(), submission_id: uuid::Uuid::new_v4().to_string(), expected_room_revision: room_revision,
            text: "Research this".into(), participant_id: None, at_ms: 13,
            parent_message_id: Some(receipt["message_id"].as_str().unwrap().into()),
            conversation_id: Some("11111111-1111-4111-8111-111111111111".into()),
        }},
    }).unwrap();
    assert_eq!(anchored["input"]["parent_message_id"], "message-user");
}
