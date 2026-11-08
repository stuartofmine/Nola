// SPDX-License-Identifier: Apache-2.0
//! Formal non-live context admission, carried by the existing Developer item.
use super::{AgentInputOrigin, AgentItem, AgentRole, ItemId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalTriggerBinding {
    pub goal_id: String,
    pub goal_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InternalTurnTrigger {
    pub trigger_id: String,
    pub core_session_id: String,
    pub conversation_id: Option<String>,
    pub origin: AgentInputOrigin,
    pub context_digest: String,
    pub goal: Option<GoalTriggerBinding>,
}

impl InternalTurnTrigger {
    pub fn validate(&self) -> Result<(), String> {
        if self.trigger_id.trim().is_empty() || self.core_session_id.trim().is_empty()
            || self.conversation_id.as_ref().is_some_and(|id| id.trim().is_empty())
            || !self.origin.is_automated()
            || self.origin == AgentInputOrigin::Maintenance
            || self.context_digest.len() != 64
            || !self.context_digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            || self.goal.as_ref().is_some_and(|goal| goal.goal_id.trim().is_empty() || goal.goal_revision == 0)
            || (self.origin == AgentInputOrigin::GoalContinuation) != self.goal.is_some()
        {
            return Err("invalid internal turn trigger identity, origin or context".into());
        }
        Ok(())
    }

    pub fn item(&self) -> Result<AgentItem, String> {
        self.validate()?;
        Ok(AgentItem::Message {
            id: ItemId::from_string(self.trigger_id.clone()),
            role: AgentRole::Developer,
            content: serde_json::to_string(&InternalTriggerPacket {
                internal_turn_trigger: self.clone(),
            }).map_err(|error| error.to_string())?,
        })
    }

    pub fn from_item(item: &AgentItem) -> Result<Self, String> {
        let AgentItem::Message { id, role: AgentRole::Developer, content } = item else {
            return Err("internal turn trigger must be a Developer item".into());
        };
        let packet: InternalTriggerPacket = serde_json::from_str(content)
            .map_err(|error| format!("invalid internal turn trigger packet: {error}"))?;
        packet.internal_turn_trigger.validate()?;
        if packet.internal_turn_trigger.trigger_id != id.as_str() {
            return Err("internal trigger item and packet identity differ".into());
        }
        Ok(packet.internal_turn_trigger)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InternalTriggerPacket {
    internal_turn_trigger: InternalTurnTrigger,
}

pub fn internal_turn_context_digest(
    items: &[AgentItem],
    platform: &crate::context_engine::ContextPlatform,
    components: &[crate::context_engine::ContextComponent],
) -> String {
    super::super::input_queue::input_digest(&serde_json::json!({
        "items": items, "platform": platform, "components": components,
    }))
}
