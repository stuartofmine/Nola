// SPDX-License-Identifier: Apache-2.0
//! Typed data-source aggregate and use-case boundary.
//!
//! Hosts own I/O. The Application Service owns configuration, validation
//! intent, enablement, revision checks, and conversation overrides.

use nola_domain::data_source::{DataSourceAccessPolicy, DataSourceValidationState, DataSourceContract};
use nola_store_port::data_source::{
    DataSourceReadRepository, DataSourceRepository, StoredDataSourceRecord, StoredDataSourceUsage,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

const DATA_SOURCE_METADATA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DataSourceKind {
    Http,
    LocalFile,
    Directory,
    Platform,
    Web,
    Database,
    Mcp,
    Custom,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DataSourceLocationKind {
    HttpUrl,
    LocalPath,
    PlatformResource,
    DatabaseReference,
    McpCapability,
    OpaqueReference,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DataSourceConfiguration {
    pub connector_id: String,
    pub location_kind: DataSourceLocationKind,
    pub location: String,
    #[serde(default)]
    pub parameters: BTreeMap<String, Value>,
    #[serde(default)]
    pub secret_references: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DataSourceAuthorizationState {
    NotRequired,
    NotDetermined,
    Authorized,
    Denied,
    Restricted,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DataSourceAuthorization {
    pub state: DataSourceAuthorizationState,
    pub capability: Option<String>,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DataSourceFreshnessPolicy {
    pub maximum_age_minutes: u64,
    pub allow_stale_fallback: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DataSourceContentReference {
    pub uri: String,
    pub mime_type: String,
    pub byte_count: u64,
    pub content_version: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DataSourceReadStatus {
    NeverRead,
    Pending,
    Ready,
    Empty,
    Denied,
    Unavailable,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DataSourceReadState {
    pub status: DataSourceReadStatus,
    pub read_token: Option<String>,
    pub content_reference: Option<DataSourceContentReference>,
    pub observed_at_ms: Option<i64>,
    pub schema_version: Option<u32>,
    pub record_count: Option<u64>,
    pub diagnostic: Option<String>,
    pub requested_at_ms: Option<i64>,
    pub completed_at_ms: Option<i64>,
}

impl DataSourceReadState {
    fn never_read() -> Self {
        Self {
            status: DataSourceReadStatus::NeverRead,
            read_token: None,
            content_reference: None,
            observed_at_ms: None,
            schema_version: None,
            record_count: None,
            diagnostic: None,
            requested_at_ms: None,
            completed_at_ms: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DataSourceLifecycle {
    Draft,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DataSourceValidationStatus {
    NotValidated,
    Pending,
    Verified,
    VerifiedEmpty,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DataSourceValidation {
    pub status: DataSourceValidationStatus,
    pub validation_token: Option<String>,
    pub contract: Option<DataSourceContract>,
    pub diagnostic: Option<String>,
    pub attempted_at_ms: Option<i64>,
    pub completed_at_ms: Option<i64>,
}

impl DataSourceValidation {
    fn not_validated() -> Self {
        Self {
            status: DataSourceValidationStatus::NotValidated,
            validation_token: None,
            contract: None,
            diagnostic: None,
            attempted_at_ms: None,
            completed_at_ms: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DataSourceDraft {
    pub id: String,
    pub name: String,
    pub kind: DataSourceKind,
    pub configuration: DataSourceConfiguration,
    pub access_policy: DataSourceAccessPolicy,
    pub authorization: DataSourceAuthorization,
    pub freshness_policy: DataSourceFreshnessPolicy,
    pub capabilities: Vec<String>,
    pub privacy: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UpdateDataSourceConfiguration {
    pub name: String,
    pub kind: DataSourceKind,
    pub configuration: DataSourceConfiguration,
    pub access_policy: DataSourceAccessPolicy,
    pub authorization: DataSourceAuthorization,
    pub freshness_policy: DataSourceFreshnessPolicy,
    pub capabilities: Vec<String>,
    pub privacy: String,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DataSource {
    pub id: String,
    pub name: String,
    pub kind: DataSourceKind,
    pub configuration: DataSourceConfiguration,
    pub access_policy: DataSourceAccessPolicy,
    pub authorization: DataSourceAuthorization,
    pub validation: DataSourceValidation,
    pub read_state: DataSourceReadState,
    pub freshness_policy: DataSourceFreshnessPolicy,
    pub default_enabled: bool,
    pub lifecycle: DataSourceLifecycle,
    pub capabilities: Vec<String>,
    pub privacy: String,
    pub revision: i64,
    pub last_successful_read_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub deleted_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum DataSourceReadResult {
    Succeeded {
        content_reference: DataSourceContentReference,
        observed_at_ms: i64,
        schema_version: u32,
        record_count: u64,
    },
    Empty {
        observed_at_ms: i64,
        schema_version: u32,
    },
    Denied {
        authorization: DataSourceAuthorization,
        diagnostic: String,
    },
    Unavailable {
        diagnostic: String,
    },
    Failed {
        code: String,
        diagnostic: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DataSourceSelectionReason {
    Ready,
    ReadyEmpty,
    DisabledByDefault,
    DisabledForConversation,
    LifecycleNotReady,
    AuthorizationUnavailable,
    ValidationUnavailable,
    RefreshRequired,
    ReadPending,
    ReadUnavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DataSourceContextSelection {
    pub source_id: String,
    pub source_revision: i64,
    pub enabled: bool,
    pub usable: bool,
    pub should_refresh: bool,
    pub reason: DataSourceSelectionReason,
    pub content_reference: Option<DataSourceContentReference>,
    pub observed_at_ms: Option<i64>,
    pub schema_version: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum DataSourceValidationResult {
    Succeeded { contract: DataSourceContract },
    Failed { code: String, diagnostic: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationDataSourceOverride {
    Inherit,
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DataSourceUsage {
    pub id: String,
    pub source_id: String,
    pub source_title: String,
    pub consumer: String,
    pub purpose: String,
    pub conversation_id: Option<String>,
    pub disclosed_to_cloud: Option<bool>,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
    pub used_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DataSourceCommand {
    CreateDraft {
        input: DataSourceDraft,
    },
    UpdateConfiguration {
        id: String,
        expected_revision: i64,
        input: UpdateDataSourceConfiguration,
    },
    RequestValidation {
        id: String,
        expected_revision: i64,
        requested_at_ms: i64,
    },
    RecordValidation {
        id: String,
        validation_token: String,
        result: DataSourceValidationResult,
        recorded_at_ms: i64,
    },
    SetDefaultEnabled {
        id: String,
        expected_revision: i64,
        enabled: bool,
        updated_at_ms: i64,
    },
    RequestRefresh {
        id: String,
        expected_revision: i64,
        reason: String,
        requested_at_ms: i64,
    },
    RecordRead {
        id: String,
        read_token: String,
        result: DataSourceReadResult,
        recorded_at_ms: i64,
    },
    ResolveForConversation {
        conversation_id: String,
        now_ms: i64,
    },
    Get {
        id: String,
    },
    List,
    Delete {
        id: String,
        expected_revision: i64,
        deleted_at_ms: i64,
    },
    SetConversationOverride {
        conversation_id: String,
        source_id: String,
        value: ConversationDataSourceOverride,
        updated_at_ms: i64,
    },
    GetConversationOverride {
        conversation_id: String,
        source_id: String,
    },
    RecordUsage {
        input: DataSourceUsage,
    },
    ListUsage {
        limit: usize,
    },
}

pub(crate) fn execute_data_source_command<R: DataSourceRepository + ?Sized>(
    store: &mut R,
    command: DataSourceCommand,
) -> Result<Value, String> {
    match command {
        DataSourceCommand::CreateDraft { input } => {
            validate_draft(&input)?;
            if store
                .data_source(&input.id)
                .map_err(store_error("load data source"))?
                .is_some()
            {
                return Err(format!("data source {} already exists", input.id));
            }
            save_source(
                store,
                DataSource {
                    id: input.id,
                    name: input.name,
                    kind: input.kind,
                    configuration: input.configuration,
                    access_policy: input.access_policy,
                    authorization: input.authorization,
                    validation: DataSourceValidation::not_validated(),
                    read_state: DataSourceReadState::never_read(),
                    freshness_policy: input.freshness_policy,
                    default_enabled: false,
                    lifecycle: DataSourceLifecycle::Draft,
                    capabilities: input.capabilities,
                    privacy: input.privacy,
                    revision: 0,
                    last_successful_read_at_ms: None,
                    created_at_ms: input.created_at_ms,
                    updated_at_ms: input.updated_at_ms,
                    deleted_at_ms: None,
                },
            )
        }
        DataSourceCommand::UpdateConfiguration {
            id,
            expected_revision,
            input,
        } => {
            validate_definition(
                &id,
                &input.name,
                &input.configuration,
                &input.freshness_policy,
                &input.capabilities,
                &input.privacy,
            )?;
            let mut source = required_source(store, &id)?;
            require_revision(&source, expected_revision)?;
            source.name = input.name;
            source.kind = input.kind;
            source.configuration = input.configuration;
            source.access_policy = input.access_policy;
            source.authorization = input.authorization;
            source.freshness_policy = input.freshness_policy;
            source.capabilities = input.capabilities;
            source.privacy = input.privacy;
            source.validation = DataSourceValidation::not_validated();
            source.read_state = DataSourceReadState::never_read();
            source.default_enabled = false;
            source.lifecycle = DataSourceLifecycle::Draft;
            source.updated_at_ms = input.updated_at_ms;
            save_source(store, source)
        }
        DataSourceCommand::RequestValidation {
            id,
            expected_revision,
            requested_at_ms,
        } => {
            let mut source = required_source(store, &id)?;
            require_revision(&source, expected_revision)?;
            if source.validation.status == DataSourceValidationStatus::Pending {
                return Err(format!("data source {id} validation is already pending"));
            }
            source.validation = DataSourceValidation {
                status: DataSourceValidationStatus::Pending,
                validation_token: Some(Uuid::new_v4().to_string()),
                contract: None,
                diagnostic: None,
                attempted_at_ms: Some(requested_at_ms),
                completed_at_ms: None,
            };
            source.default_enabled = false;
            source.lifecycle = DataSourceLifecycle::Draft;
            source.updated_at_ms = requested_at_ms;
            save_source(store, source)
        }
        DataSourceCommand::RecordValidation {
            id,
            validation_token,
            result,
            recorded_at_ms,
        } => {
            let mut source = required_source(store, &id)?;
            if source.validation.status != DataSourceValidationStatus::Pending
                || source.validation.validation_token.as_deref() != Some(validation_token.as_str())
            {
                return Err(format!(
                    "stale or invalid validation token for data source {id}"
                ));
            }
            let attempted_at_ms = source.validation.attempted_at_ms;
            match result {
                DataSourceValidationResult::Succeeded { contract } => {
                    if !contract.is_structurally_usable(&id) {
                        return Err(format!("invalid validation contract for data source {id}"));
                    }
                    if contract.connector_id != source.configuration.connector_id {
                        return Err(format!(
                            "validation contract connector does not match data source {id}"
                        ));
                    }
                    if contract.maximum_age_minutes != source.freshness_policy.maximum_age_minutes {
                        return Err(format!(
                            "validation contract freshness does not match data source {id}"
                        ));
                    }
                    let status = match contract.validation_state {
                        DataSourceValidationState::Verified => DataSourceValidationStatus::Verified,
                        DataSourceValidationState::VerifiedEmpty => {
                            DataSourceValidationStatus::VerifiedEmpty
                        }
                        DataSourceValidationState::Failed => {
                            return Err(format!(
                                "successful validation result for {id} contains a failed contract"
                            ));
                        }
                    };
                    source.validation = DataSourceValidation {
                        status,
                        validation_token: None,
                        contract: Some(contract),
                        diagnostic: None,
                        attempted_at_ms,
                        completed_at_ms: Some(recorded_at_ms),
                    };
                    source.lifecycle = DataSourceLifecycle::Ready;
                }
                DataSourceValidationResult::Failed { code, diagnostic } => {
                    let diagnostic = diagnostic.trim();
                    if code.trim().is_empty() || diagnostic.is_empty() {
                        return Err("failed validation requires a code and diagnostic".into());
                    }
                    source.validation = DataSourceValidation {
                        status: DataSourceValidationStatus::Failed,
                        validation_token: None,
                        contract: None,
                        diagnostic: Some(format!(
                            "{}: {}",
                            code.trim(),
                            bounded(diagnostic, 2_000)
                        )),
                        attempted_at_ms,
                        completed_at_ms: Some(recorded_at_ms),
                    };
                    source.default_enabled = false;
                    source.lifecycle = DataSourceLifecycle::Failed;
                }
            }
            source.updated_at_ms = recorded_at_ms;
            save_source(store, source)
        }
        DataSourceCommand::SetDefaultEnabled {
            id,
            expected_revision,
            enabled,
            updated_at_ms,
        } => {
            let mut source = required_source(store, &id)?;
            require_revision(&source, expected_revision)?;
            if enabled && !source_is_validated(&source) {
                return Err(format!(
                    "data source {id} cannot be enabled before successful validation"
                ));
            }
            source.default_enabled = enabled;
            // Default selection is independent of validation/readiness. A
            // conversation can explicitly enable a validated source whose
            // default is off; changing the default must not erase its state.
            source.updated_at_ms = updated_at_ms;
            save_source(store, source)
        }
        DataSourceCommand::RequestRefresh {
            id,
            expected_revision,
            reason,
            requested_at_ms,
        } => {
            let mut source = required_source(store, &id)?;
            require_revision(&source, expected_revision)?;
            if !source_is_validated(&source) {
                return Err(format!("data source {id} must be validated before refresh"));
            }
            if source.read_state.status == DataSourceReadStatus::Pending {
                return Err(format!("data source {id} read is already pending"));
            }
            if reason.trim().is_empty() {
                return Err("data source refresh reason is required".into());
            }
            let prior = source.read_state;
            source.read_state = DataSourceReadState {
                status: DataSourceReadStatus::Pending,
                read_token: Some(Uuid::new_v4().to_string()),
                content_reference: prior.content_reference,
                observed_at_ms: prior.observed_at_ms,
                schema_version: prior.schema_version,
                record_count: prior.record_count,
                diagnostic: Some(bounded(reason.trim(), 500)),
                requested_at_ms: Some(requested_at_ms),
                completed_at_ms: prior.completed_at_ms,
            };
            source.updated_at_ms = requested_at_ms;
            save_source(store, source)
        }
        DataSourceCommand::RecordRead {
            id,
            read_token,
            result,
            recorded_at_ms,
        } => {
            let mut source = required_source(store, &id)?;
            if source.read_state.status != DataSourceReadStatus::Pending
                || source.read_state.read_token.as_deref() != Some(read_token.as_str())
            {
                return Err(format!("stale or invalid read token for data source {id}"));
            }
            let requested_at_ms = source.read_state.requested_at_ms;
            let prior_reference = source.read_state.content_reference.clone();
            let prior_observed_at_ms = source.read_state.observed_at_ms;
            let prior_schema_version = source.read_state.schema_version;
            let prior_record_count = source.read_state.record_count;
            match result {
                DataSourceReadResult::Succeeded {
                    content_reference,
                    observed_at_ms,
                    schema_version,
                    record_count,
                } => {
                    validate_content_reference(&content_reference)?;
                    validate_read_observation(observed_at_ms)?;
                    source.last_successful_read_at_ms = Some(recorded_at_ms);
                    source.read_state = DataSourceReadState {
                        status: DataSourceReadStatus::Ready,
                        read_token: None,
                        content_reference: Some(content_reference),
                        observed_at_ms: Some(observed_at_ms),
                        schema_version: Some(schema_version),
                        record_count: Some(record_count),
                        diagnostic: None,
                        requested_at_ms,
                        completed_at_ms: Some(recorded_at_ms),
                    };
                }
                DataSourceReadResult::Empty {
                    observed_at_ms,
                    schema_version,
                } => {
                    validate_read_observation(observed_at_ms)?;
                    source.last_successful_read_at_ms = Some(recorded_at_ms);
                    source.read_state = DataSourceReadState {
                        status: DataSourceReadStatus::Empty,
                        read_token: None,
                        content_reference: None,
                        observed_at_ms: Some(observed_at_ms),
                        schema_version: Some(schema_version),
                        record_count: Some(0),
                        diagnostic: None,
                        requested_at_ms,
                        completed_at_ms: Some(recorded_at_ms),
                    };
                }
                DataSourceReadResult::Denied {
                    authorization,
                    diagnostic,
                } => {
                    source.authorization = authorization;
                    source.read_state = failed_read_state(
                        DataSourceReadStatus::Denied,
                        diagnostic,
                        requested_at_ms,
                        recorded_at_ms,
                        prior_reference,
                        prior_observed_at_ms,
                        prior_schema_version,
                        prior_record_count,
                    );
                }
                DataSourceReadResult::Unavailable { diagnostic } => {
                    source.read_state = failed_read_state(
                        DataSourceReadStatus::Unavailable,
                        diagnostic,
                        requested_at_ms,
                        recorded_at_ms,
                        prior_reference,
                        prior_observed_at_ms,
                        prior_schema_version,
                        prior_record_count,
                    );
                }
                DataSourceReadResult::Failed { code, diagnostic } => {
                    if code.trim().is_empty() {
                        return Err("failed data-source read requires an error code".into());
                    }
                    source.read_state = failed_read_state(
                        DataSourceReadStatus::Failed,
                        format!("{}: {}", code.trim(), diagnostic.trim()),
                        requested_at_ms,
                        recorded_at_ms,
                        prior_reference,
                        prior_observed_at_ms,
                        prior_schema_version,
                        prior_record_count,
                    );
                }
            }
            source.updated_at_ms = recorded_at_ms;
            save_source(store, source)
        }
        DataSourceCommand::ResolveForConversation {
            conversation_id,
            now_ms,
        } => {
            if conversation_id.trim().is_empty() {
                return Err("conversation id is required".into());
            }
            let now = chrono::DateTime::from_timestamp_millis(now_ms)
                .ok_or_else(|| format!("invalid context resolution timestamp {now_ms}"))?;
            let selections = store
                .list_data_sources_for_conversation(&conversation_id)
                .map_err(store_error("list conversation data sources"))?
                .into_iter()
                .map(|(source, conversation_override)| {
                    let source = data_source_from_stored(source)?;
                    Ok(resolve_context_source(source, conversation_override, now))
                })
                .collect::<Result<Vec<_>, String>>()?;
            serde_json::to_value(selections)
                .map_err(|error| format!("encode context source selection failed: {error}"))
        }
        DataSourceCommand::Get { id } => {
            let source = store
                .data_source(&id)
                .map_err(store_error("load data source"))?
                .map(data_source_from_stored)
                .transpose()?;
            serde_json::to_value(source)
                .map_err(|error| format!("encode data source result failed: {error}"))
        }
        DataSourceCommand::List => {
            let sources = store
                .list_data_sources()
                .map_err(store_error("list data sources"))?
                .into_iter()
                .map(data_source_from_stored)
                .collect::<Result<Vec<_>, _>>()?;
            serde_json::to_value(sources)
                .map_err(|error| format!("encode data source list failed: {error}"))
        }
        DataSourceCommand::Delete {
            id,
            expected_revision,
            deleted_at_ms,
        } => {
            let source = required_source(store, &id)?;
            require_revision(&source, expected_revision)?;
            Ok(json!(store
                .delete_data_source(&id, deleted_at_ms)
                .map_err(store_error("delete data source"))?))
        }
        DataSourceCommand::SetConversationOverride {
            conversation_id,
            source_id,
            value,
            updated_at_ms,
        } => {
            let enabled = match value {
                ConversationDataSourceOverride::Inherit => None,
                ConversationDataSourceOverride::Enabled => Some(true),
                ConversationDataSourceOverride::Disabled => Some(false),
            };
            store
                .set_data_source_override(&conversation_id, &source_id, enabled, updated_at_ms)
                .map_err(store_error("save data source override"))?;
            Ok(json!(true))
        }
        DataSourceCommand::GetConversationOverride {
            conversation_id,
            source_id,
        } => {
            let value = match store
                .data_source_override(&conversation_id, &source_id)
                .map_err(store_error("load data source override"))?
            {
                Some(true) => ConversationDataSourceOverride::Enabled,
                Some(false) => ConversationDataSourceOverride::Disabled,
                None => ConversationDataSourceOverride::Inherit,
            };
            serde_json::to_value(value)
                .map_err(|error| format!("encode data source override failed: {error}"))
        }
        DataSourceCommand::RecordUsage { input } => {
            store
                .append_data_source_usage(&input.into())
                .map_err(store_error("save data source usage"))?;
            Ok(json!(true))
        }
        DataSourceCommand::ListUsage { limit } => {
            let usage = store
                .list_data_source_usage(limit)
                .map_err(store_error("list data source usage"))?
                .into_iter()
                .map(DataSourceUsage::try_from)
                .collect::<Result<Vec<_>, _>>()?;
            serde_json::to_value(usage)
                .map_err(|error| format!("encode data source usage failed: {error}"))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredDataSourceMetadata {
    version: u32,
    configuration: DataSourceConfiguration,
    authorization: DataSourceAuthorization,
    freshness_policy: DataSourceFreshnessPolicy,
    lifecycle: DataSourceLifecycle,
    read_state: DataSourceReadState,
    last_successful_read_at_ms: Option<i64>,
}

fn save_source<R: DataSourceRepository + ?Sized>(
    store: &mut R,
    source: DataSource,
) -> Result<Value, String> {
    let stored = stored_from_data_source(source)?;
    let saved = store
        .upsert_data_source(&stored)
        .map_err(store_error("save data source"))?;
    serde_json::to_value(data_source_from_stored(saved)?)
        .map_err(|error| format!("encode data source result failed: {error}"))
}

fn required_source<R: DataSourceReadRepository + ?Sized>(
    store: &R,
    id: &str,
) -> Result<DataSource, String> {
    store
        .data_source(id)
        .map_err(store_error("load data source"))?
        .ok_or_else(|| format!("unknown data source: {id}"))
        .and_then(data_source_from_stored)
}

fn require_revision(source: &DataSource, expected_revision: i64) -> Result<(), String> {
    if source.revision != expected_revision {
        return Err(format!(
            "data source {} revision conflict: expected {}, current {}",
            source.id, expected_revision, source.revision
        ));
    }
    Ok(())
}

fn source_is_validated(source: &DataSource) -> bool {
    matches!(
        source.validation.status,
        DataSourceValidationStatus::Verified | DataSourceValidationStatus::VerifiedEmpty
    ) && source
        .validation
        .contract
        .as_ref()
        .is_some_and(|contract| contract.is_structurally_usable(&source.id))
        && !matches!(
            source.authorization.state,
            DataSourceAuthorizationState::Denied
                | DataSourceAuthorizationState::Restricted
                | DataSourceAuthorizationState::Unavailable
        )
}

fn validate_content_reference(reference: &DataSourceContentReference) -> Result<(), String> {
    if reference.uri.trim().is_empty()
        || reference.mime_type.trim().is_empty()
        || reference.content_version.trim().is_empty()
    {
        return Err("data-source content reference is incomplete".into());
    }
    Ok(())
}

fn validate_read_observation(observed_at_ms: i64) -> Result<(), String> {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(observed_at_ms)
        .ok_or_else(|| format!("invalid data-source observation timestamp {observed_at_ms}"))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn failed_read_state(
    status: DataSourceReadStatus,
    diagnostic: impl AsRef<str>,
    requested_at_ms: Option<i64>,
    completed_at_ms: i64,
    content_reference: Option<DataSourceContentReference>,
    observed_at_ms: Option<i64>,
    schema_version: Option<u32>,
    record_count: Option<u64>,
) -> DataSourceReadState {
    DataSourceReadState {
        status,
        read_token: None,
        content_reference,
        observed_at_ms,
        schema_version,
        record_count,
        diagnostic: Some(bounded(diagnostic.as_ref().trim(), 2_000)),
        requested_at_ms,
        completed_at_ms: Some(completed_at_ms),
    }
}

fn resolve_context_source(
    source: DataSource,
    conversation_override: Option<bool>,
    now: chrono::DateTime<chrono::Utc>,
) -> DataSourceContextSelection {
    let enabled = conversation_override.unwrap_or(source.default_enabled);
    if !enabled {
        return context_selection(
            &source,
            false,
            false,
            false,
            if conversation_override == Some(false) {
                DataSourceSelectionReason::DisabledForConversation
            } else {
                DataSourceSelectionReason::DisabledByDefault
            },
            None,
        );
    }
    if source.lifecycle != DataSourceLifecycle::Ready {
        return context_selection(
            &source,
            true,
            false,
            false,
            DataSourceSelectionReason::LifecycleNotReady,
            None,
        );
    }
    if !matches!(
        source.authorization.state,
        DataSourceAuthorizationState::NotRequired | DataSourceAuthorizationState::Authorized
    ) {
        return context_selection(
            &source,
            true,
            false,
            false,
            DataSourceSelectionReason::AuthorizationUnavailable,
            None,
        );
    }
    if !source_is_validated(&source) {
        return context_selection(
            &source,
            true,
            false,
            false,
            DataSourceSelectionReason::ValidationUnavailable,
            None,
        );
    }
    // Validation describes the reader/schema, not the latest content snapshot.
    if matches!(
        source.read_state.status,
        DataSourceReadStatus::Ready | DataSourceReadStatus::Empty
    ) {
        let maximum_age_ms = source
            .freshness_policy
            .maximum_age_minutes
            .saturating_mul(60_000);
        let fresh = source.read_state.observed_at_ms.is_some_and(|observed| {
            let age = now.timestamp_millis().checked_sub(observed);
            age.is_some_and(|age| age >= 0 && (age as u64) <= maximum_age_ms)
        });
        if !fresh {
            return context_selection(
                &source,
                true,
                false,
                true,
                DataSourceSelectionReason::RefreshRequired,
                None,
            );
        }
    }
    match source.read_state.status {
        DataSourceReadStatus::Ready => context_selection(
            &source,
            true,
            source.read_state.content_reference.is_some(),
            false,
            DataSourceSelectionReason::Ready,
            source.read_state.content_reference.clone(),
        ),
        DataSourceReadStatus::Empty => context_selection(
            &source,
            true,
            true,
            false,
            DataSourceSelectionReason::ReadyEmpty,
            None,
        ),
        DataSourceReadStatus::Pending => context_selection(
            &source,
            true,
            false,
            false,
            DataSourceSelectionReason::ReadPending,
            None,
        ),
        DataSourceReadStatus::NeverRead
        | DataSourceReadStatus::Denied
        | DataSourceReadStatus::Unavailable
        | DataSourceReadStatus::Failed => context_selection(
            &source,
            true,
            false,
            source.read_state.status != DataSourceReadStatus::Denied,
            DataSourceSelectionReason::ReadUnavailable,
            None,
        ),
    }
}

fn context_selection(
    source: &DataSource,
    enabled: bool,
    usable: bool,
    should_refresh: bool,
    reason: DataSourceSelectionReason,
    content_reference: Option<DataSourceContentReference>,
) -> DataSourceContextSelection {
    DataSourceContextSelection {
        source_id: source.id.clone(),
        source_revision: source.revision,
        enabled,
        usable,
        should_refresh,
        reason,
        content_reference,
        observed_at_ms: source.read_state.observed_at_ms,
        schema_version: source.read_state.schema_version,
    }
}

fn validate_draft(input: &DataSourceDraft) -> Result<(), String> {
    validate_definition(
        &input.id,
        &input.name,
        &input.configuration,
        &input.freshness_policy,
        &input.capabilities,
        &input.privacy,
    )?;
    if input.created_at_ms <= 0 || input.updated_at_ms < input.created_at_ms {
        return Err("data source timestamps are invalid".into());
    }
    Ok(())
}

fn validate_definition(
    id: &str,
    name: &str,
    configuration: &DataSourceConfiguration,
    freshness: &DataSourceFreshnessPolicy,
    capabilities: &[String],
    privacy: &str,
) -> Result<(), String> {
    for (field, value) in [
        ("id", id),
        ("name", name),
        ("connector_id", configuration.connector_id.as_str()),
        ("location", configuration.location.as_str()),
        ("privacy", privacy),
    ] {
        if value.trim().is_empty() {
            return Err(format!("data source {field} is required"));
        }
    }
    if !(1..=10_080).contains(&freshness.maximum_age_minutes) {
        return Err("data source maximum age must be between 1 and 10080 minutes".into());
    }
    let normalized = capabilities
        .iter()
        .map(|value| value.trim())
        .collect::<BTreeSet<_>>();
    if capabilities.is_empty() || normalized.len() != capabilities.len() || normalized.contains("")
    {
        return Err("data source capabilities must be non-empty and unique".into());
    }
    let secret_references = configuration
        .secret_references
        .iter()
        .map(|value| value.trim())
        .collect::<BTreeSet<_>>();
    if secret_references.len() != configuration.secret_references.len()
        || secret_references.contains("")
    {
        return Err("data source secret references must be unique opaque identifiers".into());
    }
    Ok(())
}

fn stored_from_data_source(source: DataSource) -> Result<StoredDataSourceRecord, String> {
    let metadata = StoredDataSourceMetadata {
        version: DATA_SOURCE_METADATA_VERSION,
        configuration: source.configuration.clone(),
        authorization: source.authorization,
        freshness_policy: source.freshness_policy,
        lifecycle: source.lifecycle,
        read_state: source.read_state,
        last_successful_read_at_ms: source.last_successful_read_at_ms,
    };
    Ok(StoredDataSourceRecord {
        id: source.id,
        name: source.name,
        kind: encode_identifier(&source.kind)?,
        location: source.configuration.location,
        enabled: source.default_enabled,
        access_policy: encode_identifier(&source.access_policy)?,
        validation_contract_json: serde_json::to_string(&source.validation)
            .map_err(|error| format!("encode data source validation failed: {error}"))?,
        capabilities_json: serde_json::to_string(&source.capabilities)
            .map_err(|error| format!("encode data source capabilities failed: {error}"))?,
        privacy: source.privacy,
        metadata_json: serde_json::to_string(&metadata)
            .map_err(|error| format!("encode data source metadata failed: {error}"))?,
        revision: source.revision,
        created_at_ms: source.created_at_ms,
        updated_at_ms: source.updated_at_ms,
        deleted_at_ms: source.deleted_at_ms,
    })
}

fn data_source_from_stored(value: StoredDataSourceRecord) -> Result<DataSource, String> {
    let metadata: StoredDataSourceMetadata = serde_json::from_str(&value.metadata_json)
        .map_err(|error| format!("decode data source {} metadata failed: {error}", value.id))?;
    if metadata.version != DATA_SOURCE_METADATA_VERSION {
        return Err(format!(
            "data source {} metadata version {} is unsupported",
            value.id, metadata.version
        ));
    }
    if metadata.configuration.location != value.location {
        return Err(format!(
            "data source {} location projection diverged",
            value.id
        ));
    }
    Ok(DataSource {
        id: value.id,
        name: value.name,
        kind: decode_identifier(&value.kind)?,
        configuration: metadata.configuration,
        access_policy: decode_identifier(&value.access_policy)?,
        authorization: metadata.authorization,
        validation: serde_json::from_str(&value.validation_contract_json)
            .map_err(|error| format!("decode data source validation failed: {error}"))?,
        freshness_policy: metadata.freshness_policy,
        read_state: metadata.read_state,
        default_enabled: value.enabled,
        lifecycle: metadata.lifecycle,
        capabilities: serde_json::from_str(&value.capabilities_json)
            .map_err(|error| format!("decode data source capabilities failed: {error}"))?,
        privacy: value.privacy,
        revision: value.revision,
        last_successful_read_at_ms: metadata.last_successful_read_at_ms,
        created_at_ms: value.created_at_ms,
        updated_at_ms: value.updated_at_ms,
        deleted_at_ms: value.deleted_at_ms,
    })
}

fn encode_identifier<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_value(value)
        .map_err(|error| format!("encode data source identifier failed: {error}"))?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "data source identifier did not encode as text".into())
}

fn decode_identifier<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, String> {
    serde_json::from_value(Value::String(value.to_owned()))
        .map_err(|error| format!("decode data source identifier {value} failed: {error}"))
}

fn bounded(value: &str, maximum_chars: usize) -> String {
    value.chars().take(maximum_chars).collect()
}

fn store_error<E: std::fmt::Display>(operation: &'static str) -> impl FnOnce(E) -> String {
    move |error| format!("{operation} failed: {error}")
}

impl TryFrom<StoredDataSourceUsage> for DataSourceUsage {
    type Error = String;

    fn try_from(value: StoredDataSourceUsage) -> Result<Self, Self::Error> {
        Ok(Self {
            id: value.id,
            source_id: value.source_id,
            source_title: value.source_title,
            consumer: value.consumer,
            purpose: value.purpose,
            conversation_id: value.conversation_id,
            disclosed_to_cloud: value.disclosed_to_cloud,
            metadata: serde_json::from_str(&value.metadata_json)
                .map_err(|error| format!("decode data source usage metadata failed: {error}"))?,
            used_at_ms: value.used_at_ms,
        })
    }
}

impl From<DataSourceUsage> for StoredDataSourceUsage {
    fn from(value: DataSourceUsage) -> Self {
        Self {
            id: value.id,
            source_id: value.source_id,
            source_title: value.source_title,
            consumer: value.consumer,
            purpose: value.purpose,
            conversation_id: value.conversation_id,
            disclosed_to_cloud: value.disclosed_to_cloud,
            metadata_json: serde_json::to_string(&value.metadata).unwrap_or_else(|_| "{}".into()),
            used_at_ms: value.used_at_ms,
        }
    }
}
