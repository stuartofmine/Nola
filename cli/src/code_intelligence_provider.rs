// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines the Host-owned boundary for Nola's in-process Rust code-intelligence engine.
// - Boundary: The Agent Host owns schemas, permissions, cancellation, and output provenance;
//   the engine owns indexing and queries and has no MCP transport or process lifecycle.

use nola_core::code_intelligence::CodeIntelligenceOperation;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

const DEFAULT_CODE_MEMORY_FILE_THRESHOLD: usize = 20;

/// Readiness is an execution prerequisite, not a suggestion in a model prompt.
/// Scope comes from an admitted repository tool's concrete arguments.
#[derive(Default)]
pub(crate) struct CodeMemoryInitialization {
    ready_roots: std::collections::BTreeSet<PathBuf>,
}

impl CodeMemoryInitialization {
    pub(crate) fn preflight(
        &self,
        capability: &str,
        arguments: &Value,
        working_directory: &Path,
        cancelled: &dyn Fn() -> bool,
        provider: Option<&dyn CodeIntelligenceProvider>,
    ) -> Option<Value> {
        if !matches!(capability,
            "code_search" | "repo_map" | "symbol_search" | "implementation_search"
            | "workspace_symbols" | "code_navigation" | "call_graph"
            | "file_dependencies" | "read_symbol"
        ) {
            return None;
        }
        let raw = arguments.get("path").or_else(|| arguments.get("root"))
            .and_then(Value::as_str).unwrap_or(".");
        let path = PathBuf::from(raw);
        let path = if path.is_absolute() { path } else { working_directory.join(path) };
        // Invalid paths and single-file operations retain their own validation.
        let root = path.canonicalize().ok().filter(|path| path.is_dir())?;
        // Platforms without a strong metadata change identity retain the
        // conservative per-turn refresh contract rather than trusting mtime.
        #[cfg(not(unix))]
        if self.ready_roots.iter().any(|ready| root.starts_with(ready)) {
            return None;
        }
        if let Some(provider) = provider {
            match provider.current_index(&root, cancelled) {
                Ok(Some(_)) => return None,
                Ok(None) => {}
                Err(error) => return Some(serde_json::json!({
                    "error": "code_memory_freshness_failed",
                    "message": error, "tool_execution_skipped": true,
                })),
            }
        } else if self.ready_roots.iter().any(|ready| root.starts_with(ready)) {
            return None;
        }
        let assessment = workspace_initialization_assessment(&root, cancelled);
        if assessment["state"] == "optional" {
            return None;
        }
        Some(serde_json::json!({
            "error": if assessment["state"] == "assessment_failed" {
                "code_memory_assessment_failed"
            } else {
                "code_memory_initialization_required"
            },
            "assessment": assessment,
            "tool_execution_skipped": true,
            "message": "This repository operation has not run. Initialize the concrete scope through codebase_index, then retry or use indexed architecture/symbol queries. Do not repeat the blocked operation without completing initialization. Initialization is subject to normal tool authorization.",
            "recovery": {
                "capability_id": "codebase_index",
                "arguments": {"repo_path": root},
                "discovery": {
                    "capability_id": "tool_search",
                    "arguments": {"query": "select:codebase_index"}
                }
            }
        }))
    }

    pub(crate) fn observe(&mut self, operation: CodeIntelligenceOperation, output: &CodeIntelligenceOutput) {
        if output.is_error {
            if operation == CodeIntelligenceOperation::Index {
                self.ready_roots.clear();
            }
            return;
        }
        match operation {
            CodeIntelligenceOperation::Index => {
                if output.value.get("generation").and_then(Value::as_i64).is_some_and(|g| g > 0) {
                    if let Some(root) = output.value.get("repository_root").and_then(Value::as_str)
                        .and_then(|root| Path::new(root).canonicalize().ok()) {
                        self.ready_roots.insert(root);
                    }
                }
            }
            CodeIntelligenceOperation::DeleteIndex => self.ready_roots.clear(),
            _ => {}
        }
    }
}

/// The decision is based on observed source files, not prompt classification.
/// Index creation remains a concrete tool call through Core's permission path.
pub(crate) fn workspace_initialization_assessment(
    root: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Value {
    match nola_code_memory::NativeCodeMemory::code_file_count(root, cancelled) {
        Ok(count) => serde_json::json!({
            "state": if count > DEFAULT_CODE_MEMORY_FILE_THRESHOLD {
                "initialization_required"
            } else {
                "optional"
            },
            "repo_path": root,
            "code_file_count": count,
            "default_enabled": count > DEFAULT_CODE_MEMORY_FILE_THRESHOLD,
            "threshold_exclusive": DEFAULT_CODE_MEMORY_FILE_THRESHOLD,
            "guidance": if count > DEFAULT_CODE_MEMORY_FILE_THRESHOLD {
                "Before repository code analysis or changes, initialize or incrementally refresh code memory using codebase_index for this repo_path. Discover the tool with tool_search if deferred. Then use indexed architecture and symbols instead of repeatedly guessing names with text searches. If initialization fails or is unavailable, report that state and use bounded direct inspection; do not claim the index is ready. Ordinary non-code conversation does not require indexing."
            } else {
                "Direct source inspection is appropriate. Code memory remains available when useful; initialization is not required by project size."
            }
        }),
        Err(error) => serde_json::json!({
            "state": "assessment_failed",
            "repo_path": root,
            "error": error,
            "guidance": "The source-file count is unknown; do not treat this as an empty or small project."
        }),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CodeIntelligenceToolSpec {
    pub capability_id: String,
    pub title: String,
    pub description: Option<String>,
    pub input_schema: Value,
    pub read_only: bool,
}

/// One transport-free request into the native engine. Paths have already been
/// bounded by the Host; the provider must not expand this workspace set.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CodeIntelligenceCall {
    pub operation: CodeIntelligenceOperation,
    pub arguments: Value,
    pub workspace_roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CodeIntelligenceOutput {
    pub value: Value,
    pub is_error: bool,
}

impl CodeIntelligenceOutput {
    pub(crate) fn success(value: Value) -> Self {
        Self {
            value,
            is_error: false,
        }
    }

    pub(crate) fn failure(value: Value) -> Self {
        Self {
            value,
            is_error: true,
        }
    }
}

/// Implemented by the Rust `nola-code-memory` crate. Keeping this interface in
/// the Host prevents engine storage and parser choices from leaking into Core
/// or being represented as an MCP connection.
pub(crate) trait CodeIntelligenceProvider: Send + Sync {
    /// Defaults to external/unknown; only the audited in-process implementation opts in.
    fn process_bound(&self) -> bool { false }
    fn current_index(&self, root: &Path, cancelled: &dyn Fn() -> bool) -> Result<Option<Value>, String>;
    fn execute(
        &self,
        call: CodeIntelligenceCall,
        cancellation_requested: &(dyn Fn() -> bool + Send + Sync),
    ) -> CodeIntelligenceOutput;
}

pub(crate) type SharedCodeIntelligenceProvider = Arc<dyn CodeIntelligenceProvider>;

pub(crate) struct NolaCodeMemoryProvider {
    engine: nola_code_memory::NativeCodeMemory,
}

impl NolaCodeMemoryProvider {
    pub(crate) fn open(database_path: impl AsRef<Path>) -> Result<Self, String> {
        Ok(Self {
            engine: nola_code_memory::NativeCodeMemory::open(database_path)?,
        })
    }

    pub(crate) fn open_for_application() -> Result<Self, String> {
        let application_database = crate::platform_paths::application_database_path()?;
        Self::open(code_memory_database_path(&application_database)?)
    }

    fn allowed_roots(call: &CodeIntelligenceCall) -> Vec<PathBuf> {
        call.workspace_roots
            .iter()
            .filter_map(|root| root.canonicalize().ok())
            .collect()
    }

    fn path_is_allowed(path: &Path, allowed_roots: &[PathBuf]) -> bool {
        path.canonicalize()
            .ok()
            .is_some_and(|path| allowed_roots.iter().any(|root| path.starts_with(root)))
    }

    fn project_is_allowed(&self, project: &str, allowed_roots: &[PathBuf]) -> Result<bool, String> {
        Ok(self
            .engine
            .list_projects()?
            .into_iter()
            .find(|candidate| candidate.name == project)
            .is_some_and(|candidate| {
                Self::path_is_allowed(Path::new(&candidate.repository_root), allowed_roots)
            }))
    }
}

fn code_memory_database_path(application_database: &Path) -> Result<PathBuf, String> {
    let state_root = application_database
        .parent()
        .ok_or_else(|| "application database has no parent directory".to_string())?;
    Ok(state_root
        .join("CodeIntelligence")
        .join("code-memory.sqlite3"))
}

pub(crate) fn shared_for_application() -> Result<SharedCodeIntelligenceProvider, String> {
    static PROVIDER: OnceLock<Mutex<Option<Arc<NolaCodeMemoryProvider>>>> = OnceLock::new();
    let cache = PROVIDER.get_or_init(|| Mutex::new(None));
    let provider = get_or_open_cached(cache, NolaCodeMemoryProvider::open_for_application)?;
    Ok(provider as SharedCodeIntelligenceProvider)
}

fn get_or_open_cached<T>(
    cache: &Mutex<Option<Arc<T>>>,
    open: impl FnOnce() -> Result<T, String>,
) -> Result<Arc<T>, String> {
    let mut guard = cache
        .lock()
        .map_err(|_| "code-intelligence provider cache is unavailable".to_string())?;
    if let Some(provider) = guard.as_ref() {
        return Ok(Arc::clone(provider));
    }
    // Opening can fail transiently while the application state directory is
    // being created or migrated. Cache only a usable provider so one early
    // failure cannot remove Code Memory tools for the lifetime of the Host.
    let provider = Arc::new(open()?);
    *guard = Some(Arc::clone(&provider));
    Ok(provider)
}

impl CodeIntelligenceProvider for NolaCodeMemoryProvider {
    fn process_bound(&self) -> bool { true }
    fn current_index(&self, root: &Path, cancelled: &dyn Fn() -> bool) -> Result<Option<Value>, String> {
        let mut projects = self.engine.list_projects_with_cancellation(cancelled)?;
        projects.sort_by_key(|project| std::cmp::Reverse(project.repository_root.len()));
        for project in projects {
            let indexed_root = Path::new(&project.repository_root);
            if root.starts_with(indexed_root) {
                if let Some(report) = self.engine.current_index(indexed_root, cancelled)? {
                    return serde_json::to_value(report).map(Some).map_err(|error| error.to_string());
                }
            }
        }
        Ok(None)
    }

    fn execute(
        &self,
        call: CodeIntelligenceCall,
        cancellation_requested: &(dyn Fn() -> bool + Send + Sync),
    ) -> CodeIntelligenceOutput {
        if cancellation_requested() {
            return CodeIntelligenceOutput::failure(serde_json::json!({
                "error": "code_intelligence_cancelled",
                "message": "Code-intelligence execution was cancelled before it started."
            }));
        }
        let allowed_roots = Self::allowed_roots(&call);
        if allowed_roots.is_empty() {
            return CodeIntelligenceOutput::failure(serde_json::json!({
                "error": "code_intelligence_workspace_unavailable",
                "message": "No canonical Host workspace is available to the native provider."
            }));
        }
        let provider_tool = match provider_tool_name(call.operation) {
            Some(name) => name,
            None => {
                return CodeIntelligenceOutput::failure(serde_json::json!({
                    "error": "code_intelligence_operation_unsupported",
                    "operation": call.operation,
                    "message": "The native Rust engine does not implement this operation yet."
                }))
            }
        };
        let mut arguments = call.arguments;
        if call.operation == CodeIntelligenceOperation::Index {
            let Some(repository_root) = arguments.get("repo_path").and_then(Value::as_str) else {
                return CodeIntelligenceOutput::failure(serde_json::json!({
                    "error": "code_intelligence_input_invalid",
                    "message": "repo_path is required"
                }));
            };
            let candidate = PathBuf::from(repository_root);
            let candidate = if candidate.is_absolute() {
                candidate
            } else {
                allowed_roots[0].join(candidate)
            };
            let Ok(repository_root) = candidate.canonicalize() else {
                return CodeIntelligenceOutput::failure(serde_json::json!({
                    "error": "code_intelligence_workspace_unavailable",
                    "message": "repo_path does not resolve to an existing workspace directory"
                }));
            };
            if !allowed_roots
                .iter()
                .any(|root| repository_root.starts_with(root))
            {
                return CodeIntelligenceOutput::failure(serde_json::json!({
                    "error": "code_intelligence_workspace_denied",
                    "message": "repo_path is outside the Host workspace boundary"
                }));
            }
            if let Some(object) = arguments.as_object_mut() {
                object.insert(
                    "repo_path".to_string(),
                    Value::String(repository_root.to_string_lossy().into_owned()),
                );
            }
        } else if !matches!(
            call.operation,
            CodeIntelligenceOperation::Projects | CodeIntelligenceOperation::Schema
        ) {
            let Some(project) = arguments.get("project").and_then(Value::as_str) else {
                return CodeIntelligenceOutput::failure(serde_json::json!({
                    "error": "code_intelligence_input_invalid",
                    "message": "project is required"
                }));
            };
            match self.project_is_allowed(project, &allowed_roots) {
                Ok(true) => {}
                Ok(false) => {
                    return CodeIntelligenceOutput::failure(serde_json::json!({
                        "error": "code_intelligence_project_unavailable",
                        "message": "The project is not indexed in an allowed Host workspace.",
                        "recovery": {
                            "capability_id": CodeIntelligenceOperation::Index.capability_id(),
                            "arguments": {
                                "repo_path": allowed_roots[0].to_string_lossy(),
                                "project": project
                            }
                        }
                    }))
                }
                Err(error) => return native_error(call.operation, error),
            }
        }
        let result = if call.operation == CodeIntelligenceOperation::Projects {
            self.engine.list_projects().and_then(|projects| {
                serde_json::to_value(
                    projects
                        .into_iter()
                        .filter(|project| {
                            Self::path_is_allowed(
                                Path::new(&project.repository_root),
                                &allowed_roots,
                            )
                        })
                        .collect::<Vec<_>>(),
                )
                .map_err(|error| error.to_string())
            })
        } else {
            // Preserve the stable Host schema while adapting the initial Rust
            // engine's internal pagination field.
            if call.operation == CodeIntelligenceOperation::TextSearch {
                if let Some(limit) = arguments.get("limit").cloned() {
                    if let Some(object) = arguments.as_object_mut() {
                        object.entry("max_results".to_string()).or_insert(limit);
                    }
                }
            }
            self.engine
                .call_with_cancellation(provider_tool, &arguments, cancellation_requested)
        };
        match result {
            Ok(value) => CodeIntelligenceOutput::success(value),
            Err(error) => native_error(call.operation, error),
        }
    }
}

fn native_error(operation: CodeIntelligenceOperation, message: String) -> CodeIntelligenceOutput {
    CodeIntelligenceOutput::failure(serde_json::json!({
        "error": "code_intelligence_failed",
        "operation": operation,
        "message": message,
    }))
}

fn provider_tool_name(operation: CodeIntelligenceOperation) -> Option<&'static str> {
    Some(match operation {
        CodeIntelligenceOperation::Index => "index_repository",
        CodeIntelligenceOperation::IndexStatus => "index_status",
        CodeIntelligenceOperation::Projects => "list_projects",
        CodeIntelligenceOperation::Architecture => "get_architecture",
        CodeIntelligenceOperation::Search => "search_graph",
        CodeIntelligenceOperation::TextSearch => "search_code",
        CodeIntelligenceOperation::Trace => "trace_path",
        CodeIntelligenceOperation::ReadSymbol => "get_code_snippet",
        CodeIntelligenceOperation::Schema => "get_graph_schema",
        CodeIntelligenceOperation::Coverage => "check_index_coverage",
        CodeIntelligenceOperation::DeleteIndex => "delete_project",
        CodeIntelligenceOperation::Query
        | CodeIntelligenceOperation::ChangedImpact
        | CodeIntelligenceOperation::ArchitectureDecision
        | CodeIntelligenceOperation::IngestTraces => return None,
    })
}

pub(crate) fn tool_specs() -> Vec<CodeIntelligenceToolSpec> {
    let string = || serde_json::json!({"type": "string"});
    let positive_integer = || serde_json::json!({"type": "integer", "minimum": 1});
    let non_negative_integer = || serde_json::json!({"type": "integer", "minimum": 0});
    vec![
        native_spec(
            CodeIntelligenceOperation::Index,
            "Index repository",
            "Build or incrementally refresh Nola's native code graph for an allowed workspace. Use this before codebase search, architecture, trace, or symbol reads when the project has not been indexed in the current application state.",
            serde_json::json!({"repo_path": string(), "project": string()}),
            &["repo_path"],
        ),
        native_spec(
            CodeIntelligenceOperation::Projects,
            "List indexed projects",
            "List native code indexes whose repository roots are available to this Host.",
            serde_json::json!({}),
            &[],
        ),
        native_spec(
            CodeIntelligenceOperation::IndexStatus,
            "Inspect code index status",
            "Report the generation, file count, symbol count, and incomplete-file count for one native code index.",
            serde_json::json!({"project": string()}),
            &["project"],
        ),
        native_spec(
            CodeIntelligenceOperation::Search,
            "Search indexed symbols",
            "Search repository-scale indexed symbol names and qualified names with stable pagination; results include kind and source path. Prefer this for locating definitions across many files after codebase_index.",
            serde_json::json!({
                "project": string(), "query": string(),
                "limit": positive_integer(), "offset": non_negative_integer()
            }),
            &["project", "query"],
        ),
        native_spec(
            CodeIntelligenceOperation::TextSearch,
            "Search indexed source text",
            "Search literal or regular-expression source text in indexed files with stable pagination.",
            serde_json::json!({
                "project": string(), "pattern": string(),
                "regex": {"type": "boolean", "default": false},
                "limit": positive_integer(), "offset": non_negative_integer()
            }),
            &["project", "pattern"],
        ),
        native_spec(
            CodeIntelligenceOperation::Trace,
            "Trace indexed calls",
            "Trace resolved CALLS edges across files from callers, callees, or both directions to a bounded depth after codebase_index.",
            serde_json::json!({
                "project": string(), "function_name": string(),
                "direction": {"type": "string", "enum": ["callers", "callees", "both"]},
                "max_depth": positive_integer()
            }),
            &["project", "function_name"],
        ),
        native_spec(
            CodeIntelligenceOperation::ReadSymbol,
            "Read indexed symbol",
            "Read current source for an exact qualified symbol returned by codebase_search or codebase_trace.",
            serde_json::json!({
                "project": string(), "qualified_name": string(),
                "context_lines": non_negative_integer()
            }),
            &["project", "qualified_name"],
        ),
        native_spec(
            CodeIntelligenceOperation::Architecture,
            "Summarize code architecture",
            "Summarize repository-scale indexed languages, directories, files, and symbol kinds after codebase_index.",
            serde_json::json!({"project": string()}),
            &["project"],
        ),
        native_spec(
            CodeIntelligenceOperation::Schema,
            "Describe code graph schema",
            "Return the node kinds, edge kinds, and call-edge properties implemented by Nola's native code graph.",
            serde_json::json!({}),
            &[],
        ),
        native_spec(
            CodeIntelligenceOperation::Coverage,
            "Inspect index coverage",
            "List files whose native parsing or indexing coverage is incomplete.",
            serde_json::json!({"project": string()}),
            &["project"],
        ),
        native_spec(
            CodeIntelligenceOperation::DeleteIndex,
            "Delete code index",
            "Delete Nola's native index for one project without changing repository files.",
            serde_json::json!({"project": string()}),
            &["project"],
        ),
    ]
}

fn native_spec(
    operation: CodeIntelligenceOperation,
    title: &str,
    description: &str,
    properties: Value,
    required: &[&str],
) -> CodeIntelligenceToolSpec {
    CodeIntelligenceToolSpec {
        capability_id: operation.capability_id().to_string(),
        title: title.to_string(),
        description: Some(description.to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        }),
        read_only: operation.is_read_only(),
    }
}

/// Shared CLI/Web availability projection; registration is not an index operation.
pub(crate) fn available_model_tool_schemas() -> Vec<Value> {
    shared_for_application().is_ok().then(model_tool_schemas).unwrap_or_default()
}

pub(crate) fn model_tool_schemas() -> Vec<Value> {
    tool_specs()
        .into_iter()
        .map(|spec| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": spec.capability_id,
                    "description": spec.description.unwrap_or(spec.title),
                    "parameters": spec.input_schema,
                }
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_core::code_intelligence::operation_for_capability_id;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn initialization_default_uses_actual_code_count_and_strict_threshold() {
        let directory = tempfile::tempdir().unwrap();
        let empty = workspace_initialization_assessment(directory.path(), &|| false);
        assert_eq!(empty["code_file_count"], 0);
        assert_eq!(empty["state"], "optional");
        for index in 0..20 {
            std::fs::write(directory.path().join(format!("source_{index}.rs")), "fn f() {}").unwrap();
        }
        // Non-code and ignored code must not push a 20-file project over.
        std::fs::write(directory.path().join("notes.md"), "notes").unwrap();
        std::fs::write(directory.path().join("settings.json"), "{}").unwrap();
        std::fs::write(directory.path().join("settings.yaml"), "enabled: true").unwrap();
        std::fs::write(directory.path().join(".gitignore"), "ignored.rs\n").unwrap();
        std::fs::write(directory.path().join("ignored.rs"), "fn ignored() {}").unwrap();
        let small = workspace_initialization_assessment(directory.path(), &|| false);
        assert_eq!(small["code_file_count"], 20);
        assert_eq!(small["default_enabled"], false);
        assert_eq!(small["state"], "optional");
        let mut initialization = CodeMemoryInitialization::default();
        let input = serde_json::json!({"root": directory.path(), "query": "当前输入栏 current search"});
        assert!(initialization.preflight("code_search", &input, directory.path(), &|| false, None).is_none());
        std::fs::write(directory.path().join("another.py"), "def f(): pass").unwrap();
        let large = workspace_initialization_assessment(directory.path(), &|| false);
        assert_eq!(large["code_file_count"], 21);
        assert_eq!(large["default_enabled"], true);
        assert_eq!(large["state"], "initialization_required");
        assert_eq!(initialization.preflight("code_search", &input, directory.path(), &|| false, None)
            .unwrap()["error"], "code_memory_initialization_required");
        assert!(initialization.preflight("unknown_tool", &input, directory.path(), &|| false, None).is_none());
        assert!(initialization.preflight("bash", &input, directory.path(), &|| false, None).is_none());
        initialization.observe(CodeIntelligenceOperation::Index,
            &CodeIntelligenceOutput::failure(serde_json::json!({"error":"cancelled"})));
        assert!(initialization.preflight("code_search", &input, directory.path(), &|| false, None).is_some());
        initialization.observe(CodeIntelligenceOperation::Index,
            &CodeIntelligenceOutput::success(serde_json::json!({
                "repository_root":directory.path(),"generation":0
            })));
        assert!(initialization.preflight("code_search", &input, directory.path(), &|| false, None).is_some());
        initialization.observe(CodeIntelligenceOperation::Index,
            &CodeIntelligenceOutput::success(serde_json::json!({
                "repository_root":directory.path(),"generation":1
            })));
        assert!(initialization.preflight("code_search", &input, directory.path(), &|| false, None).is_none());
        initialization.observe(CodeIntelligenceOperation::Index,
            &CodeIntelligenceOutput::failure(serde_json::json!({"error":"refresh_failed"})));
        assert!(initialization.preflight("code_search", &input, directory.path(), &|| false, None).is_some());
        initialization.observe(CodeIntelligenceOperation::Index,
            &CodeIntelligenceOutput::success(serde_json::json!({
                "repository_root":directory.path(),"generation":2
            })));
        initialization.observe(CodeIntelligenceOperation::DeleteIndex, &CodeIntelligenceOutput::success(Value::Bool(true)));
        assert!(initialization.preflight("code_search", &input, directory.path(), &|| false, None).is_some());
        let cancelled = workspace_initialization_assessment(directory.path(), &|| true);
        assert_eq!(cancelled["state"], "assessment_failed");
        assert!(cancelled.get("code_file_count").is_none());
        let unavailable = workspace_initialization_assessment(
            &directory.path().join("missing"), &|| false,
        );
        assert_eq!(unavailable["state"], "assessment_failed");
        assert!(unavailable.get("default_enabled").is_none());
    }

    #[test]
    fn provider_cache_retries_after_transient_open_failure() {
        let cache = Mutex::new(None::<Arc<usize>>);
        let attempts = AtomicUsize::new(0);
        let first = get_or_open_cached(&cache, || {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err("state directory is not ready".to_string())
        });
        assert!(first.is_err());
        assert!(cache.lock().expect("cache").is_none());

        let second = get_or_open_cached(&cache, || {
            attempts.fetch_add(1, Ordering::SeqCst);
            Ok(41)
        })
        .expect("second open should recover");
        let third = get_or_open_cached(&cache, || {
            attempts.fetch_add(1, Ordering::SeqCst);
            Ok(99)
        })
        .expect("successful provider should be reused");

        assert_eq!(*second, 41);
        assert!(Arc::ptr_eq(&second, &third));
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn native_catalog_exposes_only_core_owned_operations() {
        let specs = tool_specs();
        assert_eq!(specs.len(), 11);
        assert!(specs
            .iter()
            .any(|spec| spec.capability_id == "codebase_index"));
        assert!(specs
            .iter()
            .any(|spec| spec.capability_id == "codebase_search"));
        assert!(specs.iter().all(|spec| {
            operation_for_capability_id(&spec.capability_id).is_some()
                && spec.input_schema.get("type").and_then(Value::as_str) == Some("object")
        }));
        let search = specs
            .iter()
            .find(|spec| spec.capability_id == "codebase_search")
            .expect("symbol search");
        assert_eq!(
            search.input_schema["required"],
            serde_json::json!(["project", "query"])
        );
        assert!(search.input_schema["properties"]
            .get("name_pattern")
            .is_none());
        let index = specs
            .iter()
            .find(|spec| spec.capability_id == "codebase_index")
            .expect("repository index");
        assert!(index.input_schema["properties"].get("project").is_some());
        assert!(index.input_schema["properties"].get("name").is_none());
    }

    #[test]
    fn native_catalog_uses_core_effect_semantics() {
        let specs = tool_specs();
        assert!(
            !specs
                .iter()
                .find(|spec| spec.capability_id == "codebase_index")
                .expect("index")
                .read_only
        );
        assert!(
            specs
                .iter()
                .find(|spec| spec.capability_id == "codebase_read_symbol")
                .expect("read symbol")
                .read_only
        );
    }

    #[test]
    fn every_available_native_operation_has_one_model_schema() {
        let specs = tool_specs();
        let schemas = model_tool_schemas();
        let mut spec_ids = specs
            .iter()
            .map(|spec| spec.capability_id.as_str())
            .collect::<Vec<_>>();
        let mut schema_ids = schemas
            .iter()
            .filter_map(|schema| schema.pointer("/function/name").and_then(Value::as_str))
            .collect::<Vec<_>>();
        spec_ids.sort_unstable();
        schema_ids.sort_unstable();
        assert_eq!(schema_ids, spec_ids);
        assert_eq!(schema_ids.len(), 11);
    }

    #[test]
    fn native_provider_indexes_and_searches_without_a_transport() {
        let workspace = tempfile::tempdir().expect("workspace");
        std::fs::write(
            workspace.path().join("lib.rs"),
            "pub fn orbital_signal_processor() -> u64 { 73 }\n",
        )
        .expect("fixture source");
        let provider = NolaCodeMemoryProvider {
            engine: nola_code_memory::NativeCodeMemory::open_in_memory().expect("engine"),
        };
        let never_cancelled = || false;
        let indexed = provider.execute(
            CodeIntelligenceCall {
                operation: CodeIntelligenceOperation::Index,
                arguments: serde_json::json!({
                    "repo_path": workspace.path(),
                    "project": "fixture"
                }),
                workspace_roots: vec![workspace.path().to_path_buf()],
            },
            &never_cancelled,
        );
        assert!(!indexed.is_error, "{}", indexed.value);

        let searched = provider.execute(
            CodeIntelligenceCall {
                operation: CodeIntelligenceOperation::Search,
                arguments: serde_json::json!({
                    "project": "fixture",
                    "query": "orbital signal",
                    "limit": 10
                }),
                workspace_roots: vec![workspace.path().to_path_buf()],
            },
            &never_cancelled,
        );
        assert!(!searched.is_error, "{}", searched.value);
        assert!(searched
            .value
            .to_string()
            .contains("orbital_signal_processor"));
    }

    #[test]
    fn unavailable_project_returns_an_executable_index_recovery() {
        let workspace = tempfile::tempdir().expect("workspace");
        let provider = NolaCodeMemoryProvider {
            engine: nola_code_memory::NativeCodeMemory::open_in_memory().expect("engine"),
        };
        let outcome = provider.execute(
            CodeIntelligenceCall {
                operation: CodeIntelligenceOperation::Search,
                arguments: serde_json::json!({
                    "project": "workspace-project",
                    "query": "entry point"
                }),
                workspace_roots: vec![workspace.path().to_path_buf()],
            },
            &|| false,
        );

        assert!(outcome.is_error);
        assert_eq!(
            outcome.value["recovery"]["capability_id"],
            CodeIntelligenceOperation::Index.capability_id()
        );
        assert_eq!(
            outcome.value["recovery"]["arguments"]["project"],
            "workspace-project"
        );
        assert_eq!(
            outcome.value["recovery"]["arguments"]["repo_path"],
            workspace
                .path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .as_ref()
        );
    }

    #[test]
    fn native_provider_enforces_the_host_workspace_boundary() {
        let allowed = tempfile::tempdir().expect("allowed workspace");
        let outside = tempfile::tempdir().expect("outside workspace");
        let provider = NolaCodeMemoryProvider {
            engine: nola_code_memory::NativeCodeMemory::open_in_memory().expect("engine"),
        };
        let outcome = provider.execute(
            CodeIntelligenceCall {
                operation: CodeIntelligenceOperation::Index,
                arguments: serde_json::json!({"repo_path": outside.path()}),
                workspace_roots: vec![allowed.path().to_path_buf()],
            },
            &|| false,
        );
        assert!(outcome.is_error);
        assert_eq!(
            outcome.value.get("error").and_then(Value::as_str),
            Some("code_intelligence_workspace_denied")
        );
    }

    #[test]
    fn native_database_lives_beside_application_state() {
        assert_eq!(
            code_memory_database_path(Path::new("/var/lib/nola/nola.db")).unwrap(),
            PathBuf::from("/var/lib/nola/CodeIntelligence/code-memory.sqlite3")
        );
    }
}

#[cfg(test)]
#[test]
fn native_code_memory_policy_metadata_matches_typed_operations() {
    for spec in tool_specs() {
        let operation = nola_core::code_intelligence::operation_for_capability_id(&spec.capability_id).unwrap();
        let metadata = nola_core::tool_catalog::builtin_tool_metadata(&spec.capability_id).unwrap();
        assert_eq!(metadata.capability, if operation.is_read_only() { "files_read" } else { "code_index" });
    }
}
