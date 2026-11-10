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

fn matching_resource_rule<'a>(rules: &'a [PermissionRule], tool: &str, pattern: &str) -> Option<&'a PermissionRule> {
    let permission = permission_name(tool);
    rules
        .iter()
        .filter(|rule| {
            wildcard_match(&permission, &rule.permission)
                || wildcard_match(tool, &rule.permission)
        })
        .filter(|rule| wildcard_match(pattern, &rule.pattern))
        .last()
}

fn permission_name(tool_name: &str) -> String {
    nola_core::agent::policy::permission_name_for_tool(tool_name)
}

fn request_pattern(request: &ToolRequest) -> String {
    match request.name.as_str() {
        "bash" | "bash_background_start" | "bash_command_review" => {
            string_arg(&request.args, "command")
        }
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
}
