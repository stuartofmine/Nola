use std::collections::{BTreeMap, HashSet};

use serde_json::Value;

use crate::runtime_config::RuntimeAgent;
use crate::tools::tool_names_are_read_only;

pub use nola_domain::delegation::AgentProfile;

#[derive(Debug, Clone)]
pub struct AgentRegistry {
    agents: BTreeMap<String, AgentProfile>,
}

impl AgentRegistry {
    pub(crate) fn frozen(profile: AgentProfile) -> Self {
        Self { agents: BTreeMap::from([(profile.name.clone(), profile)]) }
    }
    pub fn from_profiles(profiles: &BTreeMap<String, RuntimeAgent>) -> Self {
        Self { agents: nola_domain::delegation::effective_agent_profiles(profiles) }
    }

    pub fn get(&self, name: &str) -> Option<&AgentProfile> {
        self.agents.get(name)
    }

    pub fn names(&self) -> Vec<String> {
        self.agents.keys().cloned().collect()
    }

    pub fn tool_schemas_for(&self, agent_type: &str, all_tools: &[Value]) -> Vec<Value> {
        let Some(profile) = self.get(agent_type) else {
            return Vec::new();
        };
        if profile.tools.is_empty() {
            return all_tools
                .iter()
                .filter(|tool| {
                    tool_name(tool)
                        .map(|name| !is_parent_only_tool(name))
                        .unwrap_or(false)
                })
                .cloned()
                .collect();
        }
        let names = profile.tools.iter().cloned().collect::<HashSet<_>>();
        all_tools
            .iter()
            .filter(|tool| {
                tool_name(tool)
                    .map(|name| !is_parent_only_tool(name) && names.contains(name))
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    }

    pub fn can_mutate_or_execute(&self, agent_type: &str) -> bool {
        let Some(profile) = self.get(agent_type) else {
            return false;
        };
        if profile.tools.is_empty() {
            return true;
        }
        !tool_names_are_read_only(&profile.tools)
    }
}

fn is_parent_only_tool(name: &str) -> bool {
    matches!(
        name,
        "spawn_agent"
            | "list_tasks"
            | "read_task"
            | "wait_task"
            | "stop_task"
            | "send_agent_message"
            | "request_user_input"
    )
}

fn tool_name(tool: &Value) -> Option<&str> {
    tool.get("function")
        .and_then(|function| function.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
}

#[cfg(test)]
mod task_execution_capability_tests {
    use super::*;
    #[test]
    fn execution_requires_a_command_capable_profile_not_workspace_file_edits() {
        let registry = AgentRegistry::from_profiles(&BTreeMap::new());
        let tools = ["read_file", "write_file", "bash", "spawn_agent"].map(|name|
            serde_json::json!({"type":"function","function":{"name":name,"parameters":{"type":"object"}}}));
        let names = |profile| registry.tool_schemas_for(profile, &tools).iter()
            .filter_map(tool_name).map(str::to_owned).collect::<Vec<_>>();
        assert_eq!(names("workspace"), vec!["read_file", "write_file"]);
        assert!(names("general").contains(&"bash".to_owned()));
        assert!(!names("general").contains(&"spawn_agent".to_owned()));
        assert!(registry.get("workspace").unwrap().description.contains("Commands/builds require parent"));
    }
}
