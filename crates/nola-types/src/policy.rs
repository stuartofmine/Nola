// SPDX-License-Identifier: Apache-2.0
//! Unified permission rules, profiles, and lifecycle hook contracts.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::CapabilityDomain;

/// User-selectable permission profile.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionProfile {
    /// Ask before writes, process execution, or network egress.
    #[default]
    Cautious,
    /// Auto-allow workspace-scoped file edits; still guard shell & outside-workspace writes.
    AcceptEdits,
    /// Allow all operations inside the session (used by `--yes` / benchmarks / containers).
    AllowAll,
}

impl PermissionProfile {
    /// Legacy/lenient parser accepting aliases (preserved for transitional callers).
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "cautious" | "ask" | "default" => Some(Self::Cautious),
            "accept_edits" | "accept-edits" => Some(Self::AcceptEdits),
            "allow_all" | "allow-all" | "yolo" => Some(Self::AllowAll),
            _ => None,
        }
    }

    /// Strict canonical parser accepting ONLY `"cautious"`, `"accept_edits"`, and `"allow_all"`.
    /// Rejects unknown strings, case/punctuation variants, and legacy aliases.
    pub fn parse_strict(raw: &str) -> Result<Self, String> {
        match raw.trim() {
            "cautious" => Ok(Self::Cautious),
            "accept_edits" => Ok(Self::AcceptEdits),
            "allow_all" => Ok(Self::AllowAll),
            other => Err(format!(
                "invalid permission profile '{other}': expected strictly 'cautious', 'accept_edits', or 'allow_all'"
            )),
        }
    }

    /// Returns true if this profile is one of the strict canonical profiles.
    pub fn is_strict_canonical(self) -> bool {
        matches!(self, Self::Cautious | Self::AcceptEdits | Self::AllowAll)
    }

    /// Returns the strict canonical wire name (`"cautious"`, `"accept_edits"`, or `"allow_all"`).
    pub fn as_canonical_str(self) -> &'static str {
        match self {
            Self::Cautious => "cautious",
            Self::AcceptEdits => "accept_edits",
            Self::AllowAll => "allow_all",
        }
    }
}

/// Permission evaluation action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionAction {
    Allow,
    Ask,
    Deny,
}

/// Source precedence of a permission rule (user Deny ALWAYS overrides project Allow).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleScope {
    Default,
    Project,
    ProjectLocal,
    User,
    CliOverride,
}

/// Single normalized permission rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRule {
    pub scope: RuleScope,
    pub action: PermissionAction,
    /// Matches either a tool name (`bash`, `write_file`, `*`) or a `CapabilityDomain`.
    pub tool_pattern: String,
    /// Optional glob matching canonicalized resource path or normalized command string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_pattern: Option<String>,
}

/// Normalized permission evaluation decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionDecision {
    pub action: PermissionAction,
    pub reason: String,
    pub domain: CapabilityDomain,
}

/// Lifecycle hook trigger points wired into `ToolExecutionMiddleware`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    SessionStart,
    Stop,
}

/// Configured command hook entry loaded from `~/.nola/hooks.json` or trusted `<ws>/.nola/hooks.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandHookConfig {
    pub event: HookEvent,
    /// Tool name glob (e.g. `"*"`, `"bash"`, `"write_file|edit_file"`).
    #[serde(default = "default_star")]
    pub matcher: String,
    pub command: String,
    #[serde(default = "default_hook_timeout_ms")]
    pub timeout_ms: u64,
    /// When true, spawn failure / timeout / non-zero exit blocks execution instead of failing open.
    #[serde(default)]
    pub fail_closed: bool,
}

fn default_star() -> String {
    "*".to_string()
}

fn default_hook_timeout_ms() -> u64 {
    10_000
}

/// Effective decision returned by a hook execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum HookDecision {
    Allow,
    Block { reason: String },
    ModifyInput { input: Value },
    InjectContext { text: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_profile_default_and_strict_parser() {
        assert_eq!(PermissionProfile::default(), PermissionProfile::Cautious);
        assert_eq!(
            PermissionProfile::parse_strict("cautious").unwrap(),
            PermissionProfile::Cautious
        );
        assert_eq!(
            PermissionProfile::parse_strict("accept_edits").unwrap(),
            PermissionProfile::AcceptEdits
        );
        assert_eq!(
            PermissionProfile::parse_strict("allow_all").unwrap(),
            PermissionProfile::AllowAll
        );
        for rejected in [
            "",
            "ask",
            "default",
            "accept-edits",
            "ACCEPT_EDITS",
            "allow-all",
            "yolo",
            "CAUTIOUS",
            "ALLOW_ALL",
            "unknown",
        ] {
            assert!(
                PermissionProfile::parse_strict(rejected).is_err(),
                "parse_strict must reject '{rejected}'"
            );
        }
        assert!(PermissionProfile::Cautious.is_strict_canonical());
        assert!(PermissionProfile::AcceptEdits.is_strict_canonical());
        assert!(PermissionProfile::AllowAll.is_strict_canonical());
        assert_eq!(PermissionProfile::Cautious.as_canonical_str(), "cautious");
        assert_eq!(
            PermissionProfile::AcceptEdits.as_canonical_str(),
            "accept_edits"
        );
        assert_eq!(PermissionProfile::AllowAll.as_canonical_str(), "allow_all");
        assert_eq!(
            PermissionProfile::parse("accept-edits"),
            Some(PermissionProfile::AcceptEdits)
        );
        assert_eq!(
            PermissionProfile::parse("accept_edits"),
            Some(PermissionProfile::AcceptEdits)
        );
    }
}
