// SPDX-License-Identifier: Apache-2.0
//! Non-secret desktop launch facts inside the original Run invocation, not a
//! second execution authority. Current permissions and credentials are rechecked.
use serde::{Deserialize, Serialize};

/// Frozen launch provenance used to reconstruct recovery restrictions, never
/// authority to submit a new turn. The original Run/Core remains authoritative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostMacInputOrigin {
    InteractiveUser,
    ScheduledAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostMacLaunchDescriptor {
    pub version: u16,
    pub input_origin: HostMacInputOrigin,
    pub project_path: String,
    pub authorized_workspace_paths: Vec<String>,
    pub allowed_directory_arguments: Vec<String>,
    pub skill_directory_arguments: Vec<String>,
    pub active_skill_ids: Vec<String>,
    pub model_profile_id: Option<String>,
    /// Exact canonical profile revision captured at launch; never refreshed on resume.
    #[serde(deserialize_with = "required_profile_revision")]
    pub model_profile_revision: Option<i64>,
    pub fallback_profile_ids: Vec<String>,
    pub permission_mode: String,
    pub memory_enabled: bool,
    pub context_window_tokens: Option<u64>,
    pub max_iterations: u64,
    pub model_capabilities: std::collections::BTreeMap<String, bool>,
    pub vision_input: bool,
    pub permission_rules: Vec<nola_domain::PermissionRule>,
    pub host_attachments: Vec<HostMacLaunchAttachment>,
}

fn required_profile_revision<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<i64>, D::Error> {
    Option::<i64>::deserialize(deserializer)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostMacLaunchAttachment {
    pub path: String,
    pub name: String,
    pub mime_type: String,
    pub byte_count: u64,
}

impl HostMacLaunchDescriptor {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != 1 || !self.project_path.starts_with('/')
            || self.model_profile_revision.is_some_and(|revision| revision <= 0 || self.model_profile_id.as_deref().is_none_or(|id| id.trim().is_empty()))
            || !matches!(self.permission_mode.as_str(), "cautious" | "allow_all")
            || self.fallback_profile_ids.len() > 16 || self.host_attachments.len() > 32
            || self.authorized_workspace_paths.len() > 256 || self.active_skill_ids.len() > 256
            || self.allowed_directory_arguments.len() > 512 || self.skill_directory_arguments.len() > 512
            || self.permission_rules.len() > 1024
            || self.authorized_workspace_paths.iter().any(|p| !p.starts_with('/'))
            || self.model_capabilities.keys().any(|key| !matches!(key.as_str(),
                "native_tool_calls" | "stream_tool_calls" | "vision_input" | "reasoning" | "stream_reasoning" | "reasoning_continuity"))
            || self.host_attachments.iter().any(|a| !a.path.starts_with('/') || a.name.is_empty() || a.mime_type.is_empty())
        {
            return Err("invalid Mac frozen launch descriptor");
        }
        for (arguments, flag) in [(&self.allowed_directory_arguments, "--allow-dir"), (&self.skill_directory_arguments, "--skill-dir")] {
            if flag == "--skill-dir" && arguments.as_slice() == ["--no-skills"] { continue; }
            if arguments.len() % 2 != 0 || arguments.chunks_exact(2).any(|pair| pair[0] != flag || !pair[1].starts_with('/')) {
                return Err("invalid Mac frozen launch path arguments");
            }
        }
        if serde_json::to_vec(self).map_err(|_| "invalid Mac launch encoding")?.len() > 131_072 {
            return Err("Mac frozen launch descriptor exceeds bound");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_launch_is_closed_bounded_and_has_no_secret_or_prompt_channel() {
        let input = serde_json::json!({"version":1,"input_origin":"interactive_user","project_path":"/workspace",
            "authorized_workspace_paths":["/workspace"],"allowed_directory_arguments":["--allow-dir","/workspace"],
            "skill_directory_arguments":["--no-skills"],"active_skill_ids":[],"model_profile_id":"profile","model_profile_revision":7,
            "fallback_profile_ids":[],"permission_mode":"cautious","memory_enabled":false,
            "context_window_tokens":4096,"max_iterations":0,"model_capabilities":{},"vision_input":false,
            "permission_rules":[],"host_attachments":[]});
        let descriptor: HostMacLaunchDescriptor = serde_json::from_value(input.clone()).unwrap();
        descriptor.validate().unwrap();
        for origin in ["interactive_user", "scheduled_action"] {
            let mut valid = input.clone();
            valid["input_origin"] = serde_json::json!(origin);
            let parsed: HostMacLaunchDescriptor = serde_json::from_value(valid.clone()).unwrap();
            parsed.validate().unwrap();
            assert_eq!(serde_json::to_value(parsed).unwrap()["input_origin"], valid["input_origin"]);
        }
        let mut missing = input.clone();
        missing.as_object_mut().unwrap().remove("input_origin");
        assert!(serde_json::from_value::<HostMacLaunchDescriptor>(missing).is_err());
        let mut missing_revision = input.clone();
        missing_revision.as_object_mut().unwrap().remove("model_profile_revision");
        assert!(serde_json::from_value::<HostMacLaunchDescriptor>(missing_revision).is_err());
        let mut explicit_null = input.clone();
        explicit_null["model_profile_revision"] = serde_json::Value::Null;
        serde_json::from_value::<HostMacLaunchDescriptor>(explicit_null).unwrap().validate().unwrap();
        for revision in [0, -1] {
            let mut invalid = descriptor.clone();
            invalid.model_profile_revision = Some(revision);
            assert!(invalid.validate().is_err());
        }
        for origin in ["unknown", "system_recovery", "delegated_agent", "goal_continuation", ""] {
            let mut invalid = input.clone();
            invalid["input_origin"] = serde_json::json!(origin);
            assert!(serde_json::from_value::<HostMacLaunchDescriptor>(invalid).is_err());
        }
        for key in ["api_key", "prompt", "context_components", "environment", "mcp_servers"] {
            let mut invalid = input.clone();
            invalid[key] = serde_json::json!("not-a-launch-fact");
            assert!(serde_json::from_value::<HostMacLaunchDescriptor>(invalid).is_err());
        }
        let mut invalid = descriptor.clone();
        invalid.allowed_directory_arguments = vec!["--api-key".into(), "/secret".into()];
        assert!(invalid.validate().is_err());
        invalid = descriptor.clone();
        invalid.model_profile_id = Some("x".repeat(131_072));
        assert!(invalid.validate().is_err());
        invalid = descriptor;
        invalid.authorized_workspace_paths = vec!["relative".into()];
        assert!(invalid.validate().is_err());
    }
}
