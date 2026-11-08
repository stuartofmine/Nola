// SPDX-License-Identifier: Apache-2.0
//! Exact current Part state contracts shared by execution and public history.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPartState {
    /// Presentation closed without an authoritative execution outcome.
    Sealed,
    Pending,
    WaitingForPermission,
    WaitingForInput,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl ToolPartState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Sealed | Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Started,
    Completed,
    Failed,
    Cancelled,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tool_wire_and_terminal_rules_remain_exact() {
        for (wire, terminal) in [("sealed",true),("pending",false),("waiting_for_permission",false),
            ("waiting_for_input",false),("running",false),("completed",true),("failed",true),("cancelled",true)] {
            let value:ToolPartState=serde_json::from_value(serde_json::json!(wire)).unwrap();
            assert_eq!(serde_json::to_value(value).unwrap(),wire);
            assert_eq!(value.is_terminal(),terminal);
        }
        assert!(serde_json::from_value::<ToolPartState>(serde_json::json!("succeeded")).is_err());
    }
    #[test]
    fn step_wire_does_not_accept_activity_or_tool_states() {
        for wire in ["started","completed","failed","cancelled"] {
            let value:StepStatus=serde_json::from_value(serde_json::json!(wire)).unwrap();
            assert_eq!(serde_json::to_value(value).unwrap(),wire);
        }
        for invalid in ["sealed","waiting_for_input","running","invented"] {
            assert!(serde_json::from_value::<StepStatus>(serde_json::json!(invalid)).is_err());
        }
    }
}
