// SPDX-License-Identifier: Apache-2.0
//! Model profile and runtime-state use-case boundary.

pub use nola_domain::model_capabilities::{ChatOutputTokenField, ModelExecutionCapabilities};
use nola_store_port::model::{ModelRepository, StoredModelProfile, StoredModelRuntime};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub use nola_domain::configuration_patch::validate_model_reasoning;

/// Product projection for one configured model. This is owned by Application;
/// the similarly shaped Store record remains private to the mapping below.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelProfile {
    pub id: String,
    pub display_name: String,
    pub provider: String,
    pub model_name: String,
    pub runtime: String,
    pub source_json: String,
    pub parameters_json: String,
    pub selected: bool,
    pub enabled: bool,
    pub revision: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub deleted_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelRuntimePhase {
    Starting,
    Ready,
    Stopping,
    Stopped,
    Failed,
}

impl ModelRuntimePhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }
}

impl TryFrom<&str> for ModelRuntimePhase {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "starting" => Ok(Self::Starting),
            "ready" => Ok(Self::Ready),
            "stopping" => Ok(Self::Stopping),
            "stopped" => Ok(Self::Stopped),
            "failed" => Ok(Self::Failed),
            other => Err(format!("unknown model runtime phase: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelRuntime {
    pub host_id: String,
    pub profile_id: String,
    pub phase: ModelRuntimePhase,
    pub endpoint_json: Option<String>,
    pub process_id: Option<i64>,
    pub error: Option<String>,
    pub started_at_ms: Option<i64>,
    pub stopped_at_ms: Option<i64>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelCommand {
    RecordGatewayAttempt {
        input: nola_domain::model_connection::GatewayAttemptRecord,
        expected_revision: Option<i64>,
    },
    ListGatewayAttempts {
        limit: usize,
        offset: usize,
    },
    SaveAccount {
        input: nola_domain::model_connection::ModelAccount,
        expected_revision: Option<i64>,
        updated_at_ms: i64,
    },
    ImportConnection {
        input: nola_domain::model_connection::ModelConnection,
        expected_revision: Option<i64>,
        account: Option<nola_domain::model_connection::ModelAccount>,
        expected_account_revision: Option<i64>,
        models: Vec<nola_domain::model_connection::CatalogModel>,
        enable_models: bool,
        updated_at_ms: i64,
    },
    GetAccount {
        id: String,
    },
    ListAccounts,
    SaveConnection {
        input: nola_domain::model_connection::ModelConnection,
        expected_revision: Option<i64>,
        updated_at_ms: i64,
    },
    GetConnection {
        id: String,
    },
    ListConnections,
    ReserveModelProbe {
        input: nola_domain::model_connection::ModelProbeAttempt,
    },
    FinishModelProbe {
        request_id: String,
        text_confirmed: bool,
        finished_at_ms: i64,
    },
    GetModelProbeAttempt {
        request_id: String,
    },
    RecordConnectionCapabilityEvidence {
        connection_id: String,
        expected_revision: i64,
        model_id: String,
        protocol: nola_domain::generation::ModelProtocol,
        evidence: nola_domain::model_connection::ModelCapabilityEvidence,
    },
    SetConnectionCapabilityOverride {
        connection_id: String,
        expected_revision: i64,
        model_id: String,
        protocol: nola_domain::generation::ModelProtocol,
        capabilities: Option<ModelExecutionCapabilities>,
        updated_at_ms: i64,
    },
    RefreshConnectionCatalog {
        connection_id: String,
        expected_revision: i64,
        models: Vec<nola_domain::model_connection::CatalogModel>,
        observed_at_ms: i64,
    },
    ConfigureConnectionModels {
        connection_id: String,
        expected_revision: i64,
        /// Each key is '<wire protocol>:<exact provider model ID>'.
        enabled_model_ids: Vec<String>,
        selected_model_id: Option<String>,
        updated_at_ms: i64,
    },
    ConfigureEndpoint {
        endpoint_id: String,
        expected_revisions: std::collections::BTreeMap<String, i64>,
        name: String,
        base_url: String,
        wire_protocol: String,
        enabled: bool,
        updated_at_ms: i64,
    },
    ConfigureParameters {
        input: ModelProfile,
    },
    ResetParameters {
        profile_ids: Vec<String>,
        parameters_json: String,
        updated_at_ms: i64,
    },
    ReplaceProfiles {
        provider: String,
        runtime: String,
        profiles: Vec<ModelProfile>,
        updated_at_ms: i64,
        expected_revisions: std::collections::BTreeMap<String, i64>,
    },
    UpsertProfile {
        input: ModelProfile,
    },
    GetProfile {
        id: String,
    },
    ListProfiles,
    DeleteProfile {
        id: String,
        deleted_at_ms: i64,
    },
    ReportRuntime {
        input: ModelRuntime,
    },
    GetRuntime {
        host_id: String,
    },
}

impl From<StoredModelProfile> for ModelProfile {
    fn from(value: StoredModelProfile) -> Self {
        Self {
            id: value.id,
            display_name: value.display_name,
            provider: value.provider,
            model_name: value.model_name,
            runtime: value.runtime,
            source_json: value.source_json,
            parameters_json: value.parameters_json,
            selected: value.selected,
            enabled: value.enabled,
            revision: value.revision,
            created_at_ms: value.created_at_ms,
            updated_at_ms: value.updated_at_ms,
            deleted_at_ms: value.deleted_at_ms,
        }
    }
}

impl From<ModelProfile> for StoredModelProfile {
    fn from(value: ModelProfile) -> Self {
        Self {
            id: value.id,
            display_name: value.display_name,
            provider: value.provider,
            model_name: value.model_name,
            runtime: value.runtime,
            source_json: value.source_json,
            parameters_json: value.parameters_json,
            selected: value.selected,
            enabled: value.enabled,
            revision: value.revision,
            created_at_ms: value.created_at_ms,
            updated_at_ms: value.updated_at_ms,
            deleted_at_ms: value.deleted_at_ms,
        }
    }
}

impl TryFrom<StoredModelRuntime> for ModelRuntime {
    type Error = String;

    fn try_from(value: StoredModelRuntime) -> Result<Self, Self::Error> {
        Ok(Self {
            host_id: value.host_id,
            profile_id: value.profile_id,
            phase: ModelRuntimePhase::try_from(value.phase.as_str())?,
            endpoint_json: value.endpoint_json,
            process_id: value.process_id,
            error: value.error,
            started_at_ms: value.started_at_ms,
            stopped_at_ms: value.stopped_at_ms,
            updated_at_ms: value.updated_at_ms,
        })
    }
}

impl From<ModelRuntime> for StoredModelRuntime {
    fn from(value: ModelRuntime) -> Self {
        Self {
            host_id: value.host_id,
            profile_id: value.profile_id,
            phase: value.phase.as_str().to_string(),
            endpoint_json: value.endpoint_json,
            process_id: value.process_id,
            error: value.error,
            started_at_ms: value.started_at_ms,
            stopped_at_ms: value.stopped_at_ms,
            updated_at_ms: value.updated_at_ms,
        }
    }
}

pub(crate) fn execute_model_command(
    store: &mut impl ModelRepository,
    command: ModelCommand,
) -> Result<Value, String> {
    // Managed connections own their source metadata and enabled selections.
    // Parameter edits remain on the existing profile command path.
    let mutation_ids: Vec<&str> = match &command {
        ModelCommand::UpsertProfile { input } => vec![input.id.as_str()],
        ModelCommand::DeleteProfile { id, .. } => vec![id.as_str()],
        _ => vec![],
    };
    if matches!(&command, ModelCommand::UpsertProfile { input }
        if serde_json::from_str::<Value>(&input.source_json).ok().is_some_and(|s| !s["connectionID"].is_null()))
    {
        return Err("managed connection profiles require a connection command".into());
    }
    if !mutation_ids.is_empty()
        || matches!(
            &command,
            ModelCommand::ConfigureEndpoint { .. } | ModelCommand::ReplaceProfiles { .. }
        )
    {
        for profile in store
            .list_model_profiles()
            .map_err(|error| error.to_string())?
        {
            let source: Value = serde_json::from_str(&profile.source_json).unwrap_or(Value::Null);
            if source["connectionID"].is_null() || profile.deleted_at_ms.is_some() {
                continue;
            }
            let overlaps = mutation_ids.contains(&profile.id.as_str())
                || match &command {
                    ModelCommand::ConfigureEndpoint { endpoint_id, .. } => {
                        source["endpointID"].as_str() == Some(endpoint_id.as_str())
                    }
                    ModelCommand::ReplaceProfiles {
                        provider, runtime, ..
                    } => profile.provider == *provider && profile.runtime == *runtime,
                    _ => false,
                };
            if overlaps {
                return Err("managed connection profiles require a connection command".into());
            }
        }
    }
    match command {
        ModelCommand::ImportConnection { input, expected_revision, account, expected_account_revision,
            models, enable_models, updated_at_ms } => crate::model_connection::import_connection(
                store, input, expected_revision, account, expected_account_revision, models, enable_models, updated_at_ms),
        ModelCommand::SaveAccount {
            input,
            expected_revision,
            updated_at_ms,
        } => {
            input.validate()?;
            serde_json::to_value(
                store
                    .save_model_account(&input, expected_revision, updated_at_ms)
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())
        }
        ModelCommand::GetAccount { id } => serde_json::to_value(
            store
                .model_account(&id)
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string()),
        ModelCommand::ReserveModelProbe { input } => store
            .reserve_model_probe(&input)
            .map(|()| Value::Null)
            .map_err(|error| error.to_string()),
        ModelCommand::FinishModelProbe {
            request_id,
            text_confirmed,
            finished_at_ms,
        } => store
            .finish_model_probe(&request_id, text_confirmed, finished_at_ms)
            .map(|()| Value::Null)
            .map_err(|error| error.to_string()),
        ModelCommand::GetModelProbeAttempt { request_id } => serde_json::to_value(
            store
                .model_probe_attempt(&request_id)
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string()),
        ModelCommand::RecordGatewayAttempt {
            input,
            expected_revision,
        } => {
            input.validate()?;
            serde_json::to_value(
                store
                    .save_gateway_attempt(&input, expected_revision)
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())
        }
        ModelCommand::ListGatewayAttempts { limit, offset } => serde_json::to_value(
            store
                .list_gateway_attempts(limit.min(256), offset)
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string()),
        ModelCommand::ListAccounts => serde_json::to_value(
            store
                .list_model_accounts()
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string()),
        ModelCommand::SaveConnection {
            input,
            expected_revision,
            updated_at_ms,
        } => {
            crate::model_connection::save_connection(store, input, expected_revision, updated_at_ms)
        }
        ModelCommand::GetConnection { id } => serde_json::to_value(
            store
                .model_connection(&id)
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string()),
        ModelCommand::ListConnections => serde_json::to_value(
            store
                .list_model_connections()
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string()),
        ModelCommand::RecordConnectionCapabilityEvidence {
            connection_id,
            expected_revision,
            model_id,
            protocol,
            evidence,
        } => crate::model_connection::set_capability_assessment(
            store,
            &connection_id,
            expected_revision,
            &model_id,
            protocol,
            Some(evidence),
            None,
            0,
        ),
        ModelCommand::SetConnectionCapabilityOverride {
            connection_id,
            expected_revision,
            model_id,
            protocol,
            capabilities,
            updated_at_ms,
        } => crate::model_connection::set_capability_assessment(
            store,
            &connection_id,
            expected_revision,
            &model_id,
            protocol,
            None,
            Some(capabilities),
            updated_at_ms,
        ),
        ModelCommand::RefreshConnectionCatalog {
            connection_id,
            expected_revision,
            models,
            observed_at_ms,
        } => crate::model_connection::refresh_catalog(
            store,
            &connection_id,
            expected_revision,
            models,
            observed_at_ms,
        ),
        ModelCommand::ConfigureConnectionModels {
            connection_id,
            expected_revision,
            enabled_model_ids,
            selected_model_id,
            updated_at_ms,
        } => crate::model_connection::configure_models(
            store,
            &connection_id,
            expected_revision,
            &enabled_model_ids,
            selected_model_id.as_deref(),
            updated_at_ms,
        ),
        ModelCommand::ConfigureEndpoint {
            endpoint_id,
            expected_revisions,
            name,
            base_url,
            wire_protocol,
            enabled,
            updated_at_ms,
        } => {
            let name = name.trim();
            if name.is_empty() || name.len() > 512 {
                return Err("endpoint name must contain 1 to 512 bytes".into());
            }
            let url = url::Url::parse(base_url.trim()).map_err(|_| "invalid endpoint URL")?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
            {
                return Err("endpoint URL must be HTTP(S), without credentials or fragment".into());
            }
            if !matches!(
                wire_protocol.as_str(),
                "responses" | "chat_completions" | "anthropic"
            ) {
                return Err("unsupported endpoint protocol".into());
            }
            let mut updated = Vec::new();
            for mut profile in store
                .list_model_profiles()
                .map_err(|error| error.to_string())?
            {
                if profile.runtime != "remote" || profile.deleted_at_ms.is_some() {
                    continue;
                }
                let mut source: Value = serde_json::from_str(&profile.source_json)
                    .map_err(|error| error.to_string())?;
                if source["endpointID"].as_str() != Some(endpoint_id.as_str()) {
                    continue;
                }
                let parameters: Value = serde_json::from_str(&profile.parameters_json)
                    .map_err(|error| error.to_string())?;
                validate_model_reasoning(
                    &source["reasoning"],
                    &wire_protocol,
                    parameters["maxOutputTokens"].as_u64().unwrap_or(16384),
                )
                .map_err(|error| format!("{}: {error}", profile.display_name))?;
                // A declaration describes one concrete transport route, not the
                // endpoint's editable display metadata. URL parsing preserves
                // equivalent spellings without carrying support to a new route.
                let previous_url = source["baseURL"]
                    .as_str()
                    .and_then(|value| url::Url::parse(value.trim()).ok());
                if previous_url.as_ref() != Some(&url)
                    || source["wireProtocol"].as_str() != Some(wire_protocol.as_str())
                {
                    source["executionCapabilities"] = Value::Null;
                }
                source["endpointName"] = json!(name);
                source["baseURL"] = json!(url.as_str());
                source["wireProtocol"] = json!(wire_protocol);
                source["endpointEnabled"] = json!(enabled);
                if source["reasoning"].is_object() {
                    source["reasoning"]["responses"] = json!(wire_protocol == "responses");
                }
                profile.source_json = source.to_string();
                profile.updated_at_ms = updated_at_ms;
                updated.push(profile);
            }
            store
                .configure_model_endpoint(&endpoint_id, &expected_revisions, &updated)
                .map_err(|error| error.to_string())?;
            let profiles: Vec<ModelProfile> = store
                .list_model_profiles()
                .map_err(|error| error.to_string())?
                .into_iter()
                .map(Into::into)
                .collect();
            serde_json::to_value(profiles).map_err(|error| error.to_string())
        }
        ModelCommand::ConfigureParameters { input } => {
            store
                .configure_model_parameters(&input.into())
                .map_err(|error| error.to_string())?;
            Ok(Value::Null)
        }
        ModelCommand::ResetParameters {
            profile_ids,
            parameters_json,
            updated_at_ms,
        } => {
            store
                .reset_model_parameters(&profile_ids, &parameters_json, updated_at_ms)
                .map_err(|error| error.to_string())?;
            Ok(Value::Null)
        }
        ModelCommand::ReplaceProfiles {
            provider,
            runtime,
            profiles,
            updated_at_ms,
            expected_revisions,
        } => {
            let profiles: Vec<StoredModelProfile> = profiles.into_iter().map(Into::into).collect();
            store
                .replace_model_profiles(
                    &provider,
                    &runtime,
                    &profiles,
                    updated_at_ms,
                    &expected_revisions,
                )
                .map_err(|error| error.to_string())?;
            Ok(Value::Null)
        }
        ModelCommand::UpsertProfile { input } => {
            store
                .upsert_model_profile(&input.into())
                .map_err(|error| error.to_string())?;
            Ok(Value::Null)
        }
        ModelCommand::GetProfile { id } => {
            let profile = store
                .model_profile(&id)
                .map_err(|error| error.to_string())?
                .map(ModelProfile::from);
            serde_json::to_value(profile)
                .map_err(|error| format!("encode model profile failed: {error}"))
        }
        ModelCommand::ListProfiles => {
            let profiles: Vec<ModelProfile> = store
                .list_model_profiles()
                .map_err(|error| error.to_string())?
                .into_iter()
                .map(ModelProfile::from)
                .collect();
            serde_json::to_value(profiles)
                .map_err(|error| format!("encode model profiles failed: {error}"))
        }
        ModelCommand::DeleteProfile { id, deleted_at_ms } => Ok(json!(store
            .delete_model_profile(&id, deleted_at_ms)
            .map_err(|error| error.to_string())?)),
        ModelCommand::ReportRuntime { input } => {
            store
                .report_model_runtime(&input.into())
                .map_err(|error| error.to_string())?;
            Ok(Value::Null)
        }
        ModelCommand::GetRuntime { host_id } => {
            let runtime = store
                .model_runtime(&host_id)
                .map_err(|error| error.to_string())?
                .map(ModelRuntime::try_from)
                .transpose()?;
            serde_json::to_value(runtime)
                .map_err(|error| format!("encode model runtime failed: {error}"))
        }
    }
}

#[cfg(test)]
#[path = "model_port_tests.rs"]
mod port_tests;

#[cfg(test)]
mod execution_capability_tests {
    use super::*;

    #[test]
    fn execution_support_is_distinct_from_product_uses_and_fails_closed() {
        let uses_only = json!({"capabilities":["chat","retrieval"]});
        assert!(ModelExecutionCapabilities::from_source(&uses_only)
            .unwrap_err()
            .starts_with("model_execution_capabilities_required:"));
        let declared = json!({"executionCapabilities":{
            "toolCalls":true,"imageInput":false,"reasoning":true,"samplingParameters":false
        }});
        let result = ModelExecutionCapabilities::from_source(&declared).unwrap();
        assert!(result.tool_calls && result.reasoning);
        assert!(!result.image_input && !result.sampling_parameters);
        assert!(!result.sampling_requires_no_reasoning);
        assert_eq!(result.chat_output_token_field, ChatOutputTokenField::Auto);
        let explicit = json!({"executionCapabilities":{
            "toolCalls":false,"imageInput":false,"reasoning":false,"samplingParameters":false,
            "chatOutputTokenField":"max_completion_tokens"
        }});
        assert_eq!(
            ModelExecutionCapabilities::from_source(&explicit)
                .unwrap()
                .chat_output_token_field,
            ChatOutputTokenField::MaxCompletionTokens
        );
        let mut invalid = explicit;
        invalid["executionCapabilities"]["chatOutputTokenField"] = json!("guess");
        assert!(ModelExecutionCapabilities::from_source(&invalid).is_err());
        assert!(
            ModelExecutionCapabilities::from_source(&json!({"executionCapabilities":{
                "toolCalls":true,"imageInput":false,"reasoning":true
            }}))
            .is_err()
        );
        assert!(
            ModelExecutionCapabilities::from_source(&json!({"executionCapabilities":{
                "toolCalls":false,"imageInput":false,"reasoning":true,
                "samplingParameters":false,"samplingRequiresNoReasoning":true
            }}))
            .is_err()
        );
    }
}
