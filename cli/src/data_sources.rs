// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Persist and validate CLI/Web host data sources before agent context injection.
// - Boundary: Host performs I/O; nola_core owns policy and freshness decisions.
// - Security: Contracts store only schema and redacted summaries, never source contents.

use chrono::Utc;
use nola_application::{
    ApplicationCommand, ConversationDataSourceOverride, DataSource, DataSourceAuthorization,
    DataSourceAuthorizationState, DataSourceCommand, DataSourceConfiguration,
    DataSourceContentReference, DataSourceFreshnessPolicy, DataSourceKind, DataSourceLocationKind,
    DataSourceReadResult, DataSourceUsage, DataSourceValidationResult,
    UpdateDataSourceConfiguration,
};
use nola_core::data_source::{
    resolve_data_source, DataSourceAccessPolicy, DataSourceContract, DataSourceReadMethod,
    DataSourceReadRecipe, DataSourceResolution, DataSourceResolutionInput, DataSourceUsageRecord,
    DataSourceValidationState, DATA_SOURCE_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_VALIDATION_BYTES: usize = 512 * 1024;
const MAX_CONTEXT_BYTES: usize = 256 * 1024;
const MAX_USAGE_RECORDS: usize = 200;
const CONNECTOR_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostDataSourceKind {
    Http,
    LocalFile,
}

impl HostDataSourceKind {
    fn as_str(self) -> &'static str {
        match self {
            HostDataSourceKind::Http => "http",
            HostDataSourceKind::LocalFile => "localFile",
        }
    }
}

impl Serialize for HostDataSourceKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for HostDataSourceKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "http" => Ok(Self::Http),
            "localFile" => Ok(Self::LocalFile),
            _ => Err(serde::de::Error::custom(format!(
                "decode data source enum {value} failed: unknown variant, expected `http` or `localFile`"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSourceDraft {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    pub kind: HostDataSourceKind,
    pub location: String,
    #[serde(default)]
    pub access_policy: DataSourceAccessPolicy,
    #[serde(default = "default_maximum_age_minutes")]
    pub maximum_age_minutes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredDataSource {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub kind: HostDataSourceKind,
    pub location: String,
    pub access_policy: DataSourceAccessPolicy,
    pub validation_contract: DataSourceContract,
    pub capabilities: Vec<String>,
    pub privacy: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSourceContextSnapshot {
    pub source_id: String,
    pub source_title: String,
    pub content: String,
    pub observed_at: String,
    pub authorization: String,
    pub maximum_age_minutes: u64,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct PersistedDataSourceState {
    schema_version: u32,
    sources: Vec<StoredDataSource>,
    conversation_overrides: BTreeMap<String, BTreeMap<String, bool>>,
    usage_records: Vec<DataSourceUsageRecord>,
}

impl Default for PersistedDataSourceState {
    fn default() -> Self {
        Self {
            schema_version: DATA_SOURCE_SCHEMA_VERSION,
            sources: Vec::new(),
            conversation_overrides: BTreeMap::new(),
            usage_records: Vec::new(),
        }
    }
}

pub struct DataSourceStore {
    path: PathBuf,
    state: PersistedDataSourceState,
}

impl DataSourceStore {
    pub fn open_default() -> Result<Self, String> {
        let path = crate::platform_paths::application_database_path()?;
        Self::open(path)
    }

    pub fn open(path: PathBuf) -> Result<Self, String> {
        let mut application = crate::application_port::CliApplicationPort::persistent(&path)?;
        let records: Vec<DataSource> = application.execute(ApplicationCommand::DataSource {
            command: DataSourceCommand::List,
        })?;
        let sources = records
            .iter()
            .filter_map(|source| match host_owned_data_source(source) {
                Ok(source) => source.map(Ok),
                Err(error) => Some(Err(error)),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let usage: Vec<DataSourceUsage> = application.execute(ApplicationCommand::DataSource {
            command: DataSourceCommand::ListUsage {
                limit: MAX_USAGE_RECORDS,
            },
        })?;
        let usage_records = usage
            .iter()
            .map(data_source_usage_record)
            .collect::<Result<Vec<_>, _>>()?;
        let state = PersistedDataSourceState {
            schema_version: DATA_SOURCE_SCHEMA_VERSION,
            sources,
            conversation_overrides: BTreeMap::new(),
            usage_records,
        };
        Ok(Self { path, state })
    }

    fn application_port(&self) -> Result<crate::application_port::CliApplicationPort, String> {
        crate::application_port::CliApplicationPort::persistent(&self.path)
    }

    pub fn sources(&self) -> &[StoredDataSource] {
        &self.state.sources
    }

    pub fn source(&self, source_id: &str) -> Option<&StoredDataSource> {
        self.state
            .sources
            .iter()
            .find(|source| source.id == source_id)
    }

    pub fn usage_records(&self) -> &[DataSourceUsageRecord] {
        &self.state.usage_records
    }

    pub fn validate_draft(&self, draft: &DataSourceDraft) -> Result<StoredDataSource, String> {
        let name = draft.name.trim();
        if name.is_empty() {
            return Err("data source name is required".to_string());
        }
        let location = draft.location.trim();
        if location.is_empty() {
            return Err("data source location is required".to_string());
        }
        validate_location_kind(draft.kind, location)?;

        let id = draft
            .id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(new_id);
        let existing = self.source(&id);
        let now = Utc::now();
        let payload = read_location(draft.kind, location, MAX_VALIDATION_BYTES)?;
        let analysis = analyze_payload(&payload.bytes, payload.content_type.as_deref());
        let read_method = match (draft.kind, payload.is_directory) {
            (HostDataSourceKind::Http, _) => DataSourceReadMethod::HttpGet,
            (HostDataSourceKind::LocalFile, true) => DataSourceReadMethod::DirectoryList,
            (HostDataSourceKind::LocalFile, false) => DataSourceReadMethod::LocalFileRead,
        };
        let operation_id = match read_method {
            DataSourceReadMethod::HttpGet => "cli.http.get",
            DataSourceReadMethod::DirectoryList => "cli.local-directory.list",
            DataSourceReadMethod::LocalFileRead => "cli.local-file.read",
            _ => unreachable!("CLI data source read method is exhaustive"),
        };
        let mut contract = DataSourceContract {
            source_id: id.clone(),
            connector_id: connector_id(draft.kind).to_string(),
            connector_version: CONNECTOR_VERSION,
            reader_description: reader_description(draft.kind, payload.is_directory),
            read_recipe: DataSourceReadRecipe {
                method: read_method,
                operation_id: operation_id.to_string(),
                configuration_keys: vec!["location".to_string(), "maximumBytes".to_string()],
                output_schema: analysis.output_schema,
            },
            schema_fields: analysis.schema_fields,
            redacted_sample: analysis.redacted_sample,
            record_count: analysis.record_count,
            maximum_age_minutes: draft.maximum_age_minutes.clamp(1, 10_080),
            validation_state: if analysis.record_count == 0 {
                DataSourceValidationState::VerifiedEmpty
            } else {
                DataSourceValidationState::Verified
            },
            validated_at: now,
            observed_at: Some(payload.observed_at),
            validation_digest: String::new(),
        };
        contract.seal_validation();
        let timestamp = now.to_rfc3339();
        Ok(StoredDataSource {
            schema_version: DATA_SOURCE_SCHEMA_VERSION,
            id,
            name: name.to_string(),
            kind: draft.kind,
            location: location.to_string(),
            access_policy: draft.access_policy,
            validation_contract: contract,
            capabilities: vec!["read".to_string(), "context".to_string()],
            privacy: "local-first; content is disclosed only when selected for a request"
                .to_string(),
            created_at: existing
                .map(|source| source.created_at.clone())
                .unwrap_or_else(|| timestamp.clone()),
            updated_at: timestamp,
        })
    }

    pub fn validate_and_save(
        &mut self,
        draft: &DataSourceDraft,
    ) -> Result<StoredDataSource, String> {
        let id = draft
            .id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(new_id);
        let mut host_draft = draft.clone();
        host_draft.id = Some(id.clone());
        let now_ms = Utc::now().timestamp_millis();
        let mut application = self.application_port()?;
        let existing: Option<DataSource> = application.execute(ApplicationCommand::DataSource {
            command: DataSourceCommand::Get { id: id.clone() },
        })?;
        let definition = canonical_definition(&host_draft, &id, now_ms);
        let draft_record: DataSource = if let Some(existing) = existing {
            application.execute(ApplicationCommand::DataSource {
                command: DataSourceCommand::UpdateConfiguration {
                    id: id.clone(),
                    expected_revision: existing.revision,
                    input: UpdateDataSourceConfiguration {
                        name: definition.name,
                        kind: definition.kind,
                        configuration: definition.configuration,
                        access_policy: definition.access_policy,
                        authorization: definition.authorization,
                        freshness_policy: definition.freshness_policy,
                        capabilities: definition.capabilities,
                        privacy: definition.privacy,
                        updated_at_ms: now_ms,
                    },
                },
            })?
        } else {
            application.execute(ApplicationCommand::DataSource {
                command: DataSourceCommand::CreateDraft { input: definition },
            })?
        };
        let pending: DataSource = application.execute(ApplicationCommand::DataSource {
            command: DataSourceCommand::RequestValidation {
                id: id.clone(),
                expected_revision: draft_record.revision,
                requested_at_ms: now_ms,
            },
        })?;
        let validation_token = pending.validation.validation_token.clone().ok_or_else(|| {
            "application did not issue a data source validation token".to_string()
        })?;
        let source = match self.validate_draft(&host_draft) {
            Ok(source) => source,
            Err(error) => {
                let _: DataSource = application.execute(ApplicationCommand::DataSource {
                    command: DataSourceCommand::RecordValidation {
                        id,
                        validation_token,
                        result: DataSourceValidationResult::Failed {
                            code: "host_validation_failed".to_string(),
                            diagnostic: error.clone(),
                        },
                        recorded_at_ms: Utc::now().timestamp_millis(),
                    },
                })?;
                return Err(error);
            }
        };
        let validated: DataSource = application.execute(ApplicationCommand::DataSource {
            command: DataSourceCommand::RecordValidation {
                id: id.clone(),
                validation_token,
                result: DataSourceValidationResult::Succeeded {
                    contract: source.validation_contract.clone(),
                },
                recorded_at_ms: Utc::now().timestamp_millis(),
            },
        })?;
        let persisted: DataSource = application.execute(ApplicationCommand::DataSource {
            command: DataSourceCommand::SetDefaultEnabled {
                id,
                expected_revision: validated.revision,
                enabled: true,
                updated_at_ms: Utc::now().timestamp_millis(),
            },
        })?;
        let source = stored_data_source(&persisted)?;
        if let Some(existing) = self
            .state
            .sources
            .iter_mut()
            .find(|existing| existing.id == source.id)
        {
            *existing = source.clone();
        } else {
            self.state.sources.push(source.clone());
        }
        self.state.sources.sort_by(|left, right| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(source)
    }

    pub fn refresh(&mut self, source_id: &str) -> Result<StoredDataSource, String> {
        let source = self
            .source(source_id)
            .cloned()
            .ok_or_else(|| format!("unknown data source: {source_id}"))?;
        self.validate_and_save(&DataSourceDraft {
            id: Some(source.id),
            name: source.name,
            kind: source.kind,
            location: source.location,
            access_policy: source.access_policy,
            maximum_age_minutes: source.validation_contract.maximum_age_minutes,
        })
    }

    pub fn remove(&mut self, source_id: &str) -> Result<bool, String> {
        if self.source(source_id).is_none() {
            return Ok(false);
        }
        let mut application = self.application_port()?;
        let current: Option<DataSource> = application.execute(ApplicationCommand::DataSource {
            command: DataSourceCommand::Get {
                id: source_id.to_string(),
            },
        })?;
        let removed: bool = application.execute(ApplicationCommand::DataSource {
            command: DataSourceCommand::Delete {
                id: source_id.to_string(),
                expected_revision: current
                    .ok_or_else(|| format!("unknown data source: {source_id}"))?
                    .revision,
                deleted_at_ms: Utc::now().timestamp_millis(),
            },
        })?;
        if removed {
            self.state.sources.retain(|source| source.id != source_id);
        }
        Ok(removed)
    }

    pub fn set_conversation_override(
        &mut self,
        conversation_id: &str,
        source_id: &str,
        enabled: Option<bool>,
    ) -> Result<(), String> {
        if self.source(source_id).is_none() {
            return Err(format!("unknown data source: {source_id}"));
        }
        let conversation_id = conversation_id.trim();
        if conversation_id.is_empty() {
            return Err("conversation id is required".to_string());
        }
        let mut application = self.application_port()?;
        let _: Value = application.execute(ApplicationCommand::DataSource {
            command: DataSourceCommand::SetConversationOverride {
                conversation_id: conversation_id.to_string(),
                source_id: source_id.to_string(),
                value: match enabled {
                    Some(true) => ConversationDataSourceOverride::Enabled,
                    Some(false) => ConversationDataSourceOverride::Disabled,
                    None => ConversationDataSourceOverride::Inherit,
                },
                updated_at_ms: Utc::now().timestamp_millis(),
            },
        })?;
        Ok(())
    }

    pub fn conversation_override(&self, conversation_id: &str, source_id: &str) -> Option<bool> {
        self.application_port()
            .ok()
            .and_then(|mut application| {
                application
                    .execute::<ConversationDataSourceOverride>(ApplicationCommand::DataSource {
                        command: DataSourceCommand::GetConversationOverride {
                            conversation_id: conversation_id.to_string(),
                            source_id: source_id.to_string(),
                        },
                    })
                    .ok()
            })
            .and_then(|value| match value {
                ConversationDataSourceOverride::Inherit => None,
                ConversationDataSourceOverride::Enabled => Some(true),
                ConversationDataSourceOverride::Disabled => Some(false),
            })
    }

    pub fn resolve(
        &self,
        source: &StoredDataSource,
        conversation_id: &str,
    ) -> DataSourceResolution {
        resolve_data_source(&DataSourceResolutionInput {
            schema_version: DATA_SOURCE_SCHEMA_VERSION,
            source_id: source.id.clone(),
            policy: source.access_policy,
            conversation_override: self.conversation_override(conversation_id, &source.id),
            contract: Some(source.validation_contract.clone()),
            now: Some(Utc::now()),
        })
    }

    pub fn resolved_sources(
        &self,
        conversation_id: &str,
    ) -> Vec<(&StoredDataSource, DataSourceResolution)> {
        self.state
            .sources
            .iter()
            .map(|source| (source, self.resolve(source, conversation_id)))
            .collect()
    }

    pub fn read_context_snapshot(
        &self,
        source_id: &str,
        conversation_id: &str,
    ) -> Result<DataSourceContextSnapshot, String> {
        let mut application = self.application_port()?;
        let canonical: DataSource = application
            .execute::<Option<DataSource>>(ApplicationCommand::DataSource {
                command: DataSourceCommand::Get {
                    id: source_id.to_string(),
                },
            })?
            .ok_or_else(|| format!("unknown canonical data source: {source_id}"))?;
        let selections: Vec<nola_application::DataSourceContextSelection> =
            application.execute(ApplicationCommand::DataSource {
                command: DataSourceCommand::ResolveForConversation {
                    conversation_id: conversation_id.to_string(),
                    now_ms: Utc::now().timestamp_millis(),
                },
            })?;
        if !selections.iter().any(|selection| {
            selection.source_id == source_id
                && selection.source_revision == canonical.revision
                && selection.enabled
                && (selection.usable || selection.should_refresh)
        }) {
            return Err(
                "application did not admit the data source for this conversation".to_string(),
            );
        }
        let source = stored_data_source(&canonical)?;
        let pending: DataSource = application.execute(ApplicationCommand::DataSource {
            command: DataSourceCommand::RequestRefresh {
                id: source_id.to_string(),
                expected_revision: canonical.revision,
                reason: "conversation_context".to_string(),
                requested_at_ms: Utc::now().timestamp_millis(),
            },
        })?;
        let read_token = pending
            .read_state
            .read_token
            .clone()
            .ok_or_else(|| "application did not issue a data-source read token".to_string())?;
        let payload = match read_location(source.kind, &source.location, MAX_CONTEXT_BYTES) {
            Ok(payload) => payload,
            Err(error) => {
                let _: DataSource = application.execute(ApplicationCommand::DataSource {
                    command: DataSourceCommand::RecordRead {
                        id: source_id.to_string(),
                        read_token,
                        result: DataSourceReadResult::Failed {
                            code: "host_read_failed".to_string(),
                            diagnostic: error.clone(),
                        },
                        recorded_at_ms: Utc::now().timestamp_millis(),
                    },
                })?;
                return Err(error);
            }
        };
        let analysis = analyze_payload(&payload.bytes, payload.content_type.as_deref());
        let completed: DataSource = if payload.bytes.is_empty() {
            application.execute(ApplicationCommand::DataSource {
                command: DataSourceCommand::RecordRead {
                    id: source_id.to_string(),
                    read_token,
                    result: DataSourceReadResult::Empty {
                        observed_at_ms: payload.observed_at.timestamp_millis(),
                        schema_version: DATA_SOURCE_SCHEMA_VERSION,
                    },
                    recorded_at_ms: Utc::now().timestamp_millis(),
                },
            })?
        } else {
            let content_version = new_id();
            let content_root = self
                .path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("DataSourceContent")
                .join(
                    source_id
                        .replace('/', "_")
                        .replace('\\', "_")
                        .replace(':', "_"),
                );
            let content_path = content_root.join(format!("{content_version}.bin"));
            let persist_result = fs::create_dir_all(&content_root)
                .map_err(|error| format!("create data-source content directory failed: {error}"))
                .and_then(|_| {
                    fs::write(&content_path, &payload.bytes)
                        .map_err(|error| format!("persist data-source content failed: {error}"))
                });
            if let Err(error) = persist_result {
                let _: DataSource = application.execute(ApplicationCommand::DataSource {
                    command: DataSourceCommand::RecordRead {
                        id: source_id.to_string(),
                        read_token,
                        result: DataSourceReadResult::Failed {
                            code: "content_persistence_failed".to_string(),
                            diagnostic: error.clone(),
                        },
                        recorded_at_ms: Utc::now().timestamp_millis(),
                    },
                })?;
                return Err(error);
            }
            application.execute(ApplicationCommand::DataSource {
                command: DataSourceCommand::RecordRead {
                    id: source_id.to_string(),
                    read_token,
                    result: DataSourceReadResult::Succeeded {
                        content_reference: DataSourceContentReference {
                            uri: format!("file://{}", content_path.display()),
                            mime_type: payload
                                .content_type
                                .clone()
                                .unwrap_or_else(|| "application/octet-stream".to_string()),
                            byte_count: payload.bytes.len() as u64,
                            content_version,
                        },
                        observed_at_ms: payload.observed_at.timestamp_millis(),
                        schema_version: DATA_SOURCE_SCHEMA_VERSION,
                        record_count: analysis.record_count,
                    },
                    recorded_at_ms: Utc::now().timestamp_millis(),
                },
            })?
        };
        let selections: Vec<nola_application::DataSourceContextSelection> =
            application.execute(ApplicationCommand::DataSource {
                command: DataSourceCommand::ResolveForConversation {
                    conversation_id: conversation_id.to_string(),
                    now_ms: Utc::now().timestamp_millis(),
                },
            })?;
        if !selections.iter().any(|selection| {
            selection.source_id == source_id
                && selection.source_revision == completed.revision
                && selection.enabled
                && selection.usable
        }) {
            return Err("data source is no longer admitted for this conversation".to_string());
        }
        let observed_at = completed
            .read_state
            .observed_at_ms
            .and_then(chrono::DateTime::<Utc>::from_timestamp_millis)
            .ok_or_else(|| {
                "application returned no valid data-source observation time".to_string()
            })?;
        let authorization = serde_json::to_value(completed.authorization.state)
            .map_err(|error| format!("encode data-source authorization failed: {error}"))?
            .as_str()
            .ok_or_else(|| "application returned invalid data-source authorization".to_string())?
            .to_string();
        Ok(DataSourceContextSnapshot {
            source_id: source.id.clone(),
            source_title: source.name.clone(),
            content: String::from_utf8_lossy(&payload.bytes).into_owned(),
            observed_at: observed_at.to_rfc3339(),
            authorization,
            maximum_age_minutes: completed.freshness_policy.maximum_age_minutes,
            truncated: payload.truncated,
        })
    }

    pub fn record_usage(
        &mut self,
        source_id: &str,
        consumer: &str,
        purpose: &str,
        conversation_id: Option<&str>,
        disclosed_to_cloud: bool,
        metadata: BTreeMap<String, Value>,
    ) -> Result<DataSourceUsageRecord, String> {
        let source = self
            .source(source_id)
            .ok_or_else(|| format!("unknown data source: {source_id}"))?;
        let record = DataSourceUsageRecord {
            id: new_id(),
            source_id: source.id.clone(),
            source_title: source.name.clone(),
            consumer: consumer.trim().to_string(),
            purpose: purpose.trim().to_string(),
            conversation_id: conversation_id.map(ToOwned::to_owned),
            used_at: Utc::now(),
            disclosed_to_cloud: Some(disclosed_to_cloud),
            metadata,
        };
        self.state.usage_records.insert(0, record.clone());
        self.state.usage_records.truncate(MAX_USAGE_RECORDS);
        let mut application = self.application_port()?;
        let _: bool = application.execute(ApplicationCommand::DataSource {
            command: DataSourceCommand::RecordUsage {
                input: stored_data_source_usage(&record)?,
            },
        })?;
        Ok(record)
    }
}

fn canonical_definition(
    draft: &DataSourceDraft,
    id: &str,
    now_ms: i64,
) -> nola_application::DataSourceDraft {
    let (kind, location_kind, connector_id) = match draft.kind {
        HostDataSourceKind::Http => (
            DataSourceKind::Http,
            DataSourceLocationKind::HttpUrl,
            "cli.http",
        ),
        HostDataSourceKind::LocalFile => (
            DataSourceKind::LocalFile,
            DataSourceLocationKind::LocalPath,
            "cli.local-file",
        ),
    };
    nola_application::DataSourceDraft {
        id: id.to_string(),
        name: draft.name.trim().to_string(),
        kind,
        configuration: DataSourceConfiguration {
            connector_id: connector_id.to_string(),
            location_kind,
            location: draft.location.trim().to_string(),
            parameters: BTreeMap::from([(
                "maximumBytes".to_string(),
                Value::from(MAX_CONTEXT_BYTES as u64),
            )]),
            secret_references: Vec::new(),
        },
        access_policy: draft.access_policy,
        authorization: DataSourceAuthorization {
            state: DataSourceAuthorizationState::NotRequired,
            capability: None,
            observed_at_ms: now_ms,
        },
        freshness_policy: DataSourceFreshnessPolicy {
            maximum_age_minutes: draft.maximum_age_minutes.clamp(1, 10_080),
            allow_stale_fallback: false,
        },
        capabilities: vec!["read".to_string(), "context".to_string()],
        privacy: "local-first; content is disclosed only when selected for a request".to_string(),
        created_at_ms: now_ms,
        updated_at_ms: now_ms,
    }
}

fn stored_data_source(source: &DataSource) -> Result<StoredDataSource, String> {
    let kind = match source.kind {
        DataSourceKind::Http => HostDataSourceKind::Http,
        DataSourceKind::LocalFile | DataSourceKind::Directory => HostDataSourceKind::LocalFile,
        _ => return Err(format!("data source {} belongs to another Host", source.id)),
    };
    let validation_contract =
        source
            .validation
            .contract
            .clone()
            .unwrap_or_else(|| DataSourceContract {
                source_id: source.id.clone(),
                connector_id: source.configuration.connector_id.clone(),
                connector_version: 1,
                reader_description: "Validation required".to_string(),
                read_recipe: DataSourceReadRecipe::default(),
                schema_fields: Vec::new(),
                redacted_sample: String::new(),
                record_count: 0,
                maximum_age_minutes: source.freshness_policy.maximum_age_minutes,
                validation_state: DataSourceValidationState::Failed,
                validated_at: Utc::now(),
                observed_at: None,
                validation_digest: String::new(),
            });
    Ok(StoredDataSource {
        schema_version: DATA_SOURCE_SCHEMA_VERSION,
        id: source.id.clone(),
        name: source.name.clone(),
        kind,
        location: source.configuration.location.clone(),
        access_policy: source.access_policy,
        validation_contract,
        capabilities: source.capabilities.clone(),
        privacy: source.privacy.clone(),
        created_at: timestamp_string(source.created_at_ms),
        updated_at: timestamp_string(source.updated_at_ms),
    })
}

/// Projects only records whose connector is owned by this Host.
///
/// The canonical Store is shared by native clients and Agent Hosts. A macOS
/// platform connector or a native Web connector is a valid record, but the
/// CLI cannot execute its read contract. Foreign records remain durable in
/// the Store without entering this Host's context projection.
fn host_owned_data_source(source: &DataSource) -> Result<Option<StoredDataSource>, String> {
    match source.kind {
        DataSourceKind::Http | DataSourceKind::LocalFile | DataSourceKind::Directory => {
            stored_data_source(source).map(Some)
        }
        _ => Ok(None),
    }
}

fn stored_data_source_usage(record: &DataSourceUsageRecord) -> Result<DataSourceUsage, String> {
    Ok(DataSourceUsage {
        id: record.id.clone(),
        source_id: record.source_id.clone(),
        source_title: record.source_title.clone(),
        consumer: record.consumer.clone(),
        purpose: record.purpose.clone(),
        conversation_id: record.conversation_id.clone(),
        disclosed_to_cloud: record.disclosed_to_cloud,
        metadata: record.metadata.clone(),
        used_at_ms: record.used_at.timestamp_millis(),
    })
}

fn data_source_usage_record(record: &DataSourceUsage) -> Result<DataSourceUsageRecord, String> {
    Ok(DataSourceUsageRecord {
        id: record.id.clone(),
        source_id: record.source_id.clone(),
        source_title: record.source_title.clone(),
        consumer: record.consumer.clone(),
        purpose: record.purpose.clone(),
        conversation_id: record.conversation_id.clone(),
        used_at: chrono::DateTime::from_timestamp_millis(record.used_at_ms)
            .ok_or_else(|| format!("invalid data source usage timestamp {}", record.used_at_ms))?,
        disclosed_to_cloud: record.disclosed_to_cloud,
        metadata: record.metadata.clone(),
    })
}

fn timestamp_string(value: i64) -> String {
    chrono::DateTime::<Utc>::from_timestamp_millis(value)
        .unwrap_or_else(Utc::now)
        .to_rfc3339()
}

struct ReadPayload {
    bytes: Vec<u8>,
    content_type: Option<String>,
    observed_at: chrono::DateTime<Utc>,
    truncated: bool,
    is_directory: bool,
}

struct PayloadAnalysis {
    schema_fields: Vec<String>,
    output_schema: Value,
    redacted_sample: String,
    record_count: u64,
}

fn read_location(
    kind: HostDataSourceKind,
    location: &str,
    maximum_bytes: usize,
) -> Result<ReadPayload, String> {
    match kind {
        HostDataSourceKind::LocalFile => read_local(location, maximum_bytes),
        HostDataSourceKind::Http => read_http(location, maximum_bytes),
    }
}

fn read_local(location: &str, maximum_bytes: usize) -> Result<ReadPayload, String> {
    let path = local_path(location);
    // Private host credentials are not user data sources, even when an explicit
    // source is configured or a symlink hides the private directory name.
    let is_private = |path: &Path| path.components().any(|part| {
        part.as_os_str().to_str().is_some_and(|name| name.eq_ignore_ascii_case(".nola-host-secrets"))
    });
    if is_private(&path) {
        return Err("private host credential storage cannot be used as a data source".into());
    }
    let canonical = fs::canonicalize(&path)
        .map_err(|_| "cannot resolve local data source path".to_string())?;
    if is_private(&canonical) {
        return Err("private host credential storage cannot be used as a data source".into());
    }
    let path = canonical;
    let metadata = fs::metadata(&path)
        .map_err(|error| format!("cannot access {}: {error}", path.display()))?;
    let observed_at = metadata
        .modified()
        .ok()
        .map(chrono::DateTime::<Utc>::from)
        .unwrap_or_else(Utc::now);
    if metadata.is_dir() {
        let mut entries = fs::read_dir(&path)
            .map_err(|error| format!("cannot read directory {}: {error}", path.display()))?
            .filter_map(Result::ok)
            .map(|entry| {
                let kind = entry
                    .file_type()
                    .ok()
                    .map(|value| if value.is_dir() { "directory" } else { "file" })
                    .unwrap_or("unknown");
                json!({"name": entry.file_name().to_string_lossy(), "kind": kind})
            })
            .take(500)
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
        let bytes = serde_json::to_vec(&entries)
            .map_err(|error| format!("cannot encode directory snapshot: {error}"))?;
        let truncated = bytes.len() > maximum_bytes;
        return Ok(ReadPayload {
            bytes: bytes.into_iter().take(maximum_bytes).collect(),
            content_type: Some("application/json".to_string()),
            observed_at,
            truncated,
            is_directory: true,
        });
    }
    if !metadata.is_file() {
        return Err(format!("unsupported local source: {}", path.display()));
    }
    let mut file = fs::File::open(&path)
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    let (bytes, truncated) = read_bounded(&mut file, maximum_bytes)?;
    Ok(ReadPayload {
        bytes,
        content_type: content_type_for_path(&path).map(ToOwned::to_owned),
        observed_at,
        truncated,
        is_directory: false,
    })
}

fn read_http(location: &str, maximum_bytes: usize) -> Result<ReadPayload, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|error| format!("cannot create HTTP reader: {error}"))?;
    let mut response = client
        .get(location)
        .header(reqwest::header::USER_AGENT, "Nola/1 DataSourceValidator")
        .send()
        .map_err(|error| format!("HTTP validation failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("HTTP validation returned {}", response.status()));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let (bytes, truncated) = read_bounded(&mut response, maximum_bytes)?;
    Ok(ReadPayload {
        bytes,
        content_type,
        observed_at: Utc::now(),
        truncated,
        is_directory: false,
    })
}

fn read_bounded(reader: &mut impl Read, maximum_bytes: usize) -> Result<(Vec<u8>, bool), String> {
    let mut bytes = Vec::with_capacity(maximum_bytes.min(64 * 1024));
    reader
        .take((maximum_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read data source: {error}"))?;
    let truncated = bytes.len() > maximum_bytes;
    bytes.truncate(maximum_bytes);
    Ok((bytes, truncated))
}

fn analyze_payload(bytes: &[u8], content_type: Option<&str>) -> PayloadAnalysis {
    if let Ok(value) = serde_json::from_slice::<Value>(bytes) {
        return analyze_json(&value);
    }
    let content_type = content_type.unwrap_or("").to_ascii_lowercase();
    if content_type.contains("csv") {
        let text = String::from_utf8_lossy(bytes);
        let mut lines = text.lines().filter(|line| !line.trim().is_empty());
        let fields = lines
            .next()
            .map(|header| {
                header
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .take(64)
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .filter(|fields| !fields.is_empty())
            .unwrap_or_else(|| vec!["row".to_string()]);
        let count = lines.count() as u64;
        let properties = fields
            .iter()
            .map(|field| (field.clone(), json!({"type": "string"})))
            .collect::<serde_json::Map<String, Value>>();
        return PayloadAnalysis {
            redacted_sample: format!("CSV · {count} data rows · {} columns", fields.len()),
            output_schema: json!({
                "type": "array",
                "items": {"type": "object", "properties": properties}
            }),
            schema_fields: fields,
            record_count: count,
        };
    }
    let text = String::from_utf8_lossy(bytes);
    let nonempty_lines = text.lines().filter(|line| !line.trim().is_empty()).count() as u64;
    let (schema_fields, kind) = if content_type.contains("html")
        || text
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("<!doctype html")
        || text.trim_start().to_ascii_lowercase().starts_with("<html")
    {
        (vec!["html".to_string()], "HTML document")
    } else {
        (vec!["content".to_string()], "Text document")
    };
    PayloadAnalysis {
        schema_fields,
        output_schema: json!({"type": "string"}),
        redacted_sample: format!(
            "{kind} · {nonempty_lines} non-empty lines · {} bytes",
            bytes.len()
        ),
        record_count: u64::from(!bytes.is_empty()),
    }
}

fn analyze_json(value: &Value) -> PayloadAnalysis {
    let mut fields = BTreeSet::new();
    let record_count = match value {
        Value::Array(values) => {
            for value in values.iter().take(100) {
                if let Value::Object(object) = value {
                    fields.extend(object.keys().cloned());
                }
            }
            values.len() as u64
        }
        Value::Object(object) => {
            fields.extend(object.keys().cloned());
            1
        }
        Value::Null => 0,
        _ => 1,
    };
    if fields.is_empty() {
        fields.insert("value".to_string());
    }
    let schema_fields = fields.into_iter().take(64).collect::<Vec<_>>();
    PayloadAnalysis {
        redacted_sample: format!(
            "JSON · {record_count} records · {} fields (values redacted)",
            schema_fields.len()
        ),
        schema_fields,
        output_schema: json_schema_for_value(value, 0),
        record_count,
    }
}

fn json_schema_for_value(value: &Value, depth: usize) -> Value {
    if depth >= 3 {
        return json!({});
    }
    match value {
        Value::Null => json!({"type": "null"}),
        Value::Bool(_) => json!({"type": "boolean"}),
        Value::Number(number) if number.is_i64() || number.is_u64() => {
            json!({"type": "integer"})
        }
        Value::Number(_) => json!({"type": "number"}),
        Value::String(_) => json!({"type": "string"}),
        Value::Array(values) => json!({
            "type": "array",
            "items": values
                .first()
                .map(|value| json_schema_for_value(value, depth + 1))
                .unwrap_or_else(|| json!({}))
        }),
        Value::Object(object) => {
            let properties = object
                .iter()
                .take(64)
                .map(|(key, value)| (key.clone(), json_schema_for_value(value, depth + 1)))
                .collect::<serde_json::Map<String, Value>>();
            let required = properties.keys().cloned().collect::<Vec<_>>();
            json!({
                "type": "object",
                "properties": properties,
                "required": required
            })
        }
    }
}

fn validate_location_kind(kind: HostDataSourceKind, location: &str) -> Result<(), String> {
    match kind {
        HostDataSourceKind::Http => {
            if location.starts_with("https://") || location.starts_with("http://") {
                Ok(())
            } else {
                Err("HTTP data source must use http:// or https://".to_string())
            }
        }
        HostDataSourceKind::LocalFile => {
            if location.starts_with("http://") || location.starts_with("https://") {
                Err("local data source cannot use an HTTP URL".to_string())
            } else {
                Ok(())
            }
        }
    }
}

fn connector_id(kind: HostDataSourceKind) -> &'static str {
    match kind {
        HostDataSourceKind::Http => "cli.http",
        HostDataSourceKind::LocalFile => "cli.local-file",
    }
}

fn reader_description(kind: HostDataSourceKind, is_directory: bool) -> String {
    match (kind, is_directory) {
        (HostDataSourceKind::Http, _) => "HTTPS GET with bounded response and content-type parsing",
        (HostDataSourceKind::LocalFile, true) => "Bounded local directory listing",
        (HostDataSourceKind::LocalFile, false) => "Bounded local file read",
    }
    .to_string()
}

fn local_path(location: &str) -> PathBuf {
    let value = location.strip_prefix("file://").unwrap_or(location);
    if value == "~" {
        return home_dir().unwrap_or_else(|| PathBuf::from(value));
    }
    if let Some(relative) = value.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(relative);
        }
    }
    PathBuf::from(value)
}

fn content_type_for_path(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "json" => Some("application/json"),
        "csv" => Some("text/csv"),
        "html" | "htm" => Some("text/html"),
        "md" | "txt" | "log" | "yaml" | "yml" | "toml" => Some("text/plain"),
        _ => None,
    }
}

fn home_dir() -> Option<PathBuf> {
    crate::platform_paths::home_dir()
}

fn default_maximum_age_minutes() -> u64 {
    60
}

fn new_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("ds-{}-{nanos:x}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_core::data_source::DataSourceResolutionReason;
    use std::io::Write;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn private_host_vault_cannot_be_read_or_validated_as_a_source() {
        let directory = tempfile::tempdir().unwrap();
        let vault = directory.path().join(".nola-host-secrets");
        fs::create_dir(&vault).unwrap();
        let secret = vault.join("fixture.txt");
        fs::write(&secret, "never-read-credential-fixture").unwrap();
        for path in [vault.clone(), secret.clone(), vault.join("../.nola-host-secrets/fixture.txt"), directory.path().join(".NOLA-HOST-SECRETS/fixture.txt")] {
            assert!(matches!(read_local(path.to_str().unwrap(), 1024), Err(error) if error.contains("private host credential storage")));
        }
        #[cfg(unix)] {
            let alias = directory.path().join("ordinary-source.txt");
            std::os::unix::fs::symlink(&secret, &alias).unwrap();
            assert!(read_local(alias.to_str().unwrap(), 1024).is_err());
            let folder = directory.path().join("ordinary-folder");
            std::os::unix::fs::symlink(&vault, &folder).unwrap();
            assert!(read_local(folder.join("fixture.txt").to_str().unwrap(), 1024).is_err());
        }
        let public = directory.path().join("public.txt");
        fs::write(&public, "public fixture").unwrap();
        assert!(read_local(public.to_str().unwrap(), 1024).is_ok());
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("nola-{name}-{}", new_id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn draft(location: &Path, policy: DataSourceAccessPolicy) -> DataSourceDraft {
        DataSourceDraft {
            id: Some("source-1".to_string()),
            name: "Test source".to_string(),
            kind: HostDataSourceKind::LocalFile,
            location: location.display().to_string(),
            access_policy: policy,
            maximum_age_minutes: 60,
        }
    }

    fn create_conversation_fixture(store: &DataSourceStore, conversation_id: &str) {
        let mut application = store.application_port().unwrap();
        application
            .create_conversation(nola_application::CreateConversation {
                id: conversation_id.to_string(),
                title: "Data source fixture".to_string(),
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
    }

    #[test]
    fn real_file_validation_persists_contract_without_secret_values() {
        let directory = TestDirectory::new("data-source-file");
        let source_path = directory.0.join("source.json");
        fs::write(
            &source_path,
            r#"[{"account":"alice","api_key":"secret-123"}]"#,
        )
        .unwrap();
        let state_path = directory.0.join("state.json");
        let mut store = DataSourceStore::open(state_path.clone()).unwrap();
        let source = store
            .validate_and_save(&draft(&source_path, DataSourceAccessPolicy::DefaultEnabled))
            .unwrap();

        assert_eq!(source.validation_contract.record_count, 1);
        assert_eq!(
            source.validation_contract.schema_fields,
            vec!["account".to_string(), "api_key".to_string()]
        );
        assert!(!source
            .validation_contract
            .redacted_sample
            .contains("secret-123"));
        assert_eq!(
            source.validation_contract.read_recipe.method,
            DataSourceReadMethod::LocalFileRead
        );
        assert_eq!(
            source.validation_contract.read_recipe.operation_id,
            "cli.local-file.read"
        );
        assert!(source
            .validation_contract
            .read_recipe
            .output_schema
            .is_object());
        assert_eq!(
            source.validation_contract.validation_digest,
            source.validation_contract.expected_validation_digest()
        );
        let canonical = nola_store::NolaStore::open(state_path).unwrap();
        let persisted = canonical.list_data_sources().unwrap();
        assert_eq!(persisted.len(), 1);
        assert!(!persisted[0].validation_contract_json.contains("secret-123"));
        assert!(!persisted[0].validation_contract_json.contains("alice"));
    }

    #[test]
    fn shared_store_projects_only_host_owned_sources_without_mutating_foreign_records() {
        let directory = TestDirectory::new("data-source-host-boundary");
        let source_path = directory.0.join("source.txt");
        fs::write(&source_path, "host-owned source").unwrap();
        let state_path = directory.0.join("state.json");
        let mut store = DataSourceStore::open(state_path.clone()).unwrap();
        let source = store
            .validate_and_save(&draft(&source_path, DataSourceAccessPolicy::DefaultEnabled))
            .unwrap();

        let mut canonical = nola_store::NolaStore::open(&state_path).unwrap();
        for (id, kind, location_kind, location) in [
            (
                "platform:health",
                DataSourceKind::Platform,
                DataSourceLocationKind::PlatformResource,
                "platform:health",
            ),
            (
                "web:native-fixture",
                DataSourceKind::Web,
                DataSourceLocationKind::HttpUrl,
                "https://example.test/context",
            ),
        ] {
            nola_application::execute_application_command(
                &mut canonical,
                ApplicationCommand::DataSource {
                    command: DataSourceCommand::CreateDraft {
                        input: nola_application::DataSourceDraft {
                            id: id.to_string(),
                            name: id.to_string(),
                            kind,
                            configuration: DataSourceConfiguration {
                                connector_id: "foreign.host".to_string(),
                                location_kind,
                                location: location.to_string(),
                                parameters: BTreeMap::new(),
                                secret_references: Vec::new(),
                            },
                            access_policy: DataSourceAccessPolicy::ConversationOptIn,
                            authorization: DataSourceAuthorization {
                                state: DataSourceAuthorizationState::NotDetermined,
                                capability: Some(id.to_string()),
                                observed_at_ms: 1,
                            },
                            freshness_policy: DataSourceFreshnessPolicy {
                                maximum_age_minutes: 60,
                                allow_stale_fallback: false,
                            },
                            capabilities: vec!["read".to_string(), "context".to_string()],
                            privacy: "local".to_string(),
                            created_at_ms: 1,
                            updated_at_ms: 1,
                        },
                    },
                },
            )
            .unwrap();
        }
        drop(canonical);

        let reopened = DataSourceStore::open(state_path.clone()).unwrap();
        assert_eq!(reopened.sources().len(), 1);
        assert_eq!(reopened.sources()[0].id, source.id);

        let mut canonical = nola_store::NolaStore::open(state_path).unwrap();
        let records: Vec<DataSource> = serde_json::from_value(
            nola_application::execute_application_command(
                &mut canonical,
                ApplicationCommand::DataSource {
                    command: DataSourceCommand::List,
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(records.len(), 3);
        assert!(records.iter().any(|record| {
            record.id == "platform:health"
                && record.kind == nola_application::DataSourceKind::Platform
        }));
        assert!(records.iter().any(|record| {
            record.id == "web:native-fixture"
                && record.kind == nola_application::DataSourceKind::Web
        }));
    }

    #[test]
    fn core_policy_resolution_requires_explicit_opt_in_and_honors_override() {
        let directory = TestDirectory::new("data-source-policy");
        let source_path = directory.0.join("source.txt");
        fs::write(&source_path, "context").unwrap();
        let mut store = DataSourceStore::open(directory.0.join("state.json")).unwrap();
        let source = store
            .validate_and_save(&draft(
                &source_path,
                DataSourceAccessPolicy::ConversationOptIn,
            ))
            .unwrap();

        let before = store.resolve(&source, "conversation-1");
        assert_eq!(
            before.reason,
            DataSourceResolutionReason::ConversationOptInRequired
        );
        let error = store
            .set_conversation_override("conversation-1", &source.id, Some(true))
            .unwrap_err();
        assert!(
            error.contains("unknown conversation conversation-1"),
            "{error}"
        );
        assert_eq!(
            store.conversation_override("conversation-1", &source.id),
            None
        );
        create_conversation_fixture(&store, "conversation-1");
        store
            .set_conversation_override("conversation-1", &source.id, Some(true))
            .unwrap();
        let after = store.resolve(&source, "conversation-1");
        assert!(after.usable);
        assert_eq!(after.reason, DataSourceResolutionReason::Ready);
    }

    #[test]
    fn state_round_trip_preserves_overrides_usage_and_context_read() {
        let directory = TestDirectory::new("data-source-state");
        let source_path = directory.0.join("source.txt");
        fs::write(&source_path, "fresh context").unwrap();
        let state_path = directory.0.join("state.json");
        let mut store = DataSourceStore::open(state_path.clone()).unwrap();
        let source = store
            .validate_and_save(&draft(&source_path, DataSourceAccessPolicy::DefaultEnabled))
            .unwrap();
        create_conversation_fixture(&store, "conversation-1");
        store
            .set_conversation_override("conversation-1", &source.id, Some(true))
            .unwrap();
        store
            .record_usage(
                &source.id,
                "chat",
                "conversation_context",
                Some("conversation-1"),
                true,
                BTreeMap::new(),
            )
            .unwrap();

        let reopened = DataSourceStore::open(state_path).unwrap();
        assert_eq!(
            reopened.conversation_override("conversation-1", &source.id),
            Some(true)
        );
        assert_eq!(reopened.usage_records().len(), 1);
        assert_eq!(reopened.usage_records()[0].disclosed_to_cloud, Some(true));
        let snapshot = reopened
            .read_context_snapshot(&source.id, "conversation-1")
            .unwrap();
        assert_eq!(snapshot.content, "fresh context");
        assert_eq!(snapshot.authorization, "not_required");
        let mut application = reopened.application_port().unwrap();
        let canonical: Option<DataSource> = application
            .execute(ApplicationCommand::DataSource {
                command: DataSourceCommand::Get {
                    id: source.id.clone(),
                },
            })
            .unwrap();
        let canonical = canonical.unwrap();
        assert_eq!(
            snapshot.maximum_age_minutes,
            canonical.freshness_policy.maximum_age_minutes
        );
        assert_eq!(
            canonical.validation.contract.as_ref(),
            Some(&source.validation_contract)
        );
        assert_eq!(
            chrono::DateTime::parse_from_rfc3339(&snapshot.observed_at)
                .unwrap()
                .timestamp_millis(),
            canonical.read_state.observed_at_ms.unwrap(),
        );
    }

    #[test]
    fn stale_projection_cannot_read_a_canonically_disabled_source() {
        let directory = TestDirectory::new("source-disabled-stale-projection");
        let source_path = directory.0.join("source.txt");
        fs::write(&source_path, "must not be read").unwrap();
        let mut store = DataSourceStore::open(directory.0.join("state.json")).unwrap();
        let source = store
            .validate_and_save(&draft(&source_path, DataSourceAccessPolicy::DefaultEnabled))
            .unwrap();
        let mut application = store.application_port().unwrap();
        let canonical: Option<DataSource> = application
            .execute(ApplicationCommand::DataSource {
                command: DataSourceCommand::Get {
                    id: source.id.clone(),
                },
            })
            .unwrap();
        let disabled: DataSource = application
            .execute(ApplicationCommand::DataSource {
                command: DataSourceCommand::SetDefaultEnabled {
                    id: source.id.clone(),
                    expected_revision: canonical.unwrap().revision,
                    enabled: false,
                    updated_at_ms: Utc::now().timestamp_millis(),
                },
            })
            .unwrap();
        fs::remove_file(&source_path).unwrap();
        let error = store
            .read_context_snapshot(&source.id, "conversation-1")
            .unwrap_err();
        assert!(error.contains("did not admit"), "{error}");
        let after: Option<DataSource> = application
            .execute(ApplicationCommand::DataSource {
                command: DataSourceCommand::Get { id: source.id },
            })
            .unwrap();
        assert_eq!(after.unwrap().revision, disabled.revision);
    }

    #[test]
    fn conversation_revoked_during_http_read_does_not_return_context() {
        let directory = TestDirectory::new("source-revoked-during-http");
        let state_path = directory.0.join("state.json");
        let server_state_path = state_path.clone();
        let mut store = DataSourceStore::open(state_path).unwrap();
        create_conversation_fixture(&store, "conversation-1");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for request_index in 0..2 {
                let deadline = std::time::Instant::now() + Duration::from_secs(20);
                let (mut stream, _) = loop {
                    match listener.accept() {
                        Ok(connection) => break connection,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                std::time::Instant::now() < deadline,
                                "HTTP request timed out"
                            );
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) => panic!("accept failed: {error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                    assert!(request.len() < 16384);
                }
                if request_index == 1 {
                    let mut other = DataSourceStore::open(server_state_path.clone()).unwrap();
                    let mut application = other.application_port().unwrap();
                    let pending: Option<DataSource> = application
                        .execute(ApplicationCommand::DataSource {
                            command: DataSourceCommand::Get {
                                id: "source-1".to_string(),
                            },
                        })
                        .unwrap();
                    assert!(pending.unwrap().read_state.read_token.is_some());
                    other
                        .set_conversation_override("conversation-1", "source-1", Some(false))
                        .unwrap();
                }
                let body = r#"{"signal":"revoked-context-must-not-escape"}"#;
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            }
        });
        let source = store
            .validate_and_save(&DataSourceDraft {
                id: Some("source-1".to_string()),
                name: "Revocation fixture".to_string(),
                kind: HostDataSourceKind::Http,
                location: format!("http://{address}/source"),
                access_policy: DataSourceAccessPolicy::DefaultEnabled,
                maximum_age_minutes: 60,
            })
            .unwrap();
        let result = store.read_context_snapshot(&source.id, "conversation-1");
        server.join().unwrap();
        let error = result.unwrap_err();
        assert!(error.contains("no longer admitted"), "{error}");
        let mut application = store.application_port().unwrap();
        let completed: Option<DataSource> = application
            .execute(ApplicationCommand::DataSource {
                command: DataSourceCommand::Get { id: source.id },
            })
            .unwrap();
        let completed = completed.unwrap();
        assert!(completed.read_state.content_reference.is_some());
        assert!(completed.read_state.observed_at_ms.is_some());
    }

    #[test]
    fn http_validation_performs_real_request_and_redacts_values() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            let body = r#"[{"headline":"private headline","url":"https://example.com"}]"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        let directory = TestDirectory::new("data-source-http");
        let store = DataSourceStore::open(directory.0.join("state.json")).unwrap();
        let source = store
            .validate_draft(&DataSourceDraft {
                id: Some("http-1".to_string()),
                name: "News feed".to_string(),
                kind: HostDataSourceKind::Http,
                location: format!("http://{address}/feed"),
                access_policy: DataSourceAccessPolicy::AskWhenRelevant,
                maximum_age_minutes: 15,
            })
            .unwrap();
        server.join().unwrap();

        assert_eq!(source.validation_contract.connector_id, "cli.http");
        assert_eq!(source.validation_contract.record_count, 1);
        assert_eq!(
            source.validation_contract.schema_fields,
            vec!["headline".to_string(), "url".to_string()]
        );
        assert!(!source
            .validation_contract
            .redacted_sample
            .contains("private headline"));
    }
}
