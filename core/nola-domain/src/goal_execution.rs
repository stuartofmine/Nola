// SPDX-License-Identifier: Apache-2.0
//! Goal-owned execution identity; never credentials or current defaults.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GoalModelSelection {
    Profile { profile_id: String, revision: i64 },
    DirectRoute,
}
impl GoalModelSelection {
    pub fn validate(&self) -> Result<(), String> {
        if let Self::Profile {
            profile_id,
            revision,
        } = self
        {
            if profile_id.trim().is_empty() || profile_id.trim() != profile_id || *revision <= 0 {
                return Err("Goal model profile requires an exact identity and revision".into());
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalExecutionBindingIdentity {
    pub conversation_id: String,
    pub goal_id: String,
    pub operation_id: String,
    pub source_host_id: String,
}
impl GoalExecutionBindingIdentity {
    pub fn validate(&self) -> Result<(), String> {
        for value in [
            &self.conversation_id,
            &self.goal_id,
            &self.operation_id,
            &self.source_host_id,
        ] {
            if value.is_empty()
                || value.trim() != value
                || value.len() > 512
                || value.chars().any(char::is_control)
            {
                return Err("invalid Goal execution binding identity".into());
            }
        }
        Ok(())
    }
}
