// SPDX-License-Identifier: Apache-2.0
//! Public, secret-free identities for model connections and source accounts.

use crate::{generation::ModelProtocol, model_capabilities::ModelExecutionCapabilities};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const CONNECTION_KEY_PREFIX: &str = "model.connection.";
pub const ACCOUNT_KEY_PREFIX: &str = "model.account.";
pub const GATEWAY_ATTEMPT_KEY_PREFIX: &str = "model.gateway_attempt.";
pub const PROBE_ATTEMPT_KEY_PREFIX: &str = "model.probe_attempt.";

pub fn owns_setting(key: &str) -> bool {
    key.starts_with(CONNECTION_KEY_PREFIX)
        || key.starts_with(ACCOUNT_KEY_PREFIX)
        || key.starts_with(GATEWAY_ATTEMPT_KEY_PREFIX)
        || key.starts_with(PROBE_ATTEMPT_KEY_PREFIX)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelProbeAttempt {
    pub request_id: String,
    pub connection_id: String,
    pub connection_revision: i64,
    pub account_id: Option<String>,
    pub account_revision: Option<i64>,
    pub credential_binding: String,
    pub model_id: String,
    pub protocol: ModelProtocol,
    pub started_at_ms: i64,
    /// None is in-flight or interrupted/unknown; never replay automatically.
    pub finished_at_ms: Option<i64>,
    pub text_confirmed: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayAttemptOutcome {
    InFlight,
    Completed,
    Interrupted,
    UpstreamRejected,
    TransportFailed,
}

/// Each field is independently unknown. Cached input is a subset/details field,
/// never added again to input_tokens. Repeated provider snapshots are not sums.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
    pub reasoning_output_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayAttemptRecord {
    pub id: String,
    pub profile_id: String,
    pub profile_revision: i64,
    pub connection_id: String,
    pub account_id: Option<String>,
    /// Authenticated gateway credential generation, not a caller-spoofable name.
    pub client_access_revision: i64,
    pub protocol: ModelProtocol,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub outcome: GatewayAttemptOutcome,
    pub http_status: Option<u16>,
    pub usage: Option<GatewayUsage>,
}

impl GatewayAttemptRecord {
    pub fn validate(&self) -> Result<(), String> {
        for id in [&self.id, &self.profile_id, &self.connection_id] {
            validate_identity(id)?;
        }
        if let Some(id) = &self.account_id {
            validate_identity(id)?;
        }
        if self.profile_revision < 1
            || self.client_access_revision < 1
            || self.started_at_ms < 0
            || self
                .finished_at_ms
                .is_some_and(|time| time < self.started_at_ms)
            || (self.outcome == GatewayAttemptOutcome::InFlight) != self.finished_at_ms.is_none()
            || self
                .http_status
                .is_some_and(|status| !(100..=599).contains(&status))
        {
            return Err("invalid gateway attempt".into());
        }
        Ok(())
    }
    pub fn same_identity(&self, next: &Self) -> bool {
        self.id == next.id
            && self.profile_id == next.profile_id
            && self.profile_revision == next.profile_revision
            && self.connection_id == next.connection_id
            && self.account_id == next.account_id
            && self.client_access_revision == next.client_access_revision
            && self.protocol == next.protocol
            && self.started_at_ms == next.started_at_ms
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayAttemptSnapshot {
    pub revision: i64,
    pub record: GatewayAttemptRecord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSourceKind {
    NolaChatgpt,
    Manual,
    OpenCode,
    Codex,
    KimiCode,
    KimiDesktop,
    KimiWork,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelAccountState {
    Ready,
    Expired,
    Denied,
    Disconnected,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelAccount {
    pub id: String,
    pub display_name: String,
    pub source: ModelSourceKind,
    /// Opaque Host vault identity, never a token, API key or source file path.
    pub credential_ref: Option<String>,
    pub state: ModelAccountState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelConnection {
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub protocols: Vec<ModelProtocol>,
    pub account_id: Option<String>,
    pub source: ModelSourceKind,
    /// Stable source-owned configuration identity; never used to infer auth.
    pub source_id: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogModel {
    pub model_id: String,
    pub display_name: String,
    pub protocol: ModelProtocol,
    pub context_window: Option<u64>,
    pub output_limit: Option<u64>,
    pub modalities: Vec<String>,
    pub reasoning_levels: Vec<String>,
    /// Public reasoning summaries are separate from effort support.
    #[serde(default)]
    pub summary_supported: Option<bool>,
    /// None is unknown, not an all-false capability declaration.
    pub execution_capabilities: Option<ModelExecutionCapabilities>,
}

impl CatalogModel {
    /// Protocol prefix makes the key unambiguous even when model IDs contain ':'.
    pub fn catalog_key(&self) -> String {
        format!("{}:{}", self.protocol.as_str(), self.model_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCatalog {
    pub observed_at_ms: i64,
    pub source: ModelSourceKind,
    pub models: Vec<CatalogModel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelConnectionDocument {
    pub format_version: u32,
    pub connection: ModelConnection,
    pub catalog: Option<ModelCatalog>,
    /// Independent evidence and user overrides; never overwrite catalog declarations.
    pub capability_assessments: Option<Vec<ModelCapabilityAssessment>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCapabilityAssessment {
    pub model_id: String,
    pub protocol: ModelProtocol,
    pub evidence: Option<ModelCapabilityEvidence>,
    pub user_override: Option<ModelExecutionCapabilities>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCapabilityEvidence {
    pub request_id: String,
    pub connection_revision: i64,
    pub account_revision: Option<i64>,
    pub credential_binding: String,
    pub observed_at_ms: i64,
    pub requests_sent: u32,
    /// A minimal successful text response proves only text generation.
    pub text_generation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelConnectionSnapshot {
    pub revision: i64,
    pub document: ModelConnectionDocument,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelAccountSnapshot {
    pub revision: i64,
    pub account: ModelAccount,
}

pub fn validate_identity(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 256 || id.trim() != id || id.chars().any(char::is_control) {
        return Err("invalid model connection identity".into());
    }
    Ok(())
}

fn validate_label(label: &str) -> Result<(), String> {
    if label.trim().is_empty() || label.len() > 512 || label.chars().any(char::is_control) {
        return Err("invalid model connection label".into());
    }
    Ok(())
}

impl ModelAccount {
    pub fn validate(&self) -> Result<(), String> {
        validate_identity(&self.id)?;
        validate_label(&self.display_name)?;
        if let Some(reference) = &self.credential_ref {
            validate_identity(reference)?;
        }
        if self.state == ModelAccountState::Ready && self.credential_ref.is_none() {
            return Err("ready model account requires a credential reference".into());
        }
        if self.source == ModelSourceKind::NolaChatgpt
            && self.credential_ref.as_ref().is_some_and(|reference| reference != &format!("siwc:{}", self.id)) {
            return Err("SIWC account reference must bind its own identity".into());
        }
        Ok(())
    }
}

impl ModelConnection {
    pub fn validate(&self) -> Result<(), String> {
        validate_identity(&self.id)?;
        validate_label(&self.display_name)?;
        let url = url::Url::parse(&self.base_url).map_err(|_| "invalid connection URL")?;
        if !matches!(url.scheme(), "https" | "http")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "connection URL must be HTTP(S) without credentials, query or fragment".into(),
            );
        }
        let mut protocols = BTreeSet::new();
        if self.protocols.is_empty() || self.protocols.iter().any(|p| !protocols.insert(p.as_str()))
        {
            return Err("connection requires unique explicit protocols".into());
        }
        if self.source == ModelSourceKind::NolaChatgpt {
            if self.protocols != [ModelProtocol::SiwcResponses]
                || self.account_id.is_none()
                || self.source_id != self.account_id
                || self.base_url != "https://api.openai.com/v1"
            { return Err("Nola ChatGPT requires its owned subscription route".into()); }
        } else if self.protocols.contains(&ModelProtocol::SiwcResponses) {
            return Err("SIWC protocol requires a Nola ChatGPT owner".into());
        }
        if let Some(id) = &self.account_id {
            validate_identity(id)?;
        }
        if let Some(id) = &self.source_id {
            validate_identity(id)?;
        }
        Ok(())
    }

    pub fn same_route(&self, other: &Self) -> bool {
        url::Url::parse(&self.base_url).ok() == url::Url::parse(&other.base_url).ok()
            && self.account_id == other.account_id
            && self.protocols.len() == other.protocols.len()
            && self.protocols.iter().all(|p| other.protocols.contains(p))
    }
}

impl ModelConnectionDocument {
    pub fn validate(&self) -> Result<(), String> {
        if self.format_version != 1 {
            return Err("unsupported model connection format".into());
        }
        self.connection.validate()?;
        if let Some(assessments) = &self.capability_assessments {
            let mut seen = BTreeSet::new();
            if assessments.len() > 4096 {
                return Err("too many capability assessments".into());
            }
            for item in assessments {
                validate_identity(&item.model_id)?;
                if !self.connection.protocols.contains(&item.protocol)
                    || !seen.insert((item.protocol.as_str(), item.model_id.as_str()))
                {
                    return Err("invalid capability assessment route".into());
                }
                if let Some(evidence) = &item.evidence {
                    if evidence.requests_sent != 1
                        || !evidence.text_generation
                        || evidence.connection_revision < 1
                        || evidence.observed_at_ms <= 0
                        || evidence.credential_binding.len() != 64
                    {
                        return Err("invalid capability evidence".into());
                    }
                }
            }
        }
        if let Some(catalog) = &self.catalog {
            if catalog.observed_at_ms < 0 || catalog.models.len() > 4096 {
                return Err("invalid model catalog bounds".into());
            }
            let mut seen = BTreeSet::new();
            for model in &catalog.models {
                validate_identity(&model.model_id)?;
                validate_label(&model.display_name)?;
                if !self.connection.protocols.contains(&model.protocol)
                    || !seen.insert((model.protocol.as_str(), model.model_id.as_str()))
                    || model.context_window == Some(0)
                    || model.output_limit == Some(0)
                    || model.modalities.len() > 16
                    || model.reasoning_levels.len() > 32
                {
                    return Err("invalid or duplicate model catalog route".into());
                }
                for value in model.modalities.iter().chain(&model.reasoning_levels) {
                    validate_identity(value)?;
                }
                if let Some(capabilities) = model.execution_capabilities {
                    if capabilities.sampling_requires_no_reasoning
                        && (!capabilities.sampling_parameters || !capabilities.reasoning)
                    {
                        return Err("invalid catalog sampling restriction".into());
                    }
                }
            }
        }
        Ok(())
    }
}
