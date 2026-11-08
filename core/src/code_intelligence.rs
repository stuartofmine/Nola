// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines the Core-owned code-intelligence capability contract.
// - Boundary: Core names semantic operations; platform Hosts own indexing, storage, and transport.

use serde::{Deserialize, Serialize};

pub const CODEBASE_MEMORY_PROVIDER_ID: &str = "nola.codebase-memory";

/// The first useful index should optimize time-to-first-query. Rich semantic
/// edges remain an explicit upgrade because they are substantially more
/// expensive on large workspaces.
pub const DEFAULT_INDEX_MODE: &str = "fast";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeFactFreshness {
    Current,
    RequiresVerification,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeFactProvenance {
    pub repository_root: String,
    pub path: String,
    pub symbol: Option<String>,
    pub path_exists: bool,
    pub current_content_sha256: Option<String>,
    pub provider_content_sha256: Option<String>,
    pub evidence_matches_current_source: Option<bool>,
    /// Host-observed lexical evidence for the provider's typed symbol identity.
    /// `None` means the file or symbol shape could not be checked safely.
    #[serde(default)]
    pub symbol_matches_current_source: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeIntelligenceProvenance {
    pub operation: CodeIntelligenceOperation,
    pub repository_root: String,
    pub current_repository_revision: Option<String>,
    pub provider_repository_revision: Option<String>,
    pub facts: Vec<CodeFactProvenance>,
    pub freshness: CodeFactFreshness,
    pub requires_current_state_verification: bool,
    pub reasons: Vec<String>,
}

/// Classifies provider facts against an independently observed repository
/// snapshot. Provider claims are evidence, never authority over freshness.
pub fn assess_code_intelligence_provenance(
    operation: CodeIntelligenceOperation,
    repository_root: String,
    current_repository_revision: Option<String>,
    provider_repository_revision: Option<String>,
    facts: Vec<CodeFactProvenance>,
) -> CodeIntelligenceProvenance {
    let mut reasons = Vec::new();
    let revision_matches = current_repository_revision.is_some()
        && current_repository_revision == provider_repository_revision;
    if facts.is_empty() {
        reasons.push("provider_result_has_no_path_and_symbol_evidence".to_string());
    }
    let mut stale = false;
    let mut every_fact_verified = !facts.is_empty();
    let mut snapshot_unverified = false;
    for fact in &facts {
        let symbol_identity_missing = operation.requires_symbol_identity()
            && fact
                .symbol
                .as_deref()
                .is_none_or(|symbol| symbol.trim().is_empty());
        if fact.repository_root != repository_root
            || fact.path.trim().is_empty()
            || symbol_identity_missing
        {
            every_fact_verified = false;
            reasons.push("code_fact_identity_incomplete".to_string());
        }
        let digest_matches = fact.current_content_sha256.is_some()
            && fact.provider_content_sha256.is_some()
            && fact.current_content_sha256 == fact.provider_content_sha256;
        if !fact.path_exists {
            stale = true;
            reasons.push("code_fact_path_missing".to_string());
        } else if fact.provider_content_sha256.is_some() && !digest_matches {
            stale = true;
            reasons.push("code_fact_content_digest_drifted".to_string());
        }
        match fact.evidence_matches_current_source {
            Some(false) => {
                stale = true;
                reasons.push("code_fact_summary_conflicts_with_current_source".to_string());
            }
            Some(true) => {}
            None => {}
        }
        if operation.requires_symbol_identity() {
            match fact.symbol_matches_current_source {
                Some(false) => {
                    stale = true;
                    reasons.push("code_fact_symbol_missing_from_current_source".to_string());
                }
                Some(true) => {}
                None => {
                    every_fact_verified = false;
                    reasons.push("code_fact_symbol_is_not_verifiable".to_string());
                }
            }
        }
        // The provider's 0.10.x result contract does not promise a repository
        // revision on every query. Host-observed digest or source evidence can
        // independently attest the current file; a bare provider fact remains
        // available but explicitly requires a current-source follow-up.
        if !digest_matches && fact.evidence_matches_current_source != Some(true) {
            every_fact_verified = false;
            snapshot_unverified = true;
        }
    }
    if snapshot_unverified {
        reasons.push(
            match (&current_repository_revision, &provider_repository_revision) {
                (None, _) => "current_repository_revision_unavailable",
                (Some(_), None) => "provider_snapshot_is_not_verifiably_current",
                (Some(_), Some(_)) if revision_matches => {
                    "code_fact_current_source_is_not_attested"
                }
                (Some(_), Some(_)) => "provider_repository_revision_drifted",
            }
            .to_string(),
        );
    }
    reasons.sort();
    reasons.dedup();
    let freshness = if stale {
        CodeFactFreshness::Stale
    } else if every_fact_verified {
        CodeFactFreshness::Current
    } else {
        CodeFactFreshness::RequiresVerification
    };
    CodeIntelligenceProvenance {
        operation,
        repository_root,
        current_repository_revision,
        provider_repository_revision,
        facts,
        freshness,
        requires_current_state_verification: freshness != CodeFactFreshness::Current,
        reasons,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeIntelligenceOperation {
    Index,
    IndexStatus,
    Projects,
    Architecture,
    Search,
    TextSearch,
    Trace,
    ReadSymbol,
    Query,
    Schema,
    Coverage,
    ChangedImpact,
    DeleteIndex,
    ArchitectureDecision,
    IngestTraces,
}

impl CodeIntelligenceOperation {
    pub const fn capability_id(self) -> &'static str {
        match self {
            Self::Index => "codebase_index",
            Self::IndexStatus => "codebase_index_status",
            Self::Projects => "codebase_projects",
            Self::Architecture => "codebase_architecture",
            Self::Search => "codebase_search",
            Self::TextSearch => "codebase_text_search",
            Self::Trace => "codebase_trace",
            Self::ReadSymbol => "codebase_read_symbol",
            Self::Query => "codebase_query",
            Self::Schema => "codebase_schema",
            Self::Coverage => "codebase_check_coverage",
            Self::ChangedImpact => "codebase_changed_impact",
            Self::DeleteIndex => "codebase_delete_index",
            Self::ArchitectureDecision => "codebase_adr",
            Self::IngestTraces => "codebase_ingest_traces",
        }
    }

    /// Provider annotations are advisory and Codebase Memory 0.10.x marks a
    /// number of graph queries as destructive. Core owns the stable semantic
    /// contract, so Hosts use this classification instead of granting a
    /// provider's incorrect annotation authority over permission behavior.
    pub const fn is_read_only(self) -> bool {
        matches!(
            self,
            Self::IndexStatus
                | Self::Projects
                | Self::Architecture
                | Self::Search
                | Self::TextSearch
                | Self::Trace
                | Self::ReadSymbol
                | Self::Query
                | Self::Schema
                | Self::Coverage
                | Self::ChangedImpact
        )
    }

    /// Operations whose result asserts facts about repository contents. Status,
    /// project-list, and schema responses are provider metadata and do not need
    /// per-file evidence. This distinction is made from the typed operation.
    pub const fn requires_code_fact_attestation(self) -> bool {
        matches!(
            self,
            Self::Architecture
                | Self::Search
                | Self::TextSearch
                | Self::Trace
                | Self::ReadSymbol
                | Self::Query
                | Self::Coverage
                | Self::ChangedImpact
        )
    }

    /// Graph symbol operations promise a symbol identity. Plain text searches
    /// and aggregate graph reports may validly return file-scoped evidence.
    pub const fn requires_symbol_identity(self) -> bool {
        matches!(self, Self::Search | Self::Trace | Self::ReadSymbol)
    }

    /// A typed verification step for graph candidates. The Host exposes this
    /// beside unverified provider output so the model can read current source
    /// without deriving a route from the user's wording.
    pub const fn verification_follow_up_capability_id(self) -> Option<&'static str> {
        match self {
            Self::Search | Self::Trace => Some(Self::ReadSymbol.capability_id()),
            _ => None,
        }
    }
}

pub fn operation_for_capability_id(capability_id: &str) -> Option<CodeIntelligenceOperation> {
    Some(match capability_id {
        "codebase_index" => CodeIntelligenceOperation::Index,
        "codebase_index_status" => CodeIntelligenceOperation::IndexStatus,
        "codebase_projects" => CodeIntelligenceOperation::Projects,
        "codebase_architecture" => CodeIntelligenceOperation::Architecture,
        "codebase_search" => CodeIntelligenceOperation::Search,
        "codebase_text_search" => CodeIntelligenceOperation::TextSearch,
        "codebase_trace" => CodeIntelligenceOperation::Trace,
        "codebase_read_symbol" => CodeIntelligenceOperation::ReadSymbol,
        "codebase_query" => CodeIntelligenceOperation::Query,
        "codebase_schema" => CodeIntelligenceOperation::Schema,
        "codebase_check_coverage" => CodeIntelligenceOperation::Coverage,
        "codebase_changed_impact" => CodeIntelligenceOperation::ChangedImpact,
        "codebase_delete_index" => CodeIntelligenceOperation::DeleteIndex,
        "codebase_adr" => CodeIntelligenceOperation::ArchitectureDecision,
        "codebase_ingest_traces" => CodeIntelligenceOperation::IngestTraces,
        _ => return None,
    })
}

/// Native read-only capabilities that keep code exploration useful while the
/// retained graph is absent or being built. This is selected from the concrete
/// operation, never from user text.
pub fn coarse_fallback_capability_ids(
    operation: CodeIntelligenceOperation,
) -> &'static [&'static str] {
    match operation {
        CodeIntelligenceOperation::Search | CodeIntelligenceOperation::TextSearch => {
            &["code_search", "workspace_symbols", "grep", "read_symbol"]
        }
        CodeIntelligenceOperation::ReadSymbol | CodeIntelligenceOperation::Trace => &[
            "workspace_symbols",
            "symbol_search",
            "call_graph",
            "read_symbol",
        ],
        CodeIntelligenceOperation::Architecture
        | CodeIntelligenceOperation::Query
        | CodeIntelligenceOperation::Schema
        | CodeIntelligenceOperation::Coverage
        | CodeIntelligenceOperation::ChangedImpact => {
            &["repo_map", "project_overview", "file_tree", "code_search"]
        }
        _ => &[],
    }
}

/// Maps a provider protocol operation to Nola's stable model-facing capability.
/// This is protocol adaptation, not user-text routing.
pub fn stable_capability_id(provider_id: &str, operation: &str) -> Option<&'static str> {
    if provider_id != CODEBASE_MEMORY_PROVIDER_ID {
        return None;
    }
    let operation = match operation {
        "index_repository" => CodeIntelligenceOperation::Index,
        "index_status" => CodeIntelligenceOperation::IndexStatus,
        "list_projects" => CodeIntelligenceOperation::Projects,
        "get_architecture" => CodeIntelligenceOperation::Architecture,
        "search_graph" => CodeIntelligenceOperation::Search,
        "search_code" => CodeIntelligenceOperation::TextSearch,
        "trace_path" => CodeIntelligenceOperation::Trace,
        "get_code_snippet" => CodeIntelligenceOperation::ReadSymbol,
        "query_graph" => CodeIntelligenceOperation::Query,
        "get_graph_schema" => CodeIntelligenceOperation::Schema,
        "check_index_coverage" => CodeIntelligenceOperation::Coverage,
        "detect_changes" => CodeIntelligenceOperation::ChangedImpact,
        "delete_project" => CodeIntelligenceOperation::DeleteIndex,
        "manage_adr" => CodeIntelligenceOperation::ArchitectureDecision,
        "ingest_traces" => CodeIntelligenceOperation::IngestTraces,
        _ => return None,
    };
    Some(operation.capability_id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_protocol_maps_to_stable_core_capabilities() {
        assert_eq!(
            stable_capability_id(CODEBASE_MEMORY_PROVIDER_ID, "search_graph"),
            Some("codebase_search")
        );
        assert_eq!(
            stable_capability_id(CODEBASE_MEMORY_PROVIDER_ID, "trace_path"),
            Some("codebase_trace")
        );
        assert_eq!(stable_capability_id("user.mcp", "search_graph"), None);
        assert_eq!(
            stable_capability_id(CODEBASE_MEMORY_PROVIDER_ID, "future_tool"),
            None
        );
    }

    #[test]
    fn missing_graph_fallback_is_operation_typed() {
        let operation = operation_for_capability_id("codebase_search").expect("known operation");
        assert_eq!(operation, CodeIntelligenceOperation::Search);
        assert_eq!(coarse_fallback_capability_ids(operation)[0], "code_search");
        assert!(operation_for_capability_id("search").is_none());
        assert_eq!(DEFAULT_INDEX_MODE, "fast");
    }

    #[test]
    fn query_operations_are_read_only_even_when_provider_annotations_are_wrong() {
        for capability_id in [
            "codebase_index_status",
            "codebase_projects",
            "codebase_architecture",
            "codebase_search",
            "codebase_text_search",
            "codebase_trace",
            "codebase_read_symbol",
            "codebase_query",
            "codebase_schema",
            "codebase_check_coverage",
            "codebase_changed_impact",
        ] {
            assert!(
                operation_for_capability_id(capability_id)
                    .expect("known code-intelligence operation")
                    .is_read_only(),
                "{capability_id} must not request mutation permission"
            );
        }
        for capability_id in [
            "codebase_index",
            "codebase_delete_index",
            "codebase_adr",
            "codebase_ingest_traces",
        ] {
            assert!(!operation_for_capability_id(capability_id)
                .expect("known code-intelligence operation")
                .is_read_only());
        }
    }

    #[test]
    fn provenance_policy_is_selected_from_the_typed_operation() {
        for operation in [
            CodeIntelligenceOperation::IndexStatus,
            CodeIntelligenceOperation::Projects,
            CodeIntelligenceOperation::Schema,
        ] {
            assert!(!operation.requires_code_fact_attestation());
        }
        assert!(CodeIntelligenceOperation::TextSearch.requires_code_fact_attestation());
        assert!(!CodeIntelligenceOperation::TextSearch.requires_symbol_identity());
        assert!(CodeIntelligenceOperation::ReadSymbol.requires_symbol_identity());
        assert_eq!(
            CodeIntelligenceOperation::Search.verification_follow_up_capability_id(),
            Some("codebase_read_symbol")
        );
        assert_eq!(
            CodeIntelligenceOperation::Architecture.verification_follow_up_capability_id(),
            None
        );
    }

    fn fact(current: Option<&str>, provider: Option<&str>) -> CodeFactProvenance {
        CodeFactProvenance {
            repository_root: "/repo".into(),
            path: "src/lib.rs".into(),
            symbol: Some("crate::answer".into()),
            path_exists: current.is_some(),
            current_content_sha256: current.map(str::to_string),
            provider_content_sha256: provider.map(str::to_string),
            evidence_matches_current_source: None,
            symbol_matches_current_source: Some(true),
        }
    }

    #[test]
    fn code_fact_requires_matching_revision_or_local_file_evidence() {
        let unverified = assess_code_intelligence_provenance(
            CodeIntelligenceOperation::Search,
            "/repo".into(),
            Some("current".into()),
            None,
            vec![fact(Some("a"), None)],
        );
        assert_eq!(
            unverified.freshness,
            CodeFactFreshness::RequiresVerification
        );
        assert!(unverified.requires_current_state_verification);

        let digest_without_revision = assess_code_intelligence_provenance(
            CodeIntelligenceOperation::Search,
            "/repo".into(),
            Some("current".into()),
            None,
            vec![CodeFactProvenance {
                evidence_matches_current_source: Some(true),
                ..fact(Some("a"), Some("a"))
            }],
        );
        assert_eq!(
            digest_without_revision.freshness,
            CodeFactFreshness::Current
        );

        let verified = assess_code_intelligence_provenance(
            CodeIntelligenceOperation::Search,
            "/repo".into(),
            Some("current".into()),
            Some("current".into()),
            vec![CodeFactProvenance {
                evidence_matches_current_source: Some(true),
                ..fact(Some("a"), Some("a"))
            }],
        );
        assert_eq!(verified.freshness, CodeFactFreshness::Current);
        assert!(!verified.requires_current_state_verification);
    }

    #[test]
    fn missing_drifted_or_conflicting_code_evidence_is_stale() {
        for evidence in [
            fact(None, Some("old")),
            fact(Some("new"), Some("old")),
            CodeFactProvenance {
                evidence_matches_current_source: Some(false),
                ..fact(Some("same"), Some("same"))
            },
            CodeFactProvenance {
                symbol_matches_current_source: Some(false),
                ..fact(Some("same"), Some("same"))
            },
        ] {
            let assessed = assess_code_intelligence_provenance(
                CodeIntelligenceOperation::ReadSymbol,
                "/repo".into(),
                Some("current".into()),
                Some("old".into()),
                vec![evidence],
            );
            assert_eq!(assessed.freshness, CodeFactFreshness::Stale);
            assert!(assessed.requires_current_state_verification);
        }
    }

    #[test]
    fn matching_revision_and_digest_do_not_replace_symbol_evidence() {
        let assessed = assess_code_intelligence_provenance(
            CodeIntelligenceOperation::ReadSymbol,
            "/repo".into(),
            Some("current".into()),
            Some("current".into()),
            vec![CodeFactProvenance {
                symbol_matches_current_source: None,
                ..fact(Some("same"), Some("same"))
            }],
        );
        assert_eq!(assessed.freshness, CodeFactFreshness::RequiresVerification);
        assert!(assessed
            .reasons
            .iter()
            .any(|reason| reason == "code_fact_symbol_is_not_verifiable"));
    }

    #[test]
    fn matching_revision_digest_and_symbol_attest_current_snapshot() {
        let assessed = assess_code_intelligence_provenance(
            CodeIntelligenceOperation::ReadSymbol,
            "/repo".into(),
            Some("current".into()),
            Some("current".into()),
            vec![fact(Some("same"), Some("same"))],
        );
        assert_eq!(assessed.freshness, CodeFactFreshness::Current);
    }

    #[test]
    fn provider_revision_claim_alone_requires_host_source_verification() {
        let assessed = assess_code_intelligence_provenance(
            CodeIntelligenceOperation::ReadSymbol,
            "/repo".into(),
            Some("current".into()),
            Some("current".into()),
            vec![CodeFactProvenance {
                provider_content_sha256: None,
                evidence_matches_current_source: None,
                ..fact(Some("same"), None)
            }],
        );
        assert_eq!(assessed.freshness, CodeFactFreshness::RequiresVerification);
        assert!(assessed
            .reasons
            .iter()
            .any(|reason| reason == "code_fact_current_source_is_not_attested"));
    }

    #[test]
    fn text_search_file_evidence_does_not_invent_a_symbol_requirement() {
        let assessed = assess_code_intelligence_provenance(
            CodeIntelligenceOperation::TextSearch,
            "/repo".into(),
            Some("current".into()),
            Some("current".into()),
            vec![CodeFactProvenance {
                symbol: None,
                evidence_matches_current_source: Some(true),
                symbol_matches_current_source: None,
                ..fact(Some("same"), Some("same"))
            }],
        );
        assert_eq!(assessed.freshness, CodeFactFreshness::Current);
    }

    #[test]
    fn unverified_snapshot_diagnostics_distinguish_unavailable_and_drifted_revisions() {
        for (current, provider, expected) in [
            (None, Some("old"), "current_repository_revision_unavailable"),
            (
                Some("current"),
                None,
                "provider_snapshot_is_not_verifiably_current",
            ),
            (
                Some("current"),
                Some("old"),
                "provider_repository_revision_drifted",
            ),
        ] {
            let assessed = assess_code_intelligence_provenance(
                CodeIntelligenceOperation::Search,
                "/repo".into(),
                current.map(str::to_string),
                provider.map(str::to_string),
                vec![fact(Some("same"), None)],
            );
            assert!(assessed.reasons.iter().any(|reason| reason == expected));
        }
    }
}
