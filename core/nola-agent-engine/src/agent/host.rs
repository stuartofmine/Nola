// SPDX-License-Identifier: Apache-2.0

pub use nola_host_protocol::{PermissionInstruction, PermissionTransportInterruptionReason};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use unicode_segmentation::UnicodeSegmentation;

pub use nola_host_protocol::{
    validate_host_artifacts, DelegatedAgentContextMode, DelegatedAgentContextV1,
    DelegatedAgentEventV1, DelegatedAgentRequestV1, EffectDeliveryLease, HostArtifact,
    HostCapabilityActivation, HostEffectId, HostError, HostId, HostLease,
    HostPermissionAuthorizationDecision, HostPermissionIntervention, HostPermissionResolution,
    HostRequestBatchIdentity, HostToolOutcome, ModelDeltaChannel, ModelResponseMode,
    ToolProgressChannel, DELEGATED_AGENT_CONTRACT_VERSION,
};

pub type HostRequest = nola_host_protocol::HostRequest<
    super::permissions::PermissionEvaluation,
    super::capability::RecoverableCapabilityError,
    crate::lifecycle_hooks::LifecycleHookEvent,
>;
pub type HostResponse = nola_host_protocol::HostResponse<
    super::protocol::ModelProviderContinuationState,
    super::task::TaskKind,
    super::task::TaskState,
    super::task::TaskLineage,
    super::capability::CapabilityCatalogRefresh,
    crate::lifecycle_hooks::LifecycleHookExecutionProjection,
>;
pub type HostRequestEnvelope = nola_host_protocol::HostRequestEnvelope<HostRequest>;
pub type HostResponseEnvelope = nola_host_protocol::HostResponseEnvelope<HostResponse>;

impl nola_host_protocol::HostTaskState for super::task::TaskState {
    fn is_terminal(&self) -> bool {
        (*self).is_terminal()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostCapabilityCatalog {
    pub host_id: HostId,
    pub revision: u64,
    pub availability_epoch: u64,
    /// `None` is reserved for an in-process Host whose lifetime is identical
    /// to the Core caller. Remote and child-process Hosts must publish a lease.
    #[serde(default)]
    pub lease: Option<HostLease>,
    pub capabilities: BTreeMap<String, HostCapabilityDescriptor>,
}

impl HostCapabilityCatalog {
    pub fn empty(host_id: impl Into<String>) -> Self {
        Self {
            host_id: HostId(host_id.into()),
            revision: 0,
            availability_epoch: 0,
            lease: None,
            capabilities: BTreeMap::new(),
        }
    }

    pub fn resolve_available(&self, capability_id: &str) -> Option<&HostCapabilityDescriptor> {
        self.capabilities
            .get(capability_id)
            .filter(|capability| capability.availability == CapabilityAvailability::Available)
    }

    /// Searches the exact Host-advertised deferred tool catalog. This is a
    /// catalog operation, not user-intent routing: callers pass the model's
    /// structured `tool_search` query unchanged and Core ranks only declared
    /// capability metadata and the top-level JSON Schema description.
    pub fn search_available_tools(
        &self,
        query: &str,
        max_results: usize,
    ) -> Vec<HostCapabilitySearchMatch> {
        let limit = max_results.clamp(1, 32);
        let normalized_query = normalize_search_text(query);
        let query_terms = search_terms(&normalized_query);
        let documents = self
            .capabilities
            .values()
            .filter(|capability| {
                capability.availability == CapabilityAvailability::Available
                    && capability.route == HostCapabilityRoute::Tool
                    && capability.exposure.is_model_discoverable()
            })
            .map(|capability| {
                let normalized_id = normalize_search_text(&capability.id);
                let normalized_title = normalize_search_text(&capability.title);
                let normalized_metadata =
                    normalize_search_text(&capability_search_text(capability));
                let terms = search_terms(&normalized_metadata);
                let title_terms = search_terms(&format!("{normalized_id} {normalized_title}"));
                (capability, normalized_id, terms, title_terms)
            })
            .collect::<Vec<_>>();
        // Document frequency comes only from the currently discoverable catalog.
        // Repeated prose cannot inflate relevance, and a ubiquitous word cannot
        // outrank a rare schema term merely because it occurs in a tool title.
        let weights = query_terms
            .iter()
            .map(|term| {
                let frequency = documents
                    .iter()
                    .filter(|(_, _, terms, _)| terms.contains(term))
                    .count();
                let weight = 100
                    + 1_000 * documents.len().saturating_sub(frequency) as u64
                        / documents.len().max(1) as u64;
                (term, weight)
            })
            .collect::<Vec<_>>();
        let mut matches = documents
            .into_iter()
            .filter_map(|(capability, id, terms, title_terms)| {
                let score = if normalized_query.is_empty() {
                    1
                } else if id == normalized_query {
                    10_000
                } else {
                    weights
                        .iter()
                        .filter(|(term, _)| terms.contains(*term))
                        .map(|(term, weight)| {
                            weight
                                + if title_terms.contains(*term) {
                                    weight / 4
                                } else {
                                    0
                                }
                        })
                        .sum::<u64>()
                        .min(9_999)
                };
                (normalized_query.is_empty() || score > 0).then(|| HostCapabilitySearchMatch {
                    capability_id: capability.id.clone(),
                    score,
                })
            })
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.capability_id.cmp(&right.capability_id))
        });
        matches.truncate(limit);
        matches
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostCapabilitySearchMatch {
    pub capability_id: String,
    pub score: u64,
}

fn capability_search_text(capability: &HostCapabilityDescriptor) -> String {
    let mut parts = vec![capability.id.clone(), capability.title.clone()];
    // Discovery ranks what a capability does, not incidental parameter names
    // such as a process memory limit. Nested fields remain in the callable schema.
    if let Some(description) = capability
        .input_schema
        .get("description")
        .and_then(serde_json::Value::as_str)
    {
        parts.push(description.to_string());
    }
    parts.join(" ")
}

fn normalize_search_text(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .map(|character| {
            if character.is_alphanumeric() || character == '_' {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn search_terms(value: &str) -> BTreeSet<String> {
    // Unicode word boundaries support unspaced scripts without a language
    // keyword list. Underscores split callable identifiers into searchable words.
    value
        .split('_')
        .flat_map(UnicodeSegmentation::unicode_words)
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod capability_search_tests {
    use super::*;
    use serde_json::json;

    fn descriptor(id: &str, title: &str, description: &str) -> HostCapabilityDescriptor {
        HostCapabilityDescriptor {
            id: id.to_string(),
            title: title.to_string(),
            route: HostCapabilityRoute::Tool,
            availability: CapabilityAvailability::Available,
            exposure: HostCapabilityExposure::default(),
            permission_domain: None,
            input_schema: json!({
                "type": "object",
                "description": description,
                "properties": {
                    "query": { "type": "string" }
                }
            }),
            input_normalization: HostInputNormalization::default(),
            output_policy: HostOutputPolicy {
                preview_char_limit: 1_000,
                persist_full_output: true,
                allow_artifacts: true,
            },
            supports_streaming: false,
            supports_argument_streaming: false,
            parallelism: HostCapabilityParallelism::default(),
            cancellation: HostCancellationMode::default(),
            replay_safety: HostReplaySafety::default(),
        }
    }

    #[test]
    fn short_words_match_words_not_substrings_or_identifier_fragments() {
        let mut catalog = HostCapabilityCatalog::empty("host");
        catalog.capabilities.insert(
            "calendar".into(),
            descriptor("calendar", "Calendar", "Schedule events"),
        );
        catalog.capabilities.insert(
            "reader".into(),
            descriptor("reader", "Reader", "Read a document"),
        );
        let matches = catalog.search_available_tools("a", 5);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].capability_id, "reader");
        assert!(catalog.search_available_tools("cal", 5).is_empty());
    }

    #[test]
    fn rare_metadata_beats_common_title_words_and_repetition_does_not_change_rank() {
        let mut catalog = HostCapabilityCatalog::empty("host");
        for index in 0..12 {
            let id = format!("generic_{index}");
            catalog.capabilities.insert(
                id.clone(),
                descriptor(&id, "Inspect current data", "Inspect current data items"),
            );
        }
        catalog.capabilities.insert(
            "specialist".into(),
            descriptor(
                "specialist",
                "Document reader",
                "Inspect current data with crystallography measurements",
            ),
        );
        let matches = catalog.search_available_tools("inspect current crystallography", 1);
        assert_eq!(matches[0].capability_id, "specialist");
        assert_eq!(
            catalog.search_available_tools(
                "inspect inspect current crystallography crystallography",
                1
            ),
            matches
        );
    }

    #[test]
    fn unicode_boundaries_and_underscored_ids_share_the_same_term_index() {
        assert!(search_terms("résumé archive_read").contains("résumé"));
        assert!(search_terms("résumé archive_read").contains("read"));
        assert!(!search_terms("résumé archive_read").contains("a"));
    }

    #[test]
    fn parameter_names_do_not_advertise_unrelated_capabilities() {
        let mut catalog = HostCapabilityCatalog::empty("host");
        let mut shell = descriptor("bash", "Run command", "Execute a shell command");
        shell.input_schema["properties"] =
            serde_json::json!({"max_memory_bytes": {"type": "integer"}});
        catalog.capabilities.insert("bash".into(), shell);
        catalog.capabilities.insert(
            "memory_save".into(),
            descriptor("memory_save", "Save memory", "Persist long-term memory"),
        );
        let matches = catalog.search_available_tools("memory", 5);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].capability_id, "memory_save");
    }

    #[test]
    fn searches_declared_schema_metadata_without_language_routing_rules() {
        let mut catalog = HostCapabilityCatalog::empty("host");
        catalog.capabilities.insert(
            "web_search".to_string(),
            descriptor("web_search", "联网检索", "检索最新网页与新闻来源"),
        );
        catalog.capabilities.insert(
            "read_file".to_string(),
            descriptor("read_file", "Read file", "Read local text content"),
        );

        let matches = catalog.search_available_tools("最新 新闻", 5);
        assert_eq!(matches[0].capability_id, "web_search");
        assert!(matches[0].score > 0);
    }

    #[test]
    fn multilingual_follow_up_is_ranked_from_declared_catalog_metadata() {
        let mut catalog = HostCapabilityCatalog::empty("host");
        catalog.capabilities.insert(
            "calendar_create".to_string(),
            descriptor(
                "calendar_create",
                "创建日历安排",
                "继续处理日历事件，设置时间与提醒",
            ),
        );
        catalog.capabilities.insert(
            "read_file".to_string(),
            descriptor("read_file", "读取文件", "查看本地源代码内容"),
        );

        let matches = catalog.search_available_tools("继续处理日历", 5);
        assert_eq!(matches[0].capability_id, "calendar_create");
    }

    #[test]
    fn ambiguous_search_and_current_words_do_not_force_a_web_capability() {
        let mut catalog = HostCapabilityCatalog::empty("host");
        catalog.capabilities.insert(
            "implementation_search".to_string(),
            descriptor(
                "implementation_search",
                "Search current implementation",
                "Find symbols in the active source workspace",
            ),
        );
        catalog.capabilities.insert(
            "web_search".to_string(),
            descriptor(
                "web_search",
                "External retrieval",
                "Retrieve cited public internet sources",
            ),
        );

        let matches = catalog.search_available_tools("search current implementation", 5);
        assert_eq!(matches[0].capability_id, "implementation_search");
        assert!(matches[0].score > matches[1].score);
    }

    #[test]
    fn exact_id_wins_and_result_limit_is_enforced() {
        let mut catalog = HostCapabilityCatalog::empty("host");
        catalog.capabilities.insert(
            "read_file".to_string(),
            descriptor("read_file", "Read file", "Read content"),
        );
        catalog.capabilities.insert(
            "read_many_files".to_string(),
            descriptor("read_many_files", "Read files", "Read content in batches"),
        );

        let matches = catalog.search_available_tools("read_file", 1);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].capability_id, "read_file");
        assert_eq!(matches[0].score, 10_000);
    }

    #[test]
    fn deferred_tools_are_discoverable_but_internal_tools_are_not() {
        let mut catalog = HostCapabilityCatalog::empty("host");
        let mut deferred = descriptor("calendar_create", "Create calendar event", "calendar");
        deferred.exposure = HostCapabilityExposure::Deferred;
        catalog.capabilities.insert(deferred.id.clone(), deferred);
        let mut internal = descriptor("calendar_secret", "Internal calendar bridge", "calendar");
        internal.exposure = HostCapabilityExposure::Internal;
        catalog.capabilities.insert(internal.id.clone(), internal);

        let matches = catalog.search_available_tools("calendar", 10);
        assert_eq!(
            matches
                .iter()
                .map(|entry| entry.capability_id.as_str())
                .collect::<Vec<_>>(),
            vec!["calendar_create"]
        );
    }

    #[test]
    fn specific_directory_capability_beats_generic_path_metadata() {
        let mut catalog = HostCapabilityCatalog::empty("host");
        catalog.capabilities.insert(
            "file_tree".to_string(),
            descriptor(
                "file_tree",
                "Browse directory tree",
                "List files under a directory root path",
            ),
        );
        catalog.capabilities.insert(
            "verification_review".to_string(),
            descriptor(
                "verification_review",
                "Verification review",
                "Review files and paths in the current project directory",
            ),
        );

        let matches = catalog.search_available_tools("find desktop directory path home", 5);
        assert_eq!(matches[0].capability_id, "file_tree");
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostCapabilityDescriptor {
    pub id: String,
    pub title: String,
    pub route: HostCapabilityRoute,
    pub availability: CapabilityAvailability,
    #[serde(default)]
    pub exposure: HostCapabilityExposure,
    pub permission_domain: Option<String>,
    pub input_schema: serde_json::Value,
    /// Host-owned, declarative input normalization applied after Core has
    /// validated the accepted input shape and before the implementation runs.
    /// This metadata is never exposed as part of the model's JSON schema.
    #[serde(default)]
    pub input_normalization: HostInputNormalization,
    pub output_policy: HostOutputPolicy,
    pub supports_streaming: bool,
    #[serde(default)]
    pub supports_argument_streaming: bool,
    #[serde(default)]
    pub parallelism: HostCapabilityParallelism,
    #[serde(default)]
    pub cancellation: HostCancellationMode,
    #[serde(default)]
    pub replay_safety: HostReplaySafety,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HostInputNormalization {
    /// Host-selected Code Mode ceiling. None means this capability does not
    /// start an isolate; scripts/model arguments cannot supply this authority.
    pub code_mode_limits: Option<nola_domain::code_mode::CodeCellLimits>,
    /// Reconstructs an owner-bound lazy MCP route after a cold restart. It
    /// grants neither a connection nor invocation authority and holds no secrets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_route: Option<HostMcpToolRoute>,
    /// Host-owned target semantics consumed by Core's task constraints. This
    /// never selects a tool or grants permission from natural language.
    pub execution_target: Option<nola_domain::execution_constraints::ExecutionTargetProjection>,
    /// Permission semantics are independent of engineering-loop operation_kind.
    #[serde(default)]
    pub permission_contract: nola_domain::permission_resource_plan::PermissionOperationContract,
    /// Host-declared provenance for presentation and audit. This is outside
    /// the model-visible input schema and must never be inferred from names.
    #[serde(default)]
    pub origin: HostCapabilityOrigin,
    /// Versioned Host-owned declaration of how a successful invocation
    /// affects the engineering loop. Core resolves this only after input
    /// schema validation; it never infers effects from command text.
    #[serde(default)]
    pub effect: HostCapabilityEffectDeclaration,
    /// Declares which durable resource owns a non-read-only effect. The Host
    /// uses this boundary to choose recovery machinery; only workspace-owned
    /// mutations may create Git or file-preimage checkpoints.
    #[serde(default)]
    pub transaction_boundary: HostCapabilityTransactionBoundary,
    /// Declares that a successful mutation opens an independent validation
    /// obligation. Core combines this flag with the resolved typed effect; it
    /// never guesses from command text, file names, or output formatting.
    #[serde(default, skip_serializing_if = "is_false")]
    pub requires_validation_after_mutation: bool,
    /// Host-declared projection from validated invocation input to the stable
    /// permission resource shown to the user and persisted in Core rules.
    /// This metadata is not model-visible and never derives from user text.
    #[serde(default)]
    pub permission_resource: HostPermissionResourceDeclaration,
    #[serde(default)]
    pub root_relative_patterns: Vec<HostRootRelativePatternNormalization>,
    #[serde(default)]
    pub integer_maximums: BTreeMap<String, u64>,
    /// Optional per-Turn invocation budget shared by related capabilities.
    /// Core evaluates this only from validated tool arguments and frozen Host
    /// metadata; it never infers a budget from user text or capability names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_budget: Option<HostCapabilityTurnBudget>,
    /// Declares how an investigative call demonstrates progress. Generic Core
    /// never guesses resource or selector semantics from argument names.
    #[serde(default, skip_serializing_if = "HostLoopProgressDeclaration::is_empty")]
    pub loop_progress: HostLoopProgressDeclaration,
    /// Declares how validated input maps to durable resources and whether a
    /// mutation is preview-only. This is loop/accounting metadata, not a
    /// permission grant; Core never infers these meanings from field names.
    #[serde(
        default,
        skip_serializing_if = "HostResourceProjectionDeclaration::is_empty"
    )]
    pub resource_projection: HostResourceProjectionDeclaration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostMcpToolRoute {
    pub server_id: String,
    pub tool_name: String,
    pub connection_ref: String,
    pub config_digest: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostLoopProgressDeclaration {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constant_resource: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selector_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub range_start_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub range_extent_keys: Vec<String>,
}

impl HostLoopProgressDeclaration {
    fn is_empty(&self) -> bool {
        self.resource_keys.is_empty()
            && self.constant_resource.is_none()
            && self.selector_keys.is_empty()
            && self.range_start_keys.is_empty()
            && self.range_extent_keys.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostResourceProjectionDeclaration {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_collection_keys: Vec<String>,
    /// When one of these top-level keys is present, its explicit scope replaces
    /// the ordinary resource projection for this call.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preferred_scope_keys: Vec<String>,
    /// Separate declared write and verification scopes; never union them as proof.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mutation_scope_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation_scope_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_key: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ephemeral_when_true_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub durable_key: Option<String>,
}

impl HostResourceProjectionDeclaration {
    fn is_empty(&self) -> bool {
        self.resource_keys.is_empty()
            && self.resource_collection_keys.is_empty()
            && self.preferred_scope_keys.is_empty()
            && self.mutation_scope_keys.is_empty()
            && self.validation_scope_keys.is_empty()
            && self.base_key.is_none()
            && self.ephemeral_when_true_keys.is_empty()
            && self.durable_key.is_none()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostCapabilityTransactionBoundary {
    /// The effect can change files below the active workspace root.
    #[default]
    Workspace,
    /// The effect changes state owned by the Nola Host, outside the workspace.
    HostState,
    /// The effect is committed by an external capability provider.
    External,
}

impl HostCapabilityTransactionBoundary {
    pub fn requires_workspace_checkpoint(self) -> bool {
        self == Self::Workspace
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostCapabilityTurnBudget {
    pub group: String,
    pub standard_max_calls: u32,
    /// When true, exhausting this group ends executable planning and moves the
    /// Turn into Core's tool-free evidence synthesis step.
    #[serde(default)]
    pub finalize_on_exhaustion: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expansion: Option<HostCapabilityTurnBudgetExpansion>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostCapabilityTurnBudgetExpansion {
    pub argument: String,
    pub equals: serde_json::Value,
    pub max_calls: u32,
}

impl HostCapabilityTurnBudget {
    pub fn limit_for(&self, arguments: &serde_json::Value) -> u32 {
        self.expansion
            .as_ref()
            .filter(|expansion| arguments.get(&expansion.argument) == Some(&expansion.equals))
            .map(|expansion| expansion.max_calls)
            .unwrap_or(self.standard_max_calls)
            .max(1)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostPermissionResourceDeclaration {
    Constant {
        value: String,
    },
    FirstOf {
        keys: Vec<String>,
    },
    Composite {
        keys: Vec<String>,
    },
    #[default]
    FullInput,
}

impl HostPermissionResourceDeclaration {
    fn resolve(&self, input: &serde_json::Value) -> String {
        let full_input = || {
            super::permissions::permission_resource_for_input(
                "__host_declared_full_input__",
                None,
                input,
            )
        };
        match self {
            Self::Constant { value } if !value.trim().is_empty() => value.clone(),
            Self::Constant { .. } => full_input(),
            Self::FirstOf { keys } => {
                let selected = input.as_object().and_then(|object| {
                    keys.iter().find_map(|key| {
                        let value = object.get(key)?;
                        match value {
                            serde_json::Value::String(value) if !value.trim().is_empty() => {
                                Some(value.clone())
                            }
                            serde_json::Value::Null => None,
                            value => serde_json::to_string(value).ok(),
                        }
                    })
                });
                selected.unwrap_or_else(full_input)
            }
            Self::Composite { keys } => {
                let selected = input
                    .as_object()
                    .and_then(|object| {
                        keys.iter()
                            .map(|key| {
                                object
                                    .get(key)
                                    .filter(|value| !value.is_null())
                                    .map(|value| (key.clone(), value.clone()))
                            })
                            .collect::<Option<BTreeMap<_, _>>>()
                    })
                    .unwrap_or_default();
                if selected.is_empty() {
                    full_input()
                } else {
                    serde_json::to_string(&selected).unwrap_or_else(|_| full_input())
                }
            }
            Self::FullInput => full_input(),
        }
    }
}

pub use nola_domain::{HostCapabilityOrigin, HostCapabilityOriginKind};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostCapabilityEffect {
    Investigate,
    Mutate,
    MutateAndValidate,
    Validate,
    #[default]
    Execute,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostCapabilityEffectDeclaration {
    #[serde(default = "default_effect_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub fixed: HostCapabilityEffect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument: Option<String>,
    #[serde(default)]
    pub variants: BTreeMap<String, HostCapabilityEffect>,
}

impl Default for HostCapabilityEffectDeclaration {
    fn default() -> Self {
        Self::fixed(HostCapabilityEffect::Execute)
    }
}

impl HostCapabilityEffectDeclaration {
    pub fn fixed(effect: HostCapabilityEffect) -> Self {
        Self {
            schema_version: default_effect_schema_version(),
            fixed: effect,
            argument: None,
            variants: BTreeMap::new(),
        }
    }

    pub fn by_argument(
        argument: impl Into<String>,
        fallback: HostCapabilityEffect,
        variants: impl IntoIterator<Item = (String, HostCapabilityEffect)>,
    ) -> Self {
        Self {
            schema_version: default_effect_schema_version(),
            fixed: fallback,
            argument: Some(argument.into()),
            variants: variants.into_iter().collect(),
        }
    }

    pub fn resolve(&self, arguments: &serde_json::Value) -> HostCapabilityEffect {
        if self.schema_version != default_effect_schema_version() {
            return HostCapabilityEffect::Execute;
        }
        self.argument
            .as_deref()
            .and_then(|argument| arguments.get(argument))
            .and_then(|value| match value {
                serde_json::Value::String(value) => Some(value.clone()),
                serde_json::Value::Bool(value) => Some(value.to_string()),
                serde_json::Value::Number(value) => Some(value.to_string()),
                _ => None,
            })
            .and_then(|variant| self.variants.get(&variant))
            .copied()
            .unwrap_or(self.fixed)
    }

    pub fn supports_non_investigative_effect(&self) -> bool {
        self.fixed != HostCapabilityEffect::Investigate
            || self
                .variants
                .values()
                .any(|effect| *effect != HostCapabilityEffect::Investigate)
    }

    pub fn supports_validation_effect(&self) -> bool {
        self.fixed == HostCapabilityEffect::Validate
            || self
                .variants
                .values()
                .any(|effect| *effect == HostCapabilityEffect::Validate)
    }
}

#[cfg(test)]
mod effect_declaration_tests {
    use super::{
        CapabilityAvailability, HostCancellationMode, HostCapabilityDescriptor,
        HostCapabilityEffect, HostCapabilityEffectDeclaration, HostCapabilityExposure,
        HostCapabilityParallelism, HostCapabilityRoute, HostInputNormalization, HostOutputPolicy,
        HostReplaySafety,
    };
    use serde_json::json;

    #[test]
    fn stage_exit_distinguishes_read_only_and_progress_capabilities() {
        let read_only = HostCapabilityEffectDeclaration::fixed(HostCapabilityEffect::Investigate);
        let writer = HostCapabilityEffectDeclaration::fixed(HostCapabilityEffect::Mutate);
        let mixed = HostCapabilityEffectDeclaration::by_argument(
            "operation",
            HostCapabilityEffect::Investigate,
            [("write".to_string(), HostCapabilityEffect::Mutate)],
        );

        assert!(!read_only.supports_non_investigative_effect());
        assert!(writer.supports_non_investigative_effect());
        assert!(mixed.supports_non_investigative_effect());
    }

    #[test]
    fn validation_projection_recognizes_fixed_and_argument_driven_validators() {
        let validator = HostCapabilityEffectDeclaration::fixed(HostCapabilityEffect::Validate);
        let mutate_and_validate =
            HostCapabilityEffectDeclaration::fixed(HostCapabilityEffect::MutateAndValidate);
        let mixed = HostCapabilityEffectDeclaration::by_argument(
            "operation",
            HostCapabilityEffect::Investigate,
            [("verify".to_string(), HostCapabilityEffect::Validate)],
        );
        let reader = HostCapabilityEffectDeclaration::fixed(HostCapabilityEffect::Investigate);

        assert!(validator.supports_validation_effect());
        assert!(!mutate_and_validate.supports_validation_effect());
        assert!(mixed.supports_validation_effect());
        assert!(!reader.supports_validation_effect());
    }

    #[test]
    fn mutation_validation_comes_only_from_the_host_declaration() {
        let mut descriptor = HostCapabilityDescriptor {
            id: "process".to_string(),
            title: "Process".to_string(),
            route: HostCapabilityRoute::Tool,
            availability: CapabilityAvailability::Available,
            exposure: HostCapabilityExposure::Visible,
            permission_domain: None,
            input_schema: json!({"type":"object"}),
            input_normalization: HostInputNormalization {
                effect: HostCapabilityEffectDeclaration::by_argument(
                    "operation_kind",
                    HostCapabilityEffect::Execute,
                    [
                        ("mutate".to_string(), HostCapabilityEffect::Mutate),
                        ("validate".to_string(), HostCapabilityEffect::Validate),
                    ],
                ),
                requires_validation_after_mutation: true,
                ..HostInputNormalization::default()
            },
            output_policy: HostOutputPolicy {
                preview_char_limit: 1_000,
                persist_full_output: true,
                allow_artifacts: true,
            },
            supports_streaming: false,
            supports_argument_streaming: false,
            parallelism: HostCapabilityParallelism::Serial,
            cancellation: HostCancellationMode::Cooperative,
            replay_safety: HostReplaySafety::Recoverable,
        };

        let misleading = json!({"command":"contains arbitrary test and delete words"});
        assert_eq!(
            descriptor.effect_for(&misleading),
            HostCapabilityEffect::Execute
        );
        assert!(!descriptor.mutation_requires_validation_for(&misleading));

        let mutation = json!({"command":"opaque", "operation_kind":"mutate"});
        assert_eq!(
            descriptor.effect_for(&mutation),
            HostCapabilityEffect::Mutate
        );
        assert!(descriptor.mutation_requires_validation_for(&mutation));

        descriptor
            .input_normalization
            .requires_validation_after_mutation = false;
        assert!(!descriptor.mutation_requires_validation_for(&mutation));
    }
}

fn default_effect_schema_version() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostRootRelativePatternNormalization {
    pub root: String,
    #[serde(default)]
    pub patterns: Vec<String>,
}

impl HostCapabilityDescriptor {
    pub fn effect_for(&self, arguments: &serde_json::Value) -> HostCapabilityEffect {
        self.input_normalization.effect.resolve(arguments)
    }

    pub fn mutation_requires_validation_for(&self, arguments: &serde_json::Value) -> bool {
        self.effect_for(arguments) == HostCapabilityEffect::Mutate
            && self.input_normalization.requires_validation_after_mutation
    }

    /// Projects a model-visible function definition from the Host catalog.
    /// The session configuration decides which capabilities are enabled; the
    /// catalog remains authoritative for the accepted input schema.
    pub fn model_tool_definition(&self, configured: &serde_json::Value) -> serde_json::Value {
        let mut definition = configured.clone();
        let model_input_schema = if self.id == "request_user_input" {
            self.input_schema.clone()
        } else {
            model_input_schema_with_execution_context(&self.input_schema)
        };
        let model_description = self
            .input_schema
            .get("description")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&self.title)
            .to_string();
        if let Some(function) = definition
            .get_mut("function")
            .and_then(serde_json::Value::as_object_mut)
        {
            function.insert(
                "name".to_string(),
                serde_json::Value::String(self.id.clone()),
            );
            function.insert("parameters".to_string(), model_input_schema.clone());
            function
                .entry("description".to_string())
                .or_insert_with(|| serde_json::Value::String(model_description.clone()));
        } else if let Some(object) = definition.as_object_mut() {
            object.insert(
                "name".to_string(),
                serde_json::Value::String(self.id.clone()),
            );
            object.insert("parameters".to_string(), model_input_schema);
            object
                .entry("description".to_string())
                .or_insert_with(|| serde_json::Value::String(model_description));
        }
        // Catalog descriptions support discovery, but repeating the identical
        // paragraph in both function.description and parameters.description
        // wastes prompt tokens. Preserve different and nested field guidance.
        let function = if definition.get("function").is_some() {
            definition.get_mut("function").unwrap()
        } else {
            &mut definition
        };
        let repeated = function
            .get("description")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|description| {
                function
                    .pointer("/parameters/description")
                    .and_then(serde_json::Value::as_str)
                    == Some(description)
            });
        if repeated {
            if let Some(parameters) = function
                .get_mut("parameters")
                .and_then(serde_json::Value::as_object_mut)
            {
                parameters.remove("description");
            }
        }
        definition
    }

    /// Resolves the concrete authorization boundary for one invocation.
    /// Hosts render the resulting request, but they do not reinterpret it.
    pub fn permission_subject(
        &self,
        input: &serde_json::Value,
    ) -> Option<super::permissions::PermissionSubject> {
        Some(super::permissions::PermissionSubject {
            capability: self
                .permission_domain
                .clone()
                .unwrap_or_else(|| super::policy::permission_name_for_tool(&self.id)),
            resource: self.input_normalization.permission_resource.resolve(input),
        })
    }

    /// Validates a model-produced capability input before Core considers
    /// permission policy. A malformed call must never reach a permission UI:
    /// approval is meaningful only for an executable operation.
    pub fn validate_input(&self, input: &serde_json::Value) -> Result<(), String> {
        validate_tool_input_schema(&self.input_schema, input)
    }
}

pub fn validate_tool_input_schema(
    schema: &serde_json::Value,
    input: &serde_json::Value,
) -> Result<(), String> {
    validate_schema_value(schema, input, "$")
}

fn model_input_schema_with_execution_context(
    input_schema: &serde_json::Value,
) -> serde_json::Value {
    let mut schema = input_schema.clone();
    if schema.get("type").and_then(serde_json::Value::as_str) != Some("object") {
        return schema;
    }
    let lineage = serde_json::json!({
        "type": "object",
        "description": "Optional durable SOP execution lineage. Supply only when an active SOP context explicitly provides the exact runId and stepId for this concrete call. Never use a Skill id, Skill run alias, skill_update run_id, or guessed workflow name here; Skill execution is tracked separately. Omit this field when no active SOP supplies both identifiers.",
        "properties": {
            "sopRunId": {"type": "string", "minLength": 1, "description": "Exact durable SOP runId from the active SOP context; never a Skill id or Skill run alias."},
            "sopStepId": {"type": "string", "minLength": 1, "description": "Exact active SOP stepId paired with sopRunId; never infer it from Skill progress steps."}
        },
        "required": ["sopRunId", "sopStepId"],
        "additionalProperties": false
    });
    fn project_object_layer(schema: &mut serde_json::Value, lineage: &serde_json::Value) {
        let Some(object) = schema.as_object_mut() else {
            return;
        };
        // Combinators describe this same argument object. Nested properties
        // and array items describe business data and must never gain lineage.
        let properties = object
            .entry("properties")
            .or_insert_with(|| serde_json::json!({}));
        if let Some(properties) = properties.as_object_mut() {
            properties.insert(
                super::tool_lifecycle::TOOL_INVOCATION_CONTEXT_ARGUMENT.into(),
                lineage.clone(),
            );
        }
        for key in ["allOf", "anyOf", "oneOf"] {
            if let Some(branches) = object
                .get_mut(key)
                .and_then(serde_json::Value::as_array_mut)
            {
                for branch in branches {
                    project_object_layer(branch, lineage);
                }
            }
        }
    }
    project_object_layer(&mut schema, &lineage);
    schema
}

fn validate_schema_value(
    schema: &serde_json::Value,
    value: &serde_json::Value,
    path: &str,
) -> Result<(), String> {
    if let Some(expected) = schema.get("const") {
        if expected != value {
            return Err(format!("{path} does not match the required constant value"));
        }
    }
    if let Some(values) = schema.get("enum").and_then(serde_json::Value::as_array) {
        if !values.contains(value) {
            return Err(format!("{path} is not one of the allowed values"));
        }
    }

    if let Some(branches) = schema.get("allOf").and_then(serde_json::Value::as_array) {
        for branch in branches {
            validate_schema_value(branch, value, path)?;
        }
    }
    if let Some(branches) = schema.get("anyOf").and_then(serde_json::Value::as_array) {
        if !branches
            .iter()
            .any(|branch| validate_schema_value(branch, value, path).is_ok())
        {
            return Err(format!("{path} does not match any allowed schema"));
        }
    }
    if let Some(branches) = schema.get("oneOf").and_then(serde_json::Value::as_array) {
        let matches = branches
            .iter()
            .filter(|branch| validate_schema_value(branch, value, path).is_ok())
            .count();
        if matches != 1 {
            return Err(format!("{path} must match exactly one allowed schema"));
        }
    }

    let valid_type = |kind: &str| match kind {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "string" => value.is_string(),
        _ => true,
    };
    if let Some(expected) = schema.get("type") {
        let matches = match expected {
            serde_json::Value::String(kind) => valid_type(kind),
            serde_json::Value::Array(kinds) => kinds
                .iter()
                .filter_map(serde_json::Value::as_str)
                .any(valid_type),
            _ => true,
        };
        if !matches {
            let expected = match expected {
                serde_json::Value::String(kind) => kind.clone(),
                serde_json::Value::Array(kinds) => kinds
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" or "),
                _ => "the declared schema type".to_string(),
            };
            return Err(format!(
                "{path} has JSON type {}; expected {expected}",
                json_type_name(value)
            ));
        }
    }

    if let Some(number) = value.as_f64() {
        if let Some(minimum) = schema.get("minimum").and_then(serde_json::Value::as_f64) {
            if number < minimum {
                return Err(format!("{path} is below the minimum value {minimum}"));
            }
        }
        if let Some(maximum) = schema.get("maximum").and_then(serde_json::Value::as_f64) {
            if number > maximum {
                return Err(format!("{path} exceeds the maximum value {maximum}"));
            }
        }
    }

    if let Some(object) = value.as_object() {
        if let Some(required) = schema.get("required").and_then(serde_json::Value::as_array) {
            let missing = required
                .iter()
                .filter_map(serde_json::Value::as_str)
                .filter(|name| !object.contains_key(*name) || object[*name].is_null())
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                return Err(format!(
                    "{path} is missing required field(s): {}",
                    missing.join(", ")
                ));
            }
        }
        if let Some(properties) = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
        {
            for (name, property_schema) in properties {
                if let Some(property_value) = object.get(name) {
                    validate_schema_value(
                        property_schema,
                        property_value,
                        &format!("{path}.{name}"),
                    )?;
                }
            }
            if schema
                .get("additionalProperties")
                .and_then(serde_json::Value::as_bool)
                == Some(false)
            {
                let unexpected = object
                    .keys()
                    .filter(|name| !properties.contains_key(*name))
                    .cloned()
                    .collect::<Vec<_>>();
                if !unexpected.is_empty() {
                    return Err(format!(
                        "{path} contains unexpected field(s): {}",
                        unexpected.join(", ")
                    ));
                }
            }
        }
    }

    if let Some(array) = value.as_array() {
        if let Some(minimum) = schema.get("minItems").and_then(serde_json::Value::as_u64) {
            if array.len() < minimum as usize {
                return Err(format!("{path} contains too few items"));
            }
        }
        if let Some(maximum) = schema.get("maxItems").and_then(serde_json::Value::as_u64) {
            if array.len() > maximum as usize {
                return Err(format!("{path} contains too many items"));
            }
        }
        if let Some(items) = schema.get("items") {
            for (index, item) in array.iter().enumerate() {
                validate_schema_value(items, item, &format!("{path}[{index}]"))?;
            }
        }
    }

    if let Some(text) = value.as_str() {
        if let Some(minimum) = schema.get("minLength").and_then(serde_json::Value::as_u64) {
            if text.chars().count() < minimum as usize {
                return Err(format!("{path} is shorter than the minimum length"));
            }
        }
    }
    Ok(())
}

fn json_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostCapabilityExposure {
    #[default]
    Visible,
    Deferred,
    Internal,
    Hidden,
}

impl HostCapabilityExposure {
    /// Only explicitly visible capabilities may be described to the model.
    ///
    /// Deferred capabilities require an explicit discovery/promotion step,
    /// while internal and hidden capabilities remain available to Core/Host
    /// orchestration without becoming model-callable.
    pub fn is_model_visible(self) -> bool {
        matches!(self, Self::Visible)
    }

    /// Visible tools are included directly; deferred tools may be returned by
    /// the explicit discovery protocol. Internal and hidden capabilities must
    /// never be described to or activated by the model.
    pub fn is_model_discoverable(self) -> bool {
        matches!(self, Self::Visible | Self::Deferred)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostCapabilityParallelism {
    #[default]
    Serial,
    ConcurrentSafe,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostCancellationMode {
    /// The Host attests the complete invocation stays in its owning process,
    /// with no child processes, remote work, or lifecycle-hook execution.
    ProcessBound,
    #[default]
    Unsupported,
    BestEffort,
    Cooperative,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostReplaySafety {
    Idempotent,
    Recoverable,
    #[default]
    RequiresRecoveryDecision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostCapabilityRoute {
    Model,
    Tool,
    Context,
    Permission,
    Storage,
    Attachment,
    Compaction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityAvailability {
    Available,
    Unavailable,
    RequiresSetup,
    RequiresConnection,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostOutputPolicy {
    pub preview_char_limit: usize,
    pub persist_full_output: bool,
    pub allow_artifacts: bool,
}

/// Optional process-local bridge for input that arrives while a Host effect is
/// running. The source is cloned before the driver is mutably borrowed, so the
/// Core loop can apply input immediately before a streamed Host response at a
/// deterministic boundary without concurrent Runtime mutation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentBoundaryInput {
    PermissionPolicyAvailable {
        policy: nola_domain::ActivePermissionPolicy,
    },
    Submission {
        submission: super::protocol::AgentSubmission,
    },
    TaskTerminalAvailable {
        task_id: String,
        receipt_id: String,
    },
    TaskGuidanceAvailable {
        task_id: String,
        message_id: String,
        settlement: bool,
    },
}

/// Core's concrete payload family. Execution machinery depends only on the
/// transport traits, never on these reducer-owned payload definitions.
#[derive(Debug, Clone)]
pub struct AgentRuntimeProtocol;

impl nola_host_protocol::HostExecutionProtocol for AgentRuntimeProtocol {
    type Request = HostRequest;
    type Response = HostResponse;
    type BoundaryInput = AgentBoundaryInput;
    type Event = super::protocol::AgentEvent;
    type Trace = crate::agent_trace::AgentTraceEvent;
    type Control = super::protocol::AgentControlSnapshot;
}

pub use nola_host_protocol::{
    BoundaryInputSource as AgentBoundarySubmissionSource, HostDriver as AgentHostDriver,
};

#[cfg(test)]
mod permission_subject_tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn composite_permission_resource_never_returns_partial_coverage() {
        let declaration = HostPermissionResourceDeclaration::Composite {
            keys: vec!["source".into(), "destination".into()],
        };
        let complete = json!({"source":"a", "destination":"b", "extra":"not a resource"});
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&declaration.resolve(&complete)).unwrap(),
            json!({"source":"a", "destination":"b"})
        );
        for incomplete in [
            json!({"destination":"b", "extra":"preserved"}),
            json!({"source":null, "destination":"b", "extra":"preserved"}),
        ] {
            assert_eq!(
                declaration.resolve(&incomplete),
                HostPermissionResourceDeclaration::FullInput.resolve(&incomplete)
            );
        }
    }

    #[test]
    fn multi_resource_catalog_does_not_select_only_one_path() {
        for capability in [
            "move_file",
            "copy_file",
            "read_many_files",
            "format_files",
            "replace_in_files",
            "apply_patch",
        ] {
            let declaration =
                super::super::permissions::permission_resource_declaration_for_builtin(capability);
            let input = json!({"source":"private/a", "destination":"public/b",
                "path":"first", "paths":["first", "second"], "patch":"payload"});
            assert_eq!(
                declaration.resolve(&input),
                HostPermissionResourceDeclaration::FullInput.resolve(&input),
                "{capability}"
            );
        }
    }

    #[test]
    fn resource_locator_preserves_literal_whitespace() {
        let declaration = HostPermissionResourceDeclaration::FirstOf {
            keys: vec!["path".into()],
        };
        assert_eq!(
            declaration.resolve(&json!({"path":" notes/today.md "})),
            " notes/today.md "
        );
    }

    #[test]
    fn host_declared_permission_domain_is_the_authorization_subject() {
        let descriptor = HostCapabilityDescriptor {
            id: "write_file".to_string(),
            title: "Write file".to_string(),
            route: HostCapabilityRoute::Tool,
            availability: CapabilityAvailability::Available,
            exposure: HostCapabilityExposure::Visible,
            permission_domain: Some("filesystem.write".to_string()),
            input_schema: json!({"type": "object"}),
            input_normalization: HostInputNormalization {
                permission_resource: HostPermissionResourceDeclaration::FirstOf {
                    keys: vec!["path".to_string()],
                },
                ..Default::default()
            },
            output_policy: HostOutputPolicy {
                preview_char_limit: 1_000,
                persist_full_output: true,
                allow_artifacts: true,
            },
            supports_streaming: false,
            supports_argument_streaming: false,
            parallelism: HostCapabilityParallelism::default(),
            cancellation: HostCancellationMode::default(),
            replay_safety: HostReplaySafety::default(),
        };

        let subject = descriptor
            .permission_subject(&json!({"path": "notes/today.md"}))
            .expect("permission subject");

        assert_eq!(subject.capability, "filesystem.write");
        assert_eq!(subject.resource, "notes/today.md");
        assert_eq!(descriptor.id, "write_file");
    }

    #[test]
    fn model_schema_advertises_execution_lineage_without_widening_host_input() {
        let descriptor = HostCapabilityDescriptor {
            id: "write_file".to_string(),
            title: "Write file".to_string(),
            route: HostCapabilityRoute::Tool,
            availability: CapabilityAvailability::Available,
            exposure: HostCapabilityExposure::Visible,
            permission_domain: None,
            input_schema: json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"],
                "additionalProperties": false
            }),
            input_normalization: HostInputNormalization::default(),
            output_policy: HostOutputPolicy {
                preview_char_limit: 1_000,
                persist_full_output: true,
                allow_artifacts: true,
            },
            supports_streaming: false,
            supports_argument_streaming: false,
            parallelism: HostCapabilityParallelism::default(),
            cancellation: HostCancellationMode::default(),
            replay_safety: HostReplaySafety::default(),
        };
        let definition = descriptor.model_tool_definition(&json!({
            "type": "function",
            "function": {"name": "placeholder"}
        }));
        assert!(definition["function"]["parameters"]["properties"]
            .get("_nolaExecution")
            .is_some());
        let lineage_description = definition["function"]["parameters"]["properties"]
            ["_nolaExecution"]["description"]
            .as_str()
            .expect("execution lineage description");
        assert!(lineage_description.contains("Never use a Skill id"));
        assert!(lineage_description.contains("Omit this field"));
        assert!(descriptor
            .validate_input(&json!({"path": "output.txt", "_nolaExecution": {}}))
            .is_err());

        // All alternatives describe the root call, but properties/items below
        // them are independent business objects, not execution containers.
        for alternative in ["oneOf", "anyOf"] {
            let options = json!({"type":"object","properties":{"label":{"type":"string"}},
                "required":["label"],"additionalProperties":false});
            let records = json!({"type":"array","items":{"oneOf":[
                {"type":"object","properties":{"label":{"type":"string"}},"required":["label"],"additionalProperties":false},
                {"type":"object","properties":{"count":{"type":"integer"}},"required":["count"],"additionalProperties":false}
            ]}});
            let query = json!({"type":"object","properties":{"query":{"type":"string"},
                "options":options.clone(),"records":records.clone()},"required":["query"],"additionalProperties":false});
            let read = json!({"type":"object","properties":{"message_id":{"type":"string"},
                "part_id":{"type":"string"},"options":options.clone(),"records":records.clone()},
                "required":["message_id","part_id"],"additionalProperties":false});
            let mut choices = json!({});
            choices[alternative] = json!([query, read]);
            let mut business = descriptor.clone();
            business.input_schema = json!({"type":"object","properties":{"query":{"type":"string"},
                "message_id":{"type":"string"},"part_id":{"type":"string"},"options":options,"records":records},
                "allOf":[choices],"additionalProperties":false});
            let definition = business
                .model_tool_definition(&json!({"type":"function","function":{"name":"fixture"}}));
            let mut projected = business.clone();
            projected.input_schema = definition["function"]["parameters"].clone();
            for input in [
                json!({"query":"decision","options":{"label":"current"},"records":[{"count":1}]}),
                json!({"message_id":"m","part_id":"p","records":[{"label":"source"}]}),
            ] {
                assert!(business.validate_input(&input).is_ok());
                assert!(projected.validate_input(&input).is_ok());
                let mut with_lineage = input;
                with_lineage["_nolaExecution"] = json!({"sopRunId":"run","sopStepId":"step"});
                assert!(
                    projected.validate_input(&with_lineage).is_ok(),
                    "{alternative}: {with_lineage}"
                );
                assert!(business.validate_input(&with_lineage).is_err());
            }
            for invalid in [
                json!({"query":"decision","message_id":"m","part_id":"p"}),
                json!({"query":"decision","_nolaExecution":{}}),
                json!({"query":"decision","_nolaExecution":{"sopRunId":"run","sopStepId":""}}),
                json!({"query":"decision","options":{"label":"current","_nolaExecution":{"sopRunId":"run","sopStepId":"step"}}}),
                json!({"query":"decision","records":[{"count":1,"_nolaExecution":{"sopRunId":"run","sopStepId":"step"}}]}),
            ] {
                assert!(
                    projected.validate_input(&invalid).is_err(),
                    "{alternative}: {invalid}"
                );
            }
        }
    }

    #[test]
    fn model_schema_uses_host_declared_capability_description() {
        let descriptor = HostCapabilityDescriptor {
            id: "media_reader".to_string(),
            title: "Media reader".to_string(),
            route: HostCapabilityRoute::Tool,
            availability: CapabilityAvailability::Available,
            exposure: HostCapabilityExposure::Visible,
            permission_domain: None,
            input_schema: json!({
                "type": "object",
                "description": "Inspect bounded media metadata and signal evidence.",
                "properties": {"path": {"type": "string"}},
                "required": ["path"]
            }),
            input_normalization: HostInputNormalization::default(),
            output_policy: HostOutputPolicy {
                preview_char_limit: 1_000,
                persist_full_output: true,
                allow_artifacts: true,
            },
            supports_streaming: false,
            supports_argument_streaming: false,
            parallelism: HostCapabilityParallelism::default(),
            cancellation: HostCancellationMode::default(),
            replay_safety: HostReplaySafety::default(),
        };

        let definition = descriptor.model_tool_definition(&json!({
            "type": "function",
            "function": {"name": "placeholder"}
        }));

        assert_eq!(
            definition["function"]["description"],
            "Inspect bounded media metadata and signal evidence."
        );
        assert!(definition["function"]["parameters"]
            .get("description")
            .is_none());
        assert_eq!(
            definition["function"]["parameters"]["required"],
            json!(["path"])
        );
        assert!(
            descriptor.input_schema.get("description").is_some(),
            "discovery keeps the authoritative description"
        );
        let different = descriptor.model_tool_definition(&json!({"type":"function",
            "function":{"name":"placeholder","description":"Additional call-specific guidance"}}));
        assert!(
            different["function"]["parameters"]
                .get("description")
                .is_some(),
            "distinct parameter guidance must not be removed"
        );
    }

    #[test]
    fn input_validation_enforces_constant_discriminators() {
        let descriptor = HostCapabilityDescriptor {
            id: "sop_update".into(),
            title: "SOP update".into(),
            route: HostCapabilityRoute::Tool,
            availability: CapabilityAvailability::Available,
            exposure: HostCapabilityExposure::Visible,
            permission_domain: None,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "action": {"const": "apply"},
                    "run_id": {"type": "string"}
                },
                "required": ["action", "run_id"]
            }),
            input_normalization: HostInputNormalization::default(),
            output_policy: HostOutputPolicy {
                preview_char_limit: 1_000,
                persist_full_output: false,
                allow_artifacts: false,
            },
            supports_streaming: false,
            supports_argument_streaming: false,
            parallelism: HostCapabilityParallelism::default(),
            cancellation: HostCancellationMode::default(),
            replay_safety: HostReplaySafety::default(),
        };

        assert!(descriptor
            .validate_input(&json!({"action": "apply", "run_id": "run-1"}))
            .is_ok());
        assert!(descriptor
            .validate_input(&json!({"action": "load", "run_id": "run-1"}))
            .unwrap_err()
            .contains("required constant"));
    }

    #[test]
    fn input_validation_enforces_generic_array_maximum_at_the_declared_boundary() {
        let schema = json!({"type":"array","minItems":1,"maxItems":2,
            "items":{"type":"object","properties":{"id":{"type":"string"}},
                "required":["id"],"additionalProperties":false}});
        assert!(validate_tool_input_schema(&schema, &json!([{"id":"one"}])).is_ok());
        assert!(validate_tool_input_schema(&schema, &json!([{"id":"one"},{"id":"two"}])).is_ok());
        assert!(validate_tool_input_schema(&schema, &json!([]))
            .unwrap_err()
            .contains("too few"));
        assert!(validate_tool_input_schema(
            &schema,
            &json!([{"id":"one"},{"id":"two"},{"id":"three"}])
        )
        .unwrap_err()
        .contains("too many"));
        assert!(validate_tool_input_schema(&schema, &json!([{"id":"one"},{}])).is_err());
        assert!(
            validate_tool_input_schema(&json!({"type":"array","maxItems":0}), &json!([])).is_ok()
        );
        assert!(
            validate_tool_input_schema(&json!({"type":"array","maxItems":0}), &json!([1])).is_err()
        );
    }
}
