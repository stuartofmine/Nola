// SPDX-License-Identifier: Apache-2.0
//! Domain command dispatchers (skills, data sources, memory, MCP, automations, goals, etc.).

use std::collections::{BTreeMap, HashSet};
use chrono::Utc;
use nola_modules::ModulesSuite;
use nola_store::Store;
use nola_types::{new_id, now_ms, McpServerConfig, McpTransport, MemoryItem};
use serde_json::{json, Value};

use super::{extract_sub_command, load_product_setting_entry};

fn canonical_skill_editor_envelope(raw: &Value, fallback_id: Option<&str>, fallback_rev: u64) -> Value {
    let rec = raw.get("record").unwrap_or(raw);
    let ts = now_ms();
    let id = rec
        .get("id")
        .or_else(|| rec.get("name"))
        .or_else(|| raw.get("id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .or(fallback_id)
        .unwrap_or("skill")
        .to_string();
    let path_str = rec.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let raw_source = rec
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or(if path_str.starts_with("builtin://") {
            "built_in"
        } else {
            "user"
        });
    let source = match raw_source {
        "builtin" | "built_in" => "built_in",
        "user" => "user",
        _ => {
            if path_str.starts_with("builtin://") {
                "built_in"
            } else {
                "user"
            }
        }
    };
    let display_name = rec
        .get("display_name")
        .or_else(|| rec.get("name"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&id)
        .to_string();
    let summary = rec
        .get("summary")
        .or_else(|| rec.get("description"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let category = rec
        .get("category")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("general")
        .to_string();
    let instructions_ref = rec
        .get("instructions_ref")
        .or_else(|| rec.get("path"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let instructions_text = rec
        .get("instructions_text")
        .or_else(|| rec.get("body"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let required_tools = rec
        .get("required_tools")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let version = rec
        .get("version")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("1.0.0")
        .to_string();
    let enabled = rec
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let updated_at_ms = rec
        .get("updated_at_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or(ts);
    let created_at_ms = rec
        .get("created_at_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or(updated_at_ms);
    let default_mode = rec
        .get("default_mode")
        .cloned()
        .unwrap_or(Value::Null);
    let deleted_at_ms = rec
        .get("deleted_at_ms")
        .cloned()
        .unwrap_or(Value::Null);
    let revision = raw
        .get("revision")
        .or_else(|| rec.get("revision"))
        .and_then(|v| v.as_u64())
        .unwrap_or(fallback_rev)
        .max(1);

    json!({
        "record": {
            "id": id,
            "source": source,
            "display_name": display_name,
            "summary": summary,
            "category": category,
            "instructions_ref": instructions_ref,
            "instructions_text": instructions_text,
            "required_tools": required_tools,
            "default_mode": default_mode,
            "version": version,
            "enabled": enabled,
            "created_at_ms": created_at_ms,
            "updated_at_ms": updated_at_ms,
            "deleted_at_ms": deleted_at_ms
        },
        "revision": revision
    })
}

fn canonicalize_data_source_record(mut rec: Value, fallback_id: Option<&str>) -> Value {
    let ts = now_ms();
    let Some(obj) = rec.as_object_mut() else {
        return json!({});
    };
    let id = obj
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
        .or_else(|| fallback_id.map(ToString::to_string))
        .unwrap_or_else(new_id);
    obj.insert("id".into(), json!(id));
    if obj
        .get("name")
        .and_then(|v| v.as_str())
        .is_none_or(|s| s.is_empty())
    {
        let fallback_name = obj
            .get("title")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&id)
            .to_string();
        obj.insert("name".into(), json!(fallback_name));
    }
    obj.entry("kind".to_string()).or_insert_with(|| json!("http"));
    obj.entry("configuration".to_string()).or_insert_with(|| {
        json!({
            "connector_id": "generic",
            "location_kind": "http_url",
            "location": "",
            "parameters": {},
            "secret_references": []
        })
    });
    obj.entry("access_policy".to_string())
        .or_insert_with(|| json!("read_only"));
    obj.entry("authorization".to_string()).or_insert_with(|| {
        json!({
            "state": "not_required",
            "capability": Value::Null,
            "observed_at_ms": ts
        })
    });
    if !obj.get("validation").is_some_and(|v| v.is_object() && v.get("status").is_some()) {
        obj.insert(
            "validation".into(),
            json!({
                "status": "not_validated",
                "validation_token": Value::Null,
                "contract": Value::Null,
                "diagnostic": Value::Null,
                "attempted_at_ms": Value::Null,
                "completed_at_ms": Value::Null
            }),
        );
    }
    if !obj.get("read_state").is_some_and(|v| v.is_object() && v.get("status").is_some()) {
        obj.insert(
            "read_state".into(),
            json!({
                "status": "never_read",
                "read_token": Value::Null,
                "content_reference": Value::Null,
                "observed_at_ms": Value::Null,
                "schema_version": Value::Null,
                "record_count": Value::Null,
                "diagnostic": Value::Null,
                "requested_at_ms": Value::Null,
                "completed_at_ms": Value::Null
            }),
        );
    } else if let Some(rs_obj) = obj.get_mut("read_state").and_then(|v| v.as_object_mut()) {
        for field in [
            "read_token",
            "content_reference",
            "observed_at_ms",
            "schema_version",
            "record_count",
            "diagnostic",
            "requested_at_ms",
            "completed_at_ms",
        ] {
            rs_obj.entry(field.to_string()).or_insert(Value::Null);
        }
    }
    obj.entry("freshness_policy".to_string()).or_insert_with(|| {
        json!({
            "maximum_age_minutes": 60,
            "allow_stale_fallback": true
        })
    });
    let default_en = obj
        .get("default_enabled")
        .or_else(|| obj.get("enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    obj.insert("default_enabled".into(), json!(default_en));
    obj.entry("lifecycle".to_string())
        .or_insert_with(|| json!("draft"));
    obj.entry("capabilities".to_string())
        .or_insert_with(|| json!([]));
    obj.entry("privacy".to_string())
        .or_insert_with(|| json!("local"));
    let rev = obj
        .get("revision")
        .and_then(|v| v.as_i64())
        .unwrap_or(1)
        .max(1);
    obj.insert("revision".into(), json!(rev));
    obj.entry("last_successful_read_at_ms".to_string())
        .or_insert(Value::Null);
    obj.entry("created_at_ms".to_string())
        .or_insert_with(|| json!(ts));
    obj.entry("updated_at_ms".to_string())
        .or_insert_with(|| json!(ts));
    obj.entry("deleted_at_ms".to_string()).or_insert(Value::Null);
    rec
}

fn save_memory_extra_meta(store: &Store, id: &str, payload: &Value) {
    let input = payload.get("input").unwrap_or(payload);
    let rec = input.get("record").unwrap_or(input);
    let mut meta = serde_json::Map::new();
    for k in [
        "owner",
        "namespace",
        "scope",
        "kind",
        "summary",
        "confidence",
        "importance",
        "sensitivity",
        "retention",
        "provenance",
        "supersedes",
    ] {
        if let Some(v) = rec.get(k) {
            meta.insert(k.to_string(), v.clone());
        }
    }
    if !meta.is_empty() {
        let _ = store.set_kv(&format!("mem_meta:{id}"), &Value::Object(meta));
    }
}

fn canonical_memory_envelope_with_meta(store: &Store, item: &MemoryItem) -> Value {
    let mut env = canonical_memory_envelope(item);
    if let Ok(Some(Value::Object(meta))) = store.get_kv(&format!("mem_meta:{}", item.id)) {
        if let Some(rec) = env.get_mut("record").and_then(|v| v.as_object_mut()) {
            for (k, v) in meta {
                rec.insert(k, v);
            }
        }
    }
    env
}

fn canonical_memory_envelope(item: &MemoryItem) -> Value {
    let ns = if item.namespace.trim().is_empty() {
        "default"
    } else {
        item.namespace.as_str()
    };
    let (scope, kind) = ns
        .split_once(':')
        .unwrap_or(("global", if ns == "default" { "preference" } else { ns }));
    let ns_kind = if matches!(ns, "long_term" | "user_preferences") {
        "custom"
    } else {
        "user"
    };
    json!({
        "record": {
            "id": item.id,
            "owner": {"kind": "profile", "profile_id": "local", "id": "local"},
            "namespace": {"kind": ns_kind, "name": ns},
            "scope": scope,
            "kind": kind,
            "title": item.title,
            "summary": item.title,
            "content": item.content,
            "tags": item.tags,
            "confidence": 1.0,
            "importance": 0.5,
            "sensitivity": Value::Null,
            "supersedes": [],
            "retention": {"mode": "permanent"},
            "provenance": {"kind": "explicit_user", "source": "user", "item_ids": []}
        },
        "created_at_ms": item.created_at_ms,
        "updated_at_ms": item.updated_at_ms,
        "last_used_at_ms": Value::Null,
        "deleted_at_ms": Value::Null
    })
}

pub(super) fn parse_memory_item(payload: &Value) -> MemoryItem {
    let input = payload.get("input").unwrap_or(payload);
    let rec = input.get("record").unwrap_or(input);
    let ts = now_ms();
    let namespace = rec
        .get("namespace")
        .and_then(|v| {
            v.as_str().map(ToString::to_string).or_else(|| {
                v.as_object().and_then(|obj| {
                    obj.get("name")
                        .and_then(|n| n.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .or_else(|| obj.get("kind").and_then(|k| k.as_str()))
                        .map(ToString::to_string)
                })
            })
        })
        .unwrap_or_else(|| {
            let scope = rec.get("scope").and_then(|v| v.as_str()).unwrap_or("global");
            let kind = rec.get("kind").and_then(|v| v.as_str()).unwrap_or("default");
            if scope == "global" && kind == "default" {
                "default".to_string()
            } else {
                format!("{scope}:{kind}")
            }
        });
    MemoryItem {
        id: rec
            .get("id")
            .or_else(|| input.get("id"))
            .or_else(|| payload.get("id"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(new_id),
        namespace,
        title: rec
            .get("title")
            .or_else(|| rec.get("summary"))
            .or_else(|| rec.get("kind"))
            .and_then(|v| v.as_str())
            .unwrap_or("Memory")
            .to_string(),
        content: rec
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        tags: rec
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        created_at_ms: input
            .get("created_at_ms")
            .or_else(|| rec.get("created_at_ms"))
            .and_then(|v| v.as_i64())
            .unwrap_or(ts),
        updated_at_ms: input
            .get("updated_at_ms")
            .or_else(|| input.get("proposed_at_ms"))
            .or_else(|| rec.get("updated_at_ms"))
            .and_then(|v| v.as_i64())
            .unwrap_or(ts),
    }
}

fn parse_mcp_server_config(payload: &Value) -> Result<McpServerConfig, (&'static str, String)> {
    let raw_inner = payload
        .get("input")
        .or_else(|| payload.get("server"))
        .or_else(|| payload.get("config"))
        .or_else(|| payload.get("record"))
        .unwrap_or(payload);
    // Merge optional `configuration_json` object/string into the config map so all fields are visible.
    let mut merged = serde_json::Map::new();
    if let Some(obj) = raw_inner.as_object() {
        for (k, v) in obj {
            merged.insert(k.clone(), v.clone());
        }
    }
    if let Some(cj) = raw_inner.get("configuration_json").or_else(|| payload.get("configuration_json")) {
        let parsed_cj = if let Some(s) = cj.as_str() {
            serde_json::from_str::<Value>(s).ok()
        } else {
            Some(cj.clone())
        };
        if let Some(Value::Object(map)) = parsed_cj {
            for (k, v) in map {
                merged.entry(k).or_insert(v);
            }
        }
    }
    let cfg_val = &Value::Object(merged);
    if let Ok(parsed) = serde_json::from_value::<McpServerConfig>(cfg_val.clone()) {
        if parsed.id.trim().is_empty() {
            return Err(("invalid_argument", "MCP server 'id' must not be empty".into()));
        }
        return Ok(parsed);
    }
    let id = cfg_val
        .get("id")
        .or_else(|| cfg_val.get("server_id"))
        .or_else(|| payload.get("id"))
        .or_else(|| payload.get("server_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(new_id);
    let name = cfg_val
        .get("name")
        .or_else(|| cfg_val.get("display_name"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&id)
        .to_string();
    let transport_raw = cfg_val
        .get("transport")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| {
            if cfg_val.get("command").and_then(|v| v.as_str()).is_some() {
                "stdio"
            } else {
                "streamable_http"
            }
        });
    let transport = match transport_raw.trim().to_ascii_lowercase().as_str() {
        "stdio" => McpTransport::Stdio,
        "http" | "streamable_http" | "streamable-http" => McpTransport::StreamableHttp,
        "sse" | "server_sent_events" | "server-sent-events" => McpTransport::ServerSentEvents,
        other => {
            return Err((
                "invalid_argument",
                format!("unsupported MCP transport '{other}'"),
            ));
        }
    };
    let enabled = cfg_val
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let command = cfg_val
        .get("command")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let args = cfg_val
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    let env = cfg_val
        .get("env")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let url = cfg_val
        .get("url")
        .or_else(|| cfg_val.get("endpoint"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let headers = cfg_val
        .get("headers")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let oauth_client_id = cfg_val
        .get("oauth_client_id")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let oauth_scopes = cfg_val
        .get("oauth_scopes")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    Ok(McpServerConfig {
        id,
        name,
        transport,
        enabled,
        command,
        args,
        env,
        url,
        headers,
        oauth_client_id,
        oauth_scopes,
    })
}

pub(super) fn dispatch_domain_command(
    store: &Store,
    suite: &ModulesSuite,
    domain: &str,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let (op, payload) = extract_sub_command(args);

    match (domain, op) {
        // --- Model Domain ---
        ("model", "list" | "list_profiles") => {
            let mut items: Vec<Value> = suite
                .list_records::<Value>("model")
                .map_err(|e| ("storage_error", e.to_string()))?
                .into_iter()
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .collect();
            if items.is_empty() {
                if let Some(arr) = crate::web_api::build_model_profiles_response(store)
                    .get("profiles")
                    .and_then(|v| v.as_array())
                {
                    items = arr.clone();
                }
            }
            Ok(json!({"kind": "model", "items": items, "profiles": items}))
        }
        ("model", "get" | "get_profile") => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("profile_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let found = suite
                .get_record::<Value>("model", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()));
            Ok(found.unwrap_or(Value::Null))
        }
        ("model", "list_connections") => {
            let cfg = crate::config::load_active_config(store);
            let mut list: Vec<Value> = Vec::new();
            let mut seen_ids: HashSet<String> = HashSet::new();
            for rec in suite.list_records::<Value>("model_connection").unwrap_or_default() {
                let rid = rec.get("id").and_then(|v| v.as_str()).map(ToString::to_string);
                if let Some(ref id) = rid {
                    seen_ids.insert(id.clone());
                }
                let rev = rec.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
                let catalog = rec.get("catalog").cloned().unwrap_or(Value::Null);
                list.push(json!({
                    "id": rid,
                    "revision": rev,
                    "document": {
                        "format_version": 1,
                        "connection": rec,
                        "catalog": catalog
                    },
                    "connection": rec,
                    "catalog": catalog
                }));
            }
            if let Some(cfg_conns) = crate::config::redacted_llm_config(&cfg.llm)
                .get("connections")
                .and_then(|v| v.as_array())
            {
                for c in cfg_conns {
                    let cid = c.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    if !cid.is_empty() && !seen_ids.contains(cid) {
                        list.push(c.clone());
                    }
                }
            }
            Ok(json!({"connections": list, "items": list}))
        }
        ("model", "list_accounts") => {
            let mut list: Vec<Value> = Vec::new();
            for rec in suite.list_records::<Value>("model_account").unwrap_or_default() {
                let rev = rec.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
                list.push(json!({
                    "revision": rev,
                    "account": rec
                }));
            }
            Ok(json!({"accounts": list, "items": list}))
        }
        ("model", "get_connection") => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("connection_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            if let Ok(Some(rec)) = suite.get_record::<Value>("model_connection", id) {
                let rev = rec.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
                let catalog = rec.get("catalog").cloned().unwrap_or(Value::Null);
                return Ok(json!({
                    "id": id,
                    "revision": rev,
                    "document": {
                        "format_version": 1,
                        "connection": rec,
                        "catalog": catalog
                    },
                    "connection": rec,
                    "catalog": catalog
                }));
            }
            let cfg = crate::config::load_active_config(store);
            let found = crate::config::redacted_llm_config(&cfg.llm)
                .get("connections")
                .and_then(|v| v.as_array())
                .and_then(|arr| {
                    arr.iter()
                        .find(|c| c.get("id").and_then(|v| v.as_str()) == Some(id))
                        .cloned()
                });
            Ok(found.unwrap_or(Value::Null))
        }
        ("model", "get_account") => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("account_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            if let Ok(Some(rec)) = suite.get_record::<Value>("model_account", id) {
                let rev = rec.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
                return Ok(json!({
                    "revision": rev,
                    "account": rec
                }));
            }
            Ok(Value::Null)
        }
        ("model", "save_account") => {
            let input = payload
                .get("input")
                .or_else(|| payload.get("account"))
                .unwrap_or(payload);
            let id = input
                .get("id")
                .or_else(|| input.get("account_id"))
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let prev_rev = suite
                .get_record::<Value>("model_account", &id)
                .ok()
                .flatten()
                .and_then(|v| v.get("revision").and_then(|r| r.as_i64()))
                .unwrap_or(0);
            let next_rev = prev_rev + 1;
            if let Some(secret) = input
                .get("api_key")
                .or_else(|| input.get("apiKey"))
                .or_else(|| input.get("token"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                if let Ok(vault) =
                    nola_store::Vault::open(crate::config::resolve_vault_path_for_store(store))
                {
                    let _ = vault.set_secret(&format!("model_account_secret:{id}"), secret);
                }
            }
            let mut rec = input.clone();
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("account_id".into(), json!(id));
                obj.insert("revision".into(), json!(next_rev));
                obj.remove("api_key");
                obj.remove("apiKey");
                obj.remove("token");
                obj.insert("updated_at_ms".into(), json!(now_ms()));
            }
            suite
                .upsert_record("model_account", &id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "id": id, "revision": next_rev, "account": rec}))
        }
        ("model", "save_connection") => {
            let input = payload
                .get("input")
                .or_else(|| payload.get("connection"))
                .unwrap_or(payload);
            let id = input
                .get("id")
                .or_else(|| input.get("connection_id"))
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let prev_rev = suite
                .get_record::<Value>("model_connection", &id)
                .ok()
                .flatten()
                .and_then(|v| v.get("revision").and_then(|r| r.as_i64()))
                .unwrap_or(0);
            let next_rev = prev_rev + 1;
            let mut sanitized = input.clone();
            if let Some(obj) = sanitized.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("connection_id".into(), json!(id));
                obj.insert("revision".into(), json!(next_rev));
                obj.remove("api_key");
                obj.remove("apiKey");
                obj.insert("updated_at_ms".into(), json!(now_ms()));
            }
            let catalog = sanitized.get("catalog").cloned().unwrap_or(Value::Null);
            suite
                .upsert_record("model_connection", &id, &sanitized)
                .map_err(|e| ("storage_error", e.to_string()))?;
            if let Some(ep) = input
                .get("endpoint")
                .or_else(|| input.get("base_url"))
                .or_else(|| input.get("baseURL"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let mut cfg = crate::config::load_active_config(store);
                let protocol = input
                    .get("protocol")
                    .or_else(|| input.get("wire_protocol"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                let provider = input
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                let api_key = input
                    .get("api_key")
                    .or_else(|| input.get("apiKey"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string);
                let models: Vec<crate::config::ModelEntry> = input
                    .get("models")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|m| {
                                m.as_str()
                                    .or_else(|| m.get("id").and_then(|v| v.as_str()))
                                    .map(|mid| crate::config::ModelEntry {
                                        id: mid.to_string(),
                                        context_window: None,
                                        max_output_tokens: None,
                                        temperature: None,
                                        top_p: None,
                                        reasoning_effort: None,
                                        capabilities: None,
                                    })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(existing) = cfg.llm.connections.iter_mut().find(|c| c.id == id) {
                    existing.endpoint = ep.to_string();
                    if protocol.is_some() {
                        existing.protocol = protocol;
                    }
                    if provider.is_some() {
                        existing.provider = provider;
                    }
                    if api_key.is_some() {
                        existing.api_key = api_key;
                    }
                    for m in models {
                        if !existing.models.iter().any(|e| e.id == m.id) {
                            existing.models.push(m);
                        }
                    }
                } else {
                    cfg.llm.connections.push(crate::config::ConnectionConfig {
                        id: id.clone(),
                        endpoint: ep.to_string(),
                        protocol,
                        provider,
                        api_key,
                        api_key_env: None,
                        models,
                    });
                }
                let _ = crate::config::save_applied_config(store, &cfg);
            }
            Ok(json!({
                "ok": true,
                "id": id,
                "revision": next_rev,
                "document": {
                    "format_version": 1,
                    "connection": sanitized,
                    "catalog": catalog
                },
                "connection": sanitized,
                "catalog": catalog
            }))
        }
        ("model", "set_connection_capability_override") => {
            let input = payload.get("input").unwrap_or(payload);
            let conn_id = input
                .get("connection_id")
                .or_else(|| input.get("connectionId"))
                .or_else(|| args.get("connection_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'connection_id'".into()))?;
            let model_id = input
                .get("model_id")
                .or_else(|| input.get("modelId"))
                .or_else(|| args.get("model_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'model_id'".into()))?;
            let protocol = input
                .get("protocol")
                .and_then(|v| v.as_str())
                .unwrap_or("chat_completions");
            let caps = input
                .get("capabilities")
                .or_else(|| input.get("override"))
                .cloned()
                .unwrap_or(Value::Null);
            let kv_key = format!("model_cap_override:{conn_id}:{model_id}");
            let stored = if caps.is_null() {
                store
                    .set_kv(&kv_key, &Value::Null)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                Value::Null
            } else {
                let rec = json!({
                    "connection_id": conn_id,
                    "model_id": model_id,
                    "protocol": protocol,
                    "user_override": caps,
                    "updated_at_ms": now_ms()
                });
                store
                    .set_kv(&kv_key, &rec)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                rec
            };
            Ok(json!({
                "ok": true,
                "connection_id": conn_id,
                "model_id": model_id,
                "protocol": protocol,
                "override": stored
            }))
        }
        ("model", "refresh_connection_catalog") => {
            let input = payload.get("input").unwrap_or(payload);
            let conn_id = input
                .get("connection_id")
                .or_else(|| input.get("connectionId"))
                .or_else(|| input.get("id"))
                .or_else(|| args.get("connection_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'connection_id'".into()))?;
            let mut cfg = crate::config::load_active_config(store);
            let mut models: Vec<String> = Vec::new();
            if let Some(conn) = cfg.llm.connections.iter_mut().find(|c| c.id == conn_id) {
                let mut changed = false;
                if let Some(incoming) = input.get("models").and_then(|v| v.as_array()) {
                    for m in incoming {
                        if let Some(mid) = m
                            .as_str()
                            .or_else(|| m.get("id").and_then(|v| v.as_str()))
                            .filter(|s| !s.trim().is_empty())
                        {
                            if !conn.models.iter().any(|e| e.id == mid) {
                                conn.models.push(crate::config::ModelEntry {
                                    id: mid.to_string(),
                                    context_window: None,
                                    max_output_tokens: None,
                                    temperature: None,
                                    top_p: None,
                                    reasoning_effort: None,
                                    capabilities: None,
                                });
                                changed = true;
                            }
                        }
                    }
                }
                models = conn.models.iter().map(|m| m.id.clone()).collect();
                if changed {
                    let _ = crate::config::save_applied_config(store, &cfg);
                }
            } else if let Ok(Some(rec)) = suite.get_record::<Value>("model_connection", conn_id) {
                if let Some(arr) = rec.get("models").and_then(|v| v.as_array()) {
                    models = arr
                        .iter()
                        .filter_map(|m| {
                            m.as_str()
                                .or_else(|| m.get("id").and_then(|v| v.as_str()))
                                .map(ToString::to_string)
                        })
                        .collect();
                }
            }
            let ts = now_ms();
            let _ = store.set_kv(
                &format!("model_catalog_refreshed:{conn_id}"),
                &json!({"connection_id": conn_id, "models": models, "refreshed_at_ms": ts}),
            );
            Ok(json!({
                "ok": true,
                "connection_id": conn_id,
                "models": models,
                "refreshed_at_ms": ts
            }))
        }
        ("model", "configure_connection_models") => {
            let input = payload.get("input").unwrap_or(payload);
            let conn_id = input
                .get("connection_id")
                .or_else(|| input.get("connectionId"))
                .or_else(|| args.get("connection_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'connection_id'".into()))?;
            let enabled_ids: Vec<String> = input
                .get("enabled_model_ids")
                .or_else(|| input.get("models"))
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(ToString::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let selected_id = input
                .get("selected_model_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string);
            let mut cfg = crate::config::load_active_config(store);
            if let Some(conn) = cfg.llm.connections.iter_mut().find(|c| c.id == conn_id) {
                for key in &enabled_ids {
                    let mid = key.split_once(':').map(|(_, m)| m).unwrap_or(key.as_str());
                    if !mid.is_empty() && !conn.models.iter().any(|m| m.id == mid) {
                        conn.models.push(crate::config::ModelEntry {
                            id: mid.to_string(),
                            context_window: None,
                            max_output_tokens: None,
                            temperature: None,
                            top_p: None,
                            reasoning_effort: None,
                            capabilities: None,
                        });
                    }
                }
                if let Some(ref sel) = selected_id {
                    let mid = sel.split_once(':').map(|(_, m)| m).unwrap_or(sel.as_str());
                    if !mid.is_empty() {
                        cfg.llm.default_model = Some(crate::config::DefaultModelSelection {
                            connection: conn_id.to_string(),
                            model: mid.to_string(),
                        });
                    }
                }
                let _ = crate::config::save_applied_config(store, &cfg);
            }
            let selection = json!({
                "connection_id": conn_id,
                "enabled": enabled_ids,
                "selected_model_id": selected_id,
                "updated_at_ms": now_ms()
            });
            store
                .set_kv(&format!("model_conn_selections:{conn_id}"), &selection)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "connection_id": conn_id,
                "enabled_model_ids": enabled_ids,
                "selected_model_id": selected_id,
                "selection": selection
            }))
        }
        ("model", "import_connection") => {
            let input = payload
                .get("input")
                .or_else(|| payload.get("connection"))
                .unwrap_or(payload);
            let id = input
                .get("id")
                .or_else(|| input.get("connection_id"))
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let ts = payload
                .get("updated_at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            if let Some(acct_val) = payload.get("account").filter(|v| v.is_object()) {
                let acct_id = acct_val
                    .get("id")
                    .or_else(|| acct_val.get("account_id"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(new_id);
                let prev_acct_rev = suite
                    .get_record::<Value>("model_account", &acct_id)
                    .ok()
                    .flatten()
                    .and_then(|v| v.get("revision").and_then(|r| r.as_i64()))
                    .unwrap_or(0);
                let mut acct_rec = acct_val.clone();
                if let Some(obj) = acct_rec.as_object_mut() {
                    obj.insert("id".into(), json!(acct_id));
                    obj.insert("account_id".into(), json!(acct_id));
                    obj.insert("revision".into(), json!(prev_acct_rev + 1));
                    obj.remove("api_key");
                    obj.remove("apiKey");
                    obj.remove("token");
                    obj.insert("updated_at_ms".into(), json!(ts));
                }
                suite
                    .upsert_record("model_account", &acct_id, &acct_rec)
                    .map_err(|e| ("storage_error", e.to_string()))?;
            }
            let prev_rev = suite
                .get_record::<Value>("model_connection", &id)
                .ok()
                .flatten()
                .and_then(|v| v.get("revision").and_then(|r| r.as_i64()))
                .unwrap_or(0);
            let next_rev = prev_rev + 1;
            let catalog = payload
                .get("models")
                .and_then(|v| v.as_array())
                .map(|models| {
                    json!({
                        "observed_at_ms": ts,
                        "source": input.get("source").and_then(|v| v.as_str()).unwrap_or("manual"),
                        "models": models
                    })
                })
                .unwrap_or(Value::Null);
            let mut rec = input.clone();
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("updated_at_ms".into(), json!(ts));
                if !catalog.is_null() {
                    obj.insert("catalog".into(), catalog.clone());
                }
            }
            suite
                .upsert_record("model_connection", &id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "id": id,
                "revision": next_rev,
                "document": {
                    "format_version": 1,
                    "connection": rec,
                    "catalog": catalog
                },
                "connection": rec,
                "catalog": catalog
            }))
        }
        ("model", "report_runtime") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = input
                .get("host_id")
                .or_else(|| input.get("runtime_id"))
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let mut rec = input.clone();
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("reported_at_ms".into(), json!(now_ms()));
            }
            store
                .set_kv(&format!("model_runtime:{id}"), &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "runtime": rec}))
        }
        ("model", "get_runtime") => {
            let id = payload
                .get("host_id")
                .or_else(|| payload.get("runtime_id"))
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("host_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let found = store
                .get_kv(&format!("model_runtime:{id}"))
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(found.unwrap_or(Value::Null))
        }
        ("model", "upsert_profile") => {
            let input = payload.get("input").unwrap_or(payload);
            let record = input
                .get("profile")
                .or_else(|| input.get("record"))
                .unwrap_or(input);
            let id = record
                .get("id")
                .or_else(|| record.get("profile_id"))
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let existing = suite
                .get_record::<Value>("model", &id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let current_rev = existing
                .as_ref()
                .and_then(|r| r.get("revision"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let expected_rev = args
                .get("__envelope_expected_revision")
                .or_else(|| payload.get("expected_revision"))
                .or_else(|| input.get("expected_revision"))
                .and_then(|v| v.as_u64());
            if let Some(exp) = expected_rev {
                if exp != current_rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": format!("model_profile:{id}"),
                            "expected_revision": exp,
                            "actual_revision": current_rev
                        })
                        .to_string(),
                    ));
                }
            }
            if let Some(ref ex) = existing {
                if ex.get("deleted_at_ms").is_some_and(|v| !v.is_null()) {
                    return Err((
                        "conflict",
                        format!("model profile '{id}' was deleted and cannot be resurrected"),
                    ));
                }
                let old_provider = ex.get("provider").and_then(|v| v.as_str()).unwrap_or("");
                let new_provider = record
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .unwrap_or(old_provider);
                if !old_provider.is_empty() && !new_provider.is_empty() && old_provider != new_provider {
                    return Err((
                        "invalid_argument",
                        format!(
                            "model profile '{id}' cannot change provider ownership from '{old_provider}' to '{new_provider}'"
                        ),
                    ));
                }
            }
            let next_rev = current_rev + 1;
            let ts = record
                .get("updated_at_ms")
                .or_else(|| input.get("updated_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let created_at = existing
                .as_ref()
                .and_then(|r| r.get("created_at_ms"))
                .and_then(|v| v.as_i64())
                .or_else(|| record.get("created_at_ms").and_then(|v| v.as_i64()))
                .unwrap_or(ts);
            let mut stored = record.clone();
            if let Some(obj) = stored.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("created_at_ms".into(), json!(created_at));
                obj.insert("updated_at_ms".into(), json!(ts));
                obj.entry("selected".to_string()).or_insert_with(|| json!(false));
                obj.entry("enabled".to_string()).or_insert_with(|| json!(true));
                obj.entry("source_json".to_string()).or_insert_with(|| json!("{}"));
                obj.entry("parameters_json".to_string()).or_insert_with(|| json!("{}"));
            }
            if stored.get("selected").and_then(|v| v.as_bool()) == Some(true) {
                if let Ok(others) = suite.list_records::<Value>("model") {
                    for mut other in others {
                        let oid = other
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if !oid.is_empty()
                            && oid != id
                            && other.get("selected").and_then(|v| v.as_bool()) == Some(true)
                        {
                            if let Some(obj) = other.as_object_mut() {
                                obj.insert("selected".into(), json!(false));
                            }
                            let _ = suite.upsert_record("model", &oid, &other);
                        }
                    }
                }
            }
            suite
                .upsert_record("model", &id, &stored)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "id": id,
                "revision": next_rev,
                "__committed_revision": next_rev,
                "profile": stored
            }))
        }
        ("model", "delete_profile" | "delete") => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("profile_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let existing = suite
                .get_record::<Value>("model", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let Some(mut rec) = existing.filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null())) else {
                return Ok(json!(false));
            };
            let cur_rev = rec.get("revision").and_then(|v| v.as_u64()).unwrap_or(1);
            let ts = payload
                .get("deleted_at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("selected".into(), json!(false));
                obj.insert("enabled".into(), json!(false));
                obj.insert("revision".into(), json!(cur_rev + 1));
                obj.insert("deleted_at_ms".into(), json!(ts));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("model", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!(true))
        }
        ("model", "configure_parameters") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = input
                .get("id")
                .or_else(|| input.get("profile_id"))
                .or_else(|| payload.get("id"))
                .or_else(|| payload.get("profile_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let existing = suite
                .get_record::<Value>("model", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let current_rev = existing
                .as_ref()
                .and_then(|r| r.get("revision"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let expected_rev = args
                .get("__envelope_expected_revision")
                .or_else(|| payload.get("expected_revision"))
                .or_else(|| input.get("expected_revision"))
                .and_then(|v| v.as_u64());
            if let Some(exp) = expected_rev {
                if exp != current_rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": format!("model_profile:{id}"),
                            "expected_revision": exp,
                            "actual_revision": current_rev
                        })
                        .to_string(),
                    ));
                }
            }
            let mut rec = match existing {
                Some(ex) => ex,
                None => {
                    let mut base = input.clone();
                    if let Some(obj) = base.as_object_mut() {
                        obj.insert("id".into(), json!(id));
                        obj.entry("display_name".to_string()).or_insert_with(|| json!(id));
                        obj.entry("provider".to_string()).or_insert_with(|| json!("local"));
                        obj.entry("model_name".to_string()).or_insert_with(|| json!(id));
                        obj.entry("runtime".to_string()).or_insert_with(|| json!("mlx"));
                        obj.entry("source_json".to_string()).or_insert_with(|| json!("{}"));
                        obj.entry("selected".to_string()).or_insert_with(|| json!(false));
                        obj.entry("enabled".to_string()).or_insert_with(|| json!(true));
                    }
                    base
                }
            };
            let (params, params_json_str) = if let Some(pj) = input
                .get("parameters_json")
                .or_else(|| payload.get("parameters_json"))
            {
                if let Some(s) = pj.as_str() {
                    let parsed = serde_json::from_str::<Value>(s).unwrap_or_else(|_| json!({}));
                    (parsed, s.to_string())
                } else {
                    let s = serde_json::to_string(pj).unwrap_or_else(|_| "{}".into());
                    (pj.clone(), s)
                }
            } else {
                let p = input.get("parameters").cloned().unwrap_or_else(|| {
                    let mut m = serde_json::Map::new();
                    for k in [
                        "temperature",
                        "top_p",
                        "topP",
                        "max_output_tokens",
                        "maxOutputTokens",
                        "context_window",
                        "maxContextLength",
                        "reasoning_effort",
                    ] {
                        if let Some(v) = input.get(k) {
                            m.insert(k.to_string(), v.clone());
                        }
                    }
                    Value::Object(m)
                });
                let s = serde_json::to_string(&p).unwrap_or_else(|_| "{}".into());
                (p, s)
            };
            let next_rev = current_rev + 1;
            let ts = input
                .get("updated_at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("parameters".into(), params);
                obj.insert("parameters_json".into(), json!(params_json_str));
                obj.insert("revision".into(), json!(next_rev));
                obj.entry("created_at_ms".to_string()).or_insert_with(|| {
                    input
                        .get("created_at_ms")
                        .cloned()
                        .unwrap_or_else(|| json!(ts))
                });
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("model", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "id": id,
                "revision": next_rev,
                "__committed_revision": next_rev,
                "parameters_json": params_json_str,
                "profile": rec
            }))
        }
        ("model", "reset_parameters") => {
            let input = payload.get("input").unwrap_or(payload);
            let params_json_str = input
                .get("parameters_json")
                .or_else(|| payload.get("parameters_json"))
                .and_then(|v| v.as_str())
                .unwrap_or("{}")
                .to_string();
            let params_val =
                serde_json::from_str::<Value>(&params_json_str).unwrap_or_else(|_| json!({}));
            let ts = input
                .get("updated_at_ms")
                .or_else(|| payload.get("updated_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            if let Some(ids_arr) = input
                .get("profile_ids")
                .or_else(|| payload.get("profile_ids"))
                .or_else(|| args.get("profile_ids"))
                .and_then(|v| v.as_array())
            {
                // Validate ALL profile_ids exist atomically before mutating any record.
                let mut staged: Vec<(String, Value)> = Vec::with_capacity(ids_arr.len());
                for id_val in ids_arr {
                    let id = id_val
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .ok_or(("invalid_argument", "profile_ids contains empty id".into()))?;
                    let mut rec = suite
                        .get_record::<Value>("model", id)
                        .map_err(|e| ("storage_error", e.to_string()))?
                        .ok_or(("not_found", format!("model profile '{id}' not found")))?;
                    let next_rev =
                        rec.get("revision").and_then(|v| v.as_u64()).unwrap_or(0) + 1;
                    if let Some(obj) = rec.as_object_mut() {
                        obj.insert("parameters".into(), params_val.clone());
                        obj.insert("parameters_json".into(), json!(params_json_str));
                        obj.insert("revision".into(), json!(next_rev));
                        obj.insert("updated_at_ms".into(), json!(ts));
                    }
                    staged.push((id.to_string(), rec));
                }
                let mut updated = Vec::with_capacity(staged.len());
                let mut ids = Vec::with_capacity(staged.len());
                for (id, rec) in staged {
                    suite
                        .upsert_record("model", &id, &rec)
                        .map_err(|e| ("storage_error", e.to_string()))?;
                    ids.push(id);
                    updated.push(rec);
                }
                return Ok(json!({"ok": true, "profile_ids": ids, "profiles": updated}));
            }
            let id = input
                .get("id")
                .or_else(|| input.get("profile_id"))
                .or_else(|| payload.get("id"))
                .or_else(|| payload.get("profile_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let mut rec = suite
                .get_record::<Value>("model", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("model profile '{id}' not found")))?;
            let next_rev = rec.get("revision").and_then(|v| v.as_u64()).unwrap_or(0) + 1;
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("parameters".into(), params_val);
                obj.insert("parameters_json".into(), json!(params_json_str));
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("model", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "id": id,
                "revision": next_rev,
                "__committed_revision": next_rev,
                "parameters_json": params_json_str,
                "profile": rec
            }))
        }
        ("model", "replace_profiles") => {
            let profiles = payload
                .get("profiles")
                .or_else(|| args.get("profiles"))
                .and_then(|v| v.as_array())
                .ok_or(("invalid_argument", "missing 'profiles' array".into()))?;
            let scope_provider = payload
                .get("provider")
                .or_else(|| args.get("provider"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            let scope_runtime = payload
                .get("runtime")
                .or_else(|| args.get("runtime"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            let expected_revs = payload
                .get("expected_revisions")
                .or_else(|| args.get("expected_revisions"))
                .and_then(|v| v.as_object());
            let ts = payload
                .get("updated_at_ms")
                .or_else(|| args.get("updated_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let existing: Vec<Value> = suite
                .list_records::<Value>("model")
                .unwrap_or_default()
                .into_iter()
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .collect();
            let mut existing_by_id: BTreeMap<String, Value> = BTreeMap::new();
            let mut scoped_existing_ids: Vec<String> = Vec::new();
            for ex in &existing {
                if let Some(oid) = ex.get("id").and_then(|v| v.as_str()) {
                    existing_by_id.insert(oid.to_string(), ex.clone());
                    let matches_provider = scope_provider
                        .is_none_or(|sp| ex.get("provider").and_then(|v| v.as_str()) == Some(sp));
                    let matches_runtime = scope_runtime
                        .is_none_or(|sr| ex.get("runtime").and_then(|v| v.as_str()) == Some(sr));
                    if matches_provider && matches_runtime {
                        scoped_existing_ids.push(oid.to_string());
                    }
                }
            }
            if let Some(exp_map) = expected_revs {
                // Verify all scoped existing profiles and expected entries match revisions atomically.
                for oid in &scoped_existing_ids {
                    let actual = existing_by_id
                        .get(oid)
                        .and_then(|r| r.get("revision"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let expected = exp_map.get(oid).and_then(|v| v.as_u64()).unwrap_or(0);
                    if expected != actual {
                        return Err((
                            "revision_conflict",
                            format!(
                                "revision_conflict: model profile '{oid}' expected revision {expected}, actual {actual}"
                            ),
                        ));
                    }
                }
                for (exp_id, exp_val) in exp_map {
                    let expected = exp_val.as_u64().unwrap_or(0);
                    let actual = existing_by_id
                        .get(exp_id)
                        .and_then(|r| r.get("revision"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    if expected != actual {
                        return Err((
                            "revision_conflict",
                            format!(
                                "revision_conflict: model profile '{exp_id}' expected revision {expected}, actual {actual}"
                            ),
                        ));
                    }
                }
            }
            let incoming_ids: HashSet<String> = profiles
                .iter()
                .filter_map(|p| p.get("id").and_then(|v| v.as_str()).map(ToString::to_string))
                .collect();
            for oid in &scoped_existing_ids {
                if !incoming_ids.contains(oid) {
                    let _ = suite.delete_record("model", oid);
                }
            }
            let mut saved = Vec::new();
            for p in profiles {
                let id = p
                    .get("id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(new_id);
                let prev = existing_by_id.get(&id);
                let prev_rev = prev
                    .and_then(|r| r.get("revision"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let unchanged = prev.is_some_and(|ex| {
                    ex.get("display_name") == p.get("display_name")
                        && ex.get("provider") == p.get("provider")
                        && ex.get("model_name") == p.get("model_name")
                        && ex.get("runtime") == p.get("runtime")
                        && ex.get("source_json") == p.get("source_json")
                        && ex.get("parameters_json") == p.get("parameters_json")
                        && ex.get("selected") == p.get("selected")
                        && ex.get("enabled") == p.get("enabled")
                        && ex.get("deleted_at_ms") == p.get("deleted_at_ms")
                });
                if unchanged {
                    if let Some(ex) = prev {
                        saved.push(ex.clone());
                        continue;
                    }
                }
                let next_rev = prev_rev + 1;
                let mut rec = p.clone();
                if let Some(obj) = rec.as_object_mut() {
                    obj.insert("id".into(), json!(id));
                    obj.insert("revision".into(), json!(next_rev));
                    if let Some(prev_rec) = prev {
                        if let Some(created) = prev_rec.get("created_at_ms") {
                            obj.insert("created_at_ms".into(), created.clone());
                        }
                    }
                    obj.entry("updated_at_ms".to_string()).or_insert_with(|| json!(ts));
                }
                suite
                    .upsert_record("model", &id, &rec)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                saved.push(rec);
            }
            let all_now: Vec<Value> = suite.list_records("model").unwrap_or_default();
            let _ = store.set_kv("web_model_profiles", &json!(all_now));
            Ok(json!({"ok": true, "count": saved.len(), "profiles": saved}))
        }

        // --- Skill Domain ---
        ("skill", "list" | "list_editors") => {
            let persisted: Vec<Value> = suite
                .list_records::<Value>("skill")
                .unwrap_or_default()
                .into_iter()
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .collect();
            if op == "list_editors" {
                let items: Vec<Value> = persisted
                    .iter()
                    .map(|raw| {
                        let rev = raw.get("revision").and_then(|v| v.as_u64()).unwrap_or(1);
                        canonical_skill_editor_envelope(raw, None, rev)
                    })
                    .collect();
                return Ok(json!({"kind": "skill", "items": items, "skills": items}));
            }
            let items: Vec<Value> = persisted
                .iter()
                .map(|raw| {
                    let rev = raw.get("revision").and_then(|v| v.as_u64()).unwrap_or(1);
                    let env = canonical_skill_editor_envelope(raw, None, rev);
                    env.get("record").cloned().unwrap_or(env)
                })
                .collect();
            Ok(json!({"kind": "skill", "items": items, "skills": items}))
        }
        ("skill", "get" | "get_editor") => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("skill_id"))
                .or_else(|| payload.get("name"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            if let Ok(Some(rec)) = suite.get_record::<Value>("skill", id) {
                if rec.get("deleted_at_ms").is_some_and(|v| !v.is_null()) {
                    return Ok(Value::Null);
                }
                let rev = rec.get("revision").and_then(|v| v.as_u64()).unwrap_or(1);
                return Ok(canonical_skill_editor_envelope(&rec, Some(id), rev));
            }
            Ok(Value::Null)
        }
        ("skill", "upsert" | "create_user" | "configure_user") => {
            let input = payload.get("input").unwrap_or(payload);
            let record = input
                .get("skill")
                .or_else(|| input.get("record"))
                .unwrap_or(input);
            let id = record
                .get("id")
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("id"))
                .or_else(|| record.get("name"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let prev = suite.get_record::<Value>("skill", &id).ok().flatten();
            let is_tombstoned = prev
                .as_ref()
                .and_then(|v| v.get("deleted_at_ms"))
                .is_some_and(|v| !v.is_null());
            let prev_rev = prev
                .as_ref()
                .and_then(|v| v.get("revision").and_then(|r| r.as_u64()))
                .unwrap_or(0);
            if op == "create_user" {
                if prev.is_some() {
                    return Err((
                        "conflict",
                        format!("skill '{id}' already exists or was previously deleted"),
                    ));
                }
            } else if op == "configure_user" {
                if prev.is_none() || is_tombstoned {
                    return Err((
                        "skill_deleted",
                        format!("skill '{id}' not found or was deleted"),
                    ));
                }
                if let Some(exp) = payload
                    .get("expected_revision")
                    .or_else(|| input.get("expected_revision"))
                    .or_else(|| args.get("__envelope_expected_revision"))
                    .and_then(|v| v.as_u64())
                {
                    if exp != prev_rev {
                        return Err((
                            "skill_revision_conflict",
                            format!(
                                "skill_revision_conflict: skill '{id}' expected revision {exp}, actual {prev_rev}"
                            ),
                        ));
                    }
                }
            } else if is_tombstoned {
                return Err((
                    "conflict",
                    format!("skill '{id}' was deleted and cannot be resurrected"),
                ));
            }
            let next_rev = prev_rev + 1;
            let ts = now_ms();
            let mut merged = record.clone();
            if let Some(obj) = merged.as_object_mut() {
                obj.insert("id".into(), json!(id));
                if let Some(src) = obj.get("source").and_then(|v| v.as_str()) {
                    if src == "builtin" {
                        obj.insert("source".into(), json!("built_in"));
                    }
                }
                if !obj.contains_key("created_at_ms") {
                    let prev_created = prev
                        .as_ref()
                        .and_then(|v| v.get("created_at_ms").and_then(|c| c.as_i64()))
                        .unwrap_or(ts);
                    obj.insert("created_at_ms".into(), json!(prev_created));
                }
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            let envelope = canonical_skill_editor_envelope(&merged, Some(&id), next_rev);
            let canonical_rec = envelope.get("record").cloned().unwrap_or(merged.clone());
            if let Some(enabled) = canonical_rec.get("enabled").and_then(|v| v.as_bool()) {
                let skill_name = canonical_rec
                    .get("display_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&id);
                let _ = nola_modules::skills::set_skill_enabled(store, &id, enabled);
                if skill_name != id {
                    let _ = nola_modules::skills::set_skill_enabled(store, skill_name, enabled);
                }
            }
            let mut to_store = canonical_rec.clone();
            if let Some(obj) = to_store.as_object_mut() {
                obj.insert("revision".into(), json!(next_rev));
            }
            suite
                .upsert_record("skill", &id, &to_store)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "id": id,
                "revision": next_rev,
                "record": canonical_rec,
                "skill": canonical_rec
            }))
        }
        ("skill", "synchronize_built_ins") => {
            let mut editors = Vec::new();
            if let Some(inputs) = payload.get("inputs").and_then(|v| v.as_array()) {
                // Validate entire batch before persisting any item (atomic rollback on invalid package).
                let mut seen_ids: HashSet<String> = HashSet::new();
                for item in inputs {
                    let id = item
                        .get("id")
                        .or_else(|| item.get("name"))
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .ok_or(("invalid_argument", "built-in skill input missing non-empty id".into()))?;
                    if !seen_ids.insert(id.to_string()) {
                        return Err(("invalid_argument", format!("duplicate built-in skill id '{id}'")));
                    }
                    let display_name = item
                        .get("display_name")
                        .or_else(|| item.get("name"))
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .unwrap_or("");
                    if display_name.is_empty() {
                        return Err(("invalid_argument", format!("built-in skill '{id}' missing display_name")));
                    }
                    if let Some(inst) = item
                        .get("instructions_text")
                        .or_else(|| item.get("instructions"))
                        .and_then(|v| v.as_str())
                    {
                        if inst.trim().is_empty() {
                            return Err(("invalid_argument", format!("built-in skill '{id}' has empty instructions")));
                        }
                    }
                }
                for item in inputs {
                    let id = item
                        .get("id")
                        .or_else(|| item.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("skill")
                        .trim()
                        .to_string();
                    let prev = suite.get_record::<Value>("skill", &id).ok().flatten();
                    let prev_rev = prev
                        .as_ref()
                        .and_then(|v| v.get("revision").and_then(|r| r.as_u64()))
                        .unwrap_or(0);
                    let mut merged = item.clone();
                    if let (Some(prev_obj), Some(obj)) = (prev.as_ref().and_then(|v| v.as_object()), merged.as_object_mut()) {
                        if let Some(en) = prev_obj.get("enabled") {
                            obj.insert("enabled".into(), en.clone());
                        }
                        if let Some(created) = prev_obj.get("created_at_ms") {
                            obj.insert("created_at_ms".into(), created.clone());
                        }
                    }
                    let cand_env = canonical_skill_editor_envelope(&merged, Some(&id), prev_rev.max(1));
                    let cand_rec = cand_env.get("record").cloned().unwrap_or_else(|| merged.clone());
                    let changed = match &prev {
                        None => true,
                        Some(p) => {
                            let prev_canon = canonical_skill_editor_envelope(p, Some(&id), prev_rev.max(1));
                            let pr = prev_canon.get("record").unwrap_or(p);
                            pr.get("display_name") != cand_rec.get("display_name")
                                || pr.get("summary") != cand_rec.get("summary")
                                || pr.get("category") != cand_rec.get("category")
                                || pr.get("instructions_text") != cand_rec.get("instructions_text")
                                || pr.get("instructions_ref") != cand_rec.get("instructions_ref")
                                || pr.get("required_tools") != cand_rec.get("required_tools")
                                || pr.get("default_mode") != cand_rec.get("default_mode")
                                || pr.get("version") != cand_rec.get("version")
                        }
                    };
                    let next_rev = if prev_rev == 0 {
                        1
                    } else if changed {
                        prev_rev + 1
                    } else {
                        prev_rev
                    };
                    if !changed {
                        if let Some(ref p) = prev {
                            editors.push(canonical_skill_editor_envelope(p, Some(&id), next_rev));
                            continue;
                        }
                    }
                    let env = canonical_skill_editor_envelope(&merged, Some(&id), next_rev);
                    let mut to_store = env.get("record").cloned().unwrap_or_else(|| merged.clone());
                    if let Some(obj) = to_store.as_object_mut() {
                        obj.insert("revision".into(), json!(next_rev));
                    }
                    suite
                        .upsert_record("skill", &id, &to_store)
                        .map_err(|e| ("storage_error", e.to_string()))?;
                    editors.push(env);
                }
            } else {
                for b in nola_modules::skills::builtin_skills() {
                    let val = serde_json::to_value(&b).unwrap_or_else(|_| json!({}));
                    editors.push(canonical_skill_editor_envelope(&val, Some(&b.name), 1));
                }
            }
            Ok(Value::Array(editors))
        }
        ("skill", "restore_built_ins") => {
            let input = payload.get("input").unwrap_or(payload);
            let ts = payload
                .get("updated_at_ms")
                .or_else(|| input.get("updated_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            // 1. Always validate expected_visibility_revision against product_setting:skill.builtin.hidden
            // BEFORE mutating anything (omitted field means Swift passed nil => expects no hidden setting).
            {
                let exp_vis = payload
                    .get("expected_visibility_revision")
                    .or_else(|| input.get("expected_visibility_revision"))
                    .and_then(|v| v.as_u64());
                let cur_vis = load_product_setting_entry(store, "skill.builtin.hidden")?
                    .map(|(_, _, rev, _)| rev);
                if exp_vis != cur_vis {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": "product_setting:skill.builtin.hidden",
                            "expected_revision": exp_vis.unwrap_or(0),
                            "actual_revision": cur_vis.unwrap_or(0)
                        })
                        .to_string(),
                    ));
                }
            }
            // 2. Validate expected_revisions on all named skills BEFORE mutating anything.
            let expected_revs = payload
                .get("expected_revisions")
                .or_else(|| payload.get("expectedRevisions"))
                .or_else(|| input.get("expected_revisions"))
                .and_then(|v| v.as_object());
            if let Some(exp_map) = expected_revs {
                for (sid, exp_val) in exp_map {
                    let exp = exp_val.as_u64().unwrap_or(0);
                    let actual = suite
                        .get_record::<Value>("skill", sid)
                        .ok()
                        .flatten()
                        .and_then(|r| r.get("revision").and_then(|v| v.as_u64()))
                        .unwrap_or(1);
                    if exp != actual {
                        return Err((
                            "revision_conflict",
                            json!({
                                "scope_id": format!("skill:{sid}"),
                                "expected_revision": exp,
                                "actual_revision": actual
                            })
                            .to_string(),
                        ));
                    }
                }
            }
            // 3. Clear product_setting:skill.builtin.hidden (tombstone so get_product_setting returns null).
            let prev_vis_rev = load_product_setting_entry(store, "skill.builtin.hidden")?
                .map(|(_, _, r, _)| r)
                .unwrap_or(0);
            let _ = store.set_kv(
                "product_setting:skill.builtin.hidden",
                &json!({
                    "__nola_setting": true,
                    "deleted": true,
                    "revision": prev_vis_rev + 1,
                    "updated_at_ms": ts
                }),
            );
            let mut editors = Vec::new();
            let builtins = nola_modules::skills::builtin_skills();
            for b in &builtins {
                let _ = nola_modules::skills::set_skill_enabled(store, &b.name, true);
                let _ = store.set_kv(&format!("skill_builtin_removed:{}", b.name), &json!(false));
            }
            for rec in suite.list_records::<Value>("skill").unwrap_or_default() {
                if rec.get("source").and_then(|v| v.as_str()) == Some("built_in") {
                    let id = rec
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("skill")
                        .to_string();
                    let cur_rev = rec.get("revision").and_then(|v| v.as_u64()).unwrap_or(1);
                    let was_enabled = rec.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
                    let next_rev = if was_enabled { cur_rev } else { cur_rev + 1 };
                    let mut updated = rec.clone();
                    if let Some(obj) = updated.as_object_mut() {
                        obj.insert("enabled".into(), json!(true));
                        obj.insert("deleted_at_ms".into(), Value::Null);
                        obj.insert("revision".into(), json!(next_rev));
                        if !was_enabled {
                            obj.insert("updated_at_ms".into(), json!(ts));
                        }
                    }
                    let _ = suite.upsert_record("skill", &id, &updated);
                    editors.push(canonical_skill_editor_envelope(&updated, Some(&id), next_rev));
                }
            }
            Ok(Value::Array(editors))
        }
        ("skill", "remove_built_in") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = input
                .get("id")
                .or_else(|| input.get("name"))
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing skill 'id' or 'name'".into()))?;
            let ts = payload
                .get("updated_at_ms")
                .or_else(|| input.get("updated_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let mut rec = suite
                .get_record::<Value>("skill", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("built-in skill '{id}' not found")))?;
            let cur_rev = rec.get("revision").and_then(|r| r.as_u64()).unwrap_or(1);
            if let Some(exp) = payload
                .get("expected_revision")
                .or_else(|| input.get("expected_revision"))
                .and_then(|v| v.as_u64())
            {
                if exp != cur_rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": format!("skill_revision_conflict:skill:{id}"),
                            "expected_revision": exp,
                            "actual_revision": cur_rev
                        })
                        .to_string(),
                    ));
                }
            }
            let cur_vis = load_product_setting_entry(store, "skill.builtin.hidden")?;
            let cur_vis_rev = cur_vis.as_ref().map(|(_, _, r, _)| *r);
            let has_vis_field = payload.get("expected_visibility_revision").is_some()
                || input.get("expected_visibility_revision").is_some();
            if has_vis_field {
                let exp_vis = payload
                    .get("expected_visibility_revision")
                    .or_else(|| input.get("expected_visibility_revision"))
                    .and_then(|v| v.as_u64());
                if exp_vis != cur_vis_rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": "product_setting:skill.builtin.hidden",
                            "expected_revision": exp_vis.unwrap_or(0),
                            "actual_revision": cur_vis_rev.unwrap_or(0)
                        })
                        .to_string(),
                    ));
                }
            }
            // Add `id` to product_setting:skill.builtin.hidden JSON {"schemaVersion":1,"ids":[...]}.
            let mut hidden_ids: Vec<String> = cur_vis
                .as_ref()
                .and_then(|(v, _, _, _)| {
                    let parsed = if let Some(s) = v.as_str() {
                        serde_json::from_str::<Value>(s).ok()
                    } else {
                        Some(v.clone())
                    };
                    parsed
                        .and_then(|p| p.get("ids").and_then(|a| a.as_array()).cloned())
                        .map(|arr| {
                            arr.into_iter()
                                .filter_map(|x| x.as_str().map(ToString::to_string))
                                .collect()
                        })
                })
                .unwrap_or_default();
            if !hidden_ids.iter().any(|x| x == id) {
                hidden_ids.push(id.to_string());
                hidden_ids.sort();
            }
            let hidden_val = json!({"schemaVersion": 1, "ids": hidden_ids});
            let hidden_json = serde_json::to_string(&hidden_val).unwrap_or_else(|_| "{}".into());
            let next_vis_rev = cur_vis_rev.unwrap_or(0) + 1;
            let _ = store.set_kv(
                "product_setting:skill.builtin.hidden",
                &json!({
                    "__nola_setting": true,
                    "deleted": false,
                    "value": hidden_val,
                    "value_json": hidden_json,
                    "revision": next_vis_rev,
                    "updated_at_ms": ts
                }),
            );
            let _ = nola_modules::skills::set_skill_enabled(store, id, false);
            let _ = store.set_kv(&format!("skill_builtin_removed:{id}"), &json!(true));
            let next_rev = cur_rev + 1;
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("enabled".into(), json!(false));
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            let env = canonical_skill_editor_envelope(&rec, Some(id), next_rev);
            let mut to_store = env.get("record").cloned().unwrap_or_else(|| rec.clone());
            if let Some(obj) = to_store.as_object_mut() {
                obj.insert("revision".into(), json!(next_rev));
            }
            suite
                .upsert_record("skill", id, &to_store)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(env)
        }
        ("skill", "apply_changes") => {
            let input = payload.get("input").unwrap_or(payload);
            if let Some(commands) = payload
                .get("commands")
                .or_else(|| input.get("commands"))
                .and_then(|v| v.as_array())
            {
                // Two-phase validation so any revision conflict or invalid item rolls back the full batch.
                let mut staged: Vec<(String, Value, Value, bool)> = Vec::with_capacity(commands.len());
                for cmd in commands {
                    let id = cmd
                        .get("id")
                        .or_else(|| cmd.get("name"))
                        .and_then(|v| v.as_str())
                        .ok_or(("invalid_argument", "skill batch command missing 'id'".into()))?;
                    let enabled = cmd
                        .get("enabled")
                        .and_then(|v| v.as_bool())
                        .ok_or(("invalid_argument", "skill batch command missing 'enabled'".into()))?;
                    let base = suite
                        .get_record::<Value>("skill", id)
                        .map_err(|e| ("storage_error", e.to_string()))?
                        .ok_or(("not_found", format!("skill '{id}' not found")))?;
                    let cur_rev = base.get("revision").and_then(|r| r.as_u64()).unwrap_or(1);
                    if let Some(exp) = cmd
                        .get("expected_revision")
                        .or_else(|| cmd.get("expectedRevision"))
                        .and_then(|v| v.as_u64())
                    {
                        if exp != cur_rev {
                            return Err((
                                "revision_conflict",
                                json!({
                                    "scope_id": format!("skill:{id}"),
                                    "expected_revision": exp,
                                    "actual_revision": cur_rev
                                })
                                .to_string(),
                            ));
                        }
                    }
                    let already_enabled = base.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
                    if already_enabled == enabled {
                        let env = canonical_skill_editor_envelope(&base, Some(id), cur_rev);
                        staged.push((id.to_string(), base, env, false));
                        continue;
                    }
                    let next_rev = cur_rev + 1;
                    let ts = cmd
                        .get("updated_at_ms")
                        .or_else(|| cmd.get("updatedAtMs"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or_else(now_ms);
                    let mut updated = base;
                    if let Some(obj) = updated.as_object_mut() {
                        obj.insert("id".into(), json!(id));
                        obj.insert("enabled".into(), json!(enabled));
                        obj.insert("revision".into(), json!(next_rev));
                        obj.insert("updated_at_ms".into(), json!(ts));
                    }
                    let env = canonical_skill_editor_envelope(&updated, Some(id), next_rev);
                    let mut to_store = env.get("record").cloned().unwrap_or(updated);
                    if let Some(obj) = to_store.as_object_mut() {
                        obj.insert("revision".into(), json!(next_rev));
                    }
                    staged.push((id.to_string(), to_store, env, true));
                }
                let mut editors = Vec::with_capacity(staged.len());
                for (id, to_store, env, mutated) in staged {
                    if mutated {
                        if let Some(en) = to_store.get("enabled").and_then(|v| v.as_bool()) {
                            let _ = nola_modules::skills::set_skill_enabled(store, &id, en);
                        }
                        suite
                            .upsert_record("skill", &id, &to_store)
                            .map_err(|e| ("storage_error", e.to_string()))?;
                    }
                    editors.push(env);
                }
                return Ok(Value::Array(editors));
            }
            let mut applied = 0usize;
            if let Some(upserts) = input
                .get("upserts")
                .or_else(|| input.get("changes"))
                .and_then(|v| v.as_array())
            {
                for entry in upserts {
                    let sid = entry
                        .get("id")
                        .or_else(|| entry.get("name"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .map(ToString::to_string)
                        .unwrap_or_else(new_id);
                    let mut stored = entry.clone();
                    if let Some(obj) = stored.as_object_mut() {
                        obj.insert("id".into(), json!(sid));
                        obj.insert("updated_at_ms".into(), json!(now_ms()));
                    }
                    let _ = suite.upsert_record("skill", &sid, &stored);
                    if let Some(enabled) = stored.get("enabled").and_then(|v| v.as_bool()) {
                        let _ = nola_modules::skills::set_skill_enabled(store, &sid, enabled);
                    }
                    applied += 1;
                }
            }
            if let Some(deletes) = input.get("deletes").and_then(|v| v.as_array()) {
                for d in deletes {
                    if let Some(sid) = d.as_str() {
                        let _ = suite.delete_record("skill", sid);
                        applied += 1;
                    }
                }
            }
            let rev = store
                .get_kv("skills_revision")
                .ok()
                .flatten()
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                + 1;
            let _ = store.set_kv("skills_revision", &json!(rev));
            Ok(json!({"ok": true, "applied": applied, "revision": rev}))
        }
        ("skill", "set_enabled_checked") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = input
                .get("id")
                .or_else(|| input.get("name"))
                .or_else(|| payload.get("id"))
                .or_else(|| payload.get("name"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing skill 'id' or 'name'".into()))?;
            let enabled = input
                .get("enabled")
                .or_else(|| payload.get("enabled"))
                .or_else(|| args.get("enabled"))
                .and_then(|v| v.as_bool())
                .ok_or(("invalid_argument", "missing boolean 'enabled'".into()))?;
            let base = suite
                .get_record::<Value>("skill", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .ok_or(("not_found", format!("skill '{id}' not found or was deleted")))?;
            let cur_rev = base.get("revision").and_then(|r| r.as_u64()).unwrap_or(1);
            if let Some(exp) = input
                .get("expected_revision")
                .or_else(|| input.get("expectedRevision"))
                .or_else(|| payload.get("expected_revision"))
                .or_else(|| payload.get("expectedRevision"))
                .and_then(|v| v.as_u64())
            {
                if exp != cur_rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": format!("skill_revision_conflict:skill:{id}"),
                            "expected_revision": exp,
                            "actual_revision": cur_rev
                        })
                        .to_string(),
                    ));
                }
            }
            if base.get("enabled").and_then(|v| v.as_bool()) == Some(enabled) {
                return Ok(canonical_skill_editor_envelope(&base, Some(id), cur_rev));
            }
            nola_modules::skills::set_skill_enabled(store, id, enabled)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let next_rev = cur_rev + 1;
            let ts = input
                .get("updated_at_ms")
                .or_else(|| input.get("updatedAtMs"))
                .or_else(|| payload.get("updated_at_ms"))
                .or_else(|| payload.get("updatedAtMs"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let mut updated = base;
            if let Some(obj) = updated.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("enabled".into(), json!(enabled));
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            let envelope = canonical_skill_editor_envelope(&updated, Some(id), next_rev);
            let canonical_rec = envelope.get("record").cloned().unwrap_or(updated.clone());
            let mut to_store = canonical_rec.clone();
            if let Some(obj) = to_store.as_object_mut() {
                obj.insert("revision".into(), json!(next_rev));
            }
            suite
                .upsert_record("skill", id, &to_store)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(envelope)
        }
        ("skill", "delete_user_checked" | "delete_user" | "delete") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = input
                .get("id")
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let existing = suite
                .get_record::<Value>("skill", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .ok_or(("not_found", format!("skill '{id}' not found")))?;
            let cur_rev = existing.get("revision").and_then(|r| r.as_u64()).unwrap_or(1);
            if let Some(exp) = input
                .get("expected_revision")
                .or_else(|| input.get("expectedRevision"))
                .or_else(|| payload.get("expected_revision"))
                .or_else(|| payload.get("expectedRevision"))
                .and_then(|v| v.as_u64())
            {
                if exp != cur_rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": format!("skill_revision_conflict:skill:{id}"),
                            "expected_revision": exp,
                            "actual_revision": cur_rev
                        })
                        .to_string(),
                    ));
                }
            }
            let ts = input
                .get("deleted_at_ms")
                .or_else(|| input.get("deletedAtMs"))
                .or_else(|| payload.get("deleted_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let mut tomb = existing;
            if let Some(obj) = tomb.as_object_mut() {
                obj.insert("enabled".into(), json!(false));
                obj.insert("revision".into(), json!(cur_rev + 1));
                obj.insert("deleted_at_ms".into(), json!(ts));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("skill", id, &tomb)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let _ = nola_modules::skills::set_skill_enabled(store, id, false);
            Ok(json!(true))
        }

        // --- Memory Domain ---
        ("memory", "list" | "recall" | "search" | "query") => {
            let input = payload.get("input").unwrap_or(payload);
            let q = input
                .get("query")
                .or_else(|| payload.get("query"))
                .or_else(|| args.get("query"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let ns = input
                .get("namespace")
                .or_else(|| payload.get("namespace"))
                .or_else(|| args.get("namespace"))
                .and_then(|v| v.as_str());
            let limit = input
                .get("limit")
                .or_else(|| payload.get("limit"))
                .or_else(|| args.get("limit"))
                .and_then(|v| v.as_u64())
                .unwrap_or(100) as usize;
            let items = store
                .search_memory(q, ns, limit)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let records: Vec<Value> = items
                .iter()
                .map(|m| canonical_memory_envelope_with_meta(store, m))
                .collect();
            Ok(json!({"items": items, "records": records, "memories": records}))
        }
        ("memory", "get") => {
            let id = payload
                .get("id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let items = store
                .search_memory("", None, 500)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let found = items
                .into_iter()
                .find(|m| m.id == id)
                .map(|m| canonical_memory_envelope_with_meta(store, &m));
            Ok(found.unwrap_or(Value::Null))
        }
        ("memory", "commit" | "save" | "upsert" | "update_if_revision") => {
            let item = parse_memory_item(payload);
            store
                .upsert_memory(&item)
                .map_err(|e| ("storage_error", e.to_string()))?;
            save_memory_extra_meta(store, &item.id, payload);
            Ok(canonical_memory_envelope_with_meta(store, &item))
        }
        ("memory", "replace") => {
            let input = payload.get("input").unwrap_or(payload);
            if let Some(arr) = input
                .get("candidates")
                .or_else(|| input.get("records"))
                .or_else(|| input.get("items"))
                .or_else(|| payload.get("candidates"))
                .or_else(|| payload.get("records"))
                .or_else(|| payload.get("items"))
                .and_then(|v| v.as_array())
            {
                let parsed_items: Vec<(MemoryItem, &Value)> =
                    arr.iter().map(|v| (parse_memory_item(v), v)).collect();
                let incoming_ids: HashSet<&str> =
                    parsed_items.iter().map(|(m, _)| m.id.as_str()).collect();
                let existing = store
                    .search_memory("", None, 10_000)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                if let Some(expected_arr) = input
                    .get("expected")
                    .or_else(|| payload.get("expected"))
                    .and_then(|v| v.as_array())
                {
                    let mut actual_map: BTreeMap<String, (String, i64)> = BTreeMap::new();
                    for item in &existing {
                        let env = canonical_memory_envelope_with_meta(store, item);
                        let id = env
                            .pointer("/record/id")
                            .and_then(|v| v.as_str())
                            .unwrap_or(&item.id)
                            .to_string();
                        let content = env
                            .pointer("/record/content")
                            .and_then(|v| v.as_str())
                            .unwrap_or(&item.content)
                            .to_string();
                        let updated = env
                            .get("updated_at_ms")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(item.updated_at_ms);
                        actual_map.insert(id, (content, updated));
                    }
                    let mut exp_map: BTreeMap<String, (String, i64)> = BTreeMap::new();
                    for ev in expected_arr {
                        let rec = ev.get("record").unwrap_or(ev);
                        let id = rec
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let content = rec
                            .get("content")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let updated = ev
                            .get("updated_at_ms")
                            .or_else(|| rec.get("updated_at_ms"))
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                        exp_map.insert(id, (content, updated));
                    }
                    if actual_map != exp_map {
                        return Err((
                            "memory_snapshot_conflict",
                            "memory_snapshot_conflict: memory snapshot changed since editor loaded"
                                .into(),
                        ));
                    }
                }
                for old in existing {
                    if !incoming_ids.contains(old.id.as_str()) {
                        store
                            .delete_memory(&old.id)
                            .map_err(|e| ("storage_error", e.to_string()))?;
                        let _ = store.set_kv(&format!("mem_meta:{}", old.id), &Value::Null);
                    }
                }
                for (item, raw) in &parsed_items {
                    store
                        .upsert_memory(item)
                        .map_err(|e| ("storage_error", e.to_string()))?;
                    save_memory_extra_meta(store, &item.id, raw);
                }
                // Read back the persisted SQLite rows so returned created_at_ms/updated_at_ms
                // match subsequent memory.list snapshots bit-for-bit.
                let persisted = store
                    .search_memory("", None, 10_000)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                let records: Vec<Value> = persisted
                    .iter()
                    .map(|m| canonical_memory_envelope_with_meta(store, m))
                    .collect();
                return Ok(Value::Array(records));
            }
            let item = parse_memory_item(input);
            store
                .upsert_memory(&item)
                .map_err(|e| ("storage_error", e.to_string()))?;
            save_memory_extra_meta(store, &item.id, input);
            Ok(Value::Array(vec![canonical_memory_envelope_with_meta(store, &item)]))
        }
        ("memory", "delete") => {
            let id = payload
                .get("id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            store
                .delete_memory(id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"deleted": id, "ok": true}))
        }

        // --- MCP Domain ---
        ("mcp", "list") => {
            let servers = store
                .list_mcp_servers()
                .map_err(|e| ("storage_error", e.to_string()))?;
            let enriched: Vec<Value> = servers
                .into_iter()
                .map(|s| enrich_mcp_server_json(store, &s))
                .collect();
            Ok(json!({"kind": "mcp", "items": enriched, "servers": enriched}))
        }
        ("mcp", "get") => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("server_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let found = store
                .list_mcp_servers()
                .map_err(|e| ("storage_error", e.to_string()))?
                .into_iter()
                .find(|s| s.id == id);
            Ok(found
                .map(|s| enrich_mcp_server_json(store, &s))
                .unwrap_or(Value::Null))
        }
        ("mcp", "create" | "configure" | "update" | "update_checked" | "upsert") => {
            let input = payload.get("input").unwrap_or(payload);
            let cfg = parse_mcp_server_config(payload)?;
            let existing = store
                .list_mcp_servers()
                .map_err(|e| ("storage_error", e.to_string()))?
                .into_iter()
                .find(|s| s.id == cfg.id);
            let prev_meta = store
                .get_kv(&format!("mcp_meta:{}", cfg.id))
                .ok()
                .flatten();
            let is_tombstoned = prev_meta
                .as_ref()
                .and_then(|m| m.get("deleted_at_ms"))
                .is_some_and(|v| !v.is_null());
            let cur_rev = prev_meta
                .as_ref()
                .and_then(|m| m.get("revision"))
                .and_then(|r| r.as_u64())
                .unwrap_or(if existing.is_some() { 1 } else { 0 });
            if op == "create" {
                if existing.is_some() || is_tombstoned {
                    return Err((
                        "mcp_identity_conflict",
                        format!(
                            "mcp_identity_conflict: mcp server '{}' already exists or was previously deleted",
                            cfg.id
                        ),
                    ));
                }
            } else if matches!(op, "configure" | "update" | "update_checked") {
                if existing.is_none() || is_tombstoned {
                    return Err((
                        "not_found",
                        format!("mcp server '{}' is missing or deleted", cfg.id),
                    ));
                }
                if let Some(exp) = payload
                    .get("expected_revision")
                    .or_else(|| input.get("expected_revision"))
                    .or_else(|| args.get("__envelope_expected_revision"))
                    .and_then(|v| v.as_u64())
                {
                    if exp != cur_rev {
                        return Err((
                            "mcp_revision_conflict",
                            format!(
                                "mcp_revision_conflict: mcp server '{}' expected revision {}, actual {}",
                                cfg.id, exp, cur_rev
                            ),
                        ));
                    }
                }
            } else if is_tombstoned {
                return Err((
                    "mcp_identity_conflict",
                    format!(
                        "mcp_identity_conflict: mcp server '{}' was deleted and cannot be resurrected",
                        cfg.id
                    ),
                ));
            }
            store
                .upsert_mcp_server(&cfg)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let prev_meta = prev_meta.unwrap_or_else(|| json!({}));
            let next_rev = cur_rev + 1;
            let ts = now_ms();
            let created_at_ms = prev_meta
                .get("created_at_ms")
                .and_then(|v| v.as_i64())
                .or_else(|| input.get("created_at_ms").and_then(|v| v.as_i64()))
                .unwrap_or(ts);
            let updated_at_ms = input
                .get("updated_at_ms")
                .or_else(|| payload.get("updated_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or(ts);
            let display_name = input
                .get("display_name")
                .or_else(|| input.get("name"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .or_else(|| {
                    prev_meta
                        .get("display_name")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                })
                .unwrap_or_else(|| cfg.name.clone());
            let configuration_json = input
                .get("configuration_json")
                .or_else(|| payload.get("configuration_json"))
                .map(|v| {
                    v.as_str()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| v.to_string())
                })
                .or_else(|| {
                    prev_meta
                        .get("configuration_json")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                })
                .unwrap_or_else(|| "{}".into());
            let _ = store.set_kv(
                &format!("mcp_meta:{}", cfg.id),
                &json!({
                    "display_name": display_name,
                    "enabled": cfg.enabled,
                    "configuration_json": configuration_json,
                    "revision": next_rev,
                    "created_at_ms": created_at_ms,
                    "updated_at_ms": updated_at_ms,
                    "deleted_at_ms": Value::Null
                }),
            );
            let enriched = enrich_mcp_server_json(store, &cfg);
            Ok(json!({
                "ok": true,
                "id": cfg.id,
                "display_name": display_name,
                "enabled": cfg.enabled,
                "configuration_json": configuration_json,
                "revision": next_rev,
                "__committed_revision": next_rev,
                "created_at_ms": created_at_ms,
                "updated_at_ms": updated_at_ms,
                "deleted_at_ms": Value::Null,
                "server": enriched
            }))
        }
        ("mcp", "delete_checked" | "delete") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = payload
                .get("id")
                .or_else(|| payload.get("server_id"))
                .or_else(|| input.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let existing = store
                .list_mcp_servers()
                .map_err(|e| ("storage_error", e.to_string()))?
                .into_iter()
                .find(|s| s.id == id);
            let prev_meta = store.get_kv(&format!("mcp_meta:{id}")).ok().flatten();
            let is_tombstoned = prev_meta
                .as_ref()
                .and_then(|m| m.get("deleted_at_ms"))
                .is_some_and(|v| !v.is_null());
            if existing.is_none() || is_tombstoned {
                if op == "delete_checked" {
                    return Err(("not_found", format!("mcp server '{id}' not found")));
                }
                return Ok(json!(false));
            }
            let cur_rev = prev_meta
                .as_ref()
                .and_then(|m| m.get("revision"))
                .and_then(|r| r.as_u64())
                .unwrap_or(1);
            if let Some(exp) = payload
                .get("expected_revision")
                .or_else(|| input.get("expected_revision"))
                .or_else(|| args.get("__envelope_expected_revision"))
                .and_then(|v| v.as_u64())
            {
                if exp != cur_rev {
                    return Err((
                        "mcp_revision_conflict",
                        format!(
                            "mcp_revision_conflict: mcp server '{id}' expected revision {exp}, actual {cur_rev}"
                        ),
                    ));
                }
            }
            let deleted = store
                .delete_mcp_server(id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let ts = payload
                .get("deleted_at_ms")
                .or_else(|| input.get("deleted_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let mut tomb = prev_meta.unwrap_or_else(|| json!({}));
            if let Some(obj) = tomb.as_object_mut() {
                obj.insert("revision".into(), json!(cur_rev + 1));
                obj.insert("deleted_at_ms".into(), json!(ts));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            let _ = store.set_kv(&format!("mcp_meta:{id}"), &tomb);
            Ok(json!(deleted))
        }

        // --- DataSource Domain ---
        ("data_source", "list") => {
            let items: Vec<Value> = suite
                .list_records::<Value>("data_source")
                .map_err(|e| ("storage_error", e.to_string()))?
                .into_iter()
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .map(|r| canonicalize_data_source_record(r, None))
                .collect();
            Ok(json!({"kind": "data_source", "items": items}))
        }
        ("data_source", "list_usage") => {
            let limit = payload
                .get("limit")
                .or_else(|| args.get("limit"))
                .and_then(|v| v.as_u64())
                .unwrap_or(200) as usize;
            let source_filter = payload
                .get("source_id")
                .or_else(|| args.get("source_id"))
                .and_then(|v| v.as_str());
            let mut items: Vec<Value> = suite
                .list_records("data_source_usage")
                .map_err(|e| ("storage_error", e.to_string()))?;
            if let Some(sf) = source_filter {
                items.retain(|item| item.get("source_id").and_then(|v| v.as_str()) == Some(sf));
            }
            items.sort_by(|a, b| {
                let ta = a.get("used_at_ms").and_then(|v| v.as_i64()).unwrap_or(0);
                let tb = b.get("used_at_ms").and_then(|v| v.as_i64()).unwrap_or(0);
                tb.cmp(&ta)
            });
            items.truncate(limit);
            Ok(json!({"kind": "data_source_usage", "items": items, "usage": items}))
        }
        ("data_source", "get") => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("data_source_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let found = suite
                .get_record::<Value>("data_source", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .map(|r| canonicalize_data_source_record(r, Some(id)));
            Ok(found.unwrap_or(Value::Null))
        }
        ("data_source", "create_draft" | "create") => {
            let input = payload.get("input").unwrap_or(payload);
            let record = input.get("record").unwrap_or(input);
            let id = record
                .get("id")
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            if op == "create_draft" {
                if let Some(ex) = suite
                    .get_record::<Value>("data_source", &id)
                    .map_err(|e| ("storage_error", e.to_string()))?
                {
                    if ex.get("deleted_at_ms").is_none_or(|v| v.is_null()) {
                        return Err((
                            "conflict",
                            format!("data source '{id}' already exists"),
                        ));
                    }
                }
            }
            let ts = record
                .get("updated_at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let created_ts = record
                .get("created_at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(ts);
            let mut stored = canonicalize_data_source_record(record.clone(), Some(&id));
            if let Some(obj) = stored.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("revision".into(), json!(1i64));
                obj.insert("lifecycle".into(), json!("draft"));
                obj.insert("default_enabled".into(), json!(false));
                obj.insert("created_at_ms".into(), json!(created_ts));
                obj.insert("updated_at_ms".into(), json!(ts));
                obj.insert("deleted_at_ms".into(), Value::Null);
            }
            suite
                .upsert_record("data_source", &id, &stored)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(stored)
        }
        ("data_source", "update_configuration" | "update") => {
            let input = payload.get("input").unwrap_or(payload);
            let record = input
                .get("configuration")
                .filter(|c| c.get("name").is_some())
                .or_else(|| input.get("record"))
                .unwrap_or(input);
            let id = payload
                .get("id")
                .or_else(|| payload.get("data_source_id"))
                .or_else(|| input.get("id"))
                .or_else(|| input.get("data_source_id"))
                .or_else(|| record.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let existing = suite
                .get_record::<Value>("data_source", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .ok_or(("not_found", format!("data source '{id}' not found")))?;
            let cur_rev = existing
                .get("revision")
                .and_then(|v| v.as_i64())
                .unwrap_or(1);
            if let Some(exp) = payload
                .get("expected_revision")
                .or_else(|| input.get("expected_revision"))
                .or_else(|| args.get("__envelope_expected_revision"))
                .and_then(|v| v.as_i64())
            {
                if exp != cur_rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": format!("data_source:{id}"),
                            "expected_revision": exp.max(0) as u64,
                            "actual_revision": cur_rev.max(0) as u64
                        })
                        .to_string(),
                    ));
                }
            }
            let mut merged = canonicalize_data_source_record(existing, Some(id));
            let next_rev = cur_rev + 1;
            let ts = record
                .get("updated_at_ms")
                .or_else(|| input.get("updated_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            if let (Some(dst), Some(src)) = (merged.as_object_mut(), record.as_object()) {
                for (k, v) in src {
                    if !matches!(k.as_str(), "type" | "expected_revision") {
                        dst.insert(k.clone(), v.clone());
                    }
                }
                dst.insert("id".into(), json!(id));
                dst.insert("revision".into(), json!(next_rev));
                dst.insert("updated_at_ms".into(), json!(ts));
                // Invalidate any in-flight read_token when configuration changes.
                if let Some(rs) = dst.get_mut("read_state").and_then(|v| v.as_object_mut()) {
                    rs.insert("read_token".into(), Value::Null);
                    if rs.get("status").and_then(|v| v.as_str()) == Some("pending") {
                        rs.insert("status".into(), json!("never_read"));
                    }
                }
            }
            let merged = canonicalize_data_source_record(merged, Some(id));
            suite
                .upsert_record("data_source", id, &merged)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(merged)
        }
        ("data_source", "request_validation") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = payload
                .get("id")
                .or_else(|| payload.get("data_source_id"))
                .or_else(|| input.get("data_source_id"))
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let existing = suite
                .get_record::<Value>("data_source", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .ok_or(("not_found", format!("data source '{id}' not found")))?;
            let cur_rev = existing
                .get("revision")
                .and_then(|v| v.as_i64())
                .unwrap_or(1);
            if let Some(exp) = payload
                .get("expected_revision")
                .or_else(|| input.get("expected_revision"))
                .and_then(|v| v.as_i64())
            {
                if exp != cur_rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": format!("data_source:{id}"),
                            "expected_revision": exp.max(0) as u64,
                            "actual_revision": cur_rev.max(0) as u64
                        })
                        .to_string(),
                    ));
                }
            }
            let ts = payload
                .get("requested_at_ms")
                .or_else(|| input.get("requested_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let token = new_id();
            let validation = json!({
                "status": "pending",
                "validation_token": token,
                "contract": Value::Null,
                "diagnostic": Value::Null,
                "attempted_at_ms": ts,
                "completed_at_ms": Value::Null
            });
            let _ = store.set_kv(&format!("ds_validation:{id}"), &validation);
            let mut rec = canonicalize_data_source_record(existing, Some(id));
            let next_rev = cur_rev + 1;
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("validation".into(), validation);
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("data_source", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(rec)
        }
        ("data_source", "record_validation") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = payload
                .get("id")
                .or_else(|| payload.get("data_source_id"))
                .or_else(|| input.get("data_source_id"))
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let existing = suite
                .get_record::<Value>("data_source", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .ok_or(("not_found", format!("data source '{id}' not found")))?;
            let mut rec = canonicalize_data_source_record(existing, Some(id));
            let expected_token = rec
                .get("validation")
                .and_then(|v| v.get("validation_token"))
                .and_then(|v| v.as_str());
            if let Some(provided_token) = payload
                .get("validation_token")
                .or_else(|| input.get("validation_token"))
                .and_then(|v| v.as_str())
            {
                if expected_token != Some(provided_token) {
                    return Err((
                        "conflict",
                        "data source validation token does not match pending validation".into(),
                    ));
                }
            }
            let ts = payload
                .get("recorded_at_ms")
                .or_else(|| input.get("recorded_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let res_obj = payload
                .get("result")
                .or_else(|| input.get("result"))
                .unwrap_or(input);
            let raw_status = res_obj
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("succeeded");
            let (norm_status, lifecycle) = match raw_status {
                "succeeded" | "verified" => ("verified", "ready"),
                "succeeded_empty" | "verified_empty" => ("verified_empty", "ready"),
                "failed" => ("failed", "failed"),
                "pending" => ("pending", "draft"),
                _ => ("verified", "ready"),
            };
            let attempted_at = rec
                .get("validation")
                .and_then(|v| v.get("attempted_at_ms"))
                .cloned()
                .unwrap_or(json!(ts));
            let validation = json!({
                "status": norm_status,
                "validation_token": Value::Null,
                "contract": res_obj.get("contract").cloned().unwrap_or(Value::Null),
                "diagnostic": res_obj.get("diagnostic").cloned().unwrap_or(Value::Null),
                "attempted_at_ms": attempted_at,
                "completed_at_ms": ts
            });
            let _ = store.set_kv(&format!("ds_validation:{id}"), &validation);
            let cur_rev = rec.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
            let next_rev = cur_rev + 1;
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("validation".into(), validation);
                obj.insert("lifecycle".into(), json!(lifecycle));
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("data_source", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(rec)
        }
        ("data_source", "set_default_enabled") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = payload
                .get("id")
                .or_else(|| payload.get("data_source_id"))
                .or_else(|| input.get("data_source_id"))
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let enabled = payload
                .get("enabled")
                .or_else(|| payload.get("default_enabled"))
                .or_else(|| input.get("default_enabled"))
                .or_else(|| input.get("enabled"))
                .and_then(|v| v.as_bool())
                .ok_or(("invalid_argument", "missing boolean 'enabled'".into()))?;
            let existing = suite
                .get_record::<Value>("data_source", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .ok_or(("not_found", format!("data source '{id}' not found")))?;
            let cur_rev = existing
                .get("revision")
                .and_then(|v| v.as_i64())
                .unwrap_or(1);
            if let Some(exp) = payload
                .get("expected_revision")
                .or_else(|| input.get("expected_revision"))
                .and_then(|v| v.as_i64())
            {
                if exp != cur_rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": format!("data_source:{id}"),
                            "expected_revision": exp.max(0) as u64,
                            "actual_revision": cur_rev.max(0) as u64
                        })
                        .to_string(),
                    ));
                }
            }
            let ts = payload
                .get("updated_at_ms")
                .or_else(|| input.get("updated_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let mut rec = canonicalize_data_source_record(existing, Some(id));
            let next_rev = cur_rev + 1;
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("default_enabled".into(), json!(enabled));
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("data_source", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(rec)
        }
        ("data_source", "get_conversation_override") => {
            let input = payload.get("input").unwrap_or(payload);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let ds_id = input
                .get("source_id")
                .or_else(|| input.get("data_source_id"))
                .or_else(|| input.get("id"))
                .or_else(|| args.get("source_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'source_id'".into()))?;
            let val = store
                .get_kv(&format!("ds_override:{conv_id}:{ds_id}"))
                .map_err(|e| ("storage_error", e.to_string()))?
                .unwrap_or_else(|| json!("inherit"));
            Ok(val)
        }
        ("data_source", "set_conversation_override") => {
            let input = payload.get("input").unwrap_or(payload);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            if store
                .get_session(conv_id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .is_none()
            {
                return Err((
                    "not_found",
                    format!("conversation '{conv_id}' not found"),
                ));
            }
            let ds_id = input
                .get("source_id")
                .or_else(|| input.get("data_source_id"))
                .or_else(|| input.get("id"))
                .or_else(|| args.get("source_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'source_id'".into()))?;
            let override_val = input
                .get("value")
                .or_else(|| input.get("override"))
                .or_else(|| input.get("enabled"))
                .cloned()
                .unwrap_or_else(|| json!("inherit"));
            let key = format!("ds_override:{conv_id}:{ds_id}");
            store
                .set_kv(&key, &override_val)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!(true))
        }
        ("data_source", "record_usage") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = input
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let source_id = input
                .get("source_id")
                .or_else(|| input.get("data_source_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let source_title = input
                .get("source_title")
                .and_then(|v| v.as_str())
                .unwrap_or(source_id);
            let consumer = input
                .get("consumer")
                .and_then(|v| v.as_str())
                .unwrap_or("chat");
            let purpose = input
                .get("purpose")
                .or_else(|| input.get("summary"))
                .and_then(|v| v.as_str())
                .unwrap_or("conversation_context");
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(ToString::to_string);
            let disclosed = input.get("disclosed_to_cloud").cloned().unwrap_or(Value::Null);
            let metadata = input
                .get("metadata")
                .cloned()
                .filter(|v| v.is_object())
                .unwrap_or_else(|| json!({}));
            let used_at_ms = input
                .get("used_at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let record = json!({
                "id": id,
                "source_id": source_id,
                "source_title": source_title,
                "consumer": consumer,
                "purpose": purpose,
                "conversation_id": conv_id,
                "disclosed_to_cloud": disclosed,
                "metadata": metadata,
                "used_at_ms": used_at_ms,
                "summary": input.get("summary").and_then(|v| v.as_str()).unwrap_or(purpose)
            });
            suite
                .upsert_record("data_source_usage", &id, &record)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!(true))
        }
        ("data_source", "request_refresh") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = payload
                .get("id")
                .or_else(|| payload.get("data_source_id"))
                .or_else(|| input.get("data_source_id"))
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let existing = suite
                .get_record::<Value>("data_source", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .ok_or(("not_found", format!("data source '{id}' not found")))?;
            let mut rec = canonicalize_data_source_record(existing, Some(id));
            let cur_rev = rec.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
            if let Some(exp) = payload
                .get("expected_revision")
                .or_else(|| input.get("expected_revision"))
                .and_then(|v| v.as_i64())
            {
                if exp != cur_rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": format!("data_source:{id}"),
                            "expected_revision": exp.max(0) as u64,
                            "actual_revision": cur_rev.max(0) as u64
                        })
                        .to_string(),
                    ));
                }
            }
            if rec
                .get("read_state")
                .and_then(|rs| rs.get("status"))
                .and_then(|v| v.as_str())
                == Some("pending")
                && rec
                    .get("read_state")
                    .and_then(|rs| rs.get("read_token"))
                    .and_then(|v| v.as_str())
                    .is_some()
            {
                return Err((
                    "conflict",
                    format!("data source '{id}' already has a pending refresh"),
                ));
            }
            let ts = payload
                .get("requested_at_ms")
                .or_else(|| input.get("requested_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let token = new_id();
            let prev_rs = rec.get("read_state").cloned().unwrap_or_else(|| json!({}));
            let next_rs = json!({
                "status": "pending",
                "read_token": token,
                "content_reference": prev_rs.get("content_reference").cloned().unwrap_or(Value::Null),
                "observed_at_ms": prev_rs.get("observed_at_ms").cloned().unwrap_or(Value::Null),
                "schema_version": prev_rs.get("schema_version").cloned().unwrap_or(Value::Null),
                "record_count": prev_rs.get("record_count").cloned().unwrap_or(Value::Null),
                "diagnostic": Value::Null,
                "requested_at_ms": ts,
                "completed_at_ms": prev_rs.get("completed_at_ms").cloned().unwrap_or(Value::Null)
            });
            let next_rev = cur_rev + 1;
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("read_state".into(), next_rs);
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("data_source", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(rec)
        }
        ("data_source", "record_read") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = payload
                .get("id")
                .or_else(|| payload.get("data_source_id"))
                .or_else(|| input.get("data_source_id"))
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let existing = suite
                .get_record::<Value>("data_source", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .ok_or(("not_found", format!("data source '{id}' not found")))?;
            let mut rec = canonicalize_data_source_record(existing, Some(id));
            let expected_token = rec
                .get("read_state")
                .and_then(|rs| rs.get("read_token"))
                .and_then(|v| v.as_str());
            if let Some(provided) = payload
                .get("read_token")
                .or_else(|| input.get("read_token"))
                .and_then(|v| v.as_str())
            {
                if expected_token != Some(provided) {
                    return Err((
                        "conflict",
                        "data source read_token does not match pending refresh".into(),
                    ));
                }
            }
            let ts = payload
                .get("recorded_at_ms")
                .or_else(|| input.get("recorded_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let res_obj = payload
                .get("result")
                .or_else(|| input.get("result"))
                .unwrap_or(input);
            let raw_status = res_obj
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("succeeded");
            let norm_status = match raw_status {
                "succeeded" | "ready" => "ready",
                "succeeded_empty" | "empty" => "empty",
                "denied" => "denied",
                "unavailable" => "unavailable",
                "failed" | "cancelled" => "failed",
                _ => "ready",
            };
            let prev_rs = rec.get("read_state").cloned().unwrap_or_else(|| json!({}));
            let is_success = matches!(norm_status, "ready" | "empty");
            let content_ref = if is_success {
                res_obj
                    .get("content_reference")
                    .cloned()
                    .unwrap_or(Value::Null)
            } else {
                prev_rs
                    .get("content_reference")
                    .cloned()
                    .unwrap_or(Value::Null)
            };
            let observed_at = if is_success {
                res_obj
                    .get("observed_at_ms")
                    .cloned()
                    .unwrap_or_else(|| json!(ts))
            } else {
                prev_rs.get("observed_at_ms").cloned().unwrap_or(Value::Null)
            };
            let schema_version = if is_success {
                res_obj.get("schema_version").cloned().unwrap_or(Value::Null)
            } else {
                prev_rs.get("schema_version").cloned().unwrap_or(Value::Null)
            };
            let record_count = if is_success {
                res_obj.get("record_count").cloned().unwrap_or(Value::Null)
            } else {
                prev_rs.get("record_count").cloned().unwrap_or(Value::Null)
            };
            let diagnostic_val = {
                let code = res_obj
                    .get("code")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty());
                let raw_diag = res_obj
                    .get("diagnostic")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty());
                match (code, raw_diag) {
                    (Some(c), Some(d)) => {
                        if d.starts_with(&format!("{c}:")) {
                            json!(d)
                        } else {
                            json!(format!("{c}: {d}"))
                        }
                    }
                    (Some(c), None) => json!(format!("{c}:")),
                    (None, Some(d)) => json!(d),
                    (None, None) => Value::Null,
                }
            };
            let next_rs = json!({
                "status": norm_status,
                "read_token": Value::Null,
                "content_reference": content_ref,
                "observed_at_ms": observed_at,
                "schema_version": schema_version,
                "record_count": record_count,
                "diagnostic": diagnostic_val,
                "requested_at_ms": prev_rs.get("requested_at_ms").cloned().unwrap_or(json!(ts)),
                "completed_at_ms": ts
            });
            let cur_rev = rec.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
            let next_rev = cur_rev + 1;
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("read_state".into(), next_rs);
                if is_success {
                    obj.insert("last_successful_read_at_ms".into(), json!(ts));
                }
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("data_source", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(rec)
        }
        ("data_source", "resolve_for_conversation") => {
            let input = payload.get("input").unwrap_or(payload);
            let conv_id = payload
                .get("conversation_id")
                .or_else(|| input.get("conversation_id"))
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str());
            let now = payload
                .get("now_ms")
                .or_else(|| input.get("now_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let all: Vec<Value> = suite
                .list_records::<Value>("data_source")
                .map_err(|e| ("storage_error", e.to_string()))?
                .into_iter()
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .map(|r| canonicalize_data_source_record(r, None))
                .collect();
            let mut selections = Vec::with_capacity(all.len());
            for rec in all {
                let ds_id = rec
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let source_revision = rec
                    .get("revision")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(1)
                    .max(1);
                let default_enabled = rec
                    .get("default_enabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let override_mode = conv_id.and_then(|cid| {
                    store
                        .get_kv(&format!("ds_override:{cid}:{ds_id}"))
                        .ok()
                        .flatten()
                });
                let (effective, disabled_reason) = match override_mode {
                    Some(Value::Bool(true)) => (true, None),
                    Some(Value::Bool(false)) => (false, Some("disabled_for_conversation")),
                    Some(Value::String(ref s)) if s.eq_ignore_ascii_case("enabled") => (true, None),
                    Some(Value::String(ref s)) if s.eq_ignore_ascii_case("disabled") => {
                        (false, Some("disabled_for_conversation"))
                    }
                    _ => {
                        if default_enabled {
                            (true, None)
                        } else {
                            (false, Some("disabled_by_default"))
                        }
                    }
                };
                let lifecycle = rec
                    .get("lifecycle")
                    .and_then(|v| v.as_str())
                    .unwrap_or("draft");
                let auth_state = rec
                    .get("authorization")
                    .and_then(|a| a.get("state"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("not_required");
                let val_status = rec
                    .get("validation")
                    .and_then(|v| v.get("status"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("not_validated");
                let rs = rec.get("read_state").cloned().unwrap_or_else(|| json!({}));
                let read_status = rs
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("never_read");
                let content_reference = rs
                    .get("content_reference")
                    .cloned()
                    .unwrap_or(Value::Null);
                let observed_at_ms = rs.get("observed_at_ms").and_then(|v| v.as_i64());
                let schema_version = rs
                    .get("schema_version")
                    .and_then(|v| v.as_i64())
                    .map(|n| n as i32);
                let max_age_min = rec
                    .get("freshness_policy")
                    .and_then(|f| f.get("maximum_age_minutes"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(60);
                let allow_stale = rec
                    .get("freshness_policy")
                    .and_then(|f| f.get("allow_stale_fallback"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let is_stale = observed_at_ms.is_some_and(|obs| {
                    max_age_min > 0 && now.saturating_sub(obs) > max_age_min.saturating_mul(60_000)
                });
                let (usable, should_refresh, reason) = if !effective {
                    (false, false, disabled_reason.unwrap_or("disabled_by_default"))
                } else if lifecycle != "ready" {
                    (false, false, "lifecycle_not_ready")
                } else if matches!(auth_state, "denied" | "restricted" | "unavailable") {
                    (false, false, "authorization_unavailable")
                } else if !matches!(val_status, "verified" | "verified_empty") {
                    (false, false, "validation_unavailable")
                } else {
                    match read_status {
                        "pending" => (false, false, "read_pending"),
                        "never_read" => (false, true, "refresh_required"),
                        "ready" => {
                            if is_stale {
                                if allow_stale {
                                    (true, true, "ready")
                                } else {
                                    (false, true, "refresh_required")
                                }
                            } else {
                                (true, false, "ready")
                            }
                        }
                        "empty" => {
                            if is_stale {
                                if allow_stale {
                                    (true, true, "ready_empty")
                                } else {
                                    (false, true, "refresh_required")
                                }
                            } else {
                                (true, false, "ready_empty")
                            }
                        }
                        _ => (false, true, "read_unavailable"),
                    }
                };
                selections.push(json!({
                    "source_id": ds_id,
                    "source_revision": source_revision,
                    "enabled": effective,
                    "usable": usable,
                    "should_refresh": should_refresh,
                    "reason": reason,
                    "content_reference": if usable { content_reference } else { Value::Null },
                    "observed_at_ms": if usable { observed_at_ms.map(|v| json!(v)).unwrap_or(Value::Null) } else { Value::Null },
                    "schema_version": if usable { schema_version.map(|v| json!(v)).unwrap_or(Value::Null) } else { Value::Null }
                }));
            }
            Ok(Value::Array(selections))
        }
        ("data_source", "resolve_sources") => {
            let input = payload.get("input").unwrap_or(payload);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str());
            let all: Vec<Value> = suite
                .list_records("data_source")
                .map_err(|e| ("storage_error", e.to_string()))?;
            let mut resolved = Vec::new();
            for mut rec in all {
                let ds_id = rec
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let default_enabled = rec
                    .get("default_enabled")
                    .or_else(|| rec.get("enabled"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let effective = if let Some(cid) = conv_id {
                    match store.get_kv(&format!("ds_override:{cid}:{ds_id}")).ok().flatten() {
                        Some(Value::Bool(b)) => b,
                        Some(Value::String(ref s)) if s.eq_ignore_ascii_case("enabled") => true,
                        Some(Value::String(ref s)) if s.eq_ignore_ascii_case("disabled") => false,
                        Some(ref o) if o.get("enabled").and_then(|v| v.as_bool()).is_some() => {
                            o.get("enabled").and_then(|v| v.as_bool()).unwrap_or(default_enabled)
                        }
                        _ => default_enabled,
                    }
                } else {
                    default_enabled
                };
                if let Some(obj) = rec.as_object_mut() {
                    obj.insert("effective_enabled".into(), json!(effective));
                }
                resolved.push(rec);
            }
            Ok(json!({
                "conversation_id": conv_id,
                "sources": resolved,
                "items": resolved
            }))
        }
        ("data_source", "delete") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = payload
                .get("id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let existing = suite
                .get_record::<Value>("data_source", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .ok_or(("not_found", format!("data source '{id}' not found")))?;
            let cur_rev = existing
                .get("revision")
                .and_then(|v| v.as_i64())
                .unwrap_or(1);
            if let Some(exp) = payload
                .get("expected_revision")
                .or_else(|| input.get("expected_revision"))
                .or_else(|| args.get("__envelope_expected_revision"))
                .and_then(|v| v.as_i64())
            {
                if exp != cur_rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": format!("data_source:{id}"),
                            "expected_revision": exp.max(0) as u64,
                            "actual_revision": cur_rev.max(0) as u64
                        })
                        .to_string(),
                    ));
                }
            }
            let deleted = suite
                .delete_record("data_source", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            if !deleted {
                return Err(("not_found", format!("data source '{id}' not found")));
            }
            Ok(json!(true))
        }

        // --- Automation Domain ---
        ("automation", "definition") => {
            let (sub_op, sub_payload) = extract_sub_command(payload);
            dispatch_automation_definition(suite, sub_op, sub_payload, args)
        }
        ("automation", "run") => {
            let (sub_op, sub_payload) = extract_sub_command(payload);
            dispatch_automation_run(suite, sub_op, sub_payload, args)
        }
        ("automation", "list" | "get" | "create" | "upsert" | "update" | "update_if_revision" | "reconcile" | "pause" | "resume" | "delete") => {
            dispatch_automation_definition(suite, op, payload, args)
        }
        ("automation", "enqueue" | "claim" | "heartbeat" | "complete" | "fail" | "retry" | "load") => {
            dispatch_automation_run(suite, op, payload, args)
        }

        _ => Err((
            "unsupported_command",
            format!("unsupported {domain} operation '{op}'"),
        )),
    }
}

fn enrich_mcp_server_json(store: &Store, cfg: &McpServerConfig) -> Value {
    let mut val = serde_json::to_value(cfg).unwrap_or_else(|_| json!({}));
    if let Some(obj) = val.as_object_mut() {
        let meta = store
            .get_kv(&format!("mcp_meta:{}", cfg.id))
            .ok()
            .flatten()
            .unwrap_or_else(|| json!({}));
        let display_name = meta
            .get("display_name")
            .and_then(|v| v.as_str())
            .unwrap_or(&cfg.name);
        let revision = meta.get("revision").and_then(|r| r.as_u64()).unwrap_or(1);
        let configuration_json = meta
            .get("configuration_json")
            .and_then(|v| v.as_str())
            .unwrap_or("{}");
        let updated_at_ms = meta
            .get("updated_at_ms")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let created_at_ms = meta
            .get("created_at_ms")
            .and_then(|v| v.as_i64())
            .unwrap_or(updated_at_ms);
        obj.insert("display_name".into(), json!(display_name));
        obj.insert("enabled".into(), json!(cfg.enabled));
        obj.insert("revision".into(), json!(revision));
        obj.insert("configuration_json".into(), json!(configuration_json));
        obj.insert("created_at_ms".into(), json!(created_at_ms));
        obj.insert("updated_at_ms".into(), json!(updated_at_ms));
    }
    val
}

fn dispatch_automation_definition(
    suite: &ModulesSuite,
    op: &str,
    payload: &Value,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let input = payload.get("input").unwrap_or(payload);
    match op {
        "list" => {
            let items: Vec<Value> = suite
                .list_records("automation")
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"kind": "automation", "items": items, "definitions": items}))
        }
        "get" => {
            let id = input
                .get("id")
                .or_else(|| input.get("definition_id"))
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let found = suite
                .get_record::<Value>("automation", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(found.unwrap_or(Value::Null))
        }
        "create" | "upsert" => {
            let record = input
                .get("definition")
                .or_else(|| input.get("record"))
                .unwrap_or(input);
            let id = record
                .get("id")
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let existing = suite
                .get_record::<Value>("automation", &id)
                .ok()
                .flatten();
            let prev_rev = existing
                .as_ref()
                .and_then(|v| v.get("revision").and_then(|r| r.as_u64()))
                .unwrap_or(0);
            let unchanged = existing.as_ref().is_some_and(|ex| {
                ex.get("name") == record.get("name")
                    && ex.get("schedule_format") == record.get("schedule_format")
                    && ex.get("enabled") == record.get("enabled")
                    && ex.get("execution_spec") == record.get("execution_spec")
                    && ex.get("metadata_json") == record.get("metadata_json")
                    && ex.get("deleted_at_ms") == record.get("deleted_at_ms")
            });
            if unchanged {
                if let Some(ex) = existing {
                    return Ok(ex);
                }
            }
            let incoming_rev = record.get("revision").and_then(|r| r.as_u64());
            let next_rev = match incoming_rev {
                Some(r) if r > prev_rev => r,
                _ => prev_rev + 1,
            };
            let ts = now_ms();
            let mut stored = record.clone();
            if let Some(obj) = stored.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("revision".into(), json!(next_rev));
                if !obj.contains_key("created_at_ms") {
                    let prev_created = existing
                        .as_ref()
                        .and_then(|v| v.get("created_at_ms").and_then(|c| c.as_i64()))
                        .unwrap_or(ts);
                    obj.insert("created_at_ms".into(), json!(prev_created));
                }
                if !obj.contains_key("updated_at_ms") {
                    obj.insert("updated_at_ms".into(), json!(ts));
                }
            }
            suite
                .upsert_record("automation", &id, &stored)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(stored)
        }
        "reconcile" => {
            let inputs = payload
                .get("inputs")
                .or_else(|| input.get("inputs"))
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let schedule_formats: HashSet<String> = payload
                .get("schedule_formats")
                .or_else(|| input.get("schedule_formats"))
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(ToString::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let incoming_ids: HashSet<String> = inputs
                .iter()
                .filter_map(|item| {
                    item.get("id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                })
                .collect();
            let existing: Vec<Value> = suite
                .list_records("automation")
                .map_err(|e| ("storage_error", e.to_string()))?;
            for ex in &existing {
                let Some(eid) = ex.get("id").and_then(|v| v.as_str()) else {
                    continue;
                };
                let fmt = ex
                    .get("schedule_format")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let in_scope = schedule_formats.is_empty() || schedule_formats.contains(fmt);
                if in_scope && !incoming_ids.contains(eid) {
                    let _ = suite.delete_record("automation", eid);
                }
            }
            let ts = payload
                .get("reconciled_at_ms")
                .or_else(|| input.get("reconciled_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let mut result = Vec::with_capacity(inputs.len());
            for item in inputs {
                let id = item
                    .get("id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(new_id);
                let prev = suite.get_record::<Value>("automation", &id).ok().flatten();
                let prev_rev = prev
                    .as_ref()
                    .and_then(|v| v.get("revision").and_then(|r| r.as_u64()))
                    .unwrap_or(0);
                let unchanged = prev.as_ref().is_some_and(|ex| {
                    ex.get("name") == item.get("name")
                        && ex.get("schedule_format") == item.get("schedule_format")
                        && ex.get("enabled") == item.get("enabled")
                        && ex.get("execution_spec") == item.get("execution_spec")
                        && ex.get("metadata_json") == item.get("metadata_json")
                });
                if unchanged {
                    if let Some(ex) = prev {
                        result.push(ex);
                        continue;
                    }
                }
                let next_rev = prev_rev + 1;
                let mut stored = item.clone();
                if let Some(obj) = stored.as_object_mut() {
                    obj.insert("id".into(), json!(id));
                    obj.insert("revision".into(), json!(next_rev));
                    obj.entry("created_at_ms".to_string()).or_insert_with(|| json!(ts));
                    obj.entry("updated_at_ms".to_string()).or_insert_with(|| json!(ts));
                }
                suite
                    .upsert_record("automation", &id, &stored)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                result.push(stored);
            }
            Ok(Value::Array(result))
        }
        "update" | "update_if_revision" => {
            let record = input
                .get("definition")
                .or_else(|| input.get("record"))
                .unwrap_or(input);
            let id = input
                .get("id")
                .or_else(|| input.get("definition_id"))
                .or_else(|| record.get("id"))
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let existing = suite
                .get_record::<Value>("automation", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let cur_rev = existing
                .as_ref()
                .and_then(|v| v.get("revision").and_then(|r| r.as_u64()))
                .unwrap_or(if existing.is_some() { 1 } else { 0 });
            if let Some(exp) = input
                .get("expected_revision")
                .or_else(|| payload.get("expected_revision"))
                .and_then(|v| v.as_u64())
            {
                if cur_rev != exp {
                    return Err((
                        "revision_conflict",
                        format!("automation '{id}' revision conflict: expected {exp}, actual {cur_rev}"),
                    ));
                }
            }
            let prev_created = existing
                .as_ref()
                .and_then(|v| v.get("created_at_ms").and_then(|c| c.as_i64()));
            let mut merged = existing.unwrap_or_else(|| json!({"id": id}));
            if let (Some(dst), Some(src)) = (merged.as_object_mut(), record.as_object()) {
                for (k, v) in src {
                    if k != "type" {
                        dst.insert(k.clone(), v.clone());
                    }
                }
                dst.insert("id".into(), json!(id));
                let next_rev = match src.get("revision").and_then(|r| r.as_u64()) {
                    Some(r) if r > cur_rev => r,
                    _ => cur_rev + 1,
                };
                dst.insert("revision".into(), json!(next_rev));
                if !dst.contains_key("created_at_ms") {
                    dst.insert("created_at_ms".into(), json!(prev_created.unwrap_or_else(now_ms)));
                }
                if !src.contains_key("updated_at_ms") {
                    dst.insert("updated_at_ms".into(), json!(now_ms()));
                }
            }
            suite
                .upsert_record("automation", id, &merged)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(merged)
        }
        "pause" | "resume" => {
            let id = input
                .get("id")
                .or_else(|| input.get("definition_id"))
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let mut rec = suite
                .get_record::<Value>("automation", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("automation '{id}' not found")))?;
            let enabled = op == "resume";
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("enabled".into(), json!(enabled));
                obj.insert("updatedAt".into(), json!(Utc::now().to_rfc3339()));
                obj.insert("updated_at_ms".into(), json!(now_ms()));
            }
            suite
                .upsert_record("automation", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "id": id, "enabled": enabled, "record": rec}))
        }
        "delete" => {
            let id = input
                .get("id")
                .or_else(|| input.get("definition_id"))
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let deleted = suite
                .delete_record("automation", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!(deleted))
        }
        other => Err((
            "unsupported_command",
            format!("unsupported automation definition operation '{other}'"),
        )),
    }
}

fn dispatch_automation_run(
    suite: &ModulesSuite,
    op: &str,
    payload: &Value,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let input = payload.get("input").unwrap_or(payload);
    match op {
        "enqueue" => {
            let automation_id = input
                .get("automation_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let occurrence_key = input
                .get("occurrence_key")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !automation_id.is_empty() && !occurrence_key.is_empty() {
                let existing_runs: Vec<Value> = suite
                    .list_records("automation_run")
                    .map_err(|e| ("storage_error", e.to_string()))?;
                if let Some(dup) = existing_runs.into_iter().find(|r| {
                    r.get("automation_id").and_then(|v| v.as_str()) == Some(automation_id)
                        && r.get("occurrence_key").and_then(|v| v.as_str()) == Some(occurrence_key)
                }) {
                    return Ok(dup);
                }
            }
            let id = input
                .get("run_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let def_rev = input
                .get("definition_revision")
                .and_then(|v| v.as_i64())
                .or_else(|| {
                    suite
                        .get_record::<Value>("automation", automation_id)
                        .ok()
                        .flatten()
                        .and_then(|d| d.get("revision").and_then(|v| v.as_i64()))
                })
                .unwrap_or(1);
            let ts = input
                .get("created_at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let mut rec = input.clone();
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("definition_revision".into(), json!(def_rev));
                obj.entry("status".to_string()).or_insert_with(|| json!("queued"));
                obj.entry("attempt".to_string()).or_insert_with(|| json!(0));
                obj.entry("created_at_ms".to_string()).or_insert_with(|| json!(ts));
                obj.entry("updated_at_ms".to_string()).or_insert_with(|| json!(ts));
            }
            suite
                .upsert_record("automation_run", &id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(rec)
        }
        "claim" => {
            let worker_id = input
                .get("worker_id")
                .and_then(|v| v.as_str())
                .unwrap_or("local-worker");
            let lease_token = input
                .get("lease_token")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let now = input
                .get("now_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let lease_until = input
                .get("lease_until_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(now + 60_000);
            let target_id = input
                .get("run_id")
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str());
            let runs: Vec<Value> = suite
                .list_records("automation_run")
                .map_err(|e| ("storage_error", e.to_string()))?;
            let candidate = runs.into_iter().find(|r| {
                let id_ok = target_id
                    .is_none_or(|tid| r.get("id").and_then(|v| v.as_str()) == Some(tid));
                if !id_ok {
                    return false;
                }
                let st = r.get("status").and_then(|v| v.as_str()).unwrap_or("");
                if matches!(st, "queued" | "pending") {
                    let next_retry = r.get("next_retry_at_ms").and_then(|v| v.as_i64()).unwrap_or(0);
                    return now >= next_retry;
                }
                if st == "running" {
                    // Lease expiry alone must never replay a running occurrence.
                    return false;
                }
                false
            });
            let Some(mut rec) = candidate else {
                return Ok(json!(false));
            };
            let rid = rec
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let attempt = rec.get("attempt").and_then(|v| v.as_i64()).unwrap_or(0) + 1;
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("status".into(), json!("running"));
                obj.insert("attempt".into(), json!(attempt));
                obj.insert("worker_id".into(), json!(worker_id));
                obj.insert("lease_token".into(), json!(lease_token));
                obj.insert("lease_until_ms".into(), json!(lease_until));
                obj.entry("started_at_ms".to_string()).or_insert_with(|| json!(now));
                obj.insert("updated_at_ms".into(), json!(now));
            }
            suite
                .upsert_record("automation_run", &rid, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!(true))
        }
        "heartbeat" => {
            let id = input
                .get("run_id")
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let Some(mut rec) = suite
                .get_record::<Value>("automation_run", id)
                .map_err(|e| ("storage_error", e.to_string()))?
            else {
                return Ok(json!(false));
            };
            let status = rec.get("status").and_then(|v| v.as_str()).unwrap_or("");
            if status != "running" {
                return Ok(json!(false));
            }
            if let Some(req_token) = input.get("lease_token").and_then(|v| v.as_str()) {
                if rec.get("lease_token").and_then(|v| v.as_str()) != Some(req_token) {
                    return Ok(json!(false));
                }
            }
            let now = input
                .get("now_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let existing_until = rec.get("lease_until_ms").and_then(|v| v.as_i64()).unwrap_or(i64::MAX);
            if now > existing_until {
                return Ok(json!(false));
            }
            let next_until = input
                .get("lease_until_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(now + 60_000);
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("lease_until_ms".into(), json!(next_until));
                obj.insert("updated_at_ms".into(), json!(now));
            }
            suite
                .upsert_record("automation_run", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!(true))
        }
        "complete" | "fail" => {
            let id = input
                .get("run_id")
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let Some(mut rec) = suite
                .get_record::<Value>("automation_run", id)
                .map_err(|e| ("storage_error", e.to_string()))?
            else {
                return Ok(json!(false));
            };
            if rec.get("status").and_then(|v| v.as_str()) != Some("running") {
                return Ok(json!(false));
            }
            if let Some(req_token) = input.get("lease_token").and_then(|v| v.as_str()) {
                if rec.get("lease_token").and_then(|v| v.as_str()) != Some(req_token) {
                    return Ok(json!(false));
                }
            }
            let finished_at = input
                .get("finished_at_ms")
                .or_else(|| input.get("now_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let existing_until = rec
                .get("lease_until_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(i64::MAX);
            if finished_at > existing_until {
                return Ok(json!(false));
            }
            if let Some(obj) = rec.as_object_mut() {
                obj.insert(
                    "status".into(),
                    json!(if op == "complete" { "succeeded" } else { "failed" }),
                );
                obj.insert("finished_at_ms".into(), json!(finished_at));
                obj.insert("lease_token".into(), Value::Null);
                obj.insert("lease_until_ms".into(), Value::Null);
                if let Some(res_json) = input.get("result_json") {
                    obj.insert("result_json".into(), res_json.clone());
                }
                if let Some(err_json) = input.get("error_json") {
                    obj.insert("error_json".into(), err_json.clone());
                }
                obj.insert("updated_at_ms".into(), json!(finished_at));
            }
            suite
                .upsert_record("automation_run", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!(true))
        }
        "retry" => {
            let id = input
                .get("run_id")
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let Some(mut rec) = suite
                .get_record::<Value>("automation_run", id)
                .map_err(|e| ("storage_error", e.to_string()))?
            else {
                return Ok(json!(false));
            };
            if rec.get("status").and_then(|v| v.as_str()) != Some("running") {
                return Ok(json!(false));
            }
            if let Some(req_token) = input.get("lease_token").and_then(|v| v.as_str()) {
                if rec.get("lease_token").and_then(|v| v.as_str()) != Some(req_token) {
                    return Ok(json!(false));
                }
            }
            let ts = input
                .get("updated_at_ms")
                .or_else(|| input.get("now_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let existing_until = rec
                .get("lease_until_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(i64::MAX);
            if ts > existing_until {
                return Ok(json!(false));
            }
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("status".into(), json!("queued"));
                obj.insert("lease_token".into(), Value::Null);
                obj.insert("lease_until_ms".into(), Value::Null);
                obj.insert("worker_id".into(), Value::Null);
                if let Some(next_retry) = input.get("next_retry_at_ms") {
                    obj.insert("next_retry_at_ms".into(), next_retry.clone());
                }
                if let Some(err_json) = input.get("error_json") {
                    obj.insert("error_json".into(), err_json.clone());
                }
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("automation_run", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!(true))
        }
        "load" | "get" => {
            let id = input
                .get("run_id")
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("run_id"))
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'run_id'".into()))?;
            let found = suite
                .get_record::<Value>("automation_run", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(found.unwrap_or(Value::Null))
        }
        "list" => {
            let filter_auto = input
                .get("automation_id")
                .or_else(|| payload.get("automation_id"))
                .and_then(|v| v.as_str());
            let limit = input
                .get("limit")
                .or_else(|| payload.get("limit"))
                .and_then(|v| v.as_u64())
                .unwrap_or(200) as usize;
            let mut items: Vec<Value> = suite
                .list_records("automation_run")
                .map_err(|e| ("storage_error", e.to_string()))?;
            if let Some(aid) = filter_auto {
                items.retain(|r| r.get("automation_id").and_then(|v| v.as_str()) == Some(aid));
            }
            for item in &mut items {
                if let Some(obj) = item.as_object_mut() {
                    obj.entry("result_json".to_string()).or_insert(Value::Null);
                    obj.entry("error_json".to_string()).or_insert(Value::Null);
                }
            }
            items.truncate(limit);
            Ok(Value::Array(items))
        }
        "delete" => {
            let id = input
                .get("run_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'run_id'".into()))?;
            let deleted = suite
                .delete_record("automation_run", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "deleted": deleted, "id": id}))
        }
        other => Err((
            "unsupported_command",
            format!("unsupported automation run operation '{other}'"),
        )),
    }
}

