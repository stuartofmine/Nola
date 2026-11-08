use crate::application_port::CliApplicationPort;
use nola_application::{
    ApplicationCommand, PermissionGrant, PermissionGrantDecision, PermissionGrantInput,
    PermissionGrantTarget,
};
use serde_json::{json, Value};
use std::collections::HashSet;

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
