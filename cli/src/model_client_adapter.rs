// SPDX-License-Identifier: Apache-2.0
//! Reverse configuration adapters. Only explicitly confirmed, Nola-owned keys
//! are inserted; disconnect compares those keys and preserves unrelated edits.
use crate::{host_credentials::HostCredentialVault, store::NolaStore};
use jsonc_parser::cst::{CstInputValue, CstRootNode};
use nola_domain::model_connection::ModelSourceKind;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::Write,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientConnectionRequest {
    pub source: ModelSourceKind,
    pub path: String,
    pub model_id: String,
}
#[derive(Serialize)]
pub struct ClientConnectionPlan {
    pub plan_id: String,
    pub source: ModelSourceKind,
    pub path: String,
    pub owned_fields: Vec<String>,
    pub expires_at_ms: i64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ClientConnectionReceipt {
    pub receipt_id: String,
    pub source: ModelSourceKind,
    pub path: String,
    pub owned_fields: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct OwnedField {
    path: Vec<String>,
    value: Value,
}
#[derive(Serialize, Deserialize)]
struct PrivateReceipt {
    public: ClientConnectionReceipt,
    fields: Vec<OwnedField>,
}
struct PrivatePlan {
    database: PathBuf,
    request: ClientConnectionRequest,
    original: Vec<u8>,
    rendered: String,
    fields: Vec<OwnedField>,
    token_revision: i64,
    expires_at_ms: i64,
}
static PLANS: OnceLock<Mutex<BTreeMap<String, PrivatePlan>>> = OnceLock::new();
static WRITES: Mutex<()> = Mutex::new(());

fn input(value: &Value) -> CstInputValue {
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(v) => CstInputValue::Bool(*v),
        Value::Number(v) => CstInputValue::Number(v.to_string()),
        Value::String(v) => CstInputValue::String(v.clone()),
        Value::Array(v) => CstInputValue::Array(v.iter().map(input).collect()),
        Value::Object(v) => {
            CstInputValue::Object(v.iter().map(|(k, v)| (k.clone(), input(v))).collect())
        }
    }
}

/// Concrete syntax trees retain comments, formatting and unrelated fields.
fn edit(
    source: ModelSourceKind,
    bytes: &[u8],
    fields: &[OwnedField],
    remove: bool,
) -> Result<String, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "client_config_not_utf8")?;
    if matches!(source, ModelSourceKind::Codex | ModelSourceKind::KimiCode) {
        let mut document: toml_edit::DocumentMut =
            text.parse().map_err(|_| "client_config_invalid_toml")?;
        for field in fields {
            let (key, parents) = field.path.split_last().ok_or("client_owned_path_invalid")?;
            let mut table = document.as_table_mut();
            for parent in parents {
                if !table.contains_key(parent) && !remove {
                    table.insert(parent, toml_edit::Item::Table(toml_edit::Table::new()));
                }
                table = table
                    .get_mut(parent)
                    .and_then(toml_edit::Item::as_table_mut)
                    .ok_or("client_config_parent_not_table")?;
            }
            if remove {
                let item = table.get(key).ok_or("client_owned_field_changed")?;
                let mut current = toml_edit::DocumentMut::new();
                current.as_table_mut().insert("owned", item.clone());
                let current: Value = toml::from_str(&current.to_string())
                    .map_err(|_| "client_owned_field_invalid")?;
                if current["owned"] != field.value {
                    return Err("client_owned_field_changed".into());
                }
                table.remove(key);
            } else {
                if table.contains_key(key) {
                    return Err("client_owned_field_already_exists".into());
                }
                let serialized = toml::to_string(&json!({"owned":field.value}))
                    .map_err(|_| "client_owned_field_invalid")?;
                let owned: toml_edit::DocumentMut = serialized
                    .parse()
                    .map_err(|_| "client_owned_field_invalid")?;
                table.insert(key, owned["owned"].clone());
            }
        }
        return Ok(document.to_string());
    }
    if source != ModelSourceKind::OpenCode {
        return Err("client_source_not_supported".into());
    }
    let root = CstRootNode::parse(text, &crate::model_connection_host::jsonc_options())
        .map_err(|_| "client_config_invalid_jsonc")?;
    for field in fields {
        let (key, parents) = field.path.split_last().ok_or("client_owned_path_invalid")?;
        let mut object = root.object_value().ok_or("client_config_not_object")?;
        for parent in parents {
            object = if remove {
                object.object_value(parent)
            } else {
                object.object_value_or_create(parent)
            }
            .ok_or("client_config_parent_not_object")?;
        }
        if remove {
            let property = object.get(key).ok_or("client_owned_field_changed")?;
            if property.to_serde_value().as_ref() != Some(&field.value) {
                return Err("client_owned_field_changed".into());
            }
            property.remove();
        } else {
            if object.get(key).is_some() {
                return Err("client_owned_field_already_exists".into());
            }
            object.append(key, input(&field.value));
        }
    }
    Ok(root.to_string())
}

fn fields(
    source: ModelSourceKind,
    protocol: &str,
    model: &str,
    base: &str,
    token: &str,
) -> Result<Vec<OwnedField>, String> {
    let provider = format!("nola-{protocol}");
    let field = |path: Vec<String>, value| OwnedField { path, value };
    match source {
        ModelSourceKind::OpenCode => {
            let npm = match protocol {
                "responses" => "@ai-sdk/openai",
                "chat_completions" => "@ai-sdk/openai-compatible",
                "anthropic" => "@ai-sdk/anthropic",
                _ => return Err("client_protocol_not_supported".into()),
            };
            Ok(vec![field(
                vec!["provider".into(), provider],
                json!({"name":"Nola Gateway","npm":npm,"options":{"baseURL":base,"apiKey":token},"models":{(model):{"name":"Nola Gateway Model"}}}),
            )])
        }
        ModelSourceKind::Codex if protocol == "responses" => Ok(vec![
            field(
                vec!["model_providers".into(), provider.clone()],
                json!({"name":"Nola Gateway","base_url":base,"wire_api":"responses","experimental_bearer_token":token}),
            ),
            field(
                vec!["profiles".into(), "nola-gateway".into()],
                json!({"model_provider":provider,"model":model}),
            ),
        ]),
        ModelSourceKind::KimiCode => {
            let kind = match protocol {
                "responses" => "openai_responses",
                "chat_completions" => "openai",
                "anthropic" => "anthropic",
                _ => return Err("client_protocol_not_supported".into()),
            };
            Ok(vec![
                field(
                    vec!["providers".into(), provider.clone()],
                    json!({"type":kind,"base_url":base,"api_key":token}),
                ),
                field(
                    vec!["models".into(), "nola-gateway".into()],
                    json!({"provider":provider,"model":model}),
                ),
            ])
        }
        _ => Err("client_protocol_not_supported".into()),
    }
}

pub fn plan_client_connection(
    store: &NolaStore,
    request: ClientConnectionRequest,
    gateway_base_url: &str,
) -> Result<ClientConnectionPlan, String> {
    if !matches!(
        request.source,
        ModelSourceKind::OpenCode | ModelSourceKind::Codex | ModelSourceKind::KimiCode
    ) {
        return Err("client_source_not_supported".into());
    }
    let url = url::Url::parse(gateway_base_url).map_err(|_| "gateway_base_url_invalid")?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path().trim_end_matches('/') != "/gateway/v1"
    {
        return Err("gateway_base_url_invalid".into());
    }
    let profile = store
        .model_profiles()?
        .into_iter()
        .find(|profile| {
            crate::model_gateway_host::model_id(profile) == request.model_id
                && profile.enabled
                && profile.runtime == "remote"
                && profile.deleted_at_ms.is_none()
        })
        .ok_or("gateway_model_unavailable")?;
    let metadata: Value =
        serde_json::from_str(&profile.source_json).map_err(|_| "gateway_model_invalid")?;
    if metadata["wireProtocol"] == "siwc_responses" || metadata["sourceKind"] == "nola_chatgpt" {
        return Err("siwc_authorization_cannot_be_exported_as_gateway_api".into());
    }
    let protocol = metadata["wireProtocol"]
        .as_str()
        .ok_or("gateway_model_invalid")?;
    let (token_revision, token) = crate::model_gateway_host::client_access_token(store)?;
    let mut fields = fields(
        request.source,
        protocol,
        &request.model_id,
        gateway_base_url,
        &token,
    )?;
    if request.source == ModelSourceKind::KimiCode {
        let parameters: Value = serde_json::from_str(&profile.parameters_json)
            .map_err(|_| "gateway_model_parameters_invalid")?;
        let context = parameters["maxContextLength"]
            .as_u64()
            .filter(|value| *value > 0)
            .ok_or("client_requires_model_context_length")?;
        let model = fields
            .iter_mut()
            .find(|field| field.path.first().is_some_and(|key| key == "models"))
            .ok_or("client_model_field_missing")?;
        model.value["max_context_size"] = json!(context);
        let capabilities = &metadata["executionCapabilities"];
        let mut declared = vec![];
        if capabilities["toolCalls"] == true {
            declared.push("tool_use");
        }
        if capabilities["imageInput"] == true {
            declared.push("image_in");
        }
        if capabilities["reasoning"] == true {
            declared.push("thinking");
        }
        model.value["capabilities"] = json!(declared);
    }
    let original = crate::model_connection_host::read_selected(&request.path)?;
    let rendered = edit(request.source, &original, &fields, false)?;
    let plan_id = uuid::Uuid::new_v4().simple().to_string();
    let now = chrono::Utc::now().timestamp_millis();
    let expires_at_ms = now + 600_000;
    let public = ClientConnectionPlan {
        plan_id: plan_id.clone(),
        source: request.source,
        path: request.path.clone(),
        owned_fields: fields.iter().map(|f| f.path.join(".")).collect(),
        expires_at_ms,
    };
    let mut plans = PLANS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "client_plan_unavailable")?;
    plans.retain(|_, plan| plan.expires_at_ms > now);
    if plans.len() >= 16 {
        return Err("client_plan_limit".into());
    }
    plans.insert(
        plan_id,
        PrivatePlan {
            database: store.path().to_owned(),
            request,
            original,
            rendered,
            fields,
            token_revision,
            expires_at_ms,
        },
    );
    Ok(public)
}

fn replace_selected(path: &str, expected: &[u8], rendered: &str) -> Result<(), String> {
    if crate::model_connection_host::read_selected(path)? != expected {
        return Err("client_config_changed".into());
    }
    let path = std::path::Path::new(path);
    let parent = path.parent().ok_or("client_config_parent_missing")?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| "client_config_write_failed")?;
    temporary
        .write_all(rendered.as_bytes())
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|_| "client_config_write_failed")?;
    if crate::model_connection_host::read_selected(
        path.to_str().ok_or("client_config_path_invalid")?,
    )? != expected
    {
        return Err("client_config_changed".into());
    }
    temporary
        .persist(path)
        .map_err(|_| "client_config_write_failed")?;
    Ok(())
}

pub fn commit_client_connection(
    store: &NolaStore,
    plan_id: &str,
) -> Result<ClientConnectionReceipt, String> {
    let _guard = WRITES.lock().map_err(|_| "client_config_busy")?;
    let plan = PLANS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "client_plan_unavailable")?
        .remove(plan_id)
        .ok_or("client_plan_expired")?;
    if plan.database != store.path() || plan.expires_at_ms <= chrono::Utc::now().timestamp_millis()
    {
        return Err("client_plan_expired".into());
    }
    if crate::model_gateway_host::client_access_token(store)?.0 != plan.token_revision {
        return Err("gateway_token_changed".into());
    }
    if !store.model_profiles()?.iter().any(|profile| {
        crate::model_gateway_host::model_id(profile) == plan.request.model_id
            && profile.enabled
            && profile.deleted_at_ms.is_none()
    }) {
        return Err("gateway_model_changed".into());
    }
    let receipt = ClientConnectionReceipt {
        receipt_id: uuid::Uuid::new_v4().simple().to_string(),
        source: plan.request.source,
        path: plan.request.path.clone(),
        owned_fields: plan.fields.iter().map(|f| f.path.join(".")).collect(),
    };
    // Store the private ownership receipt before writing, so a crash after the
    // atomic replace never leaves an untracked, unrecoverable configuration.
    let private = PrivateReceipt {
        public: receipt.clone(),
        fields: plan.fields,
    };
    HostCredentialVault::open(store.path())?.save_client_configuration_receipt(
        &receipt.receipt_id,
        &serde_json::to_string(&private).map_err(|_| "client_receipt_invalid")?,
    )?;
    if let Err(error) = replace_selected(&plan.request.path, &plan.original, &plan.rendered) {
        HostCredentialVault::open(store.path())?
            .remove_client_configuration_receipt(&receipt.receipt_id)?;
        return Err(error);
    }
    Ok(receipt)
}

pub fn disconnect_client_connection(
    store: &NolaStore,
    receipt_id: &str,
) -> Result<ClientConnectionReceipt, String> {
    let _guard = WRITES.lock().map_err(|_| "client_config_busy")?;
    let serialized = HostCredentialVault::open(store.path())?
        .client_configuration_receipts()?
        .into_iter()
        .find(|(id, _)| id == receipt_id)
        .map(|(_, value)| value)
        .ok_or("client_receipt_missing")?;
    let receipt: PrivateReceipt =
        serde_json::from_str(&serialized).map_err(|_| "client_receipt_invalid")?;
    if receipt.public.receipt_id != receipt_id {
        return Err("client_receipt_invalid".into());
    }
    let original = crate::model_connection_host::read_selected(&receipt.public.path)?;
    let rendered = edit(receipt.public.source, &original, &receipt.fields, true)?;
    replace_selected(&receipt.public.path, &original, &rendered)?;
    HostCredentialVault::open(store.path())?.remove_client_configuration_receipt(receipt_id)?;
    Ok(receipt.public)
}

pub fn list_client_connections(store: &NolaStore) -> Result<Vec<ClientConnectionReceipt>, String> {
    HostCredentialVault::open(store.path())?
        .client_configuration_receipts()?
        .into_iter()
        .map(|(id, value)| {
            let receipt: PrivateReceipt =
                serde_json::from_str(&value).map_err(|_| "client_receipt_invalid")?;
            if receipt.public.receipt_id != id {
                return Err("client_receipt_invalid".into());
            }
            Ok(receipt.public)
        })
        .collect()
}

#[cfg(test)]
#[path = "model_client_adapter_tests.rs"]
mod tests;
