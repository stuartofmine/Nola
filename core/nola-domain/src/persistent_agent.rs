// SPDX-License-Identifier: Apache-2.0
//! Persistent assistant identity and per-conversation run ownership.
//! Execution workers are children of a run, never persistent assistant identities.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Identity is opaque, validated on both construction and wire decoding.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AgentId(String);

impl TryFrom<String> for AgentId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        validate_identity(&value)?;
        Ok(Self(value))
    }
}

impl From<AgentId> for String {
    fn from(value: AgentId) -> Self {
        value.0
    }
}

impl AgentId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn validate_identity(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 512
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err("identity must be nonempty, bounded and free of surrounding whitespace or control characters".into());
    }
    Ok(())
}

/// No selected-view state participates in this identity. All callbacks retain it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "ConversationOwnerWire", into = "ConversationOwnerWire")]
pub struct ConversationOwner {
    agent_id: AgentId,
    conversation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConversationOwnerWire {
    agent_id: AgentId,
    conversation_id: String,
}

impl ConversationOwner {
    pub fn new(agent_id: AgentId, conversation_id: String) -> Result<Self, String> {
        validate_identity(&conversation_id)?;
        Ok(Self {
            agent_id,
            conversation_id,
        })
    }

    pub fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }
    pub fn conversation_id(&self) -> &str {
        &self.conversation_id
    }
}

impl TryFrom<ConversationOwnerWire> for ConversationOwner {
    type Error = String;
    fn try_from(value: ConversationOwnerWire) -> Result<Self, String> {
        Self::new(value.agent_id, value.conversation_id)
    }
}

impl From<ConversationOwner> for ConversationOwnerWire {
    fn from(value: ConversationOwner) -> Self {
        Self {
            agent_id: value.agent_id,
            conversation_id: value.conversation_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentAvailability {
    Active,
    Archived,
}

/// A closed subset of the existing configuration contract. Runtime/session
/// fields, credentials, knowledge and previously granted permissions cannot be
/// smuggled into the defaults of new conversations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "serde_json::Value", into = "serde_json::Value")]
pub struct AgentDefaults(serde_json::Map<String, serde_json::Value>);

impl TryFrom<serde_json::Value> for AgentDefaults {
    type Error = String;
    fn try_from(value: serde_json::Value) -> Result<Self, String> {
        let fields = value
            .as_object()
            .ok_or("agent defaults must be an object")?;
        for field in fields.keys() {
            if !matches!(
                field.as_str(),
                "modelSelectionID"
                    | "modelOverrides"
                    | "selectedPromptTemplateID"
                    | "expertReasoningEffort"
                    | "language"
                    | "promptResponseStyle"
                    | "contextLimitOverride"
                    | "memoryEnabledOverride"
                    | "retrievalEnabledOverride"
            ) {
                return Err(format!("field is not an agent default: {field}"));
            }
        }
        crate::configuration_patch::validate_patch(&value.to_string())?;
        Ok(Self(fields.clone()))
    }
}

impl From<AgentDefaults> for serde_json::Value {
    fn from(value: AgentDefaults) -> Self {
        Self::Object(value.0)
    }
}

impl AgentDefaults {
    /// Independent owned value: later profile edits cannot mutate a session.
    pub fn conversation_configuration(&self) -> serde_json::Value {
        serde_json::Value::Object(self.0.clone())
    }
}

/// Persisted profile validation is also applied when reading from storage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PersistentAgentWire", into = "PersistentAgentWire")]
pub struct PersistentAgent {
    id: AgentId,
    name: String,
    description: String,
    revision: i64,
    availability: AgentAvailability,
    defaults: AgentDefaults,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistentAgentWire {
    id: AgentId,
    name: String,
    description: String,
    revision: i64,
    availability: AgentAvailability,
    defaults: AgentDefaults,
}

impl PersistentAgent {
    pub fn new(
        id: AgentId,
        name: String,
        description: String,
        revision: i64,
        availability: AgentAvailability,
        defaults: AgentDefaults,
    ) -> Result<Self, String> {
        if name.trim().is_empty() || name.len() > 512 || name.chars().any(char::is_control) {
            return Err(
                "agent name must be nonempty, bounded and free of control characters".into(),
            );
        }
        validate_agent_description(&description)?;
        if revision < 1 {
            return Err("agent revision must be positive".into());
        }
        Ok(Self {
            id,
            name,
            description,
            revision,
            availability,
            defaults,
        })
    }

    pub fn id(&self) -> &AgentId {
        &self.id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Public routing information, never the agent's private model instructions.
    pub fn description(&self) -> &str {
        &self.description
    }
    pub fn revision(&self) -> i64 {
        self.revision
    }
    pub fn availability(&self) -> &AgentAvailability {
        &self.availability
    }
    pub fn defaults(&self) -> &AgentDefaults {
        &self.defaults
    }
}

impl TryFrom<PersistentAgentWire> for PersistentAgent {
    type Error = String;
    fn try_from(value: PersistentAgentWire) -> Result<Self, String> {
        Self::new(
            value.id,
            value.name,
            value.description,
            value.revision,
            value.availability,
            value.defaults,
        )
    }
}

impl From<PersistentAgent> for PersistentAgentWire {
    fn from(value: PersistentAgent) -> Self {
        Self {
            id: value.id,
            name: value.name,
            description: value.description,
            revision: value.revision,
            availability: value.availability,
            defaults: value.defaults,
        }
    }
}

pub fn validate_agent_description(value: &str) -> Result<(), String> {
    if value.len() > 4096 || value.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return Err("agent description must be bounded public text".into());
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunRegistrationError {
    InvalidRunId,
    ConversationBusy,
    OwnerMismatch,
}

/// One instance per canonical store, not per selected screen. Registration is
/// not authorization: the caller must first resolve the owner from Store.
/// Different conversations execute independently; one conversation is serial.
#[derive(Debug, Default)]
pub struct SessionRuntimeRegistry {
    runs: BTreeMap<String, (ConversationOwner, String)>,
}

impl SessionRuntimeRegistry {
    pub fn register(
        &mut self,
        owner: ConversationOwner,
        run_id: String,
    ) -> Result<(), RunRegistrationError> {
        validate_identity(&run_id).map_err(|_| RunRegistrationError::InvalidRunId)?;
        if let Some((stored, _)) = self.runs.get(owner.conversation_id()) {
            return Err(if stored != &owner {
                RunRegistrationError::OwnerMismatch
            } else {
                RunRegistrationError::ConversationBusy
            });
        }
        self.runs
            .insert(owner.conversation_id().into(), (owner, run_id));
        Ok(())
    }

    pub fn is_current(&self, owner: &ConversationOwner, run_id: &str) -> bool {
        self.runs
            .get(owner.conversation_id())
            .is_some_and(|(stored, run)| stored == owner && run == run_id)
    }

    /// Late terminal events cannot remove a newer run or another Agent's run.
    pub fn finish(&mut self, owner: &ConversationOwner, run_id: &str) -> bool {
        if !self.is_current(owner, run_id) {
            return false;
        }
        self.runs.remove(owner.conversation_id());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn owner(agent: &str, conversation: &str) -> ConversationOwner {
        ConversationOwner::new(
            AgentId::try_from(agent.to_string()).unwrap(),
            conversation.into(),
        )
        .unwrap()
    }

    #[test]
    fn independent_sessions_and_stale_events() {
        let mut registry = SessionRuntimeRegistry::default();
        let a = owner("a", "one");
        let b = owner("b", "two");
        registry.register(a.clone(), "r1".into()).unwrap();
        registry.register(b.clone(), "r2".into()).unwrap();
        assert_eq!(
            registry.register(a.clone(), "r3".into()),
            Err(RunRegistrationError::ConversationBusy)
        );
        assert!(registry.finish(&a, "r1"));
        registry.register(a.clone(), "r3".into()).unwrap();
        assert!(!registry.finish(&a, "r1"));
        assert!(registry.is_current(&a, "r3"));
        assert!(registry.is_current(&b, "r2"));
    }

    #[test]
    fn spoofed_owner_cannot_replace_or_finish_run() {
        let mut registry = SessionRuntimeRegistry::default();
        let a = owner("a", "one");
        let impostor = owner("b", "one");
        registry.register(a.clone(), "r1".into()).unwrap();
        assert_eq!(
            registry.register(impostor.clone(), "r1".into()),
            Err(RunRegistrationError::OwnerMismatch)
        );
        assert!(!registry.finish(&impostor, "r1"));
        assert!(registry.is_current(&a, "r1"));
    }

    #[test]
    fn defaults_cannot_copy_session_data_or_authorization() {
        for field in [
            "knowledge",
            "externalExecutionSessions",
            "permissionProfile",
            "apiKey",
            "agent_id",
        ] {
            assert!(
                AgentDefaults::try_from(json!({field: "value"})).is_err(),
                "{field}"
            );
        }
        let defaults = AgentDefaults::try_from(
            json!({"language":"zh", "modelSelectionID":"configured:model"}),
        )
        .unwrap();
        let mut snapshot = defaults.conversation_configuration();
        snapshot["language"] = json!("en");
        assert_eq!(defaults.conversation_configuration()["language"], "zh");
    }

    #[test]
    fn wire_decode_does_not_bypass_validation() {
        assert!(serde_json::from_value::<AgentId>(json!(" ")).is_err());
        assert!(serde_json::from_value::<ConversationOwner>(
            json!({"agent_id":"a", "conversation_id":""})
        )
        .is_err());
        assert!(serde_json::from_value::<PersistentAgent>(json!({
            "id":"a", "name":"Assistant", "description":"Research", "revision":0, "availability":"active", "defaults":{}
        }))
        .is_err());
        assert!(serde_json::from_value::<AgentDefaults>(json!({"modelSelectionID":42})).is_err());
    }

    #[test]
    fn public_description_is_bounded_and_required_on_wire() {
        assert!(validate_agent_description("研究资料\nCompare evidence").is_ok());
        assert!(validate_agent_description(&"x".repeat(4097)).is_err());
        assert!(validate_agent_description("invalid\0text").is_err());
        let mut value = json!({"id":"a", "name":"Assistant", "description":"Research",
            "revision":1, "availability":"active", "defaults":{}});
        assert_eq!(serde_json::from_value::<PersistentAgent>(value.clone()).unwrap().description(), "Research");
        value.as_object_mut().unwrap().remove("description");
        assert!(serde_json::from_value::<PersistentAgent>(value).is_err());
    }
}
