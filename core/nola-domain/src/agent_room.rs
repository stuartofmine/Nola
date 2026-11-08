// SPDX-License-Identifier: Apache-2.0
//! A shared presentation room containing independent model contexts.
use crate::persistent_agent::AgentId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const ROOM_KEY_PREFIX: &str = "agent_chat_room/";
pub const CONTEXT_OWNER_FIELD: &str = "agentRoomOwner";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextOwner {
    pub room_id: String,
    pub participant_id: String,
    pub agent_id: AgentId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomParticipant {
    pub id: String,
    pub agent_id: AgentId,
    pub agent_revision: i64,
    pub display_name: String,
    pub description: String,
    pub context_conversation_id: String,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RoomWire", into = "RoomWire")]
pub struct AgentRoom {
    id: String,
    title: String,
    participants: Vec<RoomParticipant>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoomWire {
    id: String,
    title: String,
    participants: Vec<RoomParticipant>,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

impl AgentRoom {
    pub fn new(
        id: String,
        title: String,
        participants: Vec<RoomParticipant>,
    ) -> Result<Self, String> {
        if !valid_id(&id)
            || title.trim().is_empty()
            || title.len() > 512
            || title.chars().any(char::is_control)
        {
            return Err("invalid room identity or title".into());
        }
        let mut ids = BTreeSet::new();
        let mut agents = BTreeSet::new();
        let mut contexts = BTreeSet::new();
        for member in &participants {
            crate::persistent_agent::validate_agent_description(&member.description)?;
            if !valid_id(&member.id)
                || !valid_id(&member.context_conversation_id)
                || member.context_conversation_id == id
                || member.agent_revision < 1
                || member.display_name.trim().is_empty()
                || member.display_name.len() > 512
                || member.display_name.chars().any(char::is_control)
                || !ids.insert(&member.id)
                || !agents.insert(&member.agent_id)
                || !contexts.insert(&member.context_conversation_id)
            {
                return Err("invalid or duplicate room participant/context".into());
            }
        }
        Ok(Self {
            id,
            title,
            participants,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn participants(&self) -> &[RoomParticipant] {
        &self.participants
    }

    pub fn recipient(&self, participant_id: &str) -> Result<&RoomParticipant, String> {
        let member = self
            .participants
            .iter()
            .find(|member| member.id == participant_id)
            .ok_or("room_participant_not_found")?;
        if !member.active {
            return Err("room_participant_inactive".into());
        }
        Ok(member)
    }

    pub fn add(&self, participant: RoomParticipant) -> Result<Self, String> {
        let mut participants = self.participants.clone();
        participants.push(participant);
        Self::new(self.id.clone(), self.title.clone(), participants)
    }

    pub fn deactivate(&self, participant_id: &str) -> Result<Self, String> {
        self.recipient(participant_id)?;
        let mut participants = self.participants.clone();
        participants
            .iter_mut()
            .find(|p| p.id == participant_id)
            .unwrap()
            .active = false;
        Self::new(self.id.clone(), self.title.clone(), participants)
    }
}

impl TryFrom<RoomWire> for AgentRoom {
    type Error = String;
    fn try_from(value: RoomWire) -> Result<Self, String> {
        Self::new(value.id, value.title, value.participants)
    }
}
impl From<AgentRoom> for RoomWire {
    fn from(value: AgentRoom) -> Self {
        Self {
            id: value.id,
            title: value.title,
            participants: value.participants,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn member(agent: &str, context: &str) -> RoomParticipant {
        RoomParticipant {
            id: format!("member-{agent}"),
            agent_id: agent.to_string().try_into().unwrap(),
            agent_revision: 1,
            display_name: agent.into(),
            description: "Public responsibilities".into(),
            context_conversation_id: context.into(),
            active: true,
        }
    }
    #[test]
    fn recipients_have_independent_contexts_and_removed_history_is_retained() {
        let room = AgentRoom::new(
            "room".into(),
            "Discussion".into(),
            vec![member("a", "a-context"), member("b", "b-context")],
        )
        .unwrap();
        assert_eq!(
            room.recipient("member-b").unwrap().context_conversation_id,
            "b-context"
        );
        let removed = room.deactivate("member-a").unwrap();
        assert!(removed.recipient("member-a").is_err());
        assert_eq!(removed.participants().len(), 2);
        assert!(removed.recipient("member-b").is_ok());
    }
    #[test]
    fn duplicate_contexts_or_agents_and_invalid_wire_are_rejected() {
        assert!(AgentRoom::new(
            "room".into(),
            "Discussion".into(),
            vec![member("a", "same"), member("b", "same")]
        )
        .is_err());
        let room = AgentRoom::new(
            "room".into(),
            "Discussion".into(),
            vec![member("a", "context")],
        )
        .unwrap();
        assert!(room.add(member("a", "other")).is_err());
        let mut wire = serde_json::to_value(room).unwrap();
        wire["participants"][0]["agent_revision"] = serde_json::json!(0);
        assert!(serde_json::from_value::<AgentRoom>(wire).is_err());
    }
}
