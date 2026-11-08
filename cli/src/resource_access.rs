// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: CLI/Web adapter for Core-owned managed-resource visibility and editing.
// - Boundary: Resource grants never authorize Skill, MCP, data-source, or tool execution.

use nola_core::agent::resource_access::{
    evaluate_resource_access, AgentResourceAccessGrant, AgentResourceAccessRequest,
    AgentResourceIdentity, AgentResourceKind, AgentResourcePermission, AgentResourcePrincipal,
    AgentResourcePrincipalKind,
};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const LOCAL_WEB_USER_ID: &str = "cli-local-user";

pub(crate) fn reconcile_owned_resources(
    existing: &[Value],
    incoming: &[Value],
    kind: AgentResourceKind,
) -> Result<Vec<Value>, String> {
    let existing_by_id = existing
        .iter()
        .map(|value| Ok((resource_id(value, kind)?, value)))
        .collect::<Result<HashMap<_, _>, String>>()?;
    let mut seen = HashSet::new();
    let mut reconciled = Vec::with_capacity(incoming.len());
    for value in incoming {
        let id = resource_id(value, kind)?;
        if !seen.insert(id.clone()) {
            return Err(format!("duplicate managed resource ID: {id}"));
        }
        if let Some(previous) = existing_by_id.get(&id) {
            if !allows(previous, kind, AgentResourcePermission::EditContent) {
                return Err(format!("resource edit denied: {id}"));
            }
            reconciled.push(preserve_access_metadata(previous, value, kind)?);
        } else {
            reconciled.push(normalize_new_owned_resource(value, kind)?);
        }
    }
    for (id, previous) in existing_by_id {
        if !seen.contains(&id) && !allows(previous, kind, AgentResourcePermission::Delete) {
            return Err(format!("resource deletion denied: {id}"));
        }
    }
    Ok(reconciled)
}

pub(crate) fn allows(
    value: &Value,
    kind: AgentResourceKind,
    permission: AgentResourcePermission,
) -> bool {
    let Ok(resource) = identity(value, kind) else {
        return false;
    };
    let grants = value
        .get("access_grants")
        .cloned()
        .and_then(|raw| serde_json::from_value::<Vec<AgentResourceAccessGrant>>(raw).ok())
        .unwrap_or_default();
    evaluate_resource_access(&AgentResourceAccessRequest {
        schema_version: 1,
        resource,
        actor: local_actor(),
        permission,
        grants,
        now_epoch_ms: now_epoch_ms(),
        request_host_id: None,
    })
    .allowed
}

pub(crate) fn api_projection(value: &Value, kind: AgentResourceKind) -> Option<Value> {
    api_projection_with_content(value, kind, true)
}

fn api_projection_with_content(
    value: &Value,
    kind: AgentResourceKind,
    include_content: bool,
) -> Option<Value> {
    if !allows(value, kind, AgentResourcePermission::ReadMetadata) {
        return None;
    }
    let can_read_content = allows(value, kind, AgentResourcePermission::ReadContent);
    let can_edit_content = allows(value, kind, AgentResourcePermission::EditContent);
    let can_delete = allows(value, kind, AgentResourcePermission::Delete);
    // Exclude bodies before cloning, rather than cloning large private text
    // just to remove it from a metadata-only response afterwards.
    let mut projected = Value::Object(
        value
            .as_object()?
            .iter()
            .filter(|(key, _)| !matches!(key.as_str(), "access_owner" | "access_grants"))
            .filter(|(key, _)| {
                (include_content && can_read_content)
                    || !matches!(
                        key.as_str(),
                        "content" | "instructions" | "default_content" | "text"
                    )
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    );
    if let Some(object) = projected.as_object_mut() {
        object.remove("access_grants");
        object.remove("access_owner");
        if !can_read_content {
            for key in ["content", "instructions", "default_content", "text"] {
                object.remove(key);
            }
        }
        object.insert(
            "access".to_string(),
            json!({
                "read_metadata": true,
                "read_content": can_read_content,
                "edit_content": can_edit_content,
                "delete": can_delete,
                "tool_execution_authorized": false,
            }),
        );
        if matches!(kind, AgentResourceKind::DataSource) && !include_content {
            object.insert("content_omitted".into(), Value::Bool(true));
            if can_read_content {
                if let Some(content) = value.get("content").and_then(Value::as_str) {
                    object.insert(
                        "content_characters".into(),
                        Value::from(content.chars().count()),
                    );
                }
            }
        }
    }
    Some(projected)
}

pub(crate) fn reconcile_mcp_resources(
    existing: &[Value],
    incoming: &[Value],
) -> Result<Vec<Value>, String> {
    reconcile_owned_resources(existing, incoming, AgentResourceKind::McpServer)
}

pub(crate) fn reconcile_data_sources(
    existing: &[Value],
    incoming: &[Value],
) -> Result<Vec<Value>, String> {
    reconcile_owned_resources(existing, incoming, AgentResourceKind::DataSource)
}

pub(crate) fn data_source_api_projection(value: &Value) -> Option<Value> {
    api_projection(value, AgentResourceKind::DataSource)
}

pub(crate) fn data_source_api_projections(values: &[Value]) -> Vec<Value> {
    values
        .iter()
        .filter_map(|value| {
            api_projection_with_content(value, AgentResourceKind::DataSource, false)
        })
        .collect()
}

pub(crate) fn data_source_model_context_resources(values: &[Value]) -> Vec<Value> {
    values
        .iter()
        .filter(|value| {
            allows(
                value,
                AgentResourceKind::DataSource,
                AgentResourcePermission::ReadContent,
            )
        })
        .filter_map(data_source_api_projection)
        .collect()
}

pub(crate) fn mcp_api_projection(value: &Value) -> Option<Value> {
    let mut projected = api_projection(value, AgentResourceKind::McpServer)?;
    if let Some(object) = projected.as_object_mut() {
        for key in [
            "authorization",
            "headers",
            "env",
            "environment",
            "token",
            "api_key",
            "apiKey",
        ] {
            object.remove(key);
        }
    }
    Some(projected)
}

pub(crate) fn mcp_model_context_resources(values: &[Value]) -> Vec<Value> {
    values
        .iter()
        .filter(|value| {
            allows(
                value,
                AgentResourceKind::McpServer,
                AgentResourcePermission::ReadMetadata,
            )
        })
        .filter_map(mcp_api_projection)
        .map(|mut value| {
            if let Some(object) = value.as_object_mut() {
                for key in [
                    "endpoint",
                    "url",
                    "command",
                    "args",
                    "cwd",
                    "connection_ref",
                ] {
                    object.remove(key);
                }
            }
            value
        })
        .collect()
}

pub(crate) fn mcp_runtime_resources(values: &[Value]) -> Vec<Value> {
    values
        .iter()
        .filter(|value| {
            allows(
                value,
                AgentResourceKind::McpServer,
                AgentResourcePermission::ReadContent,
            )
        })
        .cloned()
        .collect()
}

fn normalize_new_owned_resource(value: &Value, kind: AgentResourceKind) -> Result<Value, String> {
    let mut normalized = value.clone();
    let id = resource_id(&normalized, kind)?;
    let object = normalized
        .as_object_mut()
        .ok_or_else(|| "managed resource must be a JSON object".to_string())?;
    object.insert("resource_id".to_string(), Value::String(id));
    object.insert(
        "access_owner".to_string(),
        serde_json::to_value(local_actor()).unwrap(),
    );
    object.insert("access_revision".to_string(), Value::from(1_u64));
    object.insert("access_grants".to_string(), Value::Array(Vec::new()));
    refresh_digest(object, kind)?;
    Ok(normalized)
}

/// Restores the local Web principal on a projection loaded from a canonical
/// repository whose typed schema owns revision/content state but does not
/// expose the adapter-only access envelope. Callers must use this only after
/// the repository has established that the record is owned locally (a custom
/// resource or this user’s built-in Skill selection). Never apply it to foreign metadata.
pub(crate) fn restore_local_user_resource_identity(
    value: &Value,
    kind: AgentResourceKind,
) -> Result<Value, String> {
    normalize_new_owned_resource(value, kind)
}

fn preserve_access_metadata(
    previous: &Value,
    incoming: &Value,
    kind: AgentResourceKind,
) -> Result<Value, String> {
    let mut normalized = incoming.clone();
    let previous_object = previous
        .as_object()
        .ok_or_else(|| "stored managed resource must be a JSON object".to_string())?;
    let object = normalized
        .as_object_mut()
        .ok_or_else(|| "managed resource must be a JSON object".to_string())?;
    for key in ["resource_id", "access_owner", "access_grants"] {
        if let Some(value) = previous_object.get(key) {
            object.insert(key.to_string(), value.clone());
        }
    }
    if matches!(kind, AgentResourceKind::DataSource) {
        // UI documents are metadata projections. An absent body is unchanged;
        // explicit null/empty/replacement remains explicit input for validation.
        if !object.contains_key("content") {
            if let Some(content) = previous_object.get("content") {
                object.insert("content".into(), content.clone());
            }
        }
        object.remove("content_omitted");
        object.remove("content_characters");
    }
    if matches!(kind, AgentResourceKind::McpServer) {
        // API projections deliberately omit connection credentials. Absence
        // therefore means "not edited", while an explicit null remains a
        // user-requested clear operation.
        for key in [
            "authorization",
            "headers",
            "env",
            "environment",
            "token",
            "api_key",
            "apiKey",
        ] {
            if !object.contains_key(key) {
                if let Some(value) = previous_object.get(key) {
                    object.insert(key.to_string(), value.clone());
                }
            }
        }
    }
    let normalized_snapshot = Value::Object(object.clone());
    let changed = comparable_content(previous) != comparable_content(&normalized_snapshot);
    let previous_revision = previous_object
        .get("access_revision")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    object.insert(
        "access_revision".to_string(),
        Value::from(if changed {
            previous_revision.saturating_add(1)
        } else {
            previous_revision
        }),
    );
    refresh_digest(object, kind)?;
    Ok(normalized)
}

fn identity(value: &Value, kind: AgentResourceKind) -> Result<AgentResourceIdentity, String> {
    let owner = value
        .get("access_owner")
        .cloned()
        .and_then(|raw| serde_json::from_value::<AgentResourcePrincipal>(raw).ok())
        .filter(|owner| !owner.id.trim().is_empty())
        .ok_or_else(|| "managed resource owner is missing or invalid".to_string())?;
    let revision = value
        .get("access_revision")
        .and_then(Value::as_u64)
        .filter(|revision| *revision > 0)
        .ok_or_else(|| "managed resource revision is missing or invalid".to_string())?;
    let digest = value
        .get("content_digest")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| content_digest(value, kind).ok());
    Ok(AgentResourceIdentity {
        kind,
        resource_id: resource_id(value, kind)?,
        owner,
        host_id: None,
        revision,
        content_digest: digest,
    })
}

fn resource_id(value: &Value, kind: AgentResourceKind) -> Result<String, String> {
    let key = match kind {
        AgentResourceKind::McpServer => "server_id",
        AgentResourceKind::Skill
        | AgentResourceKind::DataSource
        | AgentResourceKind::PromptTemplate => "id",
    };
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{kind:?} resource ID is required"))
}

fn refresh_digest(object: &mut Map<String, Value>, kind: AgentResourceKind) -> Result<(), String> {
    let digest = content_digest(&Value::Object(object.clone()), kind)?;
    object.insert("content_digest".to_string(), Value::String(digest));
    Ok(())
}

fn content_digest(value: &Value, kind: AgentResourceKind) -> Result<String, String> {
    let bytes = match kind {
        AgentResourceKind::Skill => value
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .as_bytes()
            .to_vec(),
        _ => serde_json::to_vec(&comparable_content(value))
            .map_err(|error| format!("serialize managed resource content failed: {error}"))?,
    };
    Ok(format!("sha256:{}", hex::encode(Sha256::digest(bytes))))
}

fn comparable_content(value: &Value) -> Value {
    let mut comparable = value.clone();
    if let Some(object) = comparable.as_object_mut() {
        for key in [
            "resource_id",
            "access_owner",
            "access_revision",
            "access_grants",
            "content_digest",
            "access",
        ] {
            object.remove(key);
        }
    }
    comparable
}

fn local_actor() -> AgentResourcePrincipal {
    AgentResourcePrincipal {
        kind: AgentResourcePrincipalKind::LocalUser,
        id: LOCAL_WEB_USER_ID.to_string(),
    }
}

fn now_epoch_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod context_document_projection_tests {
    use super::*;

    #[test]
    fn context_document_projection_excludes_ungranted_foreign_resources() {
        let owned = normalize_new_owned_resource(
            &json!({"id":"owned", "name":"Owned", "content":"visible body"}),
            AgentResourceKind::DataSource,
        )
        .unwrap();
        let mut foreign = normalize_new_owned_resource(
            &json!({"id":"foreign", "name":"Foreign", "content":"private body"}),
            AgentResourceKind::DataSource,
        )
        .unwrap();
        foreign["access_owner"] = serde_json::to_value(AgentResourcePrincipal {
            kind: AgentResourcePrincipalKind::LocalUser,
            id: "another-user".into(),
        })
        .unwrap();
        let projected = data_source_api_projections(&[owned.clone(), foreign]);
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0]["id"], "owned");
        assert!(projected[0].get("content").is_none());
        assert_eq!(projected[0]["content_characters"], 12);
        assert!(projected[0].get("access_owner").is_none());
        assert!(projected[0].get("access_grants").is_none());
        assert_eq!(projected[0]["access"]["tool_execution_authorized"], false);
        let full = data_source_api_projection(&owned).unwrap();
        assert_eq!(full["content"], "visible body");
        assert_eq!(projected[0]["access"], full["access"]);
    }

    #[test]
    fn context_document_projection_does_not_return_malformed_resources() {
        let projected = data_source_api_projections(&[
            json!({"name":"Missing identity", "content":"secret"}),
            json!({"id":" ", "content":"secret"}),
            json!("not a resource"),
        ]);
        assert!(projected.is_empty());
    }

    #[test]
    fn corrupt_ownership_and_revision_never_default_to_the_local_user() {
        let owned = normalize_new_owned_resource(
            &json!({"id":"owned", "name":"Owned", "content":"private body"}),
            AgentResourceKind::DataSource,
        )
        .unwrap();
        assert!(data_source_api_projection(&owned).is_some());
        for mutation in 0..7 {
            let mut resource = owned.clone();
            match mutation {
                0 => {
                    resource.as_object_mut().unwrap().remove("access_owner");
                }
                1 => resource["access_owner"] = json!("invalid owner"),
                2 => {
                    resource["access_owner"] = serde_json::to_value(AgentResourcePrincipal {
                        kind: AgentResourcePrincipalKind::LocalUser,
                        id: " ".into(),
                    })
                    .unwrap()
                }
                3 => {
                    resource.as_object_mut().unwrap().remove("access_revision");
                }
                4 => resource["access_revision"] = json!(0),
                5 => resource["access_revision"] = json!(-1),
                _ => resource["access_revision"] = json!("1"),
            }
            assert!(
                data_source_api_projection(&resource).is_none(),
                "mutation {mutation}"
            );
            for permission in [
                AgentResourcePermission::ReadContent,
                AgentResourcePermission::EditContent,
                AgentResourcePermission::Delete,
            ] {
                assert!(
                    !allows(&resource, AgentResourceKind::DataSource, permission),
                    "mutation {mutation}"
                );
            }
        }
    }

    #[test]
    fn metadata_only_document_save_preserves_body_and_model_context() {
        let owned = normalize_new_owned_resource(
            &json!({"id":"large", "name":"Large", "enabled":true, "content":"x".repeat(2_000_000)}),
            AgentResourceKind::DataSource,
        )
        .unwrap();
        let mut projected = data_source_api_projections(std::slice::from_ref(&owned));
        assert!(serde_json::to_vec(&projected).unwrap().len() < 2048);
        assert_eq!(projected[0]["content_characters"], 2_000_000);
        projected[0]["enabled"] = json!(false);
        let saved = reconcile_data_sources(std::slice::from_ref(&owned), &projected).unwrap();
        assert_eq!(saved[0]["content"], owned["content"]);
        assert_eq!(saved[0]["enabled"], false);
        assert!(saved[0].get("content_characters").is_none());
        assert!(saved[0].get("content_omitted").is_none());
        let model = data_source_model_context_resources(&saved);
        assert_eq!(model[0]["content"], owned["content"]);
        // An actual replacement is not confused with an omitted body.
        projected[0]["content"] = json!("replacement");
        let changed = reconcile_data_sources(&saved, &projected).unwrap();
        assert_eq!(changed[0]["content"], "replacement");
        projected[0]["content"] = Value::Null;
        let explicit_null = reconcile_data_sources(&changed, &projected).unwrap();
        assert!(explicit_null[0]["content"].is_null());
    }
}
