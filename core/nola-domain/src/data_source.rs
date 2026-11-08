// SPDX-License-Identifier: Apache-2.0
//! Portable data-source policy and reader declarations; no platform I/O or ABI.
use serde::{Deserialize,Serialize};
use serde_json::Value;
use chrono::{DateTime, Duration, Utc};
use sha2::{Digest,Sha256};
pub const DATA_SOURCE_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSourceContract {
    pub source_id: String,
    pub connector_id: String,
    pub connector_version: u32,
    pub reader_description: String,
    #[serde(default)]
    pub read_recipe: DataSourceReadRecipe,
    pub schema_fields: Vec<String>,
    /// A bounded preview that the platform host has already redacted.
    pub redacted_sample: String,
    pub record_count: u64,
    pub maximum_age_minutes: u64,
    pub validation_state: DataSourceValidationState,
    pub validated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub validation_digest: String,
}

impl DataSourceContract {
    pub fn is_structurally_usable(&self, source_id: &str) -> bool {
        self.source_id == source_id
            && !self.connector_id.trim().is_empty()
            && self.connector_version > 0
            && !self.reader_description.trim().is_empty()
            && self.read_recipe.is_structurally_usable()
            && (!self.schema_fields.is_empty()
                || self
                    .read_recipe
                    .output_schema
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind != "object" && kind != "array"))
            && self.validation_state != DataSourceValidationState::Failed
            && self.validation_digest == self.expected_validation_digest()
    }

    pub fn seal_validation(&mut self) {
        self.validation_digest = self.expected_validation_digest();
    }

    pub fn expected_validation_digest(&self) -> String {
        let material = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": DATA_SOURCE_SCHEMA_VERSION,
            "sourceId": self.source_id,
            "connectorId": self.connector_id,
            "connectorVersion": self.connector_version,
            "readerDescription": self.reader_description,
            "readRecipe": self.read_recipe,
            "schemaFields": self.schema_fields,
            "redactedSample": self.redacted_sample,
            "recordCount": self.record_count,
            "maximumAgeMinutes": self.maximum_age_minutes,
            "validationState": self.validation_state,
            "validatedAt": self.validated_at,
            "observedAt": self.observed_at,
        }))
        .unwrap_or_default();
        Sha256::digest(material).iter().map(|byte| format!("{byte:02x}")).collect()
    }

    pub fn is_fresh_at(&self, now: DateTime<Utc>) -> bool {
        let observed_at = self.observed_at.unwrap_or(self.validated_at);
        if observed_at > now + Duration::minutes(5) {
            return false;
        }
        let maximum_age = self.maximum_age_minutes.min(10_080);
        now.signed_duration_since(observed_at) <= Duration::minutes(maximum_age as i64)
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataSourceReadMethod {
    #[default]
    Unknown,
    HttpGet,
    LocalFileRead,
    DirectoryList,
    PlatformApi,
    DatabaseQuery,
    McpTool,
    HostTool,
    Custom,
}

/// Replayable, secret-free description of the reader exercised by validation.
/// Tokens, paths, URLs, and parameter values remain Host-owned.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSourceReadRecipe {
    #[serde(default)]
    pub method: DataSourceReadMethod,
    pub operation_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub configuration_keys: Vec<String>,
    pub output_schema: Value,
}

impl DataSourceReadRecipe {
    pub fn is_structurally_usable(&self) -> bool {
        let keys = self
            .configuration_keys
            .iter()
            .map(|key| key.trim())
            .collect::<std::collections::BTreeSet<_>>();
        self.method != DataSourceReadMethod::Unknown
            && !self.operation_id.trim().is_empty()
            && keys.len() == self.configuration_keys.len()
            && !keys.contains("")
            && self
                .output_schema
                .as_object()
                .and_then(|schema| schema.get("type"))
                .and_then(Value::as_str)
                .is_some_and(|kind| {
                    matches!(
                        kind,
                        "array" | "object" | "string" | "number" | "integer" | "boolean" | "null"
                    )
                })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataSourceAccessPolicy {
    #[default]
    DefaultEnabled,
    ConversationOptIn,
    AskWhenRelevant,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataSourceValidationState {
    Verified,
    VerifiedEmpty,
    Failed,
}
