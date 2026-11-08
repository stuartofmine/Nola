// SPDX-License-Identifier: Apache-2.0

use crate::extractor::{
    extract, module_identity, supported_language, ExtractedReference, ExtractedSymbol,
    SEMANTIC_LANGUAGES, SUPPORTED_GRAMMARS,
};
use crate::schema;
use crate::types::{
    ArchitectureSummary, CoverageReport, FileCoverage, IndexReport, IndexStatus, ProjectInfo,
    SearchHit, SearchPage, Snippet, TraceEdge, TraceResult,
};
use ignore::WalkBuilder;
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_SOURCE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_SEARCH_LIMIT: usize = 200;
const MAX_CACHE_ENTRIES: usize = 128;

#[path = "incremental.rs"]
mod incremental;
use incremental::EdgeChanges;
#[path = "freshness.rs"]
mod freshness;
use freshness::WorkspaceSnapshot;

struct PreparedFile {
    rel_path: String,
    modified_ns: i64,
    byte_count: i64,
    outcome: PreparedOutcome,
}

enum PreparedOutcome {
    CoverageOnly {
        language: &'static str,
        kind: &'static str,
        detail: String,
    },
    Unchanged,
    Changed {
        language: &'static str,
        digest: String,
        symbols: Vec<ExtractedSymbol>,
        references: Vec<ExtractedReference>,
        coverage_kind: Option<&'static str>,
        coverage_detail: String,
    },
}

/// A process-local handle to the durable code index.
///
/// One connection and one mutex intentionally serialize writes per database.
/// SQLite WAL keeps separately opened read handles viable for future Host
/// adapters without exposing partial generations.
pub struct NativeCodeMemory {
    connection: Mutex<Connection>,
    search_cache: Mutex<HashMap<String, (i64, i64, SearchPage)>>,
    snapshots: Mutex<HashMap<PathBuf, WorkspaceSnapshot>>,
}

impl NativeCodeMemory {
    /// Metadata-only resource inventory for Host permission resolution.
    pub fn index_file_paths(root: &Path, cancelled: &dyn Fn() -> bool) -> Result<Vec<PathBuf>, String> {
        if !root.is_dir() { return Err("code memory workspace is not a directory".into()); }
        discover(root, cancelled)
    }

    /// Metadata-only assessment using the indexer's discovery rules, excluding
    /// documentation and data/config formats which it also indexes.
    /// Does not open a database or read source contents.
    pub fn code_file_count(
        root: &Path,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<usize, String> {
        if !root.is_dir() {
            return Err("code memory workspace is not a directory".into());
        }
        Ok(discover(root, cancelled)?.into_iter().filter(|path| {
            supported_language(path).is_some_and(|language| !matches!(
                language,
                "asciidoc" | "markdown" | "jsdoc" | "diff" | "dot"
                    | "gitattributes" | "ini" | "json" | "ssh-config"
                    | "textproto" | "toml" | "xml" | "yaml"
            ))
        }).count())
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create index directory: {error}"))?;
        }
        let connection = Connection::open(path).map_err(db_error)?;
        schema::initialize(&connection).map_err(db_error)?;
        Ok(Self {
            connection: Mutex::new(connection),
            search_cache: Mutex::new(HashMap::new()),
            snapshots: Mutex::new(HashMap::new()),
        })
    }

    pub fn open_in_memory() -> Result<Self, String> {
        let connection = Connection::open_in_memory().map_err(db_error)?;
        schema::initialize(&connection).map_err(db_error)?;
        Ok(Self {
            connection: Mutex::new(connection),
            search_cache: Mutex::new(HashMap::new()),
            snapshots: Mutex::new(HashMap::new()),
        })
    }

    pub fn index(
        &self,
        repository_root: impl AsRef<Path>,
        project_name: Option<&str>,
    ) -> Result<IndexReport, String> {
        self.index_with_cancellation(repository_root, project_name, &|| false)
    }

    /// Readiness is scoped to a committed generation and current file
    /// identities, not a conversation turn. This reads metadata, not sources.
    pub fn current_index(&self, root: &Path, cancelled: &dyn Fn() -> bool) -> Result<Option<IndexReport>, String> {
        let root = root.canonicalize().map_err(|error| error.to_string())?;
        let connection = self.lock_connection_cancellable(cancelled)?;
        let snapshot = self.snapshots.lock().map_err(|_| "code snapshot lock poisoned")?.get(&root).cloned();
        let Some(snapshot) = snapshot else { return Ok(None); };
        let generation: Option<i64> = connection.query_row(
            "SELECT generation FROM projects WHERE name=?1 AND root_path=?2",
            params![snapshot.report.project, root.to_string_lossy()], |row| row.get(0)
        ).optional().map_err(db_error)?;
        if generation != Some(snapshot.report.generation) { return Ok(None); }
        let paths = discover(&root, cancelled)?;
        let identities = freshness::stamps(&paths);
        check_cancelled(cancelled)?;
        Ok((paths == snapshot.paths && identities.len() == paths.len()
            && identities == snapshot.stamps).then_some(snapshot.report))
    }

    pub fn index_with_cancellation(
        &self,
        repository_root: impl AsRef<Path>,
        project_name: Option<&str>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<IndexReport, String> {
        let root = repository_root
            .as_ref()
            .canonicalize()
            .map_err(|error| format!("resolve repository root: {error}"))?;
        if !root.is_dir() {
            return Err("repository root is not a directory".to_string());
        }
        let project = project_name
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| {
                root.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .ok_or_else(|| "repository root has no project name".to_string())?;
        let files = discover(&root, cancelled)?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "code index lock is poisoned".to_string())?;
        let transaction = connection.transaction().map_err(db_error)?;
        let project_id = ensure_project(&transaction, &project, &root)?;
        let previous_generation: i64 = transaction
            .query_row(
                "SELECT generation FROM projects WHERE id=?1",
                [project_id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        let generation = previous_generation + 1;
        let identities = freshness::stamps(&files);
        let snapshot = self.snapshots.lock().map_err(|_| "code snapshot lock poisoned")?.get(&root).cloned();
        let fast_unchanged: HashSet<PathBuf> = snapshot.as_ref()
            .filter(|snapshot| snapshot.report.generation == previous_generation
                && snapshot.report.project == project)
            .map(|snapshot| identities.iter().filter_map(|(path, identity)|
                (snapshot.stamps.get(path) == Some(identity)).then_some(path.clone())).collect())
            .unwrap_or_default();
        if let Some(snapshot) = snapshot.as_ref().filter(|snapshot|
            previous_generation > 0 && snapshot.report.generation == previous_generation
                && snapshot.report.project == project && snapshot.paths == files
                && fast_unchanged.len() == files.len())
        {
            check_cancelled(cancelled)?;
            let mut report = snapshot.report.clone();
            report.indexed_files = 0;
            report.unchanged_files = files.len();
            report.removed_files = 0;
            return Ok(report);
        }
        ensure_project_node(&transaction, project_id, &project)?;
        let mut known = load_file_hashes(&transaction, project_id)?;
        let mut indexed_files = 0;
        let mut unchanged_files = 0;
        let mut graph_changed = previous_generation == 0;
        let mut edge_changes = EdgeChanges::default();

        // Reading, hashing, and parsing are independent per file. Keep SQLite
        // publication single-threaded below so readers still observe one
        // complete generation, while a bounded standard-library worker pool
        // performs CPU-heavy grammar work.
        let prepared = prepare_files_parallel(&root, &files, &known, &fast_unchanged, cancelled)?;
        if freshness::stamps(&files) != identities {
            return Err("code_memory_source_changed_during_index: retry with current sources".into());
        }

        for prepared_file in prepared {
            known.remove(&prepared_file.rel_path);
            match prepared_file.outcome {
                PreparedOutcome::CoverageOnly {
                    language,
                    kind,
                    detail,
                } => {
                    edge_changes.record_file(&transaction, project_id, &prepared_file.rel_path)?;
                    upsert_coverage_only(
                        &transaction,
                        project_id,
                        &project,
                        &prepared_file.rel_path,
                        language,
                        prepared_file.modified_ns,
                        prepared_file.byte_count,
                        generation,
                        kind,
                        &detail,
                    )?;
                    edge_changes.record_file(&transaction, project_id, &prepared_file.rel_path)?;
                    graph_changed = true;
                }
                PreparedOutcome::Unchanged => {
                    transaction
                        .execute(
                            "UPDATE files SET indexed_generation=?1,mtime_ns=?2,byte_count=?3
                             WHERE project_id=?4 AND rel_path=?5",
                            params![
                                generation,
                                prepared_file.modified_ns,
                                prepared_file.byte_count,
                                project_id,
                                prepared_file.rel_path
                            ],
                        )
                        .map_err(db_error)?;
                    unchanged_files += 1;
                }
                PreparedOutcome::Changed {
                    language,
                    digest,
                    symbols,
                    references,
                    coverage_kind,
                    coverage_detail,
                } => {
                    edge_changes.record_file(&transaction, project_id, &prepared_file.rel_path)?;
                    replace_file(
                        &transaction,
                        project_id,
                        &project,
                        &prepared_file.rel_path,
                        language,
                        &digest,
                        prepared_file.modified_ns,
                        prepared_file.byte_count,
                        generation,
                        coverage_kind,
                        &coverage_detail,
                        &symbols,
                        &references,
                    )?;
                    edge_changes.record_file(&transaction, project_id, &prepared_file.rel_path)?;
                    indexed_files += 1;
                    graph_changed = true;
                }
            }
        }

        let removed_files = known.len();
        graph_changed |= removed_files > 0;
        for rel_path in known.keys() {
            edge_changes.record_file(&transaction, project_id, rel_path)?;
            transaction
                .execute(
                    "DELETE FROM files WHERE project_id=?1 AND rel_path=?2",
                    params![project_id, rel_path],
                )
                .map_err(db_error)?;
        }
        if graph_changed {
            rebuild_edges(&transaction, project_id, &project,
                (previous_generation > 0).then_some(&edge_changes), cancelled)?;
        }
        if cancelled() {
            return Err("code_memory_cancelled".to_string());
        }
        // Reports describe the published generation, including unchanged
        // files, rather than only the rows re-extracted by this invocation.
        let symbols: usize = transaction
            .query_row(
                "SELECT COUNT(*) FROM symbols
                 WHERE project_id=?1 AND kind NOT IN ('Project','File','Module')",
                [project_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(db_error)? as usize;
        let incomplete_files: usize = transaction
            .query_row(
                "SELECT COUNT(*) FROM files
                 WHERE project_id=?1 AND coverage_kind IS NOT NULL",
                [project_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(db_error)? as usize;
        transaction
            .execute(
                "UPDATE projects SET root_path=?1,generation=?2,indexed_at_unix_ms=?3 WHERE id=?4",
                params![root.to_string_lossy(), generation, unix_ms(), project_id],
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        self.invalidate_project_cache(&project);
        let report = IndexReport {
            project,
            repository_root: root.to_string_lossy().into_owned(),
            generation,
            discovered_files: files.len(),
            indexed_files,
            unchanged_files,
            removed_files,
            symbols,
            incomplete_files,
        };
        let mut snapshots = self.snapshots.lock().map_err(|_| "code snapshot lock poisoned")?;
        if snapshots.len() >= MAX_CACHE_ENTRIES && !snapshots.contains_key(&root) {
            snapshots.clear();
        }
        snapshots.insert(root, WorkspaceSnapshot { report: report.clone(), paths: files, stamps: identities });
        Ok(report)
    }

    pub fn list_projects(&self) -> Result<Vec<ProjectInfo>, String> {
        self.list_projects_with_cancellation(&|| false)
    }

    pub fn list_projects_with_cancellation(&self, cancelled: &dyn Fn() -> bool) -> Result<Vec<ProjectInfo>, String> {
        let connection = self.lock_connection_cancellable(cancelled)?;
        let mut statement = connection
            .prepare(
                "SELECT p.name,p.root_path,p.generation,p.indexed_at_unix_ms,
                        (SELECT COUNT(*) FROM files f WHERE f.project_id=p.id),
                        (SELECT COUNT(*) FROM symbols s WHERE s.project_id=p.id
                         AND s.kind NOT IN ('Project','File','Module'))
                 FROM projects p ORDER BY p.name",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok(ProjectInfo {
                    name: row.get(0)?,
                    repository_root: row.get(1)?,
                    generation: row.get(2)?,
                    indexed_at_unix_ms: row.get(3)?,
                    file_count: row.get::<_, i64>(4)? as usize,
                    symbol_count: row.get::<_, i64>(5)? as usize,
                })
            })
            .map_err(db_error)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_error)
    }

    pub fn index_status(&self, project: &str) -> Result<IndexStatus, String> {
        let connection = self.lock_connection()?;
        connection
            .query_row(
                "SELECT p.name,p.generation,
                        (SELECT COUNT(*) FROM files f WHERE f.project_id=p.id),
                        (SELECT COUNT(*) FROM files f WHERE f.project_id=p.id
                         AND f.coverage_kind IS NOT NULL),
                        (SELECT COUNT(*) FROM symbols s WHERE s.project_id=p.id
                         AND s.kind NOT IN ('Project','File','Module'))
                 FROM projects p WHERE p.name=?1",
                [project],
                |row| {
                    Ok(IndexStatus {
                        project: row.get(0)?,
                        generation: row.get(1)?,
                        indexed_files: row.get::<_, i64>(2)? as usize,
                        incomplete_files: row.get::<_, i64>(3)? as usize,
                        symbol_count: row.get::<_, i64>(4)? as usize,
                    })
                },
            )
            .optional()
            .map_err(db_error)?
            .ok_or_else(|| format!("unknown project: {project}"))
    }

    pub fn search_symbols(
        &self,
        project: &str,
        query: &str,
        limit: usize,
        offset: usize,
    ) -> Result<SearchPage, String> {
        self.search_symbols_with_cancellation(project, query, limit, offset, &|| false)
    }

    pub fn search_symbols_with_cancellation(
        &self,
        project: &str,
        query: &str,
        limit: usize,
        offset: usize,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<SearchPage, String> {
        self.ensure_project_fresh(project, cancelled)?;
        let limit = limit.clamp(1, MAX_SEARCH_LIMIT);
        let fts_query = fts_query(query)?;
        let generation = self.index_status(project)?.generation;
        let data_version = self.data_version()?;
        check_cancelled(cancelled)?;
        let cache_key = format!("{project}\0{fts_query}\0{limit}\0{offset}");
        if let Some((cached_generation, cached_data_version, page)) = self
            .search_cache
            .lock()
            .map_err(|_| "code search cache lock is poisoned".to_string())?
            .get(&cache_key)
            .cloned()
        {
            if cached_generation == generation && cached_data_version == data_version {
                self.validate_search_hits(project, &page.results, cancelled)?;
                return Ok(page);
            }
        }
        let connection = self.lock_connection()?;
        let total: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM symbols_fts
                 JOIN symbols_search ss ON ss.symbol_id=symbols_fts.rowid
                 JOIN symbols s ON s.id=ss.symbol_id
                 JOIN files f ON f.id=s.file_id
                 JOIN projects p ON p.id=s.project_id
                 WHERE symbols_fts MATCH ?1 AND p.name=?2",
                params![fts_query, project],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        let mut statement = connection
            .prepare(
                "SELECT s.qualified_name,s.name,s.kind,f.rel_path,s.start_line,s.end_line,
                        bm25(symbols_fts,8.0,4.0,2.0,1.0)
                 FROM symbols_fts
                 JOIN symbols_search ss ON ss.symbol_id=symbols_fts.rowid
                 JOIN symbols s ON s.id=ss.symbol_id
                 JOIN files f ON f.id=s.file_id
                 JOIN projects p ON p.id=s.project_id
                 WHERE symbols_fts MATCH ?1 AND p.name=?2
                 ORDER BY 7,s.qualified_name LIMIT ?3 OFFSET ?4",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map(
                params![fts_query, project, limit as i64, offset as i64],
                |row| {
                    Ok(SearchHit {
                        qualified_name: row.get(0)?,
                        name: row.get(1)?,
                        kind: row.get(2)?,
                        path: row.get(3)?,
                        start_line: row.get::<_, i64>(4)? as usize,
                        end_line: row.get::<_, i64>(5)? as usize,
                        score: row.get(6)?,
                    })
                },
            )
            .map_err(db_error)?;
        let mut results = Vec::new();
        for row in rows {
            check_cancelled(cancelled)?;
            results.push(row.map_err(db_error)?);
        }
        drop(statement);
        drop(connection);
        self.validate_search_hits(project, &results, cancelled)?;
        let page = SearchPage {
            project: project.to_string(),
            total: total as usize,
            offset,
            has_more: offset.saturating_add(results.len()) < total as usize,
            results,
        };
        let mut cache = self
            .search_cache
            .lock()
            .map_err(|_| "code search cache lock is poisoned".to_string())?;
        if cache.len() >= MAX_CACHE_ENTRIES {
            cache.clear();
        }
        cache.insert(cache_key, (generation, data_version, page.clone()));
        Ok(page)
    }

    pub fn search_text(
        &self,
        project: &str,
        pattern: &str,
        regex: bool,
        limit: usize,
        offset: usize,
    ) -> Result<SearchPage, String> {
        self.search_text_with_cancellation(project, pattern, regex, limit, offset, &|| false)
    }

    pub fn search_text_with_cancellation(
        &self,
        project: &str,
        pattern: &str,
        regex: bool,
        limit: usize,
        offset: usize,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<SearchPage, String> {
        self.ensure_project_fresh(project, cancelled)?;
        let expression_source = if regex {
            pattern.to_string()
        } else {
            regex::escape(pattern)
        };
        let expression =
            Regex::new(&expression_source).map_err(|error| format!("invalid regex: {error}"))?;
        let (root, files) = self.project_files(project)?;
        let limit = limit.clamp(1, MAX_SEARCH_LIMIT);
        let page_end = offset.saturating_add(limit);
        let mut total = 0usize;
        let mut results = Vec::with_capacity(limit);
        for (path, digest) in files {
            check_cancelled(cancelled)?;
            let bytes = read_current_indexed_file(&root, &path, &digest)?;
            let source = String::from_utf8_lossy(&bytes);
            let mut line = 1usize;
            let mut scanned_through = 0usize;
            for found in expression.find_iter(&source) {
                check_cancelled(cancelled)?;
                line += source[scanned_through..found.start()]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count();
                scanned_through = found.start();
                if total >= offset && total < page_end {
                    results.push(SearchHit {
                        qualified_name: format!("{path}:{line}"),
                        name: found.as_str().chars().take(160).collect(),
                        kind: "Text".to_string(),
                        path: path.clone(),
                        start_line: line,
                        end_line: line,
                        score: 0.0,
                    });
                }
                total = total.saturating_add(1);
            }
        }
        Ok(SearchPage {
            project: project.to_string(),
            total,
            offset,
            has_more: offset.saturating_add(results.len()) < total,
            results,
        })
    }

    pub fn get_snippet(
        &self,
        project: &str,
        qualified_name: &str,
        context_lines: usize,
    ) -> Result<Snippet, String> {
        self.get_snippet_with_cancellation(project, qualified_name, context_lines, &|| false)
    }

    pub fn get_snippet_with_cancellation(
        &self,
        project: &str,
        qualified_name: &str,
        context_lines: usize,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Snippet, String> {
        self.ensure_project_fresh(project, cancelled)?;
        check_cancelled(cancelled)?;
        let connection = self.lock_connection()?;
        let row = connection
            .query_row(
                "SELECT p.root_path,f.rel_path,f.content_sha256,s.start_line,s.end_line
                 FROM symbols s JOIN files f ON f.id=s.file_id JOIN projects p ON p.id=s.project_id
                 WHERE p.name=?1 AND s.qualified_name=?2 ORDER BY s.start_line LIMIT 1",
                params![project, qualified_name],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)? as usize,
                        row.get::<_, i64>(4)? as usize,
                    ))
                },
            )
            .optional()
            .map_err(db_error)?
            .ok_or_else(|| format!("unknown symbol: {qualified_name}"))?;
        drop(connection);
        let bytes = read_current_indexed_file(Path::new(&row.0), &row.1, &row.2)?;
        let source = String::from_utf8(bytes)
            .map_err(|_| "indexed source is no longer valid UTF-8".to_string())?;
        let lines = source.lines().collect::<Vec<_>>();
        let start_line = row.3.saturating_sub(context_lines).max(1);
        let end_line = row.4.saturating_add(context_lines).min(lines.len());
        let content = lines[start_line - 1..end_line].join("\n");
        Ok(Snippet {
            project: project.to_string(),
            qualified_name: qualified_name.to_string(),
            path: row.1,
            start_line,
            end_line,
            content_sha256: row.2,
            content,
        })
    }

    pub fn trace(
        &self,
        project: &str,
        function_name: &str,
        direction: &str,
        max_depth: usize,
    ) -> Result<TraceResult, String> {
        self.trace_with_cancellation(project, function_name, direction, max_depth, &|| false)
    }

    pub fn trace_with_cancellation(
        &self,
        project: &str,
        function_name: &str,
        direction: &str,
        max_depth: usize,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<TraceResult, String> {
        self.ensure_project_fresh(project, cancelled)?;
        let max_depth = max_depth.clamp(1, 12);
        if !matches!(direction, "callers" | "callees" | "both") {
            return Err("trace direction must be callers, callees, or both".to_string());
        }
        let connection = self.lock_connection()?;
        let mut roots_statement = connection
            .prepare(
                "SELECT s.id,s.qualified_name,p.root_path,f.rel_path,f.content_sha256
                 FROM symbols s JOIN projects p ON p.id=s.project_id JOIN files f ON f.id=s.file_id
                 WHERE p.name=?1 AND (s.qualified_name=?2 OR s.name=?2)
                   AND s.kind IN ('Function','Method') ORDER BY s.id LIMIT 2",
            )
            .map_err(db_error)?;
        let roots = roots_statement
            .query_map(params![project, function_name], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(db_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error)?;
        drop(roots_statement);
        if roots.is_empty() {
            return Err(format!("unknown function: {function_name}"));
        }
        if roots.len() != 1 {
            return Err(format!("ambiguous function name: {function_name}"));
        }
        let root_path = PathBuf::from(&roots[0].2);
        let mut files_to_validate = vec![(roots[0].3.clone(), roots[0].4.clone())];
        let mut frontier = vec![(roots[0].0, 0usize)];
        let mut visited = HashSet::from([roots[0].0]);
        let mut output = Vec::new();
        let mut truncated = false;
        while let Some((node_id, depth)) = frontier.pop() {
            check_cancelled(cancelled)?;
            if depth >= max_depth {
                truncated = true;
                continue;
            }
            let directions = match direction {
                "callees" => &[true][..],
                "callers" => &[false][..],
                _ => &[true, false][..],
            };
            for outgoing in directions {
                let sql = if *outgoing {
                    "SELECT e.target_id,src.qualified_name,tgt.qualified_name,e.kind,e.confidence,
                            e.strategy,e.source_line,srcf.rel_path,srcf.content_sha256,
                            tgtf.rel_path,tgtf.content_sha256
                     FROM edges e JOIN symbols src ON src.id=e.source_id
                     JOIN symbols tgt ON tgt.id=e.target_id
                     JOIN files srcf ON srcf.id=src.file_id JOIN files tgtf ON tgtf.id=tgt.file_id
                     WHERE e.project_id=(SELECT id FROM projects WHERE name=?1)
                       AND e.source_id=?2 AND e.kind='CALLS' ORDER BY e.id"
                } else {
                    "SELECT e.source_id,src.qualified_name,tgt.qualified_name,e.kind,e.confidence,
                            e.strategy,e.source_line,srcf.rel_path,srcf.content_sha256,
                            tgtf.rel_path,tgtf.content_sha256
                     FROM edges e JOIN symbols src ON src.id=e.source_id
                     JOIN symbols tgt ON tgt.id=e.target_id
                     JOIN files srcf ON srcf.id=src.file_id JOIN files tgtf ON tgtf.id=tgt.file_id
                     WHERE e.project_id=(SELECT id FROM projects WHERE name=?1)
                       AND e.target_id=?2 AND e.kind='CALLS' ORDER BY e.id"
                };
                let mut statement = connection.prepare(sql).map_err(db_error)?;
                let rows = statement
                    .query_map(params![project, node_id], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            TraceEdge {
                                source: row.get(1)?,
                                target: row.get(2)?,
                                kind: row.get(3)?,
                                confidence: row.get(4)?,
                                strategy: row.get(5)?,
                                source_line: row.get::<_, Option<i64>>(6)?.map(|v| v as usize),
                                depth: depth + 1,
                            },
                            row.get::<_, String>(7)?,
                            row.get::<_, String>(8)?,
                            row.get::<_, String>(9)?,
                            row.get::<_, String>(10)?,
                        ))
                    })
                    .map_err(db_error)?;
                for row in rows {
                    check_cancelled(cancelled)?;
                    let (next_id, edge, source_path, source_digest, target_path, target_digest) =
                        row.map_err(db_error)?;
                    files_to_validate.push((source_path, source_digest));
                    files_to_validate.push((target_path, target_digest));
                    output.push(edge);
                    if visited.insert(next_id) {
                        frontier.push((next_id, depth + 1));
                    }
                }
            }
        }
        drop(connection);
        let mut validated = HashSet::new();
        for (path, digest) in files_to_validate {
            check_cancelled(cancelled)?;
            if validated.insert((path.clone(), digest.clone())) {
                read_current_indexed_file(&root_path, &path, &digest)?;
            }
        }
        Ok(TraceResult {
            project: project.to_string(),
            root: roots[0].1.clone(),
            direction: direction.to_string(),
            edges: output,
            truncated,
        })
    }

    pub fn architecture(&self, project: &str) -> Result<ArchitectureSummary, String> {
        self.architecture_with_cancellation(project, &|| false)
    }

    pub fn architecture_with_cancellation(
        &self,
        project: &str,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<ArchitectureSummary, String> {
        self.ensure_project_fresh(project, cancelled)?;
        let status = self.index_status(project)?;
        let connection = self.lock_connection()?;
        let files_by_language = grouped_counts(
            &connection,
            "SELECT f.language,COUNT(*) FROM files f JOIN projects p ON p.id=f.project_id
             WHERE p.name=?1 GROUP BY f.language ORDER BY COUNT(*) DESC,f.language",
            project,
        )?;
        let symbols_by_kind = grouped_counts(
            &connection,
            "SELECT s.kind,COUNT(*) FROM symbols s JOIN projects p ON p.id=s.project_id
             WHERE p.name=?1 AND s.kind NOT IN ('Project','File','Module')
             GROUP BY s.kind ORDER BY COUNT(*) DESC,s.kind",
            project,
        )?;
        let mut directories = BTreeMap::<String, usize>::new();
        for (_, files) in &files_by_language {
            let _ = files;
        }
        let mut statement = connection
            .prepare(
                "SELECT f.rel_path FROM files f JOIN projects p ON p.id=f.project_id WHERE p.name=?1",
            )
            .map_err(db_error)?;
        for path in statement
            .query_map([project], |row| row.get::<_, String>(0))
            .map_err(db_error)?
        {
            check_cancelled(cancelled)?;
            let path = path.map_err(db_error)?;
            let directory = path.split('/').next().unwrap_or(".");
            *directories.entry(directory.to_string()).or_default() += 1;
        }
        let mut top_level_directories = directories.into_iter().collect::<Vec<_>>();
        top_level_directories
            .sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
        Ok(ArchitectureSummary {
            project: project.to_string(),
            generation: status.generation,
            files_by_language,
            symbols_by_kind,
            top_level_directories,
        })
    }

    pub fn coverage(&self, project: &str) -> Result<CoverageReport, String> {
        self.coverage_with_cancellation(project, &|| false)
    }

    pub fn coverage_with_cancellation(
        &self,
        project: &str,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<CoverageReport, String> {
        self.ensure_project_fresh(project, cancelled)?;
        let status = self.index_status(project)?;
        let connection = self.lock_connection()?;
        let mut statement = connection
            .prepare(
                "SELECT f.rel_path,f.coverage_kind,f.coverage_detail
                 FROM files f JOIN projects p ON p.id=f.project_id
                 WHERE p.name=?1 AND f.coverage_kind IS NOT NULL ORDER BY f.rel_path",
            )
            .map_err(db_error)?;
        let issues = statement
            .query_map([project], |row| {
                Ok(FileCoverage {
                    path: row.get(0)?,
                    kind: row.get(1)?,
                    detail: row.get(2)?,
                })
            })
            .map_err(db_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error)?;
        Ok(CoverageReport {
            project: project.to_string(),
            generation: status.generation,
            complete_files: status.indexed_files.saturating_sub(issues.len()),
            incomplete_files: issues.len(),
            issues,
            repository_scope_complete: false,
            limitations: vec!["ignored_and_unsupported_files_are_not_yet_enumerated".to_string()],
        })
    }

    pub fn delete_project(&self, project: &str) -> Result<bool, String> {
        let connection = self.lock_connection()?;
        let changed = connection
            .execute("DELETE FROM projects WHERE name=?1", [project])
            .map_err(db_error)?;
        drop(connection);
        self.invalidate_project_cache(project);
        Ok(changed > 0)
    }

    /// JSON adapter for the Host tool executor. Dispatch is by the registered
    /// capability name; natural-language prompt text never enters this method.
    pub fn call(&self, tool: &str, arguments: &Value) -> Result<Value, String> {
        self.call_with_cancellation(tool, arguments, &|| false)
    }

    pub fn call_with_cancellation(
        &self,
        tool: &str,
        arguments: &Value,
        cancelled: &(dyn Fn() -> bool + Send + Sync),
    ) -> Result<Value, String> {
        match tool {
            "index_repository" => serde_json::to_value(
                self.index_with_cancellation(
                    required_str(arguments, "repo_path")?,
                    arguments
                        .get("project")
                        .or_else(|| arguments.get("name"))
                        .and_then(Value::as_str),
                    cancelled,
                )?,
            )
            .map_err(json_error),
            "list_projects" => serde_json::to_value(self.list_projects()?).map_err(json_error),
            "index_status" => {
                serde_json::to_value(self.index_status(required_str(arguments, "project")?)?)
                    .map_err(json_error)
            }
            "search_graph" => serde_json::to_value(self.search_symbols_with_cancellation(
                required_str(arguments, "project")?,
                required_str(arguments, "query")?,
                usize_arg(arguments, "limit", 50),
                usize_arg(arguments, "offset", 0),
                cancelled,
            )?)
            .map_err(json_error),
            "search_code" => serde_json::to_value(
                self.search_text_with_cancellation(
                    required_str(arguments, "project")?,
                    required_str(arguments, "pattern")?,
                    arguments
                        .get("regex")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    usize_arg(arguments, "max_results", 50),
                    usize_arg(arguments, "offset", 0),
                    cancelled,
                )?,
            )
            .map_err(json_error),
            "trace_path" => serde_json::to_value(
                self.trace_with_cancellation(
                    required_str(arguments, "project")?,
                    required_str(arguments, "function_name")?,
                    arguments
                        .get("direction")
                        .and_then(Value::as_str)
                        .unwrap_or("both"),
                    usize_arg(arguments, "max_depth", 3),
                    cancelled,
                )?,
            )
            .map_err(json_error),
            "get_code_snippet" => serde_json::to_value(self.get_snippet_with_cancellation(
                required_str(arguments, "project")?,
                required_str(arguments, "qualified_name")?,
                usize_arg(arguments, "context_lines", 0),
                cancelled,
            )?)
            .map_err(json_error),
            "get_architecture" => {
                serde_json::to_value(self.architecture_with_cancellation(
                    required_str(arguments, "project")?,
                    cancelled,
                )?)
                .map_err(json_error)
            }
            "check_index_coverage" => serde_json::to_value(
                self.coverage_with_cancellation(required_str(arguments, "project")?, cancelled)?,
            )
            .map_err(json_error),
            "delete_project" => Ok(json!({
                "project": required_str(arguments, "project")?,
                "deleted": self.delete_project(required_str(arguments, "project")?)?
            })),
            "get_graph_schema" => Ok(json!({
                "schema_version": schema::SCHEMA_VERSION,
                "supported_grammar_count": SUPPORTED_GRAMMARS.len(),
                "supported_languages": SUPPORTED_GRAMMARS,
                "semantic_languages": SEMANTIC_LANGUAGES,
                "support_policy": {
                    "programming_language_minimum_percent": 0.1,
                    "measurement": "GitHub Innovation Graph 2026 Q1 developer-language push activity aggregated across reported economies",
                    "engineering_formats_exempt": true
                },
                "coverage_note": "All listed grammars support discovery and text search; semantic_languages additionally have explicit symbol extraction profiles.",
                "node_kinds": ["Project","File","Module","Function","Method","Class","Struct","Enum","Interface","Type","Constant","Macro"],
                "edge_kinds": ["CONTAINS","DEFINES","IMPORTS","CALLS","INSTANTIATES"],
                "call_edge_properties": ["confidence","strategy","source_line"]
            })),
            "query_graph" | "detect_changes" | "manage_adr" | "ingest_traces" => Ok(json!({
                "type": "unsupported_operation",
                "code": "native_code_memory_operation_not_implemented",
                "operation": tool,
                "retryable": false
            })),
            _ => Err(format!("unknown native code-memory tool: {tool}")),
        }
    }

    fn project_files(&self, project: &str) -> Result<(PathBuf, Vec<(String, String)>), String> {
        let connection = self.lock_connection()?;
        let root: String = connection
            .query_row(
                "SELECT root_path FROM projects WHERE name=?1",
                [project],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?
            .ok_or_else(|| format!("unknown project: {project}"))?;
        let mut statement = connection
            .prepare(
                "SELECT f.rel_path,f.content_sha256 FROM files f JOIN projects p ON p.id=f.project_id
                 WHERE p.name=?1 ORDER BY f.rel_path",
            )
            .map_err(db_error)?;
        let files = statement
            .query_map([project], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(db_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error)?;
        Ok((PathBuf::from(root), files))
    }

    fn ensure_project_fresh(
        &self,
        project: &str,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), String> {
        let connection = self.lock_connection()?;
        let root: String = connection
            .query_row(
                "SELECT root_path FROM projects WHERE name=?1",
                [project],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?
            .ok_or_else(|| format!("unknown project: {project}"))?;
        let mut statement = connection
            .prepare(
                "SELECT f.rel_path,f.mtime_ns,f.byte_count FROM files f
                 JOIN projects p ON p.id=f.project_id WHERE p.name=?1",
            )
            .map_err(db_error)?;
        let stored_files = statement
            .query_map([project], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    (row.get::<_, i64>(1)?, row.get::<_, i64>(2)?),
                ))
            })
            .map_err(db_error)?
            .collect::<rusqlite::Result<HashMap<_, _>>>()
            .map_err(db_error)?;
        drop(statement);
        drop(connection);

        let root = PathBuf::from(root);
        let discovered = discover(&root, cancelled)?;
        if discovered.len() != stored_files.len() {
            return Err("code_index_stale:file_set_changed".to_string());
        }
        for path in discovered {
            check_cancelled(cancelled)?;
            let rel_path = path
                .strip_prefix(&root)
                .map_err(|_| "discovered path escaped repository root".to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            let Some((expected_mtime_ns, expected_size)) = stored_files.get(&rel_path) else {
                return Err("code_index_stale:file_set_changed".to_string());
            };
            let metadata = fs::metadata(&path)
                .map_err(|error| format!("code_index_stale:file_unavailable:{error}"))?;
            if modified_ns(&metadata) != *expected_mtime_ns
                || metadata.len() as i64 != *expected_size
            {
                return Err("code_index_stale:file_metadata_changed".to_string());
            }
        }
        Ok(())
    }

    fn validate_search_hits(
        &self,
        project: &str,
        hits: &[SearchHit],
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), String> {
        let requested = hits
            .iter()
            .map(|hit| hit.path.as_str())
            .collect::<HashSet<_>>();
        if requested.is_empty() {
            return check_cancelled(cancelled);
        }
        let (root, files) = self.project_files(project)?;
        let mut found = HashSet::new();
        for (path, digest) in files {
            check_cancelled(cancelled)?;
            if requested.contains(path.as_str()) && found.insert(path.clone()) {
                read_current_indexed_file(&root, &path, &digest)?;
            }
        }
        if found.len() != requested.len() {
            return Err("code_index_stale:file_set_changed".to_string());
        }
        Ok(())
    }

    fn lock_connection_cancellable(&self, cancelled: &dyn Fn() -> bool) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        let started = std::time::Instant::now();
        loop {
            check_cancelled(cancelled)?;
            match self.connection.try_lock() {
                Ok(guard) => return Ok(guard),
                Err(std::sync::TryLockError::Poisoned(_)) => return Err("code index lock is poisoned".into()),
                Err(std::sync::TryLockError::WouldBlock) => {},
            }
            if started.elapsed() >= std::time::Duration::from_secs(30) { return Err("code_memory_lock_timeout".into()); }
            std::thread::park_timeout(std::time::Duration::from_millis(5));
        }
    }

    fn lock_connection(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.connection
            .lock()
            .map_err(|_| "code index lock is poisoned".to_string())
    }

    fn data_version(&self) -> Result<i64, String> {
        self.lock_connection()?
            .query_row("PRAGMA data_version", [], |row| row.get(0))
            .map_err(db_error)
    }

    fn invalidate_project_cache(&self, project: &str) {
        if let Ok(mut cache) = self.search_cache.lock() {
            let prefix = format!("{project}\0");
            cache.retain(|key, _| !key.starts_with(&prefix));
        }
    }
}

fn prepare_files_parallel(
    root: &Path,
    files: &[PathBuf],
    known: &HashMap<String, String>,
    fast_unchanged: &HashSet<PathBuf>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Vec<PreparedFile>, String> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let worker_count = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(8)
        .min(files.len());
    let next = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel();
    thread::scope(|scope| -> Result<Vec<PreparedFile>, String> {
        for worker_index in 0..worker_count {
            let sender = sender.clone();
            let next = &next;
            thread::Builder::new()
                .name(format!("nola-code-index-{worker_index}"))
                .stack_size(16 * 1024 * 1024)
                .spawn_scoped(scope, move || loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = files.get(index) else {
                        break;
                    };
                    let result = path
                        .strip_prefix(root)
                        .map_err(|_| "discovered path escaped repository root".to_string())
                        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
                        .and_then(|rel_path| {
                            let previous_digest = known.get(&rel_path).map(String::as_str);
                            prepare_file(path, rel_path, previous_digest, fast_unchanged.contains(path), cancelled)
                        });
                    if sender.send((index, result)).is_err() {
                        break;
                    }
                })
                .map_err(|error| format!("start code index worker: {error}"))?;
        }
        drop(sender);
        let mut ordered = Vec::with_capacity(files.len());
        for received in receiver {
            ordered.push(received);
        }
        if ordered.len() != files.len() {
            return Err("code index worker stopped before preparing every file".to_string());
        }
        ordered.sort_by_key(|(index, _)| *index);
        ordered
            .into_iter()
            .map(|(_, result)| result)
            .collect::<Result<Vec<_>, String>>()
    })
}

fn prepare_file(
    path: &Path,
    rel_path: String,
    previous_digest: Option<&str>,
    metadata_unchanged: bool,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<PreparedFile, String> {
    if cancelled() {
        return Err("code_memory_cancelled".to_string());
    }
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            return Ok(PreparedFile {
                rel_path,
                modified_ns: 0,
                byte_count: 0,
                outcome: PreparedOutcome::CoverageOnly {
                    language: "unknown",
                    kind: "read",
                    detail: error.to_string(),
                },
            });
        }
    };
    let modified = modified_ns(&metadata);
    let byte_count = metadata.len() as i64;
    let language = supported_language(path).unwrap_or("unknown");
    if metadata_unchanged && previous_digest.is_some_and(|digest| !digest.is_empty()) {
        return Ok(PreparedFile {
            rel_path, modified_ns: modified, byte_count,
            outcome: PreparedOutcome::Unchanged,
        });
    }
    if metadata.len() > MAX_SOURCE_BYTES {
        return Ok(PreparedFile {
            rel_path,
            modified_ns: modified,
            byte_count,
            outcome: PreparedOutcome::CoverageOnly {
                language,
                kind: "oversized",
                detail: format!(
                    "{} bytes exceeds {} byte limit",
                    metadata.len(),
                    MAX_SOURCE_BYTES
                ),
            },
        });
    }
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return Ok(PreparedFile {
                rel_path,
                modified_ns: modified,
                byte_count,
                outcome: PreparedOutcome::CoverageOnly {
                    language,
                    kind: "read",
                    detail: error.to_string(),
                },
            });
        }
    };
    let digest = sha256_hex(&bytes);
    if previous_digest == Some(digest.as_str()) {
        return Ok(PreparedFile {
            rel_path,
            modified_ns: modified,
            byte_count,
            outcome: PreparedOutcome::Unchanged,
        });
    }
    let (language, symbols, references, coverage_kind, coverage_detail) =
        match extract(Path::new(&rel_path), &bytes) {
            Ok(extraction) => {
                let issue = extraction.parse_issue;
                (
                    extraction.language,
                    extraction.symbols,
                    extraction.references,
                    issue.as_ref().map(|_| "parse_partial"),
                    issue.unwrap_or_default(),
                )
            }
            Err(error) => (language, Vec::new(), Vec::new(), Some("extract"), error),
        };
    Ok(PreparedFile {
        rel_path,
        modified_ns: modified,
        byte_count,
        outcome: PreparedOutcome::Changed {
            language,
            digest,
            symbols,
            references,
            coverage_kind,
            coverage_detail,
        },
    })
}

fn discover(root: &Path, cancelled: &dyn Fn() -> bool) -> Result<Vec<PathBuf>, String> {
    let started = std::time::Instant::now();
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .follow_links(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .parents(true)
        .max_filesize(None)
        .filter_entry(|entry| entry.depth() == 0 || !entry.file_type().is_some_and(|kind| kind.is_dir())
            || !matches!(entry.file_name().to_str(), Some("target" | "node_modules" | "build" | "dist" | ".build" | "DerivedData" | ".cache" | ".venv" | "__pycache__")));
    let mut files = Vec::new();
    for result in builder.build() {
        if started.elapsed() >= std::time::Duration::from_secs(30) {
            return Err("code_memory_discovery_timeout: narrow the repository scope".into());
        }
        if cancelled() {
            return Err("code_memory_cancelled".to_string());
        }
        let entry = result.map_err(|error| format!("code memory discovery incomplete: {error}"))?;
        if entry.file_type().is_some_and(|kind| kind.is_file())
            && supported_language(entry.path()).is_some()
        {
            files.push(entry.into_path());
        }
    }
    files.sort();
    Ok(files)
}

fn check_cancelled(cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        Err("code_memory_cancelled".to_string())
    } else {
        Ok(())
    }
}

fn ensure_project(transaction: &Transaction<'_>, name: &str, root: &Path) -> Result<i64, String> {
    let existing = transaction
        .query_row(
            "SELECT id,root_path FROM projects WHERE name=?1",
            [name],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(db_error)?;
    if let Some((id, existing_root)) = existing {
        if Path::new(&existing_root) != root {
            return Err(format!(
                "project name {name:?} already indexes {existing_root}"
            ));
        }
        return Ok(id);
    }
    transaction
        .execute(
            "INSERT INTO projects(name,root_path) VALUES(?1,?2)",
            params![name, root.to_string_lossy()],
        )
        .map_err(db_error)?;
    Ok(transaction.last_insert_rowid())
}

fn load_file_hashes(
    transaction: &Transaction<'_>,
    project_id: i64,
) -> Result<HashMap<String, String>, String> {
    let mut statement = transaction
        .prepare("SELECT rel_path,content_sha256 FROM files WHERE project_id=?1")
        .map_err(db_error)?;
    let rows = statement
        .query_map([project_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(db_error)?;
    rows.collect::<rusqlite::Result<HashMap<_, _>>>()
        .map_err(db_error)
}

#[allow(clippy::too_many_arguments)]
fn replace_file(
    transaction: &Transaction<'_>,
    project_id: i64,
    project: &str,
    rel_path: &str,
    language: &str,
    digest: &str,
    mtime_ns: i64,
    byte_count: i64,
    generation: i64,
    coverage_kind: Option<&str>,
    coverage_detail: &str,
    symbols: &[ExtractedSymbol],
    references: &[ExtractedReference],
) -> Result<(), String> {
    transaction
        .prepare_cached(
            "INSERT INTO files(project_id,rel_path,language,content_sha256,mtime_ns,byte_count,
                               indexed_generation,coverage_kind,coverage_detail)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(project_id,rel_path) DO UPDATE SET
               language=excluded.language,content_sha256=excluded.content_sha256,
               mtime_ns=excluded.mtime_ns,byte_count=excluded.byte_count,
               indexed_generation=excluded.indexed_generation,
               coverage_kind=excluded.coverage_kind,coverage_detail=excluded.coverage_detail",
        )
        .map_err(db_error)?
        .execute(params![
            project_id,
            rel_path,
            language,
            digest,
            mtime_ns,
            byte_count,
            generation,
            coverage_kind,
            coverage_detail
        ])
        .map_err(db_error)?;
    let file_id: i64 = transaction
        .query_row(
            "SELECT id FROM files WHERE project_id=?1 AND rel_path=?2",
            params![project_id, rel_path],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    transaction
        .prepare_cached("DELETE FROM symbols WHERE file_id=?1")
        .map_err(db_error)?
        .execute([file_id])
        .map_err(db_error)?;
    transaction
        .prepare_cached("DELETE FROM references_raw WHERE file_id=?1")
        .map_err(db_error)?
        .execute([file_id])
        .map_err(db_error)?;
    let file_qn = format!("{project}::file::{rel_path}");
    let module_local = module_identity(Path::new(rel_path));
    let module_qn = format!("{project}::{module_local}");
    insert_node(
        transaction,
        project_id,
        Some(file_id),
        "File",
        rel_path,
        &file_qn,
        0,
        byte_count.max(0) as usize,
        1,
        1,
        "",
        rel_path,
    )?;
    insert_node(
        transaction,
        project_id,
        Some(file_id),
        "Module",
        &module_local,
        &module_qn,
        0,
        byte_count.max(0) as usize,
        1,
        1,
        "",
        rel_path,
    )?;
    for symbol in symbols {
        let qualified_name = format!("{project}::{}", symbol.qualified_name);
        insert_node(
            transaction,
            project_id,
            Some(file_id),
            &symbol.kind,
            &symbol.name,
            &qualified_name,
            symbol.start_byte,
            symbol.end_byte,
            symbol.start_line,
            symbol.end_line,
            &symbol.signature,
            rel_path,
        )?;
    }
    for reference in references {
        let source_qualified_name = if reference.source_qualified_name == module_local {
            module_qn.clone()
        } else {
            format!("{project}::{}", reference.source_qualified_name)
        };
        transaction
            .prepare_cached(
                "INSERT INTO references_raw(project_id,file_id,source_qualified_name,kind,target_name,source_line)
                 VALUES(?1,?2,?3,?4,?5,?6)",
            )
            .map_err(db_error)?
            .execute(params![
                    project_id,
                    file_id,
                    source_qualified_name,
                    reference.kind,
                    reference.target_name,
                    reference.source_line as i64
                ])
            .map_err(db_error)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn upsert_coverage_only(
    transaction: &Transaction<'_>,
    project_id: i64,
    project: &str,
    rel_path: &str,
    language: &str,
    mtime_ns: i64,
    byte_count: i64,
    generation: i64,
    kind: &str,
    detail: &str,
) -> Result<(), String> {
    replace_file(
        transaction,
        project_id,
        project,
        rel_path,
        language,
        "",
        mtime_ns,
        byte_count,
        generation,
        Some(kind),
        detail,
        &[],
        &[],
    )
}

#[allow(clippy::too_many_arguments)]
fn insert_node(
    transaction: &Transaction<'_>,
    project_id: i64,
    file_id: Option<i64>,
    kind: &str,
    name: &str,
    qualified_name: &str,
    start_byte: usize,
    end_byte: usize,
    start_line: usize,
    end_line: usize,
    signature: &str,
    path: &str,
) -> Result<i64, String> {
    transaction
        .prepare_cached(
            "INSERT INTO symbols(project_id,file_id,kind,name,qualified_name,start_byte,end_byte,
                                 start_line,end_line,signature)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        )
        .map_err(db_error)?
        .execute(params![
            project_id,
            file_id,
            kind,
            name,
            qualified_name,
            start_byte as i64,
            end_byte as i64,
            start_line as i64,
            end_line as i64,
            signature
        ])
        .map_err(db_error)?;
    let node_id = transaction.last_insert_rowid();
    transaction
        .prepare_cached(
            "INSERT INTO symbols_search(symbol_id,name,qualified_name,kind,path)
             VALUES(?1,?2,?3,?4,?5)",
        )
        .map_err(db_error)?
        .execute(params![
            node_id,
            searchable_identifier(name),
            searchable_identifier(qualified_name),
            kind,
            path
        ])
        .map_err(db_error)?;
    Ok(node_id)
}

fn ensure_project_node(
    transaction: &Transaction<'_>,
    project_id: i64,
    project: &str,
) -> Result<(), String> {
    let exists = transaction
        .query_row(
            "SELECT 1 FROM symbols WHERE project_id=?1 AND kind='Project' LIMIT 1",
            [project_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(db_error)?
        .is_some();
    if !exists {
        insert_node(
            transaction,
            project_id,
            None,
            "Project",
            project,
            project,
            0,
            0,
            1,
            1,
            "",
            "",
        )?;
    }
    Ok(())
}

fn rebuild_edges(
    transaction: &Transaction<'_>,
    project_id: i64,
    project: &str,
    changes: Option<&EdgeChanges>,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), String> {
    EdgeChanges::stage(transaction, project_id, changes)?;
    let full = changes.is_none();
    if full {
        transaction.execute("DELETE FROM edges WHERE project_id=?1", [project_id]).map_err(db_error)?;
    }
    let project_node: i64 = transaction
        .query_row(
            "SELECT id FROM symbols WHERE project_id=?1 AND kind='Project' AND name=?2",
            params![project_id, project],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO edges(project_id,source_id,target_id,kind,confidence,strategy)
             SELECT ?1,?2,id,'CONTAINS',1.0,'structural'
             FROM symbols WHERE project_id=?1 AND kind='File'
               AND (?3 OR qualified_name IN (SELECT name FROM temp.nola_dirty_qns))",
            params![project_id, project_node, full],
        )
        .map_err(db_error)?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO edges(project_id,source_id,target_id,kind,confidence,strategy)
             SELECT ?1,f.id,m.id,'CONTAINS',1.0,'structural'
             FROM symbols f JOIN symbols m ON m.file_id=f.file_id
             WHERE f.project_id=?1 AND f.kind='File' AND m.kind='Module'
               AND (?2 OR f.qualified_name IN (SELECT name FROM temp.nola_dirty_qns)
                       OR m.qualified_name IN (SELECT name FROM temp.nola_dirty_qns))",
            params![project_id, full],
        )
        .map_err(db_error)?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO edges(project_id,source_id,target_id,kind,confidence,strategy)
             SELECT ?1,m.id,n.id,'DEFINES',1.0,'syntax_tree'
             FROM symbols m JOIN symbols n ON n.file_id=m.file_id
             WHERE m.project_id=?1 AND m.kind='Module'
               AND n.kind NOT IN ('File','Module','Project')
               AND (?2 OR m.qualified_name IN (SELECT name FROM temp.nola_dirty_qns)
                       OR n.qualified_name IN (SELECT name FROM temp.nola_dirty_qns))",
            params![project_id, full],
        )
        .map_err(db_error)?;

    check_cancelled(cancelled)?;
    let mut qualified_ids = HashMap::new();
    let mut name_ids: HashMap<String, Vec<i64>> = HashMap::new();
    let mut symbol_statement = transaction
        .prepare(
            "SELECT id,qualified_name,name,kind FROM symbols
             WHERE project_id=?1 ORDER BY id",
        )
        .map_err(db_error)?;
    let symbols = symbol_statement
        .query_map([project_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(db_error)?;
    for symbol in symbols {
        check_cancelled(cancelled)?;
        let (id, qualified_name, name, kind) = symbol.map_err(db_error)?;
        qualified_ids.entry(qualified_name).or_insert(id);
        if kind != "Project" && kind != "File" {
            let ids = name_ids.entry(name).or_default();
            if ids.len() < 2 {
                ids.push(id);
            }
        }
    }
    drop(symbol_statement);

    let mut reference_statement = transaction
        .prepare(
            "SELECT source_qualified_name,kind,target_name,source_line
             FROM references_raw WHERE project_id=?1
               AND (?2 OR source_qualified_name IN (SELECT name FROM temp.nola_dirty_sources))
             ORDER BY id",
        )
        .map_err(db_error)?;
    let references = reference_statement
        .query_map(params![project_id, full], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(db_error)?;
    let mut insert_statement = transaction
        .prepare_cached(
            "INSERT OR IGNORE INTO edges(project_id,source_id,target_id,kind,confidence,strategy,source_line)
             VALUES(?1,?2,?3,?4,?5,?6,?7)",
        )
        .map_err(db_error)?;
    for reference in references {
        check_cancelled(cancelled)?;
        let (source_qn, kind, target_name, source_line) = reference.map_err(db_error)?;
        let Some(source_id) = qualified_ids.get(&source_qn).copied() else {
            continue;
        };
        let Some(targets) = name_ids.get(&target_name) else {
            continue;
        };
        // A textual target is only authoritative when it identifies exactly
        // one node. Ambiguous names stay in references_raw for diagnostics.
        if targets.len() != 1 {
            continue;
        }
        let (confidence, strategy) = match kind.as_str() {
            "CALLS" => (0.75, "unique_name"),
            "INSTANTIATES" => (0.9, "unique_design_unit"),
            _ => (0.7, "unique_import_name"),
        };
        insert_statement
            .execute(params![
                project_id,
                source_id,
                targets[0],
                kind,
                confidence,
                strategy,
                source_line
            ])
            .map_err(db_error)?;
    }
    Ok(())
}

fn grouped_counts(
    connection: &Connection,
    sql: &str,
    project: &str,
) -> Result<Vec<(String, usize)>, String> {
    let mut statement = connection.prepare(sql).map_err(db_error)?;
    let result = statement
        .query_map([project], |row| {
            Ok((row.get(0)?, row.get::<_, i64>(1)? as usize))
        })
        .map_err(db_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db_error);
    result
}

fn fts_query(query: &str) -> Result<String, String> {
    let normalized = searchable_identifier(query);
    let raw_terms = Regex::new(r"[\p{L}\p{N}_]+")
        .expect("static token expression")
        .find_iter(&normalized)
        .collect::<Vec<_>>();
    if raw_terms.is_empty() {
        return Err("search query has no indexable terms".to_string());
    }
    // Prefix expansion is useful for the last, possibly unfinished token. It
    // is prohibitively expensive on every token of a compound identifier:
    // common prefixes such as `cli`, `agent`, and `host` create large FTS
    // position lists before their intersection is known.
    let last = raw_terms.len() - 1;
    let terms = raw_terms
        .into_iter()
        .enumerate()
        .map(|(index, found)| {
            let suffix = if index == last { "*" } else { "" };
            format!("\"{}\"{suffix}", found.as_str().replace('"', "\"\""))
        })
        .collect::<Vec<_>>();
    Ok(terms.join(" "))
}

fn searchable_identifier(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 16);
    let mut previous_lowercase = false;
    for character in value.chars() {
        if character.is_uppercase() && previous_lowercase {
            output.push(' ');
        }
        if matches!(character, ':' | '.' | '/' | '\\' | '-' | '_') {
            output.push(' ');
            previous_lowercase = false;
        } else {
            output.push(character);
            previous_lowercase = character.is_lowercase();
        }
    }
    output
}

fn required_str<'a>(arguments: &'a Value, key: &str) -> Result<&'a str, String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("missing required string argument: {key}"))
}

fn usize_arg(arguments: &Value, key: &str, default: usize) -> usize {
    arguments
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(default)
}

fn modified_ns(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn read_current_indexed_file(
    root: &Path,
    rel_path: &str,
    expected_digest: &str,
) -> Result<Vec<u8>, String> {
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("resolve indexed root: {error}"))?;
    let candidate = canonical_root.join(rel_path);
    let canonical_candidate = candidate
        .canonicalize()
        .map_err(|error| format!("code_index_stale:file_unavailable:{error}"))?;
    if !canonical_candidate.starts_with(&canonical_root) {
        return Err("code_index_stale:path_escaped_repository".to_string());
    }
    if expected_digest.is_empty() {
        return Ok(Vec::new());
    }
    let bytes = fs::read(&canonical_candidate)
        .map_err(|error| format!("code_index_stale:file_unavailable:{error}"))?;
    if sha256_hex(&bytes) != expected_digest {
        return Err("code_index_stale:content_changed".to_string());
    }
    Ok(bytes)
}

fn db_error(error: rusqlite::Error) -> String {
    format!("code index database: {error}")
}

fn json_error(error: serde_json::Error) -> String {
    format!("encode code-memory result: {error}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn waiting_for_code_index_lock_can_be_cancelled() {
        let memory = NativeCodeMemory::open_in_memory().unwrap();
        let _held = memory.lock_connection().unwrap();
        assert!(memory.list_projects_with_cancellation(&|| true).unwrap_err().contains("cancelled"));
    }
    #[test]
    fn discovery_excludes_generated_sources_without_git_metadata() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("target")).unwrap();
        std::fs::write(root.path().join("target/generated.rs"), "fn generated() {}").unwrap();
        std::fs::write(root.path().join("source.rs"), "fn source() {}").unwrap();
        assert_eq!(discover(root.path(), &|| false).unwrap().len(), 1);
        assert!(discover(root.path(), &|| true).unwrap_err().contains("cancelled"));
    }

    use super::*;
    use std::fs::{self, FileTimes, OpenOptions};
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;
    use tempfile::TempDir;

    fn fixture() -> (TempDir, NativeCodeMemory) {
        let directory = TempDir::new().expect("tempdir");
        fs::create_dir(directory.path().join("src")).expect("src");
        fs::write(
            directory.path().join("src/lib.rs"),
            "pub struct SearchCache;\nimpl SearchCache { pub fn invalidate(&self) {} }\npub fn run_query() {}\n",
        )
        .expect("fixture source");
        fs::write(
            directory.path().join("ignored.py"),
            "def hidden():\n    pass\n",
        )
        .expect("ignored fixture");
        fs::write(directory.path().join(".gitignore"), "ignored.py\n").expect("gitignore");
        (
            directory,
            NativeCodeMemory::open_in_memory().expect("memory index"),
        )
    }

    #[test]
    fn indexes_searches_and_reads_symbols_in_process() {
        let (directory, memory) = fixture();
        let report = memory
            .index(directory.path(), Some("fixture"))
            .expect("index");
        assert_eq!(report.discovered_files, 1);
        assert!(report.symbols >= 2);
        let found = memory
            .search_symbols("fixture", "search cache", 10, 0)
            .expect("search");
        assert!(found.total >= 1);
        let found_symbol = found
            .results
            .iter()
            .find(|hit| hit.name == "SearchCache")
            .expect("struct search hit");
        let snippet = memory
            .get_snippet("fixture", &found_symbol.qualified_name, 0)
            .expect("snippet");
        assert!(snippet.content.contains("struct SearchCache"));
        assert!(
            memory
                .search_text("fixture", "run_query", false, 10, 0)
                .unwrap()
                .total
                >= 1
        );
        let project_only = memory.search_symbols("fixture", "fixture", 10, 0).unwrap();
        assert_eq!(project_only.total, project_only.results.len());
    }

    #[test]
    fn incrementally_replaces_changed_files_and_removes_deleted_files() {
        let (directory, memory) = fixture();
        memory.index(directory.path(), Some("fixture")).unwrap();
        fs::write(
            directory.path().join("src/lib.rs"),
            "pub fn replacement() {}\n",
        )
        .unwrap();
        let changed = memory.index(directory.path(), Some("fixture")).unwrap();
        assert_eq!(changed.indexed_files, 1);
        assert_eq!(changed.unchanged_files, 0);
        assert_eq!(
            memory
                .search_symbols("fixture", "SearchCache", 10, 0)
                .unwrap()
                .total,
            0
        );
        assert_eq!(
            memory
                .search_symbols("fixture", "replacement", 10, 0)
                .unwrap()
                .total,
            1
        );

        fs::remove_file(directory.path().join("src/lib.rs")).unwrap();
        let removed = memory.index(directory.path(), Some("fixture")).unwrap();
        assert_eq!(removed.removed_files, 1);
        assert_eq!(memory.index_status("fixture").unwrap().indexed_files, 0);
    }

    #[test]
    fn unchanged_generation_does_not_rebuild_edges() {
        let (directory, memory) = fixture();
        memory.index(directory.path(), Some("fixture")).unwrap();
        {
            let connection = memory.lock_connection().unwrap();
            let (project_id, symbol_id): (i64, i64) = connection
                .query_row(
                    "SELECT project_id,id FROM symbols
                     WHERE kind NOT IN ('Project','File','Module') LIMIT 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO edges(
                         project_id,source_id,target_id,kind,confidence,strategy
                     ) VALUES(?1,?2,?2,'TEST_SENTINEL',1.0,'regression_probe')",
                    params![project_id, symbol_id],
                )
                .unwrap();
        }

        let report = memory.index(directory.path(), Some("fixture")).unwrap();
        assert_eq!(report.indexed_files, 0);
        assert_eq!(report.unchanged_files, report.discovered_files);
        let connection = memory.lock_connection().unwrap();
        let sentinel_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM edges WHERE strategy='regression_probe'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            sentinel_count, 1,
            "an unchanged generation must leave the published graph untouched"
        );
    }

    #[cfg(unix)]
    #[test]
    fn committed_freshness_survives_noop_but_detects_rewrites_and_inventory_changes() {
        let directory = TempDir::new().unwrap();
        let source = directory.path().join("main.rs");
        fs::write(&source, "fn original_name() {}\n").unwrap();
        let memory = NativeCodeMemory::open_in_memory().unwrap();
        let initial = memory.index(directory.path(), Some("fresh")).unwrap();
        assert!(memory.current_index(directory.path(), &|| false).unwrap().is_some());
        let noop = memory.index(directory.path(), Some("fresh")).unwrap();
        assert_eq!(noop.generation, initial.generation, "no-op must not invalidate published queries");
        assert_eq!(noop.indexed_files, 0);
        rewrite_same_size_preserving_mtime(&source, "fn replaced_name() {}\n");
        assert!(memory.current_index(directory.path(), &|| false).unwrap().is_none());
        let updated = memory.index(directory.path(), Some("fresh")).unwrap();
        assert_eq!(updated.indexed_files, 1);
        assert!(memory.search_symbols("fresh","replaced_name",10,0).unwrap().total > 0);
        fs::write(directory.path().join("new.rs"), "fn added() {}").unwrap();
        assert!(memory.current_index(directory.path(), &|| false).unwrap().is_none());
        assert!(memory.index_with_cancellation(directory.path(), Some("fresh"), &|| true).is_err());
        assert_eq!(memory.index_status("fresh").unwrap().generation, updated.generation);
        assert!(memory.current_index(directory.path(), &|| false).unwrap().is_none(),
            "cancelled update cannot acknowledge new files");
        memory.index(directory.path(), Some("fresh")).unwrap();
        fs::rename(&source, directory.path().join("moved.rs")).unwrap();
        assert!(memory.current_index(directory.path(), &|| false).unwrap().is_none());
        memory.index(directory.path(), Some("fresh")).unwrap();
        fs::remove_file(directory.path().join("moved.rs")).unwrap();
        assert!(memory.current_index(directory.path(), &|| false).unwrap().is_none());
    }

    #[test]
    fn incremental_relations_match_full_build_for_changed_callers_targets_and_ambiguity() {
        fn edges(memory: &NativeCodeMemory) -> Vec<(String, String, String, Option<i64>)> {
            let connection = memory.lock_connection().unwrap();
            let mut stmt = connection.prepare(
                "SELECT s.qualified_name,t.qualified_name,e.kind,e.source_line
                 FROM edges e JOIN symbols s ON s.id=e.source_id JOIN symbols t ON t.id=e.target_id
                 ORDER BY 1,2,3,4").unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))
                .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap()
        }
        let directory = TempDir::new().unwrap();
        fs::write(directory.path().join("caller.rs"), "fn caller() { target(); other(); }\n").unwrap();
        fs::write(directory.path().join("target.rs"), "fn target() {}\nfn other() {}\n").unwrap();
        fs::write(directory.path().join("stable.rs"), "fn stable_a() { stable_b(); }\nfn stable_b() {}\n").unwrap();
        let memory = NativeCodeMemory::open_in_memory().unwrap();
        memory.index(directory.path(), Some("delta")).unwrap();
        let stable_edge = || memory.lock_connection().unwrap().query_row(
            "SELECT e.id FROM edges e JOIN symbols s ON s.id=e.source_id
             WHERE s.name='stable_a' AND e.kind='CALLS'", [], |row| row.get::<_,i64>(0)).unwrap();
        let original_edge_id = stable_edge();
        for (path, content) in [
            ("target.rs", Some("// shifted definitions\nfn target() {}\nfn other() {}\n")),
            ("duplicate.rs", Some("fn target() {}\n")),
            ("duplicate.rs", None),
            ("target.rs", Some("fn renamed() {}\nfn other() {}\n")),
            ("caller.rs", Some("fn caller() { renamed(); other(); }\n")),
            ("target.rs", None),
            ("moved.rs", Some("fn renamed() {}\nfn other() {}\n")),
            (".gitignore", Some("moved.rs\n")),
        ] {
            let path = directory.path().join(path);
            if let Some(content) = content { fs::write(path, content).unwrap(); }
            else { fs::remove_file(path).unwrap(); }
            memory.index(directory.path(), Some("delta")).unwrap();
            assert_eq!(stable_edge(), original_edge_id, "unrelated relations must not be rewritten");
            let full = NativeCodeMemory::open_in_memory().unwrap();
            full.index(directory.path(), Some("delta")).unwrap();
            assert_eq!(edges(&memory), edges(&full), "incremental relation resolution differs from full build");
        }
    }

    #[test]
    fn symbol_dense_file_can_be_replaced_without_stale_graph_rows() {
        const DECLARATION_COUNT: usize = 512;

        fn source(prefix: &str) -> String {
            let mut source = String::new();
            for index in 0..DECLARATION_COUNT {
                if index == 0 {
                    source.push_str(&format!("pub fn {prefix}_{index}() {{}}\n"));
                } else {
                    source.push_str(&format!(
                        "pub fn {prefix}_{index}() {{ {prefix}_{}(); }}\n",
                        index - 1
                    ));
                }
            }
            source
        }

        let directory = TempDir::new().unwrap();
        let path = directory.path().join("dense.rs");
        fs::write(&path, source("before")).unwrap();
        let memory = NativeCodeMemory::open_in_memory().unwrap();
        memory.index(directory.path(), Some("dense")).unwrap();

        fs::write(&path, source("afterx")).unwrap();
        let report = memory.index(directory.path(), Some("dense")).unwrap();
        assert_eq!(report.indexed_files, 1);
        assert_eq!(report.unchanged_files, 0);

        let connection = memory.lock_connection().unwrap();
        let old_symbols: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM symbols WHERE name LIKE 'before_%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let new_symbols: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM symbols WHERE name LIKE 'afterx_%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let call_edges: i64 = connection
            .query_row("SELECT COUNT(*) FROM edges WHERE kind='CALLS'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(old_symbols, 0);
        assert_eq!(new_symbols, DECLARATION_COUNT as i64);
        assert_eq!(call_edges, DECLARATION_COUNT.saturating_sub(1) as i64);
    }

    #[test]
    fn exposes_status_architecture_coverage_and_delete_tools() {
        let (directory, memory) = fixture();
        memory.index(directory.path(), Some("fixture")).unwrap();
        assert_eq!(memory.list_projects().unwrap()[0].name, "fixture");
        assert_eq!(memory.index_status("fixture").unwrap().indexed_files, 1);
        assert!(memory
            .architecture("fixture")
            .unwrap()
            .symbols_by_kind
            .iter()
            .any(|(kind, _)| kind == "Struct"));
        assert_eq!(memory.coverage("fixture").unwrap().incomplete_files, 0);
        let output = memory.call("list_projects", &json!({})).unwrap();
        assert_eq!(output[0]["name"], "fixture");
        assert!(memory.delete_project("fixture").unwrap());
        assert!(memory.list_projects().unwrap().is_empty());
    }

    #[test]
    fn records_partial_parse_coverage_without_dropping_symbols() {
        let directory = TempDir::new().unwrap();
        fs::write(
            directory.path().join("broken.py"),
            "def useful():\n    return 1\n\n@\n",
        )
        .unwrap();
        let memory = NativeCodeMemory::open_in_memory().unwrap();
        let report = memory.index(directory.path(), Some("fixture")).unwrap();
        assert_eq!(report.incomplete_files, 1);
        assert_eq!(
            memory.coverage("fixture").unwrap().issues[0].kind,
            "parse_partial"
        );
        assert_eq!(
            memory
                .search_symbols("fixture", "useful", 10, 0)
                .unwrap()
                .total,
            1
        );
    }

    #[test]
    fn extracts_declared_languages_with_types_and_callables() {
        let directory = TempDir::new().unwrap();
        let fixtures = [
            (
                "sample.rs",
                "struct RustType {}\nfn rust_call() {}\n",
                "RustType",
                "rust_call",
            ),
            (
                "sample.py",
                "class PythonType:\n    pass\ndef python_call():\n    pass\n",
                "PythonType",
                "python_call",
            ),
            (
                "sample.ts",
                "class TypeScriptType {}\nfunction typescriptCall() {}\n",
                "TypeScriptType",
                "typescriptCall",
            ),
            (
                "sample.js",
                "class JavaScriptType {}\nfunction javascriptCall() {}\n",
                "JavaScriptType",
                "javascriptCall",
            ),
            (
                "sample.swift",
                "struct SwiftType {}\nfunc swiftCall() {}\n",
                "SwiftType",
                "swiftCall",
            ),
            (
                "sample.kt",
                "class KotlinType\nfun kotlinCall() {}\n",
                "KotlinType",
                "kotlinCall",
            ),
        ];
        for (path, source, _, _) in fixtures {
            fs::write(directory.path().join(path), source).unwrap();
        }
        let memory = NativeCodeMemory::open_in_memory().unwrap();
        memory.index(directory.path(), Some("languages")).unwrap();
        for (_, _, type_name, callable) in fixtures {
            let type_results = memory
                .search_symbols("languages", type_name, 10, 0)
                .unwrap();
            assert!(
                type_results.results.iter().any(|hit| {
                    hit.name == type_name
                        && matches!(hit.kind.as_str(), "Class" | "Struct" | "Interface")
                }),
                "missing type {type_name}: {:?}",
                type_results.results
            );
            assert_eq!(
                memory
                    .search_symbols("languages", callable, 10, 0)
                    .unwrap()
                    .total,
                1,
                "missing callable {callable}"
            );
        }
    }

    #[test]
    fn trace_contains_only_unambiguous_call_edges_with_provenance() {
        let directory = TempDir::new().unwrap();
        fs::write(
            directory.path().join("calls.rs"),
            "fn target() {}\nfn caller() { target(); }\n",
        )
        .unwrap();
        let memory = NativeCodeMemory::open_in_memory().unwrap();
        memory.index(directory.path(), Some("calls")).unwrap();
        let trace = memory.trace("calls", "caller", "callees", 2).unwrap();
        assert_eq!(trace.edges.len(), 1);
        assert!(trace.edges[0].target.contains("::target/"));
        assert_eq!(trace.edges[0].strategy, "unique_name");
        assert!(trace.edges[0].confidence > 0.0 && trace.edges[0].confidence < 1.0);

        fs::write(directory.path().join("duplicate.rs"), "fn target() {}\n").unwrap();
        memory.index(directory.path(), Some("calls")).unwrap();
        let trace = memory.trace("calls", "caller", "callees", 2).unwrap();
        assert!(
            trace.edges.is_empty(),
            "ambiguous call target must fail closed"
        );
    }

    #[test]
    fn resolves_systemverilog_design_unit_instantiations() {
        let directory = TempDir::new().unwrap();
        fs::write(
            directory.path().join("design.sv"),
            r#"
module processing_element(input logic clock);
endmodule

module processing_array(input logic clock);
    processing_element element(.clock(clock));
endmodule
"#,
        )
        .unwrap();
        let memory = NativeCodeMemory::open_in_memory().unwrap();
        memory.index(directory.path(), Some("eda-design")).unwrap();

        let connection = memory.lock_connection().unwrap();
        let edge: (String, String, String) = connection
            .query_row(
                "SELECT source.name,target.name,edges.strategy
                 FROM edges
                 JOIN symbols AS source ON source.id=edges.source_id
                 JOIN symbols AS target ON target.id=edges.target_id
                 WHERE edges.kind='INSTANTIATES'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(edge.0, "processing_array");
        assert_eq!(edge.1, "processing_element");
        assert_eq!(edge.2, "unique_design_unit");
    }

    #[test]
    fn qualified_names_survive_restart_and_delete_reindex() {
        let directory = TempDir::new().unwrap();
        fs::write(
            directory.path().join("stable.rs"),
            "fn stable_identity() {}\n",
        )
        .unwrap();
        let database = directory.path().join("index.sqlite3");
        let first_name = {
            let memory = NativeCodeMemory::open(&database).unwrap();
            memory
                .index(directory.path(), Some("stable-project"))
                .unwrap();
            memory
                .search_symbols("stable-project", "stable_identity", 10, 0)
                .unwrap()
                .results[0]
                .qualified_name
                .clone()
        };
        let memory = NativeCodeMemory::open(&database).unwrap();
        let reopened_name = memory
            .search_symbols("stable-project", "stable_identity", 10, 0)
            .unwrap()
            .results[0]
            .qualified_name
            .clone();
        assert_eq!(first_name, reopened_name);
        assert!(!first_name.contains(directory.path().to_string_lossy().as_ref()));
        memory.delete_project("stable-project").unwrap();
        memory
            .index(directory.path(), Some("stable-project"))
            .unwrap();
        let rebuilt_name = memory
            .search_symbols("stable-project", "stable_identity", 10, 0)
            .unwrap()
            .results[0]
            .qualified_name
            .clone();
        assert_eq!(first_name, rebuilt_name);
    }

    #[test]
    fn search_cache_observes_same_generation_replacement_from_another_connection() {
        let directory = TempDir::new().unwrap();
        let source = directory.path().join("cache.rs");
        fs::write(&source, "fn original_symbol() {}\n").unwrap();
        let database = directory.path().join("index.sqlite3");
        let reader = NativeCodeMemory::open(&database).unwrap();
        reader.index(directory.path(), Some("shared")).unwrap();
        assert_eq!(
            reader
                .search_symbols("shared", "original_symbol", 10, 0)
                .unwrap()
                .total,
            1
        );

        let writer = NativeCodeMemory::open(&database).unwrap();
        writer.delete_project("shared").unwrap();
        fs::write(&source, "fn replacement_symbol() {}\n").unwrap();
        writer.index(directory.path(), Some("shared")).unwrap();

        // Delete + reindex resets the project generation to the same value.
        // SQLite data_version must still invalidate the other handle's cache.
        assert_eq!(
            reader
                .search_symbols("shared", "original_symbol", 10, 0)
                .unwrap()
                .total,
            0
        );
        assert_eq!(
            reader
                .search_symbols("shared", "replacement_symbol", 10, 0)
                .unwrap()
                .total,
            1
        );
    }

    #[test]
    fn callable_identity_distinguishes_arity_and_module_references_resolve() {
        let directory = TempDir::new().unwrap();
        fs::write(
            directory.path().join("types.rs"),
            "struct Worker;\nimpl Worker { fn run(&self) {} fn run_with(&self, value: i32) {} }\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("caller.rs"),
            "use crate::Worker;\nfn entry() { helper(); }\nfn helper() {}\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("overload.ts"),
            "function parse(value: string): string;\nfunction parse(value: number): number;\nfunction parse(value: unknown) { return value; }\n",
        )
        .unwrap();
        let memory = NativeCodeMemory::open_in_memory().unwrap();
        memory.index(directory.path(), Some("identity")).unwrap();
        let run = memory.search_symbols("identity", "run", 10, 0).unwrap();
        assert!(run
            .results
            .iter()
            .any(|hit| hit.qualified_name.contains("Worker::run/1")));
        let run_with = memory
            .search_symbols("identity", "run_with", 10, 0)
            .unwrap();
        assert!(run_with
            .results
            .iter()
            .any(|hit| hit.qualified_name.contains("Worker::run_with/2")));
        let overloads = memory.search_symbols("identity", "parse", 10, 0).unwrap();
        let identities = overloads
            .results
            .iter()
            .filter(|hit| hit.name == "parse")
            .map(|hit| &hit.qualified_name)
            .collect::<HashSet<_>>();
        assert_eq!(identities.len(), 3);
        let trace = memory.trace("identity", "entry", "callees", 2).unwrap();
        assert!(trace
            .edges
            .iter()
            .any(|edge| edge.target.contains("helper/0")));
        let connection = memory.lock_connection().unwrap();
        let import_edges: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM edges WHERE kind='IMPORTS'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(import_edges >= 1);
    }

    #[test]
    fn unavailable_compatibility_operations_have_a_stable_result_contract() {
        let memory = NativeCodeMemory::open_in_memory().unwrap();
        for tool in [
            "query_graph",
            "detect_changes",
            "manage_adr",
            "ingest_traces",
        ] {
            let result = memory.call(tool, &json!({})).unwrap();
            assert_eq!(result["type"], "unsupported_operation");
            assert_eq!(result["operation"], tool);
            assert_eq!(result["retryable"], false);
        }
        let graph_schema = memory.call("get_graph_schema", &json!({})).unwrap();
        assert_eq!(graph_schema["schema_version"], 1);
        assert_eq!(graph_schema["supported_grammar_count"], 67);
        assert_eq!(
            graph_schema["supported_languages"].as_array().map(Vec::len),
            Some(67)
        );
        assert_eq!(
            graph_schema["semantic_languages"].as_array().map(Vec::len),
            Some(19)
        );
        assert_eq!(
            graph_schema["support_policy"]["programming_language_minimum_percent"],
            0.1
        );
    }

    #[test]
    fn changed_source_returns_stale_error_instead_of_panicking_or_serving_old_graph() {
        let (directory, memory) = fixture();
        memory.index(directory.path(), Some("fixture")).unwrap();
        let symbol = memory
            .search_symbols("fixture", "SearchCache", 10, 0)
            .unwrap()
            .results
            .into_iter()
            .find(|hit| hit.name == "SearchCache")
            .unwrap();
        fs::write(directory.path().join("src/lib.rs"), "x\n").unwrap();
        let snippet_error = memory
            .get_snippet("fixture", &symbol.qualified_name, 0)
            .unwrap_err();
        assert!(snippet_error.starts_with("code_index_stale:"));
        assert!(memory
            .search_symbols("fixture", "SearchCache", 10, 0)
            .unwrap_err()
            .starts_with("code_index_stale:"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_swap_cannot_escape_repository_during_live_text_search() {
        use std::os::unix::fs::symlink;

        let (directory, memory) = fixture();
        memory.index(directory.path(), Some("fixture")).unwrap();
        let outside = TempDir::new().unwrap();
        let secret = outside.path().join("secret.rs");
        fs::write(&secret, "secret_outside_repository\n").unwrap();
        fs::remove_file(directory.path().join("src/lib.rs")).unwrap();
        symlink(&secret, directory.path().join("src/lib.rs")).unwrap();
        let error = memory
            .search_text("fixture", "secret_outside_repository", false, 10, 0)
            .unwrap_err();
        assert!(error.starts_with("code_index_stale:"));
    }

    #[test]
    fn oversized_supported_files_are_reported_in_coverage() {
        let directory = TempDir::new().unwrap();
        fs::write(
            directory.path().join("large.rs"),
            vec![b' '; MAX_SOURCE_BYTES as usize + 1],
        )
        .unwrap();
        let memory = NativeCodeMemory::open_in_memory().unwrap();
        let report = memory.index(directory.path(), Some("large")).unwrap();
        assert_eq!(report.discovered_files, 1);
        assert_eq!(report.incomplete_files, 1);
        let coverage = memory.coverage("large").unwrap();
        assert_eq!(coverage.issues[0].kind, "oversized");
    }

    #[test]
    fn cancellation_rolls_back_an_unpublished_generation() {
        let (directory, memory) = fixture();
        let error = memory
            .index_with_cancellation(directory.path(), Some("cancelled"), &|| true)
            .unwrap_err();
        assert_eq!(error, "code_memory_cancelled");
        assert!(memory.list_projects().unwrap().is_empty());
    }

    fn rewrite_same_size_preserving_mtime(path: &Path, replacement: &str) {
        let original_modified = fs::metadata(path).unwrap().modified().unwrap();
        let original_size = fs::metadata(path).unwrap().len();
        assert_eq!(replacement.len() as u64, original_size);
        let mut file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(path)
            .unwrap();
        file.write_all(replacement.as_bytes()).unwrap();
        file.sync_all().unwrap();
        file.set_times(FileTimes::new().set_modified(original_modified))
            .unwrap();
    }

    #[test]
    fn freshness_uses_metadata_but_symbol_results_are_digest_verified() {
        let directory = TempDir::new().unwrap();
        let stable = directory.path().join("stable.rs");
        let changed = directory.path().join("changed.rs");
        fs::write(&stable, "fn stable_symbol() {}\n").unwrap();
        fs::write(&changed, "fn original_name() {}\n").unwrap();
        let memory = NativeCodeMemory::open_in_memory().unwrap();
        memory.index(directory.path(), Some("freshness")).unwrap();

        rewrite_same_size_preserving_mtime(&changed, "fn replaced_name() {}\n");

        let unrelated = memory
            .search_symbols("freshness", "stable_symbol", 10, 0)
            .unwrap();
        assert_eq!(unrelated.total, 1);
        let error = memory
            .search_symbols("freshness", "original_name", 10, 0)
            .unwrap_err();
        assert_eq!(error, "code_index_stale:content_changed");
    }

    #[test]
    fn trace_digest_verifies_every_returned_endpoint() {
        let directory = TempDir::new().unwrap();
        let source = directory.path().join("calls.rs");
        fs::write(&source, "fn target() {}\nfn caller() { target(); }\n").unwrap();
        let memory = NativeCodeMemory::open_in_memory().unwrap();
        memory
            .index(directory.path(), Some("trace-freshness"))
            .unwrap();

        rewrite_same_size_preserving_mtime(&source, "fn targex() {}\nfn caller() { targex(); }\n");
        let error = memory
            .trace("trace-freshness", "caller", "callees", 2)
            .unwrap_err();
        assert_eq!(error, "code_index_stale:content_changed");
    }

    fn checks_before_query_loop(root: &Path) -> usize {
        let discovery_checks = AtomicUsize::new(0);
        let files = discover(root, &|| {
            discovery_checks.fetch_add(1, Ordering::Relaxed);
            false
        })
        .unwrap();
        discovery_checks.load(Ordering::Relaxed) + files.len()
    }

    #[test]
    fn cancellation_reaches_text_search_and_trace_after_freshness() {
        let directory = TempDir::new().unwrap();
        fs::write(
            directory.path().join("calls.rs"),
            "fn target() {}\nfn caller() { target(); }\n",
        )
        .unwrap();
        let memory = NativeCodeMemory::open_in_memory().unwrap();
        memory
            .index(directory.path(), Some("cancel-query"))
            .unwrap();
        let threshold = checks_before_query_loop(directory.path());

        for operation in ["search", "trace"] {
            let checks = AtomicUsize::new(0);
            let cancelled = || checks.fetch_add(1, Ordering::Relaxed) >= threshold;
            let error = if operation == "search" {
                memory
                    .search_text_with_cancellation(
                        "cancel-query",
                        "target",
                        false,
                        10,
                        0,
                        &cancelled,
                    )
                    .unwrap_err()
            } else {
                memory
                    .trace_with_cancellation("cancel-query", "caller", "callees", 2, &cancelled)
                    .unwrap_err()
            };
            assert_eq!(error, "code_memory_cancelled", "operation={operation}");
            assert!(checks.load(Ordering::Relaxed) > threshold);
        }
    }

    /// Manual release-build acceptance for the repository-scale measurements
    /// recorded in CODE_MEMORY_NATIVE_ARCHITECTURE.md. The generous ceiling
    /// catches order-of-magnitude regressions while tolerating host variance.
    #[test]
    #[ignore = "run explicitly with --release for repository-scale performance acceptance"]
    fn nola_repository_release_performance_baseline() {
        assert!(
            !cfg!(debug_assertions),
            "performance acceptance must run with cargo test --release"
        );
        const OBSERVED_COLD_INDEX_MS: u128 = 5_171;
        const OBSERVED_UNCHANGED_INDEX_MS: u128 = 118;
        const COLD_REGRESSION_CEILING_MS: u128 = OBSERVED_COLD_INDEX_MS * 3;
        const UNCHANGED_REGRESSION_CEILING_MS: u128 = 1_000;

        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let database_root = TempDir::new().unwrap();
        let memory =
            NativeCodeMemory::open(database_root.path().join("code-memory.sqlite3")).unwrap();

        let cold_started = Instant::now();
        let cold_report = memory
            .index(&repository_root, Some("Nola-performance"))
            .unwrap();
        let cold_ms = cold_started.elapsed().as_millis();
        let unchanged_started = Instant::now();
        let unchanged_report = memory
            .index(&repository_root, Some("Nola-performance"))
            .unwrap();
        let unchanged_ms = unchanged_started.elapsed().as_millis();

        eprintln!(
            "code_memory_performance cold_ms={cold_ms} unchanged_ms={unchanged_ms} files={} symbols={}",
            cold_report.discovered_files, cold_report.symbols
        );
        assert!(cold_report.indexed_files > 0);
        assert_eq!(unchanged_report.indexed_files, 0);
        assert_eq!(
            unchanged_report.unchanged_files,
            unchanged_report.discovered_files
        );
        assert!(
            cold_ms <= COLD_REGRESSION_CEILING_MS,
            "cold index took {cold_ms} ms; observed baseline {OBSERVED_COLD_INDEX_MS} ms"
        );
        assert!(
            unchanged_ms <= UNCHANGED_REGRESSION_CEILING_MS,
            "unchanged index took {unchanged_ms} ms; observed baseline {OBSERVED_UNCHANGED_INDEX_MS} ms"
        );
    }
}
