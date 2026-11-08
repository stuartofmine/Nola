// SPDX-License-Identifier: Apache-2.0
//! Workspace-owned persistent assistant directory. This is not the execution
//! worker registry. Writes join the caller-owned command transaction.
use nola_domain::persistent_agent::{AgentAvailability, AgentDefaults, AgentId, PersistentAgent};
use nola_store_port::product_setting::ProductSettingRepository;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const DIRECTORY_KEY: &str = "persistent_agent_directory";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct DirectoryDocument {
    agents: Vec<PersistentAgent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDirectory {
    pub revision: Option<i64>,
    pub agents: Vec<PersistentAgent>,
}

/// Absence means no agents have been created, not an implicit default owner.
pub fn load<R: ProductSettingRepository + ?Sized>(store: &R) -> Result<AgentDirectory, String> {
    let Some(setting) = store
        .product_setting(DIRECTORY_KEY)
        .map_err(|e| e.to_string())?
    else {
        return Ok(AgentDirectory {
            revision: None,
            agents: vec![],
        });
    };
    let document: DirectoryDocument = serde_json::from_str(&setting.value_json)
        .map_err(|e| format!("invalid persistent Agent directory: {e}"))?;
    if setting.revision < 1 {
        return Err("invalid persistent Agent directory revision".into());
    }
    let mut identities = std::collections::BTreeSet::new();
    if document
        .agents
        .iter()
        .any(|agent| !identities.insert(agent.id()))
    {
        return Err("duplicate persistent Agent identity".into());
    }
    Ok(AgentDirectory {
        revision: Some(setting.revision),
        agents: document.agents,
    })
}

/// A command carries both the directory CAS and the target Agent CAS. Revision
/// numbers are produced here, never accepted as a new value from the client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigureAgent {
    pub id: AgentId,
    pub expected_directory_revision: Option<i64>,
    pub expected_agent_revision: Option<i64>,
    pub name: String,
    pub description: String,
    pub defaults: AgentDefaults,
    pub availability: AgentAvailability,
    pub updated_at_ms: i64,
}

pub fn configure<R: ProductSettingRepository + ?Sized>(
    store: &mut R,
    command: ConfigureAgent,
) -> Result<AgentDirectory, String> {
    if !store.has_write_unit_of_work() {
        return Err("Agent configuration requires an admitted write unit of work".into());
    }
    let current = load(store)?;
    if current.revision != command.expected_directory_revision {
        return Err("agent_directory_revision_conflict".into());
    }
    let mut agents: BTreeMap<AgentId, PersistentAgent> = current
        .agents
        .into_iter()
        .map(|agent| (agent.id().clone(), agent))
        .collect();
    let previous = agents.get(&command.id).map(PersistentAgent::revision);
    if previous != command.expected_agent_revision {
        return Err("agent_revision_conflict".into());
    }
    let revision = previous
        .unwrap_or(0)
        .checked_add(1)
        .ok_or("agent revision overflow")?;
    let agent = PersistentAgent::new(
        command.id.clone(),
        command.name,
        command.description,
        revision,
        command.availability,
        command.defaults,
    )?;
    agents.insert(command.id, agent);
    let agents: Vec<_> = agents.into_values().collect();
    let value_json = serde_json::to_string(&DirectoryDocument {
        agents: agents.clone(),
    })
    .map_err(|e| e.to_string())?;
    let committed = store
        .put_product_setting(DIRECTORY_KEY, &value_json, command.updated_at_ms)
        .map_err(|e| e.to_string())?;
    Ok(AgentDirectory {
        revision: Some(committed.revision),
        agents,
    })
}

/// Capture defaults within the same transaction as conversation creation.
/// Caller must persist this origin alongside the new conversation owner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentConversationOrigin {
    pub agent_id: AgentId,
    pub agent_revision: i64,
    pub initial_configuration: serde_json::Value,
}

pub fn capture_defaults<R: ProductSettingRepository + ?Sized>(
    store: &R,
    id: &AgentId,
    expected_revision: i64,
) -> Result<AgentConversationOrigin, String> {
    if !store.has_write_unit_of_work() {
        return Err(
            "Agent defaults must be captured inside conversation creation transaction".into(),
        );
    }
    let agent = load(store)?
        .agents
        .into_iter()
        .find(|agent| agent.id() == id)
        .ok_or("agent_not_found")?;
    if agent.revision() != expected_revision {
        return Err("agent_revision_conflict".into());
    }
    if agent.availability() != &AgentAvailability::Active {
        return Err("agent_archived".into());
    }
    Ok(AgentConversationOrigin {
        agent_id: id.clone(),
        agent_revision: agent.revision(),
        initial_configuration: agent.defaults().conversation_configuration(),
    })
}
