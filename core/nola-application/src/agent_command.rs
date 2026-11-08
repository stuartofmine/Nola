// SPDX-License-Identifier: Apache-2.0
//! Durable Agent command queue contracts owned by the Application layer.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCommand {
    pub session_id: String,
    pub command_id: String,
    pub payload_json: String,
    pub created_at_ms: i64,
}

/// Scheduling hint derived from the typed durable envelope, never from text.
/// A peek is observational; admission still validates the claimed queue head.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentCommandKind {
    UserInput,
    ActiveSubmission,
    PermissionPolicy,
    TaskTerminal,
    TaskGuidance,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCommandHead {
    pub command_id: String,
    pub kind: AgentCommandKind,
    /// Exact single-message identity for idle user admission; no body is exposed.
    pub user_message_id: Option<String>,
    pub lease_owner: Option<String>,
    pub lease_until_ms: Option<i64>,
}

impl TryFrom<nola_store_port::agent_queue::StoredAgentCommand> for AgentCommandHead {
    type Error = String;

    fn try_from(command: nola_store_port::agent_queue::StoredAgentCommand) -> Result<Self, Self::Error> {
        if command.lease_owner.is_some() != command.lease_until_ms.is_some() {
            return Err("Agent command has incomplete lease evidence".into());
        }
        use nola_agent_engine::agent::{host::AgentBoundaryInput, AgentSubmission};
        let envelope: AgentBoundaryInput = serde_json::from_str(&command.payload_json)
            .map_err(|error| format!("invalid durable Agent envelope: {error}"))?;
        let user_message_id = match &envelope {
            AgentBoundaryInput::Submission { submission: AgentSubmission::UserInput { items, submission_id, .. } }
                if submission_id.as_deref() == Some(command.command_id.as_str()) => match items.as_slice() {
                    [nola_agent_engine::agent::AgentItem::UiAction { id, .. }]
                    | [nola_agent_engine::agent::AgentItem::Message { id, role: nola_agent_engine::agent::AgentRole::User, .. }] =>
                        Some(id.as_str().to_owned()),
                    _ => None,
                },
            _ => None,
        };
        let kind = match envelope {
            AgentBoundaryInput::Submission { submission: AgentSubmission::UserInput { .. } } => AgentCommandKind::UserInput,
            AgentBoundaryInput::Submission { .. } => AgentCommandKind::ActiveSubmission,
            AgentBoundaryInput::PermissionPolicyAvailable { .. } => AgentCommandKind::PermissionPolicy,
            AgentBoundaryInput::TaskTerminalAvailable { .. } => AgentCommandKind::TaskTerminal,
            AgentBoundaryInput::TaskGuidanceAvailable { .. } => AgentCommandKind::TaskGuidance,
        };
        Ok(Self { command_id: command.command_id, kind, user_message_id, lease_owner: command.lease_owner, lease_until_ms: command.lease_until_ms })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCommandLease {
    pub session_id: String,
    pub command_id: String,
    pub payload_json: String,
    pub created_at_ms: i64,
    pub lease_owner: String,
    pub lease_until_ms: i64,
    pub delivery_attempt: i64,
}

impl From<AgentCommand> for nola_store_port::agent_queue::EnqueueAgentCommandInput {
    fn from(command: AgentCommand) -> Self {
        Self {
            session_id: command.session_id,
            command_id: command.command_id,
            payload_json: command.payload_json,
            created_at_ms: command.created_at_ms,
        }
    }
}

impl TryFrom<nola_store_port::agent_queue::StoredAgentCommand> for AgentCommandLease {
    type Error = String;

    fn try_from(
        command: nola_store_port::agent_queue::StoredAgentCommand,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            session_id: command.session_id,
            command_id: command.command_id,
            payload_json: command.payload_json,
            created_at_ms: command.created_at_ms,
            lease_owner: command
                .lease_owner
                .ok_or_else(|| "claimed Agent command has no lease owner".to_string())?,
            lease_until_ms: command
                .lease_until_ms
                .ok_or_else(|| "claimed Agent command has no lease deadline".to_string())?,
            delivery_attempt: command.attempt_count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::AgentCommandLease;

    #[test]
    fn head_kind_comes_from_protocol_not_user_text_and_preserves_lease() {
        use super::{AgentCommandHead, AgentCommandKind};
        use nola_agent_engine::agent::{AgentItem, AgentSubmission, ItemId, AgentInputDelivery};
        use nola_agent_engine::agent::host::AgentBoundaryInput;
        let envelope = AgentBoundaryInput::Submission { submission: AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(ItemId::from_string("user".into()),
                "permission_policy_available task_terminal_available 搜索 current")],
            turn_context_items: Vec::new(), turn_context_platform: Default::default(),
            turn_context_components: Vec::new(), submission_id: Some("action".into()),
            submitted_at_epoch_ms: Some(1), delivery: AgentInputDelivery::NextTurn,
        }};
        let mut stored = nola_store_port::agent_queue::StoredAgentCommand {
            sequence: 1, session_id: "conversation".into(), command_id: "action".into(),
            payload_json: serde_json::to_string(&envelope).unwrap(), created_at_ms: 1,
            lease_owner: Some("owner".into()), lease_until_ms: Some(100), attempt_count: 1,
        };
        let head = AgentCommandHead::try_from(stored.clone()).unwrap();
        assert_eq!(head.kind, AgentCommandKind::UserInput);
        assert_eq!(head.user_message_id.as_deref(), Some("user"));
        assert_eq!(head.lease_owner.as_deref(), Some("owner"));
        assert_eq!(head.lease_until_ms, Some(100));
        let mut mismatched = stored.clone();
        mismatched.command_id = "another-command".into();
        assert!(AgentCommandHead::try_from(mismatched).unwrap().user_message_id.is_none());
        stored.payload_json = serde_json::to_string(&AgentBoundaryInput::TaskTerminalAvailable {
            task_id: "user_input".into(), receipt_id: "action".into(),
        }).unwrap();
        assert_eq!(AgentCommandHead::try_from(stored.clone()).unwrap().kind, AgentCommandKind::TaskTerminal);
        stored.payload_json = "{}".into();
        assert!(AgentCommandHead::try_from(stored).is_err());
    }

    #[test]
    fn lease_wire_contract_does_not_expose_store_sequence() {
        let value = serde_json::to_value(AgentCommandLease {
            session_id: "session-1".into(),
            command_id: "command-1".into(),
            payload_json: r#"{"type":"submission","submission":{"type":"interrupt","reason":null}}"#.into(),
            created_at_ms: 10,
            lease_owner: "host-1".into(),
            lease_until_ms: 20,
            delivery_attempt: 1,
        })
        .unwrap();

        assert_eq!(value["delivery_attempt"], 1);
        assert!(value.get("sequence").is_none());
        assert!(value.get("attempt_count").is_none());
    }
}
