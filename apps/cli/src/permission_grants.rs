use crate::application_port::CliApplicationPort;
use crate::permissions::CliPermissionProfile;
use crate::runtime_config::{PermissionAction, PermissionRule};
use nola_application::{
    ApplicationCommand, PermissionGrant, PermissionGrantDecision, PermissionGrantInput,
    PermissionGrantTarget,
};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

pub fn add_permission_grant(
    scope: &str,
    tool_name: &str,
    behavior: &str,
    pattern: &str,
    action: &str,
) -> Result<(), String> {
    let scope = required_permission_field(scope, "permission scope")?;
    let tool_name = required_permission_field(tool_name, "permission tool name")?;
    let behavior = required_permission_field(behavior, "permission behavior")?;
    let pattern = required_permission_field(pattern, "permission match pattern")?;
    let action = validated_permission_action(action)?;
    let target = canonical_scope(&scope)?;
    let decision = canonical_decision(&action)?;
    let capability_pattern = canonical_capability(&tool_name, &behavior);
    let now = unix_epoch_millis();
    let mut store = CliApplicationPort::persistent_default()?;
    let _: Value = store.execute(ApplicationCommand::UpsertPermissionGrant {
        input: PermissionGrantInput {
            id: permission_grant_id(&scope, &tool_name, &behavior, &pattern),
            target,
            capability_pattern,
            resource_pattern: pattern,
            decision,
            created_at_ms: now,
            updated_at_ms: now,
        },
    })?;
    Ok(())
}

pub fn permission_grants(
    scope: &str,
    tool_name: &str,
    behavior: &str,
    action: &str,
) -> Result<Vec<String>, String> {
    let scope = required_permission_field(scope, "permission scope")?;
    let tool_name = required_permission_field(tool_name, "permission tool name")?;
    let behavior = required_permission_field(behavior, "permission behavior")?;
    let action = validated_permission_action(action)?;
    let decision = canonical_decision(&action)?;
    let capability = canonical_capability(&tool_name, &behavior);
    Ok(permission_grants_for_scope(&scope)?
        .into_iter()
        .filter(|grant| grant.capability_pattern == capability && grant.decision == decision)
        .map(|grant| grant.resource_pattern)
        .collect())
}

/// Loads every persisted grant visible to one canonical scope in a single
/// Application command. Callers that hydrate several permission categories
/// should filter this snapshot locally instead of reopening the service for
/// each category.
pub(crate) fn permission_grants_for_scope(scope: &str) -> Result<Vec<PermissionGrant>, String> {
    let scope = required_permission_field(scope, "permission scope")?;
    let target = canonical_scope(&scope)?;
    let (conversation_id, host_id) = permission_target_query(&target);
    let mut store = CliApplicationPort::persistent_default()?;
    store.execute(ApplicationCommand::ListPermissionGrants {
        conversation_id,
        host_id,
    })
}

pub fn load_web_permission_rules() -> Result<Vec<Value>, String> {
    let mut store = CliApplicationPort::persistent_default()?;
    Ok(web_grants(&mut store)?
        .into_iter()
        .filter_map(|grant| {
            let (tool_name, behavior) = parse_canonical_capability(&grant.capability_pattern)?;
            Some(json!({
                "id": stable_numeric_id(&grant.id),
                "tool_name": tool_name,
                "behavior": behavior,
                "match": grant.resource_pattern,
                "action": permission_action(grant.decision),
            }))
        })
        .collect())
}

pub fn replace_web_permission_rules(rules: &[Value]) -> Result<(), String> {
    validate_web_permission_rules(rules)?;
    let now = unix_epoch_millis();
    let mut store = CliApplicationPort::persistent_default()?;
    let existing = web_grants(&mut store)?;
    let mut desired_ids = HashSet::new();
    for (index, rule) in rules.iter().enumerate() {
        let tool_name = web_permission_field(rule, &["toolName", "tool_name"])
            .expect("web permission tool names were validated");
        let behavior = web_permission_field(rule, &["behavior"])
            .expect("web permission behaviors were validated");
        let match_pattern = web_permission_field(rule, &["match", "match_pattern"])
            .expect("web permission patterns were validated");
        let action = web_permission_field(rule, &["action"]).unwrap_or_else(|| "allow".to_string());
        let decision = canonical_decision(&validated_permission_action(&action)?)?;
        let id = permission_grant_id("web:local", &tool_name, &behavior, &match_pattern);
        desired_ids.insert(id.clone());
        let _: Value = store.execute(ApplicationCommand::UpsertPermissionGrant {
            input: PermissionGrantInput {
                id,
                target: PermissionGrantTarget::Host {
                    host_id: "web:local".to_string(),
                },
                capability_pattern: canonical_capability(&tool_name, &behavior),
                resource_pattern: match_pattern,
                decision,
                created_at_ms: now + i64::try_from(index).unwrap_or(0),
                updated_at_ms: now + i64::try_from(index).unwrap_or(0),
            },
        })?;
    }
    for grant in existing
        .into_iter()
        .filter(|grant| !desired_ids.contains(&grant.id))
    {
        let _: bool = store.execute(ApplicationCommand::RevokePermissionGrant {
            id: grant.id,
            revoked_at_ms: now,
        })?;
    }
    Ok(())
}

fn canonical_scope(scope: &str) -> Result<PermissionGrantTarget, String> {
    if let Some(id) = scope
        .strip_prefix("session:")
        .or_else(|| scope.strip_prefix("conversation:"))
    {
        let id = required_permission_field(id, "conversation permission id")?;
        return Ok(PermissionGrantTarget::Conversation {
            conversation_id: id,
        });
    }
    if scope == "web:local" {
        return Ok(PermissionGrantTarget::Host {
            host_id: scope.to_string(),
        });
    }
    if let Some(id) = scope.strip_prefix("host:") {
        let id = required_permission_field(id, "Host permission id")?;
        return Ok(PermissionGrantTarget::Host { host_id: id });
    }
    if scope == "user:global" || scope == "global" {
        return Ok(PermissionGrantTarget::UserGlobal);
    }
    Err(format!("unsupported permission scope: {scope}"))
}

fn permission_target_query(target: &PermissionGrantTarget) -> (Option<String>, Option<String>) {
    match target {
        PermissionGrantTarget::UserGlobal => (None, None),
        PermissionGrantTarget::Host { host_id } => (None, Some(host_id.clone())),
        PermissionGrantTarget::Conversation { conversation_id } => {
            (Some(conversation_id.clone()), None)
        }
    }
}

fn canonical_capability(tool_name: &str, behavior: &str) -> String {
    format!("cli::{tool_name}::{behavior}")
}

fn parse_canonical_capability(value: &str) -> Option<(String, String)> {
    let mut parts = value.strip_prefix("cli::")?.splitn(2, "::");
    Some((parts.next()?.to_string(), parts.next()?.to_string()))
}

fn canonical_decision(action: &str) -> Result<PermissionGrantDecision, String> {
    match action {
        "allow" => Ok(PermissionGrantDecision::Allow),
        "deny" => Ok(PermissionGrantDecision::Deny),
        "ask" => Err("transient ask decisions cannot be persisted".to_string()),
        value => Err(format!("unsupported permission decision: {value}")),
    }
}

fn permission_action(decision: PermissionGrantDecision) -> &'static str {
    match decision {
        PermissionGrantDecision::Allow => "allow",
        PermissionGrantDecision::Deny => "deny",
    }
}

fn permission_grant_id(scope: &str, tool_name: &str, behavior: &str, pattern: &str) -> String {
    let mut hash = 14_695_981_039_346_656_037_u64;
    for byte in format!("{scope}\0{tool_name}\0{behavior}\0{pattern}").bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    format!("cli-permission-{hash:016x}")
}

fn stable_numeric_id(raw: &str) -> u64 {
    let digits = raw
        .chars()
        .filter(|character| character.is_ascii_digit())
        .collect::<String>();
    digits.parse::<u64>().unwrap_or(0)
}

fn unix_epoch_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn web_grants(store: &mut CliApplicationPort) -> Result<Vec<PermissionGrant>, String> {
    let grants: Vec<PermissionGrant> = store.execute(ApplicationCommand::ListPermissionGrants {
        conversation_id: None,
        host_id: Some("web:local".to_string()),
    })?;
    Ok(grants
        .into_iter()
        .filter(|grant| {
            grant.target
                == (PermissionGrantTarget::Host {
                    host_id: "web:local".to_string(),
                })
        })
        .collect())
}

fn required_permission_field(value: &str, label: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{label} must be non-empty"));
    }
    if value.chars().count() > 1024 {
        return Err(format!("{label} exceeds 1024 characters"));
    }
    Ok(value.to_string())
}

fn validated_permission_action(value: &str) -> Result<String, String> {
    let action = required_permission_field(value, "permission action")?.to_ascii_lowercase();
    if !matches!(action.as_str(), "allow" | "ask" | "deny") {
        return Err("permission action must be allow, ask, or deny".to_string());
    }
    Ok(action)
}

fn web_permission_field(rule: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| rule.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn web_permission_rule_id(rule: &Value, index: usize) -> String {
    rule.get("id")
        .and_then(Value::as_u64)
        .map(|value| format!("web-perm-{value}"))
        .unwrap_or_else(|| format!("web-perm-{index}"))
}

fn validate_web_permission_rules(rules: &[Value]) -> Result<(), String> {
    if rules.len() > 512 {
        return Err("web permission rules are limited to 512 entries".to_string());
    }
    let mut ids = HashSet::new();
    let mut matches = HashSet::new();
    for (index, rule) in rules.iter().enumerate() {
        let id = web_permission_rule_id(rule, index);
        if !ids.insert(id.clone()) {
            return Err(format!("duplicate web permission rule id: {id}"));
        }
        let tool_name = web_permission_field(rule, &["toolName", "tool_name"])
            .ok_or_else(|| format!("web permission rule at index {index} requires tool_name"))?;
        let behavior = web_permission_field(rule, &["behavior"])
            .ok_or_else(|| format!("web permission rule at index {index} requires behavior"))?;
        let pattern = web_permission_field(rule, &["match", "match_pattern"])
            .ok_or_else(|| format!("web permission rule at index {index} requires match"))?;
        for (label, value) in [
            ("tool_name", &tool_name),
            ("behavior", &behavior),
            ("match", &pattern),
        ] {
            if value.chars().count() > 1024 {
                return Err(format!(
                    "web permission rule {label} at index {index} exceeds 1024 characters"
                ));
            }
        }
        let action = web_permission_field(rule, &["action"]).unwrap_or_else(|| "allow".to_string());
        let action = validated_permission_action(&action)?;
        if !matches.insert((tool_name, behavior, pattern)) {
            return Err(format!("duplicate web permission rule at index {index}"));
        }
        if action == "ask" {
            return Err("persisted web permission rules cannot use action=ask".to_string());
        }
    }
    Ok(())
}

const MAX_PROJECT_PERMISSION_RULES_PER_FILE: usize = 512;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectPermissionSettings {
    pub profile: Option<CliPermissionProfile>,
    pub shared_profile: Option<CliPermissionProfile>,
    pub local_profile: Option<CliPermissionProfile>,
    pub shared_rules: Vec<PermissionRule>,
    pub local_rules: Vec<PermissionRule>,
    pub merged_rules: Vec<PermissionRule>,
}

impl ProjectPermissionSettings {
    pub fn effective_profile(&self, fallback: CliPermissionProfile) -> CliPermissionProfile {
        self.profile.unwrap_or(fallback)
    }
}

/// Loads project-scoped permission rules from `<workspace_root>/.nola/settings.json`
/// (shared, checked into git) and `<workspace_root>/.nola/settings.local.json`
/// (local override, gitignored) when present.
pub fn load_project_permission_rules(
    workspace_root: &Path,
) -> Result<ProjectPermissionSettings, String> {
    let nola_dir = workspace_root.join(".nola");
    let shared_path = nola_dir.join("settings.json");
    let local_path = nola_dir.join("settings.local.json");

    let (shared_profile, shared_rules) = if shared_path.is_file() {
        parse_project_settings_file(&shared_path)?
    } else {
        (None, Vec::new())
    };

    let (local_profile, local_rules) = if local_path.is_file() {
        parse_project_settings_file(&local_path)?
    } else {
        (None, Vec::new())
    };

    let profile = local_profile.or(shared_profile);
    let merged_rules = merge_scoped_permission_rules(&[], &shared_rules, &local_rules);

    Ok(ProjectPermissionSettings {
        profile,
        shared_profile,
        local_profile,
        shared_rules,
        local_rules,
        merged_rules,
    })
}

/// Merges permission rules across user-global, project-shared, and project-local
/// scopes with deterministic precedence for last-match-wins evaluation:
/// 1. Explicit `Deny` rules from any scope override `Allow` and `Ask`.
/// 2. Within non-deny rules, project-local overrides project-shared overrides user-global.
pub fn merge_scoped_permission_rules(
    user_rules: &[PermissionRule],
    project_shared: &[PermissionRule],
    project_local: &[PermissionRule],
) -> Vec<PermissionRule> {
    let mut non_deny = Vec::new();
    let mut deny = Vec::new();

    for scope in [user_rules, project_shared, project_local] {
        for rule in scope {
            let target = match rule.action {
                PermissionAction::Deny => &mut deny,
                PermissionAction::Allow | PermissionAction::Ask => &mut non_deny,
            };
            push_rule_dedup_last_wins(target, rule.clone());
        }
    }

    non_deny.extend(deny);
    non_deny
}

fn push_rule_dedup_last_wins(rules: &mut Vec<PermissionRule>, rule: PermissionRule) {
    let perm_key = rule.permission.to_ascii_lowercase();
    let pat_key = rule.pattern.to_ascii_lowercase();
    rules.retain(|existing| {
        !(existing.permission.eq_ignore_ascii_case(&perm_key)
            && existing.pattern.eq_ignore_ascii_case(&pat_key))
    });
    rules.push(rule);
}

fn parse_project_settings_file(
    path: &Path,
) -> Result<(Option<CliPermissionProfile>, Vec<PermissionRule>), String> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let parsed: Value = serde_json::from_str(&raw)
        .map_err(|error| format!("invalid JSON in {}: {error}", path.display()))?;
    let Some(root) = parsed.as_object() else {
        return Err(format!("{} must contain a JSON object", path.display()));
    };

    let mut profile = parse_optional_profile(
        root.get("permissionProfile")
            .or_else(|| root.get("permission_profile")),
        path,
    )?;
    let mut rules = Vec::new();

    if let Some(permissions) = root.get("permissions") {
        match permissions {
            Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    rules.push(parse_raw_permission_rule(item, path, index)?);
                }
            }
            Value::Object(obj) => {
                if let Some(inner_profile) = parse_optional_profile(obj.get("profile"), path)? {
                    profile = Some(inner_profile);
                }
                if let Some(allow_items) = obj.get("allow") {
                    parse_action_rule_list(
                        allow_items,
                        PermissionAction::Allow,
                        "allow",
                        path,
                        &mut rules,
                    )?;
                }
                if let Some(ask_items) = obj.get("ask") {
                    parse_action_rule_list(
                        ask_items,
                        PermissionAction::Ask,
                        "ask",
                        path,
                        &mut rules,
                    )?;
                }
                if let Some(raw_rules) = obj.get("rules") {
                    let Some(items) = raw_rules.as_array() else {
                        return Err(format!(
                            "'permissions.rules' in {} must be an array",
                            path.display()
                        ));
                    };
                    for (index, item) in items.iter().enumerate() {
                        rules.push(parse_raw_permission_rule(item, path, index)?);
                    }
                }
                if let Some(deny_items) = obj.get("deny") {
                    parse_action_rule_list(
                        deny_items,
                        PermissionAction::Deny,
                        "deny",
                        path,
                        &mut rules,
                    )?;
                }
            }
            Value::Null => {}
            _ => {
                return Err(format!(
                    "'permissions' in {} must be an object or array",
                    path.display()
                ));
            }
        }
    }

    if rules.len() > MAX_PROJECT_PERMISSION_RULES_PER_FILE {
        return Err(format!(
            "{} exceeds limit of {MAX_PROJECT_PERMISSION_RULES_PER_FILE} permission rules",
            path.display()
        ));
    }

    Ok((profile, rules))
}

fn parse_optional_profile(
    value: Option<&Value>,
    path: &Path,
) -> Result<Option<CliPermissionProfile>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let raw = value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| format!("permission profile in {} must be a string", path.display()))?;
    let normalized = raw.to_ascii_lowercase().replace('-', "_");
    CliPermissionProfile::from_canonical_name(&normalized)
        .map(Some)
        .ok_or_else(|| {
            format!(
                "invalid permission profile '{raw}' in {} (expected cautious, accept_edits, or allow_all)",
                path.display()
            )
        })
}

fn parse_action_rule_list(
    value: &Value,
    action: PermissionAction,
    field: &str,
    path: &Path,
    out: &mut Vec<PermissionRule>,
) -> Result<(), String> {
    let Some(items) = value.as_array() else {
        return Err(format!(
            "'permissions.{field}' in {} must be an array",
            path.display()
        ));
    };
    for (index, item) in items.iter().enumerate() {
        match item {
            Value::String(expr) => {
                out.push(parse_permission_expression(expr, action, path, field, index)?);
            }
            Value::Object(obj) => {
                let permission = extract_rule_string(obj, &["permission", "tool"]).ok_or_else(|| {
                    format!(
                        "'permissions.{field}[{index}]' in {} requires 'permission' or 'tool'",
                        path.display()
                    )
                })?;
                let pattern = extract_rule_string(obj, &["pattern", "match"])
                    .unwrap_or_else(|| "*".to_string());
                out.push(PermissionRule {
                    permission: normalize_permission_tool_name(&permission),
                    pattern,
                    action,
                });
            }
            _ => {
                return Err(format!(
                    "'permissions.{field}[{index}]' in {} must be a string or object",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

fn parse_raw_permission_rule(
    value: &Value,
    path: &Path,
    index: usize,
) -> Result<PermissionRule, String> {
    let Some(obj) = value.as_object() else {
        return Err(format!(
            "permission rule at index {index} in {} must be an object",
            path.display()
        ));
    };
    let permission = extract_rule_string(obj, &["permission", "tool"]).ok_or_else(|| {
        format!(
            "permission rule at index {index} in {} requires 'permission' or 'tool'",
            path.display()
        )
    })?;
    let pattern =
        extract_rule_string(obj, &["pattern", "match"]).unwrap_or_else(|| "*".to_string());
    let raw_action = extract_rule_string(obj, &["action"]).ok_or_else(|| {
        format!(
            "permission rule at index {index} in {} requires 'action'",
            path.display()
        )
    })?;
    let action = parse_project_permission_action(&raw_action).ok_or_else(|| {
        format!(
            "permission rule at index {index} in {} has invalid action '{raw_action}'",
            path.display()
        )
    })?;
    Ok(PermissionRule {
        permission: normalize_permission_tool_name(&permission),
        pattern,
        action,
    })
}

fn parse_permission_expression(
    raw: &str,
    action: PermissionAction,
    path: &Path,
    field: &str,
    index: usize,
) -> Result<PermissionRule, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!(
            "'permissions.{field}[{index}]' in {} must be non-empty",
            path.display()
        ));
    }

    // Syntax 1: Tool(pattern), e.g. "Bash(git status*)" or "Bash(rm -rf *)"
    if trimmed.ends_with(')') {
        if let Some(open_idx) = trimmed.find('(') {
            let tool = trimmed[..open_idx].trim();
            if tool.is_empty() {
                return Err(format!(
                    "'permissions.{field}[{index}]' in {} has empty tool name",
                    path.display()
                ));
            }
            let inner = trimmed[open_idx + 1..trimmed.len() - 1].trim();
            let pattern = if inner.is_empty() { "*" } else { inner };
            return Ok(PermissionRule {
                permission: normalize_permission_tool_name(tool),
                pattern: pattern.to_string(),
                action,
            });
        }
    }

    // Syntax 2: tool:pattern, e.g. "edit_file:/workspace/*"
    if let Some((tool, rest)) = trimmed.split_once(':') {
        let tool = tool.trim();
        if tool.is_empty() {
            return Err(format!(
                "'permissions.{field}[{index}]' in {} has empty tool name",
                path.display()
            ));
        }
        let rest = rest.trim();
        let pattern = if rest.is_empty() { "*" } else { rest };
        return Ok(PermissionRule {
            permission: normalize_permission_tool_name(tool),
            pattern: pattern.to_string(),
            action,
        });
    }

    // Syntax 3: bare tool/permission name, e.g. "Bash" or "*"
    Ok(PermissionRule {
        permission: normalize_permission_tool_name(trimmed),
        pattern: "*".to_string(),
        action,
    })
}

fn normalize_permission_tool_name(raw: &str) -> String {
    let lower = raw.trim().to_ascii_lowercase();
    match lower.as_str() {
        "bash" | "shell" => "bash".to_string(),
        "read" => "read".to_string(),
        "edit" => "edit".to_string(),
        "write" => "write_file".to_string(),
        "webfetch" => "web_fetch".to_string(),
        "websearch" => "web_search".to_string(),
        _ => lower,
    }
}

fn parse_project_permission_action(raw: &str) -> Option<PermissionAction> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "allow" | "accept" | "always" => Some(PermissionAction::Allow),
        "ask" | "prompt" => Some(PermissionAction::Ask),
        "deny" | "reject" | "block" => Some(PermissionAction::Deny),
        _ => None,
    }
}

fn extract_rule_string(obj: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| obj.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod project_permission_tests {
    use super::*;
    use crate::permissions::{evaluate_tool_request, PermissionDecision};
    use crate::tools::ToolRequest;

    fn tool_request(name: &str, args: Value) -> ToolRequest {
        ToolRequest {
            id: "req-1".to_string(),
            name: name.to_string(),
            args: args.as_object().cloned().unwrap_or_default(),
        }
    }

    #[test]
    fn loads_shared_and_local_project_settings_with_structured_and_raw_rules() {
        let dir = tempfile::tempdir().unwrap();
        let nola_dir = dir.path().join(".nola");
        fs::create_dir_all(&nola_dir).unwrap();

        fs::write(
            nola_dir.join("settings.json"),
            serde_json::to_string_pretty(&json!({
                "permissions": {
                    "profile": "accept_edits",
                    "allow": [
                        "Bash(git status*)",
                        "edit_file:/workspace/*"
                    ],
                    "deny": [
                        "Bash(rm -rf *)"
                    ]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        fs::write(
            nola_dir.join("settings.local.json"),
            serde_json::to_string_pretty(&json!({
                "permissionProfile": "allow_all",
                "permissions": [
                    {
                        "permission": "bash",
                        "pattern": "cargo test*",
                        "action": "allow"
                    },
                    {
                        "permission": "bash",
                        "pattern": "rm -rf *",
                        "action": "allow"
                    }
                ]
            }))
            .unwrap(),
        )
        .unwrap();

        let settings = load_project_permission_rules(dir.path())
            .expect("project permission settings should load cleanly");

        assert_eq!(
            settings.shared_profile,
            Some(CliPermissionProfile::AcceptEdits)
        );
        assert_eq!(settings.local_profile, Some(CliPermissionProfile::AllowAll));
        assert_eq!(settings.profile, Some(CliPermissionProfile::AllowAll));
        assert_eq!(settings.shared_rules.len(), 3);
        assert_eq!(settings.local_rules.len(), 2);

        assert_eq!(
            evaluate_tool_request(
                &settings.merged_rules,
                &tool_request("bash", json!({"command": "git status -sb"}))
            ),
            PermissionDecision::Allow
        );
        assert_eq!(
            evaluate_tool_request(
                &settings.merged_rules,
                &tool_request("edit_file", json!({"path": "/workspace/src/main.rs"}))
            ),
            PermissionDecision::Allow
        );
        assert_eq!(
            evaluate_tool_request(
                &settings.merged_rules,
                &tool_request("bash", json!({"command": "cargo test --lib"}))
            ),
            PermissionDecision::Allow
        );
        // Shared explicit Deny overrides local Allow for the same pattern.
        assert_eq!(
            evaluate_tool_request(
                &settings.merged_rules,
                &tool_request("bash", json!({"command": "rm -rf /tmp/build"}))
            ),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn merge_scoped_permission_rules_enforces_deny_over_local_over_shared_over_user() {
        let user_rules = vec![
            PermissionRule {
                permission: "edit_file".to_string(),
                pattern: "*".to_string(),
                action: PermissionAction::Allow,
            },
            PermissionRule {
                permission: "bash".to_string(),
                pattern: "rm -rf *".to_string(),
                action: PermissionAction::Deny,
            },
        ];
        let project_shared = vec![
            PermissionRule {
                permission: "edit_file".to_string(),
                pattern: "*".to_string(),
                action: PermissionAction::Ask,
            },
            PermissionRule {
                permission: "bash".to_string(),
                pattern: "curl *".to_string(),
                action: PermissionAction::Deny,
            },
        ];
        let project_local = vec![
            PermissionRule {
                permission: "edit_file".to_string(),
                pattern: "src/*".to_string(),
                action: PermissionAction::Allow,
            },
            PermissionRule {
                permission: "bash".to_string(),
                pattern: "*".to_string(),
                action: PermissionAction::Allow,
            },
        ];

        let merged = merge_scoped_permission_rules(&user_rules, &project_shared, &project_local);

        // Project-shared Ask overrides user-global Allow for general edits.
        assert_eq!(
            evaluate_tool_request(
                &merged,
                &tool_request("edit_file", json!({"path": "README.md"}))
            ),
            PermissionDecision::Ask
        );
        // Project-local Allow overrides project-shared Ask for src/*.
        assert_eq!(
            evaluate_tool_request(
                &merged,
                &tool_request("edit_file", json!({"path": "src/lib.rs"}))
            ),
            PermissionDecision::Allow
        );
        // Project-local Allow permits benign bash commands.
        assert_eq!(
            evaluate_tool_request(
                &merged,
                &tool_request("bash", json!({"command": "git log -n 5"}))
            ),
            PermissionDecision::Allow
        );
        // Explicit Deny from user-global still overrides project-local wildcard Allow.
        assert_eq!(
            evaluate_tool_request(
                &merged,
                &tool_request("bash", json!({"command": "rm -rf /workspace"}))
            ),
            PermissionDecision::Deny
        );
        // Explicit Deny from project-shared still overrides project-local wildcard Allow.
        assert_eq!(
            evaluate_tool_request(
                &merged,
                &tool_request("bash", json!({"command": "curl https://example.com"}))
            ),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn missing_settings_returns_defaults_and_invalid_profile_errors() {
        let dir = tempfile::tempdir().unwrap();
        let empty = load_project_permission_rules(dir.path()).unwrap();
        assert_eq!(empty, ProjectPermissionSettings::default());
        assert_eq!(
            empty.effective_profile(CliPermissionProfile::Cautious),
            CliPermissionProfile::Cautious
        );

        let nola_dir = dir.path().join(".nola");
        fs::create_dir_all(&nola_dir).unwrap();
        fs::write(
            nola_dir.join("settings.json"),
            r#"{"permissions":{"profile":"invalid_mode"}}"#,
        )
        .unwrap();
        let err = load_project_permission_rules(dir.path()).unwrap_err();
        assert!(err.contains("invalid permission profile"));
    }
}
