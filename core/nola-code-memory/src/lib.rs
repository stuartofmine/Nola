// SPDX-License-Identifier: Apache-2.0
//! Native, in-process code intelligence for Nola.
//!
//! The engine owns discovery, syntax extraction, durable indexing, and queries.
//! It deliberately has no MCP transport or subprocess boundary.

mod engine;
mod extractor;
mod schema;
mod types;

pub use engine::NativeCodeMemory;
pub use types::{
    ArchitectureSummary, CoverageReport, FileCoverage, IndexReport, IndexStatus, ProjectInfo,
    SearchHit, SearchPage, Snippet, TraceEdge, TraceResult,
};

/// Stable tool names exposed by the native Host adapter.
pub const TOOL_NAMES: &[&str] = &[
    "index_repository",
    "list_projects",
    "index_status",
    "search_graph",
    "trace_path",
    "search_code",
    "get_code_snippet",
    "get_graph_schema",
    "get_architecture",
    "check_index_coverage",
    "delete_project",
];
