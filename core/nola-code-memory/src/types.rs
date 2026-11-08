// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexReport {
    pub project: String,
    pub repository_root: String,
    pub generation: i64,
    pub discovered_files: usize,
    pub indexed_files: usize,
    pub unchanged_files: usize,
    pub removed_files: usize,
    pub symbols: usize,
    pub incomplete_files: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub name: String,
    pub repository_root: String,
    pub generation: i64,
    pub indexed_at_unix_ms: i64,
    pub file_count: usize,
    pub symbol_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexStatus {
    pub project: String,
    pub generation: i64,
    pub indexed_files: usize,
    pub incomplete_files: usize,
    pub symbol_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub qualified_name: String,
    pub name: String,
    pub kind: String,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub score: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchPage {
    pub project: String,
    pub total: usize,
    pub offset: usize,
    pub has_more: bool,
    pub results: Vec<SearchHit>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    pub project: String,
    pub qualified_name: String,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub content_sha256: String,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectureSummary {
    pub project: String,
    pub generation: i64,
    pub files_by_language: Vec<(String, usize)>,
    pub symbols_by_kind: Vec<(String, usize)>,
    pub top_level_directories: Vec<(String, usize)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileCoverage {
    pub path: String,
    pub kind: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageReport {
    pub project: String,
    pub generation: i64,
    pub complete_files: usize,
    pub incomplete_files: usize,
    pub issues: Vec<FileCoverage>,
    /// False until discovery also records every ignored and unsupported file.
    pub repository_scope_complete: bool,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceEdge {
    pub source: String,
    pub target: String,
    pub kind: String,
    pub confidence: f64,
    pub strategy: String,
    pub source_line: Option<usize>,
    pub depth: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceResult {
    pub project: String,
    pub root: String,
    pub direction: String,
    pub edges: Vec<TraceEdge>,
    pub truncated: bool,
}
