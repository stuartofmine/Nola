use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::runtime_config::{PermissionAction, PermissionRule};
use crate::tools::ToolRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    Ask,
    Deny,
}

pub(crate) use nola_domain::PermissionProfile;

pub const ACCEPT_EDITS_AUTO_ALLOW_TOOLS: &[&str] = &[
    "read_file",
    "edit_file",
    "write_file",
    "multi_edit_file",
    "edit_file_lines",
    "apply_patch",
    "apply_patch_file",
    "replace_in_files",
    "replace_symbol_body",
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CliPermissionProfile {
    #[default]
    Cautious,
    AcceptEdits,
    AllowAll,
}

impl CliPermissionProfile {
    pub const ALL: [Self; 3] = [Self::Cautious, Self::AcceptEdits, Self::AllowAll];

    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::Cautious => "cautious",
            Self::AcceptEdits => "accept_edits",
            Self::AllowAll => "allow_all",
        }
    }

    pub fn from_canonical_name(value: &str) -> Option<Self> {
        match value {
            "cautious" => Some(Self::Cautious),
            "accept_edits" => Some(Self::AcceptEdits),
            "allow_all" => Some(Self::AllowAll),
            _ => None,
        }
    }

    pub const fn base_domain_profile(self) -> PermissionProfile {
        match self {
            Self::Cautious | Self::AcceptEdits => PermissionProfile::Cautious,
            Self::AllowAll => PermissionProfile::AllowAll,
        }
    }

    pub const fn network_policy(self) -> &'static str {
        match self {
            Self::Cautious | Self::AcceptEdits => "ask",
            Self::AllowAll => "allow",
        }
    }

    /// Builds workspace-scoped baseline permission rules for the profile.
    /// `accept_edits` auto-allows workspace read/edit tools while keeping
    /// delete, shell, and network operations prompting (`Ask`).
    pub fn default_rules_for_workspace(self, workspace_root: &Path) -> Vec<PermissionRule> {
        match self {
            Self::Cautious => Vec::new(),
            Self::AllowAll => vec![PermissionRule {
                permission: "*".to_string(),
                pattern: "*".to_string(),
                action: PermissionAction::Allow,
            }],
            Self::AcceptEdits => {
                let root = workspace_root
                    .to_string_lossy()
                    .trim_end_matches(['/', '\\'])
                    .to_string();
                let pattern = if root.is_empty() {
                    "*".to_string()
                } else {
                    format!("{root}/*")
                };
                ACCEPT_EDITS_AUTO_ALLOW_TOOLS
                    .iter()
                    .map(|tool| PermissionRule {
                        permission: (*tool).to_string(),
                        pattern: pattern.clone(),
                        action: PermissionAction::Allow,
                    })
                    .collect()
            }
        }
    }
}

pub fn evaluate_tool_request_with_profile(
    profile: CliPermissionProfile,
    rules: &[PermissionRule],
    request: &ToolRequest,
    workspace_root: Option<&Path>,
) -> PermissionDecision {
    if let Some(rule) = matching_rule(rules, request) {
        return match rule.action {
            PermissionAction::Allow => PermissionDecision::Allow,
            PermissionAction::Ask => PermissionDecision::Ask,
            PermissionAction::Deny => PermissionDecision::Deny,
        };
    }
    match profile {
        CliPermissionProfile::Cautious => PermissionDecision::Ask,
        CliPermissionProfile::AllowAll => PermissionDecision::Allow,
        CliPermissionProfile::AcceptEdits => {
            let Some(root) = workspace_root else {
                return PermissionDecision::Ask;
            };
            if is_accept_edits_allowed_tool(&request.name)
                && request_target_within_workspace(request, root)
            {
                PermissionDecision::Allow
            } else {
                PermissionDecision::Ask
            }
        }
    }
}

fn is_accept_edits_allowed_tool(tool_name: &str) -> bool {
    ACCEPT_EDITS_AUTO_ALLOW_TOOLS.contains(&tool_name)
}

fn path_within_workspace(raw_path: &str, workspace_root: &Path) -> bool {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() || trimmed == "*" {
        return false;
    }
    let candidate = Path::new(trimmed);
    if candidate
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return false;
    }
    let root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let resolved: PathBuf = if candidate.is_absolute() {
        candidate
            .canonicalize()
            .unwrap_or_else(|_| candidate.to_path_buf())
    } else {
        let joined = root.join(candidate);
        joined.canonicalize().unwrap_or(joined)
    };
    if resolved
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return false;
    }
    resolved == root || resolved.starts_with(&root)
}

fn request_target_within_workspace(request: &ToolRequest, workspace_root: &Path) -> bool {
    let target = request_pattern(request);
    if !path_within_workspace(&target, workspace_root) {
        return false;
    }
    if request.name == "apply_patch_file" {
        if let Some(patch) = request.args.get("patch").and_then(Value::as_str) {
            for line in patch.lines() {
                if let Some(raw) = line.strip_prefix("--- ").or_else(|| line.strip_prefix("+++ ")) {
                    let cleaned = raw
                        .split('\t')
                        .next()
                        .unwrap_or(raw)
                        .trim()
                        .strip_prefix("a/")
                        .or_else(|| raw.split('\t').next().unwrap_or(raw).trim().strip_prefix("b/"))
                        .unwrap_or(raw.split('\t').next().unwrap_or(raw).trim());
                    if cleaned != "/dev/null" && !path_within_workspace(cleaned, workspace_root) {
                        return false;
                    }
                }
            }
        }
    }
    true
}

pub fn evaluate_tool_request(
    rules: &[PermissionRule],
    request: &ToolRequest,
) -> PermissionDecision {
    matching_rule(rules, request)
        .map(|rule| match rule.action {
            PermissionAction::Allow => PermissionDecision::Allow,
            PermissionAction::Ask => PermissionDecision::Ask,
            PermissionAction::Deny => PermissionDecision::Deny,
        })
        .unwrap_or(PermissionDecision::Ask)
}

pub fn has_explicit_ask_rule(rules: &[PermissionRule], request: &ToolRequest) -> bool {
    matching_rule(rules, request)
        .map(|rule| matches!(rule.action, PermissionAction::Ask))
        .unwrap_or(false)
}

fn matching_rule<'a>(
    rules: &'a [PermissionRule],
    request: &ToolRequest,
) -> Option<&'a PermissionRule> {
    let pattern = request_pattern(request);
    matching_resource_rule(rules, &request.name, &pattern)
}

pub(crate) fn evaluate_tool_resource(rules: &[PermissionRule], tool: &str, resource: &str) -> Option<PermissionDecision> {
    matching_resource_rule(rules, tool, resource).map(|rule| match rule.action {
        PermissionAction::Allow => PermissionDecision::Allow,
        PermissionAction::Ask => PermissionDecision::Ask,
        PermissionAction::Deny => PermissionDecision::Deny,
    })
}

fn is_shell_permission(permission: &str, tool: &str) -> bool {
    permission == "bash"
        || matches!(
            tool,
            "bash"
                | "bash_background_start"
                | "bash_command_review"
                | "pty_exec"
                | "shell.execute"
        )
}

fn contains_shell_compound_operator(text: &str) -> bool {
    text.contains(';')
        || text.contains("&&")
        || text.contains("||")
        || text.contains('|')
        || text.contains("$(")
        || text.contains('`')
        || text.contains('\n')
        || text.contains('\r')
}

fn matching_resource_rule<'a>(rules: &'a [PermissionRule], tool: &str, pattern: &str) -> Option<&'a PermissionRule> {
    let permission = permission_name(tool);
    let is_shell = is_shell_permission(&permission, tool);
    rules
        .iter()
        .filter(|rule| {
            wildcard_match(&permission, &rule.permission)
                || wildcard_match(tool, &rule.permission)
        })
        .filter(|rule| {
            if is_shell
                && matches!(rule.action, PermissionAction::Allow)
                && rule.pattern != "*"
                && !contains_shell_compound_operator(&rule.pattern)
                && contains_shell_compound_operator(pattern)
            {
                return false;
            }
            wildcard_match(pattern, &rule.pattern)
        })
        .last()
}

fn permission_name(tool_name: &str) -> String {
    match tool_name {
        "pty_exec" | "pty_write" | "pty_close" => "bash".to_string(),
        _ => nola_core::agent::policy::permission_name_for_tool(tool_name),
    }
}

fn request_pattern(request: &ToolRequest) -> String {
    match request.name.as_str() {
        "bash" | "bash_background_start" | "bash_command_review" | "pty_exec" => {
            string_arg(&request.args, "command")
        }
        "apply_patch" | "apply_patch_file" => first_string_arg(
            &request.args,
            &["path", "destination", "file_path", "root", "cwd"],
        ),
        name if is_read_permission_tool(name) => first_string_arg(
            &request.args,
            &[
                "path",
                "root",
                "cwd",
                "source",
                "destination",
                "query",
                "pattern",
                "symbol",
                "name",
            ],
        ),
        name if is_edit_permission_tool(name) => first_string_arg(
            &request.args,
            // For copy/move-style operations the destination is the security
            // boundary. Checking the source first could allow a write outside
            // an approved tree merely because the input lives inside it.
            &["destination", "path", "root", "cwd", "source", "glob"],
        ),
        "web_fetch" => string_arg(&request.args, "url"),
        "web_search" => string_arg(&request.args, "query"),
        "chrome_native_open_page" => string_arg(&request.args, "url"),
        name if nola_core::agent::policy::capability_for_tool_name(name)
            == nola_core::agent::policy::HostCapability::Network =>
        {
            first_string_arg(&request.args, &["url", "query", "domain", "host", "path"])
        }
        _ => "*".to_string(),
    }
}

fn is_read_permission_tool(tool_name: &str) -> bool {
    nola_core::agent::policy::capability_for_tool_name(tool_name)
        == nola_core::agent::policy::HostCapability::FilesRead
}

fn is_edit_permission_tool(tool_name: &str) -> bool {
    nola_core::agent::policy::capability_for_tool_name(tool_name)
        == nola_core::agent::policy::HostCapability::FilesWrite
}

fn first_string_arg(args: &serde_json::Map<String, Value>, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| {
            args.get(*key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or("*")
        .to_string()
}

fn string_arg(args: &serde_json::Map<String, Value>, key: &str) -> String {
    first_string_arg(args, &[key])
}

fn wildcard_match(value: &str, pattern: &str) -> bool {
    let value = value.to_ascii_lowercase();
    let pattern = pattern.to_ascii_lowercase();
    wildcard_match_bytes(value.as_bytes(), pattern.as_bytes())
}

fn wildcard_match_bytes(value: &[u8], pattern: &[u8]) -> bool {
    let (mut vi, mut pi) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut star_value = 0usize;
    while vi < value.len() {
        if pi < pattern.len() && (pattern[pi] == b'?' || pattern[pi] == value[vi]) {
            vi += 1;
            pi += 1;
        } else if pi < pattern.len() && pattern[pi] == b'*' {
            star = Some(pi);
            star_value = vi;
            pi += 1;
        } else if let Some(star_index) = star {
            pi = star_index + 1;
            star_value += 1;
            vi = star_value;
        } else {
            return false;
        }
    }
    while pi < pattern.len() && pattern[pi] == b'*' {
        pi += 1;
    }
    pi == pattern.len()
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Map};

    use super::*;

    #[test]
    fn permission_profiles_have_two_strict_wire_values_and_cautious_default() {
        assert_eq!(
            PermissionProfile::ALL.map(PermissionProfile::canonical_name),
            ["cautious", "allow_all"]
        );
        assert_eq!(
            PermissionProfile::from_canonical_name("moderate"),
            None,
            "wire aliases would let platform adapters drift"
        );
        for invalid in ["safe", "recommended", "moderate", "allow-all", "allowall", "AllowAll", " cautious ", "", "unknown"] {
            assert_eq!(PermissionProfile::from_canonical_name(invalid), None);
        }
        assert_eq!(
            PermissionProfile::default(),
            PermissionProfile::Cautious
        );
        assert!(PermissionProfile::Cautious.auto_approve());
        assert_eq!(PermissionProfile::Cautious.network_policy(), "ask");
        assert_eq!(PermissionProfile::AllowAll.network_policy(), "allow");
    }

    #[test]
    fn evaluates_last_matching_rule() {
        let rules = vec![
            PermissionRule {
                permission: "bash".to_string(),
                pattern: "*".to_string(),
                action: PermissionAction::Ask,
            },
            PermissionRule {
                permission: "bash".to_string(),
                pattern: "cargo test*".to_string(),
                action: PermissionAction::Allow,
            },
        ];
        let args: Map<String, Value> = json!({"command": "cargo test --quiet"})
            .as_object()
            .unwrap()
            .clone();
        let request = ToolRequest {
            id: "1".to_string(),
            name: "bash".to_string(),
            args,
        };
        assert_eq!(
            evaluate_tool_request(&rules, &request),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn background_start_uses_bash_command_rules() {
        let rules = vec![PermissionRule {
            permission: "bash".to_string(),
            pattern: "*npm install*".to_string(),
            action: PermissionAction::Deny,
        }];
        let args: Map<String, Value> = json!({
            "command": "cd /workspace && npm install"
        })
        .as_object()
        .unwrap()
        .clone();
        let request = ToolRequest {
            id: "background-1".to_string(),
            name: "bash_background_start".to_string(),
            args,
        };

        assert_eq!(
            evaluate_tool_request(&rules, &request),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn read_permission_covers_advanced_repository_tools_and_root_patterns() {
        let rules = vec![PermissionRule {
            permission: "read".to_string(),
            pattern: "/workspace/src/*".to_string(),
            action: PermissionAction::Deny,
        }];
        for name in [
            "repo_map",
            "code_search",
            "git_history",
            "workspace_symbols",
            "tool_output_read",
        ] {
            let args = json!({"root": "/workspace/src/project"})
                .as_object()
                .unwrap()
                .clone();
            let request = ToolRequest {
                id: format!("read-{name}"),
                name: name.to_string(),
                args,
            };
            assert_eq!(
                evaluate_tool_request(&rules, &request),
                PermissionDecision::Deny,
                "{name} should inherit the read permission family"
            );
        }
    }

    #[test]
    fn format_files_inherits_edit_permission() {
        let rules = vec![PermissionRule {
            permission: "edit".to_string(),
            pattern: "/workspace/*".to_string(),
            action: PermissionAction::Ask,
        }];
        let request = ToolRequest {
            id: "format-1".to_string(),
            name: "format_files".to_string(),
            args: json!({"root": "/workspace/project"})
                .as_object()
                .unwrap()
                .clone(),
        };

        assert_eq!(
            evaluate_tool_request(&rules, &request),
            PermissionDecision::Ask
        );
    }

    #[test]
    fn copy_style_edits_are_authorized_against_the_destination() {
        let rules = vec![PermissionRule {
            permission: "edit".to_string(),
            pattern: "/outside/*".to_string(),
            action: PermissionAction::Deny,
        }];
        let request = ToolRequest {
            id: "copy-outside".to_string(),
            name: "move_file".to_string(),
            args: json!({
                "source": "/workspace/cautious.txt",
                "destination": "/outside/copied.txt"
            })
            .as_object()
            .unwrap()
            .clone(),
        };

        assert_eq!(
            evaluate_tool_request(&rules, &request),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn web_search_permissions_match_the_original_query() {
        let rules = vec![PermissionRule {
            permission: "web_search".to_string(),
            pattern: "*Iran*".to_string(),
            action: PermissionAction::Deny,
        }];
        let request = ToolRequest {
            id: "search-sensitive-topic".to_string(),
            name: "web_search".to_string(),
            args: json!({"query": "2026 July Iran situation"})
                .as_object()
                .unwrap()
                .clone(),
        };

        assert_eq!(
            evaluate_tool_request(&rules, &request),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn exact_tool_rule_can_override_permission_family() {
        let rules = vec![
            PermissionRule {
                permission: "read".to_string(),
                pattern: "*".to_string(),
                action: PermissionAction::Deny,
            },
            PermissionRule {
                permission: "repo_map".to_string(),
                pattern: "/workspace/cautious/*".to_string(),
                action: PermissionAction::Allow,
            },
        ];
        let request = ToolRequest {
            id: "repo-map-override".to_string(),
            name: "repo_map".to_string(),
            args: json!({"root": "/workspace/cautious/project"})
                .as_object()
                .unwrap()
                .clone(),
        };

        assert_eq!(
            evaluate_tool_request(&rules, &request),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn pty_exec_inherits_bash_allow_and_deny_rules() {
        let rules = vec![
            PermissionRule {
                permission: "bash".to_string(),
                pattern: "git status*".to_string(),
                action: PermissionAction::Allow,
            },
            PermissionRule {
                permission: "bash".to_string(),
                pattern: "*rm -rf*".to_string(),
                action: PermissionAction::Deny,
            },
        ];
        let allowed_req = ToolRequest {
            id: "pty-1".to_string(),
            name: "pty_exec".to_string(),
            args: json!({"command": "git status --short"})
                .as_object()
                .unwrap()
                .clone(),
        };
        let denied_req = ToolRequest {
            id: "pty-2".to_string(),
            name: "pty_exec".to_string(),
            args: json!({"command": "rm -rf /tmp/demo"})
                .as_object()
                .unwrap()
                .clone(),
        };
        assert_eq!(
            evaluate_tool_request(&rules, &allowed_req),
            PermissionDecision::Allow
        );
        assert_eq!(
            evaluate_tool_request(&rules, &denied_req),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn shell_allow_glob_rejects_compound_operators_while_deny_still_blocks() {
        let rules = vec![PermissionRule {
            permission: "bash".to_string(),
            pattern: "git status*".to_string(),
            action: PermissionAction::Allow,
        }];
        let req = |tool: &str, command: &str| ToolRequest {
            id: "cmd".to_string(),
            name: tool.to_string(),
            args: json!({"command": command}).as_object().unwrap().clone(),
        };

        assert_eq!(
            evaluate_tool_request(&rules, &req("bash", "git status --short")),
            PermissionDecision::Allow
        );
        for chained in [
            "git status && rm -rf ~",
            "git status; rm -rf ~",
            "git status || evil",
            "git status | sh",
            "git status $(whoami)",
            "git status `whoami`",
            "git status\nrm -rf ~",
        ] {
            assert_eq!(
                evaluate_tool_request(&rules, &req("bash", chained)),
                PermissionDecision::Ask,
                "bash allow glob must reject compound command: {chained}"
            );
            assert_eq!(
                evaluate_tool_request(&rules, &req("pty_exec", chained)),
                PermissionDecision::Ask,
                "pty_exec allow glob must reject compound command: {chained}"
            );
        }
    }

    #[test]
    fn accept_edits_profile_auto_allows_workspace_edits_and_prompts_for_sensitive_ops() {
        assert_eq!(
            CliPermissionProfile::ALL.map(CliPermissionProfile::canonical_name),
            ["cautious", "accept_edits", "allow_all"]
        );
        assert_eq!(
            CliPermissionProfile::from_canonical_name("accept_edits"),
            Some(CliPermissionProfile::AcceptEdits)
        );
        assert_eq!(CliPermissionProfile::AcceptEdits.network_policy(), "ask");

        let ws = Path::new("/workspace");
        let rules: Vec<PermissionRule> = Vec::new();
        let req = |name: &str, args: Value| ToolRequest {
            id: format!("req-{name}"),
            name: name.to_string(),
            args: args.as_object().unwrap().clone(),
        };

        for tool in ACCEPT_EDITS_AUTO_ALLOW_TOOLS {
            assert_eq!(
                evaluate_tool_request_with_profile(
                    CliPermissionProfile::AcceptEdits,
                    &rules,
                    &req(tool, json!({"path": "/workspace/src/lib.rs"})),
                    Some(ws),
                ),
                PermissionDecision::Allow,
                "{tool} inside workspace should be auto-allowed under accept_edits"
            );
        }

        // Outside workspace or path traversal must still prompt.
        for outside in [
            "/etc/passwd",
            "/workspace-other/src/lib.rs",
            "/workspace/../etc/passwd",
            "../outside.rs",
        ] {
            assert_eq!(
                evaluate_tool_request_with_profile(
                    CliPermissionProfile::AcceptEdits,
                    &rules,
                    &req("edit_file", json!({"path": outside})),
                    Some(ws),
                ),
                PermissionDecision::Ask,
                "edit outside workspace ({outside}) must still prompt under accept_edits"
            );
        }

        // Delete, shell, PTY, and network tools must still prompt under accept_edits.
        for (tool, args) in [
            ("delete_file", json!({"path": "/workspace/src/lib.rs"})),
            ("bash", json!({"command": "cargo test"})),
            ("pty_exec", json!({"command": "cargo test"})),
            ("web_fetch", json!({"url": "https://example.com"})),
            ("web_search", json!({"query": "rust"})),
        ] {
            assert_eq!(
                evaluate_tool_request_with_profile(
                    CliPermissionProfile::AcceptEdits,
                    &rules,
                    &req(tool, args),
                    Some(ws),
                ),
                PermissionDecision::Ask,
                "{tool} must still prompt under accept_edits"
            );
        }

        // Explicit user deny rules always override accept_edits.
        let deny_env = vec![PermissionRule {
            permission: "edit".to_string(),
            pattern: "/workspace/.env".to_string(),
            action: PermissionAction::Deny,
        }];
        assert_eq!(
            evaluate_tool_request_with_profile(
                CliPermissionProfile::AcceptEdits,
                &deny_env,
                &req("edit_file", json!({"path": "/workspace/.env"})),
                Some(ws),
            ),
            PermissionDecision::Deny
        );
    }
}
