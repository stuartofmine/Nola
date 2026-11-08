// SPDX-License-Identifier: Apache-2.0
//! A resend creates a fresh conversation containing only edited user input.
//! These records carry model identity, never previous execution authority.
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const EDITED_RESEND_PREFIX: &str = "edited_resend/";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareEditedResend {
    pub operation_id: String,
    pub source_conversation_id: String,
    pub source_message_id: String,
    pub expected_source_revision: i64,
    pub expected_source_message_revision: u64,
    pub edited_text: String,
    pub target_conversation_id: String,
    pub title: String,
    pub created_at_ms: i64,
}

/// Closed model-only subset of a frozen Host invocation. No policy hash,
/// recovery handle, workspace, tool catalog, or previous session is retained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenResendModel {
    pub version: u32,
    pub primary: Value,
    pub fallbacks: Value,
    pub generation: Value,
    pub output_admission: Value,
    /// Original intrinsic model facts; no previous Host tool selection.
    pub model_capability: Value,
}
impl FrozenResendModel {
    pub fn from_invocation(config: &Value) -> Option<Self> {
        let identity = config.get("turn_identity")?;
        let binding = identity.get("host_invocation")?;
        let mut model_capability = identity.get("model_capability")?.as_object()?.clone();
        model_capability.insert("direct_tool_capability_ids".into(), Value::Array(vec![]));
        Some(Self {
            version: u32::try_from(binding.get("version")?.as_u64()?).ok()?,
            primary: binding.get("primary")?.clone(),
            fallbacks: binding.get("fallbacks")?.clone(),
            generation: binding.get("generation")?.clone(),
            output_admission: binding.get("output_admission")?.clone(),
            model_capability: Value::Object(model_capability),
        })
    }
}

/// Written only by ordinary canonical Run admission in the same transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageSendWitness {
    pub conversation_id: String,
    pub message_id: String,
    pub run_id: String,
    pub assistant_message_id: String,
    pub body_sha256: String,
    pub invocation_config_sha256: String,
    pub model: FrozenResendModel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedEditedResend {
    pub input: PrepareEditedResend,
    pub source: MessageSendWitness,
    pub user_message_id: String,
    pub assistant_message_id: String,
    pub run_id: String,
    /// None is prepared, not sent. Some proves admission only, never success.
    pub admitted_run_id: Option<String>,
}

/// Integrity of a trusted Host's newly rebuilt target policy. This is not a
/// signature, a permission grant, or proof that a hostile Host told the truth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreshResendPolicyWitness {
    pub operation_id: String,
    pub conversation_id: String,
    pub excluded_source_conversation_id: String,
    pub host_id: String,
    pub catalog_revision: u64,
    pub catalog_sha256: String,
    pub permission_rules_sha256: String,
    pub execution_policy_sha256: String,
    pub context_components_sha256: String,
    pub input_payload_sha256: String,
}

pub fn canonical_resend_value_sha256(value: &Value) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    fn ordered(value: &Value) -> Value {
        match value {
            Value::Object(fields) => {
                let mut keys: Vec<_> = fields.keys().collect();
                keys.sort();
                let mut sorted = serde_json::Map::new();
                for key in keys {
                    sorted.insert(key.clone(), ordered(&fields[key]));
                }
                Value::Object(sorted)
            }
            Value::Array(values) => Value::Array(values.iter().map(ordered).collect()),
            other => other.clone(),
        }
    }
    let bytes = serde_json::to_vec(&ordered(value)).map_err(|_| "invalid resend policy value")?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
