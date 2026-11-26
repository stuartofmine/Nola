// SPDX-License-Identifier: Apache-2.0
//! SQLite WAL + FTS5 unified store with per-scope event retention,
//! atomic `VACUUM INTO` backup, and generic typed business-module CRUD.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nola_types::{now_ms, McpServerConfig, MemoryItem, Session, SessionEvent};
use rusqlite::{params, params_from_iter, Connection, OpenFlags, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Default maximum retained events per scope before oldest events in that scope are pruned.
pub const DEFAULT_MAX_EVENTS_PER_SCOPE: i64 = 10_000;
/// SQLite `PRAGMA application_id` identifying a Nola database ("NOLA" in ASCII).
pub const NOLA_SQLITE_APPLICATION_ID: u32 = 0x4E4F_4C41;
/// SQLite `PRAGMA user_version` for Nola v2 schema.
pub const NOLA_SQLITE_USER_VERSION: u32 = 2;

/// Stored transcript event alias matching `nola_types::SessionEvent`.
pub type StoredEvent = SessionEvent;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("crypto error: {0}")]
    Crypto(String),
    #[error("service lock busy: {0}")]
    LockBusy(String),
    #[error("incompatible schema: {0}")]
    IncompatibleSchema(String),
    #[error("store error: {0}")]
    Other(String),
}

/// Sync outbox/change-log record persisted in `sync_changes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncChangeRecord {
    pub id: String,
    pub scope: String,
    pub entity_kind: String,
    pub entity_id: String,
    pub payload_json: String,
    pub created_at_ms: i64,
    pub resolved: bool,
}

/// Document chunk input/storage record persisted in `doc_chunks`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocChunkRecord {
    pub id: String,
    pub source_id: String,
    pub path: String,
    pub chunk_index: i64,
    pub content: String,
    pub sha256: String,
    pub updated_at_ms: i64,
}

/// Ranked document chunk hit returned by `Store::search_doc_chunks`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocChunkHit {
    pub id: String,
    pub source_id: String,
    pub path: String,
    pub chunk_index: i64,
    pub content: String,
    pub sha256: String,
    pub updated_at_ms: i64,
    pub score: f64,
}

/// Indexed source file metadata stored in `code_files`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeFileRow {
    pub workspace_root: String,
    pub rel_path: String,
    pub sha256: String,
    pub language: String,
    pub line_count: i64,
    pub updated_at_ms: i64,
}

/// Indexed AST symbol stored in `code_symbols` / `code_symbols_fts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeSymbolRow {
    pub id: String,
    pub workspace_root: String,
    pub rel_path: String,
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub language: String,
    pub start_line: i64,
    pub end_line: i64,
    pub signature: String,
}

/// Directed symbol reference / call-graph edge stored in `code_edges`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeEdgeRow {
    pub workspace_root: String,
    pub from_symbol: String,
    pub to_symbol: String,
    pub rel_path: String,
    pub line: i64,
    pub edge_kind: String,
}

/// Summary counts for a workspace's SQLite code index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeIndexStats {
    pub file_count: i64,
    pub symbol_count: i64,
    pub edge_count: i64,
    pub last_updated_at_ms: Option<i64>,
}

/// Result of reading events after a cursor in a specific scope.
#[derive(Debug, Clone, PartialEq)]
pub enum EventPage {
    /// Contiguous ordered events strictly after `after_seq`.
    Events(Vec<SessionEvent>),
    /// Requested cursor fell behind per-scope retention watermark; client MUST resync.
    ResyncRequired {
        earliest_retained_seq: i64,
        latest_seq: i64,
    },
}

/// Thread-safe handle to Nola v2's SQLite WAL store.
#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
    poisoned: Arc<AtomicBool>,
}

impl Store {
    /// Open or create a persistent SQLite WAL database at `path`.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    /// Open an isolated in-memory database (for tests and ephemeral runs).
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Self, StoreError> {
        conn.execute_batch("PRAGMA busy_timeout = 5000;")?;

        let app_id: u32 = conn.query_row("PRAGMA application_id;", [], |r| {
            let v: i64 = r.get(0)?;
            Ok(v as u32)
        })?;
        let user_ver: u32 = conn.query_row("PRAGMA user_version;", [], |r| {
            let v: i64 = r.get(0)?;
            Ok(v as u32)
        })?;

        if app_id != 0 && app_id != NOLA_SQLITE_APPLICATION_ID {
            return Err(StoreError::IncompatibleSchema(format!(
                "refusing to open foreign SQLite database with application_id 0x{app_id:08X} (expected 0x{NOLA_SQLITE_APPLICATION_ID:08X})"
            )));
        }
        if app_id == NOLA_SQLITE_APPLICATION_ID && user_ver == 1 {
            return Err(StoreError::IncompatibleSchema(
                "legacy Nola v1 database (user_version = 1) is unsupported by the Nola v2 baseline and was NOT modified; use a fresh v2 database path"
                    .into(),
            ));
        }
        if user_ver > NOLA_SQLITE_USER_VERSION {
            return Err(StoreError::IncompatibleSchema(format!(
                "database schema user_version {user_ver} is newer than supported version {NOLA_SQLITE_USER_VERSION}"
            )));
        }
        if app_id == NOLA_SQLITE_APPLICATION_ID && user_ver != NOLA_SQLITE_USER_VERSION {
            return Err(StoreError::IncompatibleSchema(format!(
                "incompatible Nola database schema user_version {user_ver} (expected {NOLA_SQLITE_USER_VERSION}); database was NOT modified"
            )));
        }
        if app_id == 0 && user_ver == 1 {
            return Err(StoreError::IncompatibleSchema(
                "legacy database (user_version = 1) is unsupported by the Nola v2 baseline and was NOT modified"
                    .into(),
            ));
        }
        if app_id == 0 {
            let table_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
                [],
                |r| r.get(0),
            )?;
            let has_sessions: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'sessions'",
                [],
                |r| r.get(0),
            )?;
            if table_count > 0 && has_sessions == 0 {
                return Err(StoreError::IncompatibleSchema(
                    "refusing to open non-Nola SQLite database (missing 'sessions' table and application_id)"
                        .into(),
                ));
            }
        }

        let schema_batch = format!(
            "PRAGMA busy_timeout = 5000;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA foreign_keys = ON;
             PRAGMA application_id = {NOLA_SQLITE_APPLICATION_ID};
             PRAGMA user_version = {NOLA_SQLITE_USER_VERSION};

             CREATE TABLE IF NOT EXISTS sessions (
                 id TEXT PRIMARY KEY,
                 title TEXT NOT NULL,
                 workspace_root TEXT NOT NULL,
                 model_connection TEXT,
                 model_id TEXT,
                 created_at_ms INTEGER NOT NULL,
                 updated_at_ms INTEGER NOT NULL
             );

             CREATE TABLE IF NOT EXISTS events (
                 scope TEXT NOT NULL,
                 seq INTEGER NOT NULL,
                 kind TEXT NOT NULL,
                 payload_json TEXT NOT NULL,
                 created_at_ms INTEGER NOT NULL,
                 PRIMARY KEY (scope, seq)
             );

             CREATE VIRTUAL TABLE IF NOT EXISTS events_fts USING fts5(
                 scope_id UNINDEXED,
                 seq UNINDEXED,
                 kind UNINDEXED,
                 text,
                 tokenize='unicode61'
             );

             CREATE TABLE IF NOT EXISTS event_watermarks (
                 scope TEXT PRIMARY KEY,
                 min_retained_seq INTEGER NOT NULL,
                 max_seq INTEGER NOT NULL
             );

             CREATE TABLE IF NOT EXISTS memory_items (
                 id TEXT PRIMARY KEY,
                 namespace TEXT NOT NULL,
                 title TEXT NOT NULL,
                 content TEXT NOT NULL,
                 tags_json TEXT NOT NULL,
                 created_at_ms INTEGER NOT NULL,
                 updated_at_ms INTEGER NOT NULL
             );

             CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
                 id UNINDEXED,
                 namespace,
                 title,
                 content,
                 tags
             );

             CREATE TABLE IF NOT EXISTS kv (
                 key TEXT PRIMARY KEY,
                 value_json TEXT NOT NULL,
                 updated_at_ms INTEGER NOT NULL
             );

             CREATE TABLE IF NOT EXISTS mcp_servers (
                 id TEXT PRIMARY KEY,
                 json TEXT NOT NULL,
                 updated_at_ms INTEGER NOT NULL
             );

             CREATE TABLE IF NOT EXISTS module_records (
                 kind TEXT NOT NULL,
                 id TEXT NOT NULL,
                 json TEXT NOT NULL,
                 updated_at_ms INTEGER NOT NULL,
                 PRIMARY KEY (kind, id)
             );
             CREATE INDEX IF NOT EXISTS idx_module_records_kind_updated
                 ON module_records (kind, updated_at_ms DESC);

             CREATE TABLE IF NOT EXISTS sync_changes (
                 id TEXT PRIMARY KEY,
                 scope TEXT NOT NULL,
                 entity_kind TEXT NOT NULL,
                 entity_id TEXT NOT NULL,
                 payload_json TEXT NOT NULL,
                 created_at_ms INTEGER NOT NULL,
                 resolved INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX IF NOT EXISTS idx_sync_changes_pending
                 ON sync_changes (resolved, created_at_ms ASC);

             CREATE TABLE IF NOT EXISTS sync_cursors (
                 peer_id TEXT PRIMARY KEY,
                 cursor TEXT NOT NULL,
                 updated_at_ms INTEGER NOT NULL
             );

             CREATE TABLE IF NOT EXISTS doc_chunks (
                 id TEXT PRIMARY KEY,
                 source_id TEXT NOT NULL,
                 path TEXT NOT NULL,
                 chunk_index INTEGER NOT NULL,
                 content TEXT NOT NULL,
                 sha256 TEXT NOT NULL,
                 updated_at_ms INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_doc_chunks_source
                 ON doc_chunks (source_id, path, chunk_index);

             CREATE VIRTUAL TABLE IF NOT EXISTS doc_chunks_fts USING fts5(
                 id UNINDEXED,
                 source_id UNINDEXED,
                 path,
                 content,
                 tokenize='unicode61'
             );

             CREATE TABLE IF NOT EXISTS code_files (
                 workspace_root TEXT NOT NULL,
                 rel_path TEXT NOT NULL,
                 sha256 TEXT NOT NULL,
                 language TEXT NOT NULL,
                 line_count INTEGER NOT NULL,
                 updated_at_ms INTEGER NOT NULL,
                 PRIMARY KEY (workspace_root, rel_path)
             );

             CREATE TABLE IF NOT EXISTS code_symbols (
                 id TEXT PRIMARY KEY,
                 workspace_root TEXT NOT NULL,
                 rel_path TEXT NOT NULL,
                 name TEXT NOT NULL,
                 qualified_name TEXT NOT NULL,
                 kind TEXT NOT NULL,
                 language TEXT NOT NULL,
                 start_line INTEGER NOT NULL,
                 end_line INTEGER NOT NULL,
                 signature TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_code_symbols_ws_file
                 ON code_symbols (workspace_root, rel_path);

             CREATE VIRTUAL TABLE IF NOT EXISTS code_symbols_fts USING fts5(
                 id UNINDEXED,
                 workspace_root UNINDEXED,
                 rel_path,
                 name,
                 qualified_name,
                 signature,
                 tokenize='unicode61'
             );

             CREATE TABLE IF NOT EXISTS code_edges (
                 workspace_root TEXT NOT NULL,
                 from_symbol TEXT NOT NULL,
                 to_symbol TEXT NOT NULL,
                 rel_path TEXT NOT NULL,
                 line INTEGER NOT NULL,
                 edge_kind TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_code_edges_from
                 ON code_edges (workspace_root, from_symbol);
             CREATE INDEX IF NOT EXISTS idx_code_edges_to
                 ON code_edges (workspace_root, to_symbol);
             CREATE INDEX IF NOT EXISTS idx_code_edges_file
                 ON code_edges (workspace_root, rel_path);"
        );
        conn.execute_batch(&schema_batch)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            poisoned: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Returns true if a previous transaction commit failed and fenced this store read-only.
    pub fn is_poisoned(&self) -> bool {
        self.poisoned.load(Ordering::SeqCst)
    }

    fn check_write_fence(&self) -> Result<(), StoreError> {
        if self.poisoned.load(Ordering::SeqCst) {
            return Err(StoreError::Other(
                "store write fence active: connection poisoned after failed commit (query_only)"
                    .into(),
            ));
        }
        Ok(())
    }

    fn poison_connection(&self, conn: &Connection) {
        self.poisoned.store(true, Ordering::SeqCst);
        let _ = conn.execute_batch("PRAGMA query_only = ON;");
    }

    fn with_conn<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T, StoreError>) -> Result<T, StoreError> {
        let mut guard = self
            .conn
            .lock()
            .map_err(|_| StoreError::Other("store mutex poisoned".into()))?;
        f(&mut guard)
    }

    fn with_write_conn<T>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        self.check_write_fence()?;
        self.with_conn(f)
    }

    fn run_write_tx<T>(
        &self,
        f: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        self.check_write_fence()?;
        self.with_conn(|conn| {
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let out = f(&tx)?;
            if let Err(e) = tx.commit() {
                let is_transient_lock = matches!(
                    e,
                    rusqlite::Error::SqliteFailure(ffi_err, _)
                        if matches!(
                            ffi_err.code,
                            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                        )
                );
                if !is_transient_lock {
                    self.poison_connection(conn);
                }
                return Err(StoreError::Sqlite(e));
            }
            Ok(out)
        })
    }

    // --- Sessions ---

    pub fn upsert_session(&self, session: &Session) -> Result<(), StoreError> {
        self.with_write_conn(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, title, workspace_root, model_connection, model_id, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO UPDATE SET
                   title = excluded.title,
                   workspace_root = excluded.workspace_root,
                   model_connection = excluded.model_connection,
                   model_id = excluded.model_id,
                   updated_at_ms = excluded.updated_at_ms",
                params![
                    session.id,
                    session.title,
                    session.workspace_root,
                    session.model_connection,
                    session.model_id,
                    session.created_at_ms,
                    session.updated_at_ms,
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_session(&self, id: &str) -> Result<Option<Session>, StoreError> {
        self.with_conn(|conn| {
            let row = conn
                .query_row(
                    "SELECT id, title, workspace_root, model_connection, model_id, created_at_ms, updated_at_ms
                     FROM sessions WHERE id = ?1",
                    params![id],
                    |r| {
                        Ok(Session {
                            id: r.get(0)?,
                            title: r.get(1)?,
                            workspace_root: r.get(2)?,
                            model_connection: r.get(3)?,
                            model_id: r.get(4)?,
                            created_at_ms: r.get(5)?,
                            updated_at_ms: r.get(6)?,
                        })
                    },
                )
                .optional()?;
            Ok(row)
        })
    }

    pub fn list_sessions(&self, limit: usize) -> Result<Vec<Session>, StoreError> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, title, workspace_root, model_connection, model_id, created_at_ms, updated_at_ms
                 FROM sessions ORDER BY updated_at_ms DESC LIMIT ?1",
            )?;
            let rows = stmt
                .query_map(params![limit.max(1) as i64], |r| {
                    Ok(Session {
                        id: r.get(0)?,
                        title: r.get(1)?,
                        workspace_root: r.get(2)?,
                        model_connection: r.get(3)?,
                        model_id: r.get(4)?,
                        created_at_ms: r.get(5)?,
                        updated_at_ms: r.get(6)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn delete_session(&self, id: &str) -> Result<(), StoreError> {
        let state_key = format!("session_state:{id}");
        let todos_key = format!("todos:{id}");
        let plan_key = format!("plan:{id}");
        self.run_write_tx(|tx| {
            tx.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
            tx.execute("DELETE FROM events WHERE scope = ?1", params![id])?;
            tx.execute("DELETE FROM events_fts WHERE scope_id = ?1", params![id])?;
            tx.execute("DELETE FROM event_watermarks WHERE scope = ?1", params![id])?;
            tx.execute(
                "DELETE FROM kv WHERE key IN (?1, ?2, ?3)",
                params![state_key, todos_key, plan_key],
            )?;
            Ok(())
        })
    }

    // --- Per-Scope Append-Only Events with Isolated Retention ---

    pub fn append_event(
        &self,
        scope: &str,
        kind: &str,
        payload: Value,
        max_events_per_scope: i64,
    ) -> Result<SessionEvent, StoreError> {
        let cap = max_events_per_scope.max(1);
        let payload_json = serde_json::to_string(&payload)?;
        let searchable_text = extract_searchable_event_text(&payload);
        let ts = now_ms();
        self.run_write_tx(|tx| {
            let current_max: i64 = tx
                .query_row(
                    "SELECT max_seq FROM event_watermarks WHERE scope = ?1",
                    params![scope],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or(0);
            let next_seq = current_max + 1;
            tx.execute(
                "INSERT INTO events (scope, seq, kind, payload_json, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![scope, next_seq, kind, payload_json, ts],
            )?;
            if !searchable_text.is_empty() {
                tx.execute(
                    "INSERT INTO events_fts (scope_id, seq, kind, text)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![scope, next_seq, kind, searchable_text],
                )?;
            }
            // Per-scope pruning: NEVER evicts events belonging to any other scope!
            let cutoff_seq = next_seq.saturating_sub(cap);
            if cutoff_seq > 0 {
                tx.execute(
                    "DELETE FROM events WHERE scope = ?1 AND seq <= ?2",
                    params![scope, cutoff_seq],
                )?;
                tx.execute(
                    "DELETE FROM events_fts WHERE scope_id = ?1 AND CAST(seq AS INTEGER) <= ?2",
                    params![scope, cutoff_seq],
                )?;
            }
            let min_retained = (cutoff_seq + 1).max(1);
            tx.execute(
                "INSERT INTO event_watermarks (scope, min_retained_seq, max_seq)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(scope) DO UPDATE SET
                   min_retained_seq = MAX(event_watermarks.min_retained_seq, excluded.min_retained_seq),
                   max_seq = excluded.max_seq",
                params![scope, min_retained, next_seq],
            )?;
            tx.execute(
                "UPDATE sessions SET updated_at_ms = ?1 WHERE id = ?2",
                params![ts, scope],
            )?;
            Ok(SessionEvent {
                scope: scope.to_string(),
                seq: next_seq,
                kind: kind.to_string(),
                payload,
                created_at_ms: ts,
            })
        })
    }

    /// Delete events at or after `from_seq` within `scope` (for Edit & Resend / Regenerate).
    pub fn truncate_events_from(&self, scope: &str, from_seq: i64) -> Result<usize, StoreError> {
        self.run_write_tx(|tx| {
            let deleted = tx.execute(
                "DELETE FROM events WHERE scope = ?1 AND seq >= ?2",
                params![scope, from_seq],
            )?;
            tx.execute(
                "DELETE FROM events_fts WHERE scope_id = ?1 AND CAST(seq AS INTEGER) >= ?2",
                params![scope, from_seq],
            )?;
            let new_max: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(seq), 0) FROM events WHERE scope = ?1",
                    params![scope],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let new_min: i64 = tx
                .query_row(
                    "SELECT COALESCE(MIN(seq), 1) FROM events WHERE scope = ?1",
                    params![scope],
                    |r| r.get(0),
                )
                .unwrap_or(1);
            tx.execute(
                "INSERT INTO event_watermarks (scope, min_retained_seq, max_seq)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(scope) DO UPDATE SET
                   min_retained_seq = excluded.min_retained_seq,
                   max_seq = excluded.max_seq",
                params![scope, new_min, new_max],
            )?;
            Ok(deleted)
        })
    }

    /// Full-text search across conversation transcript events (`events_fts`) with BM25 ranking,
    /// capped token expression depth, and CJK / non-ASCII `LIKE` fallback.
    pub fn search_events_fts(
        &self,
        scope_id: Option<&str>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<StoredEvent>, StoreError> {
        let sanitized: String = query.chars().filter(|&c| c != '\0').collect();
        let trimmed = sanitized.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        let lim = limit.max(1) as i64;
        self.with_conn(|conn| {
            let mut out = Vec::new();
            if let Some(fts_query) = build_fts5_or_query(trimmed) {
                let mut stmt = conn.prepare(
                    "SELECT e.scope, e.seq, e.kind, e.payload_json, e.created_at_ms
                     FROM events_fts f
                     JOIN events e ON e.scope = f.scope_id AND e.seq = CAST(f.seq AS INTEGER)
                     WHERE events_fts MATCH ?1
                       AND (?2 IS NULL OR e.scope = ?2)
                     ORDER BY bm25(events_fts) ASC, e.created_at_ms DESC, e.seq DESC
                     LIMIT ?3",
                )?;
                let rows = stmt.query_map(params![fts_query, scope_id, lim], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                })?;
                for row in rows {
                    let (scope, seq, kind, raw, created_at_ms) = row?;
                    let payload = serde_json::from_str(&raw)?;
                    out.push(StoredEvent {
                        scope,
                        seq,
                        kind,
                        payload,
                        created_at_ms,
                    });
                }
            }

            if out.is_empty() && trimmed.chars().any(|c| !c.is_ascii()) {
                let like_pat = escape_like_pattern(trimmed);
                let mut stmt = conn.prepare(
                    "SELECT e.scope, e.seq, e.kind, e.payload_json, e.created_at_ms
                     FROM events_fts f
                     JOIN events e ON e.scope = f.scope_id AND e.seq = CAST(f.seq AS INTEGER)
                     WHERE (?1 IS NULL OR e.scope = ?1)
                       AND f.text LIKE ?2 ESCAPE '\\'
                     ORDER BY e.created_at_ms DESC, e.seq DESC
                     LIMIT ?3",
                )?;
                let rows = stmt.query_map(params![scope_id, like_pat, lim], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                })?;
                for row in rows {
                    let (scope, seq, kind, raw, created_at_ms) = row?;
                    let payload = serde_json::from_str(&raw)?;
                    out.push(StoredEvent {
                        scope,
                        seq,
                        kind,
                        payload,
                        created_at_ms,
                    });
                }
            }
            Ok(out)
        })
    }

    /// Read an ordered slice of transcript events starting at `from_seq` (inclusive).
    pub fn read_event_range(
        &self,
        scope_id: &str,
        from_seq: i64,
        limit: usize,
    ) -> Result<Vec<StoredEvent>, StoreError> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT scope, seq, kind, payload_json, created_at_ms
                 FROM events
                 WHERE scope = ?1 AND seq >= ?2
                 ORDER BY seq ASC
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![scope_id, from_seq, limit.max(1) as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (scope, seq, kind, raw, created_at_ms) = row?;
                let payload = serde_json::from_str(&raw)?;
                out.push(StoredEvent {
                    scope,
                    seq,
                    kind,
                    payload,
                    created_at_ms,
                });
            }
            Ok(out)
        })
    }

    pub fn events_after(
        &self,
        scope: &str,
        after_seq: i64,
        limit: usize,
    ) -> Result<EventPage, StoreError> {
        self.with_conn(|conn| {
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)?;
            let watermark: Option<(i64, i64)> = tx
                .query_row(
                    "SELECT min_retained_seq, max_seq FROM event_watermarks WHERE scope = ?1",
                    params![scope],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let Some((min_retained, max_seq)) = watermark else {
                return Ok(EventPage::Events(Vec::new()));
            };
            if after_seq > 0 && after_seq.saturating_add(1) < min_retained {
                return Ok(EventPage::ResyncRequired {
                    earliest_retained_seq: min_retained,
                    latest_seq: max_seq,
                });
            }
            let mut stmt = tx.prepare(
                "SELECT scope, seq, kind, payload_json, created_at_ms
                 FROM events
                 WHERE scope = ?1 AND seq > ?2
                 ORDER BY seq ASC
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![scope, after_seq, limit.max(1) as i64], |r| {
                let raw: String = r.get(3)?;
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    raw,
                    r.get::<_, i64>(4)?,
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (s, seq, kind, raw, ts) = row?;
                let payload = serde_json::from_str(&raw)?;
                out.push(SessionEvent {
                    scope: s,
                    seq,
                    kind,
                    payload,
                    created_at_ms: ts,
                });
            }
            Ok(EventPage::Events(out))
        })
    }

    // --- FTS5 Long-Term Memory ---

    pub fn upsert_memory(&self, item: &MemoryItem) -> Result<(), StoreError> {
        let tags_json = serde_json::to_string(&item.tags)?;
        let tags_text = item.tags.join(" ");
        self.run_write_tx(|tx| {
            tx.execute(
                "INSERT INTO memory_items (id, namespace, title, content, tags_json, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO UPDATE SET
                   namespace = excluded.namespace,
                   title = excluded.title,
                   content = excluded.content,
                   tags_json = excluded.tags_json,
                   updated_at_ms = excluded.updated_at_ms",
                params![
                    item.id,
                    item.namespace,
                    item.title,
                    item.content,
                    tags_json,
                    item.created_at_ms,
                    item.updated_at_ms,
                ],
            )?;
            tx.execute("DELETE FROM memory_fts WHERE id = ?1", params![item.id])?;
            tx.execute(
                "INSERT INTO memory_fts (id, namespace, title, content, tags)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![item.id, item.namespace, item.title, item.content, tags_text],
            )?;
            Ok(())
        })
    }

    pub fn search_memory(
        &self,
        query: &str,
        namespace: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryItem>, StoreError> {
        let sanitized: String = query.chars().filter(|&c| c != '\0').collect();
        let trimmed = sanitized.trim();
        self.with_conn(|conn| {
            if trimmed.is_empty() {
                return Self::list_memory_inner(conn, namespace, limit);
            }
            // Bound tokens to 24 to prevent SQLite FTS5 expression-depth errors on long prompts.
            let tokens: Vec<String> = trimmed
                .split_whitespace()
                .filter(|t| !t.is_empty())
                .take(24)
                .map(|tok| format!("\"{}\"", tok.replace('"', "\"\"")))
                .collect();
            if tokens.is_empty() {
                return Self::list_memory_inner(conn, namespace, limit);
            }
            let fts_query = tokens.join(" OR ");
            let mut stmt = conn.prepare(
                "SELECT m.id, m.namespace, m.title, m.content, m.tags_json, m.created_at_ms, m.updated_at_ms
                 FROM memory_fts f
                 JOIN memory_items m ON m.id = f.id
                 WHERE memory_fts MATCH ?1
                   AND (?2 IS NULL OR m.namespace = ?2)
                 ORDER BY bm25(memory_fts) ASC, m.updated_at_ms DESC
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![fts_query, namespace, limit.max(1) as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (id, ns, title, content, tags_raw, c, u) = row?;
                out.push(MemoryItem {
                    id,
                    namespace: ns,
                    title,
                    content,
                    tags: serde_json::from_str(&tags_raw).unwrap_or_default(),
                    created_at_ms: c,
                    updated_at_ms: u,
                });
            }
            // CJK / non-space-delimited substring fallback when unicode61 FTS5 finds 0 hits
            if out.is_empty() && trimmed.chars().any(|c| !c.is_ascii()) {
                let like_pat = format!(
                    "%{}%",
                    trimmed
                        .replace('\\', "\\\\")
                        .replace('%', "\\%")
                        .replace('_', "\\_")
                );
                let mut stmt = conn.prepare(
                    "SELECT id, namespace, title, content, tags_json, created_at_ms, updated_at_ms
                     FROM memory_items
                     WHERE (?1 IS NULL OR namespace = ?1)
                       AND (title LIKE ?2 ESCAPE '\\' OR content LIKE ?2 ESCAPE '\\' OR tags_json LIKE ?2 ESCAPE '\\')
                     ORDER BY updated_at_ms DESC
                     LIMIT ?3",
                )?;
                let rows = stmt.query_map(params![namespace, like_pat, limit.max(1) as i64], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, i64>(5)?,
                        r.get::<_, i64>(6)?,
                    ))
                })?;
                for row in rows {
                    let (id, ns, title, content, tags_raw, c, u) = row?;
                    out.push(MemoryItem {
                        id,
                        namespace: ns,
                        title,
                        content,
                        tags: serde_json::from_str(&tags_raw).unwrap_or_default(),
                        created_at_ms: c,
                        updated_at_ms: u,
                    });
                }
            }
            Ok(out)
        })
    }

    fn list_memory_inner(
        conn: &Connection,
        namespace: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryItem>, StoreError> {
        let mut stmt = conn.prepare(
            "SELECT id, namespace, title, content, tags_json, created_at_ms, updated_at_ms
             FROM memory_items
             WHERE (?1 IS NULL OR namespace = ?1)
             ORDER BY updated_at_ms DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![namespace, limit.max(1) as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, ns, title, content, tags_raw, c, u) = row?;
            out.push(MemoryItem {
                id,
                namespace: ns,
                title,
                content,
                tags: serde_json::from_str(&tags_raw).unwrap_or_default(),
                created_at_ms: c,
                updated_at_ms: u,
            });
        }
        Ok(out)
    }

    pub fn delete_memory(&self, id: &str) -> Result<bool, StoreError> {
        self.run_write_tx(|tx| {
            let n = tx.execute("DELETE FROM memory_items WHERE id = ?1", params![id])?;
            tx.execute("DELETE FROM memory_fts WHERE id = ?1", params![id])?;
            Ok(n > 0)
        })
    }

    // --- KV Settings / Model Config ---

    pub fn set_kv(&self, key: &str, value: &Value) -> Result<(), StoreError> {
        let raw = serde_json::to_string(value)?;
        let ts = now_ms();
        self.with_write_conn(|conn| {
            conn.execute(
                "INSERT INTO kv (key, value_json, updated_at_ms) VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at_ms = excluded.updated_at_ms",
                params![key, raw, ts],
            )?;
            Ok(())
        })
    }

    pub fn get_kv(&self, key: &str) -> Result<Option<Value>, StoreError> {
        self.with_conn(|conn| {
            let raw: Option<String> = conn
                .query_row("SELECT value_json FROM kv WHERE key = ?1", params![key], |r| {
                    r.get(0)
                })
                .optional()?;
            match raw {
                Some(s) => Ok(Some(serde_json::from_str(&s)?)),
                None => Ok(None),
            }
        })
    }

    // --- MCP Servers ---

    pub fn upsert_mcp_server(&self, cfg: &McpServerConfig) -> Result<(), StoreError> {
        let raw = serde_json::to_string(cfg)?;
        let ts = now_ms();
        self.with_write_conn(|conn| {
            conn.execute(
                "INSERT INTO mcp_servers (id, json, updated_at_ms) VALUES (?1, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET json = excluded.json, updated_at_ms = excluded.updated_at_ms",
                params![cfg.id, raw, ts],
            )?;
            Ok(())
        })
    }

    pub fn list_mcp_servers(&self) -> Result<Vec<McpServerConfig>, StoreError> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare("SELECT json FROM mcp_servers ORDER BY id ASC")?;
            let raws = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            raws.into_iter()
                .map(|s| serde_json::from_str(&s).map_err(StoreError::from))
                .collect()
        })
    }

    pub fn delete_mcp_server(&self, id: &str) -> Result<bool, StoreError> {
        self.with_write_conn(|conn| {
            let n = conn.execute("DELETE FROM mcp_servers WHERE id = ?1", params![id])?;
            Ok(n > 0)
        })
    }

    // --- Unified Generic Business-Module CRUD (Eliminates 15K+ lines of v1 DTO boilerplate!) ---

    pub fn put_module_record<T: Serialize>(
        &self,
        kind: &str,
        id: &str,
        record: &T,
    ) -> Result<(), StoreError> {
        let raw = serde_json::to_string(record)?;
        let ts = now_ms();
        self.with_write_conn(|conn| {
            conn.execute(
                "INSERT INTO module_records (kind, id, json, updated_at_ms) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(kind, id) DO UPDATE SET json = excluded.json, updated_at_ms = excluded.updated_at_ms",
                params![kind, id, raw, ts],
            )?;
            Ok(())
        })
    }

    pub fn get_module_record<T: DeserializeOwned>(
        &self,
        kind: &str,
        id: &str,
    ) -> Result<Option<T>, StoreError> {
        self.with_conn(|conn| {
            let raw: Option<String> = conn
                .query_row(
                    "SELECT json FROM module_records WHERE kind = ?1 AND id = ?2",
                    params![kind, id],
                    |r| r.get(0),
                )
                .optional()?;
            match raw {
                Some(s) => Ok(Some(serde_json::from_str(&s)?)),
                None => Ok(None),
            }
        })
    }

    pub fn list_module_records<T: DeserializeOwned>(&self, kind: &str) -> Result<Vec<T>, StoreError> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, json FROM module_records WHERE kind = ?1 ORDER BY updated_at_ms DESC",
            )?;
            let rows = stmt
                .query_map(params![kind], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let mut out = Vec::with_capacity(rows.len());
            for (id, raw) in rows {
                match serde_json::from_str::<T>(&raw) {
                    Ok(rec) => out.push(rec),
                    Err(e) => {
                        eprintln!(
                            "nola-store: skipping incompatible module_record kind='{kind}' id='{id}': {e}"
                        );
                    }
                }
            }
            Ok(out)
        })
    }

    pub fn delete_module_record(&self, kind: &str, id: &str) -> Result<bool, StoreError> {
        self.with_write_conn(|conn| {
            let n = conn.execute(
                "DELETE FROM module_records WHERE kind = ?1 AND id = ?2",
                params![kind, id],
            )?;
            Ok(n > 0)
        })
    }

    // --- Sync Engine (Changes & Peer Cursors) ---

    pub fn record_sync_change(&self, record: &SyncChangeRecord) -> Result<(), StoreError> {
        self.with_write_conn(|conn| {
            conn.execute(
                "INSERT INTO sync_changes (id, scope, entity_kind, entity_id, payload_json, created_at_ms, resolved)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO UPDATE SET
                   scope = excluded.scope,
                   entity_kind = excluded.entity_kind,
                   entity_id = excluded.entity_id,
                   payload_json = excluded.payload_json,
                   created_at_ms = excluded.created_at_ms,
                   resolved = excluded.resolved",
                params![
                    record.id,
                    record.scope,
                    record.entity_kind,
                    record.entity_id,
                    record.payload_json,
                    record.created_at_ms,
                    i64::from(record.resolved),
                ],
            )?;
            Ok(())
        })
    }

    pub fn list_pending_sync_changes(
        &self,
        scope: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SyncChangeRecord>, StoreError> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, scope, entity_kind, entity_id, payload_json, created_at_ms, resolved
                 FROM sync_changes
                 WHERE resolved = 0
                   AND (?1 IS NULL OR scope = ?1)
                 ORDER BY created_at_ms ASC, id ASC
                 LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(params![scope, limit.max(1) as i64], |r| {
                    Ok(SyncChangeRecord {
                        id: r.get(0)?,
                        scope: r.get(1)?,
                        entity_kind: r.get(2)?,
                        entity_id: r.get(3)?,
                        payload_json: r.get(4)?,
                        created_at_ms: r.get(5)?,
                        resolved: r.get::<_, i64>(6)? != 0,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn resolve_sync_change(&self, id: &str) -> Result<bool, StoreError> {
        self.with_write_conn(|conn| {
            let n = conn.execute(
                "UPDATE sync_changes SET resolved = 1 WHERE id = ?1",
                params![id],
            )?;
            Ok(n > 0)
        })
    }

    pub fn get_sync_cursor(&self, peer_id: &str) -> Result<Option<String>, StoreError> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT cursor FROM sync_cursors WHERE peer_id = ?1",
                params![peer_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(StoreError::from)
        })
    }

    pub fn save_sync_cursor(&self, peer_id: &str, cursor: &str) -> Result<(), StoreError> {
        let ts = now_ms();
        self.with_write_conn(|conn| {
            conn.execute(
                "INSERT INTO sync_cursors (peer_id, cursor, updated_at_ms)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(peer_id) DO UPDATE SET
                   cursor = excluded.cursor,
                   updated_at_ms = excluded.updated_at_ms",
                params![peer_id, cursor, ts],
            )?;
            Ok(())
        })
    }

    // --- Data Source Document Chunks FTS5 (RAG) ---

    pub fn replace_source_chunks(
        &self,
        source_id: &str,
        chunks: &[DocChunkRecord],
    ) -> Result<usize, StoreError> {
        self.run_write_tx(|tx| {
            tx.execute(
                "DELETE FROM doc_chunks WHERE source_id = ?1",
                params![source_id],
            )?;
            tx.execute(
                "DELETE FROM doc_chunks_fts WHERE source_id = ?1",
                params![source_id],
            )?;
            for chunk in chunks {
                tx.execute(
                    "INSERT INTO doc_chunks (id, source_id, path, chunk_index, content, sha256, updated_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        chunk.id,
                        source_id,
                        chunk.path,
                        chunk.chunk_index,
                        chunk.content,
                        chunk.sha256,
                        chunk.updated_at_ms,
                    ],
                )?;
                tx.execute(
                    "INSERT INTO doc_chunks_fts (id, source_id, path, content)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![chunk.id, source_id, chunk.path, chunk.content],
                )?;
            }
            Ok(chunks.len())
        })
    }

    pub fn delete_source_chunks(&self, source_id: &str) -> Result<usize, StoreError> {
        self.run_write_tx(|tx| {
            let deleted = tx.execute(
                "DELETE FROM doc_chunks WHERE source_id = ?1",
                params![source_id],
            )?;
            tx.execute(
                "DELETE FROM doc_chunks_fts WHERE source_id = ?1",
                params![source_id],
            )?;
            Ok(deleted)
        })
    }

    pub fn search_doc_chunks(
        &self,
        source_ids: Option<&[String]>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<DocChunkHit>, StoreError> {
        if let Some(ids) = source_ids {
            if ids.is_empty() {
                return Ok(Vec::new());
            }
        }
        let sanitized: String = query.chars().filter(|&c| c != '\0').collect();
        let trimmed = sanitized.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        let lim = limit.max(1) as i64;
        self.with_conn(|conn| {
            let mut out = Vec::new();
            if let Some(fts_query) = build_fts5_or_query(trimmed) {
                let (sql, bind_values) = match source_ids {
                    Some(ids) => {
                        let placeholders = vec!["?"; ids.len()].join(", ");
                        let sql = format!(
                            "SELECT d.id, d.source_id, d.path, d.chunk_index, d.content, d.sha256, d.updated_at_ms, bm25(doc_chunks_fts)
                             FROM doc_chunks_fts f
                             JOIN doc_chunks d ON d.id = f.id
                             WHERE doc_chunks_fts MATCH ?
                               AND d.source_id IN ({placeholders})
                             ORDER BY bm25(doc_chunks_fts) ASC, d.updated_at_ms DESC, d.chunk_index ASC
                             LIMIT ?"
                        );
                        let mut vals: Vec<rusqlite::types::Value> =
                            Vec::with_capacity(ids.len() + 2);
                        vals.push(rusqlite::types::Value::Text(fts_query));
                        for id in ids {
                            vals.push(rusqlite::types::Value::Text(id.clone()));
                        }
                        vals.push(rusqlite::types::Value::Integer(lim));
                        (sql, vals)
                    }
                    None => (
                        "SELECT d.id, d.source_id, d.path, d.chunk_index, d.content, d.sha256, d.updated_at_ms, bm25(doc_chunks_fts)
                         FROM doc_chunks_fts f
                         JOIN doc_chunks d ON d.id = f.id
                         WHERE doc_chunks_fts MATCH ?
                         ORDER BY bm25(doc_chunks_fts) ASC, d.updated_at_ms DESC, d.chunk_index ASC
                         LIMIT ?"
                            .to_string(),
                        vec![
                            rusqlite::types::Value::Text(fts_query),
                            rusqlite::types::Value::Integer(lim),
                        ],
                    ),
                };
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params_from_iter(bind_values), |r| {
                    Ok(DocChunkHit {
                        id: r.get(0)?,
                        source_id: r.get(1)?,
                        path: r.get(2)?,
                        chunk_index: r.get(3)?,
                        content: r.get(4)?,
                        sha256: r.get(5)?,
                        updated_at_ms: r.get(6)?,
                        score: r.get(7)?,
                    })
                })?;
                for row in rows {
                    out.push(row?);
                }
            }

            if out.is_empty() && trimmed.chars().any(|c| !c.is_ascii()) {
                let like_pat = escape_like_pattern(trimmed);
                let (sql, bind_values) = match source_ids {
                    Some(ids) => {
                        let placeholders = vec!["?"; ids.len()].join(", ");
                        let sql = format!(
                            "SELECT id, source_id, path, chunk_index, content, sha256, updated_at_ms
                             FROM doc_chunks
                             WHERE source_id IN ({placeholders})
                               AND (path LIKE ? ESCAPE '\\' OR content LIKE ? ESCAPE '\\')
                             ORDER BY updated_at_ms DESC, chunk_index ASC
                             LIMIT ?"
                        );
                        let mut vals: Vec<rusqlite::types::Value> =
                            Vec::with_capacity(ids.len() + 3);
                        for id in ids {
                            vals.push(rusqlite::types::Value::Text(id.clone()));
                        }
                        vals.push(rusqlite::types::Value::Text(like_pat.clone()));
                        vals.push(rusqlite::types::Value::Text(like_pat));
                        vals.push(rusqlite::types::Value::Integer(lim));
                        (sql, vals)
                    }
                    None => (
                        "SELECT id, source_id, path, chunk_index, content, sha256, updated_at_ms
                         FROM doc_chunks
                         WHERE path LIKE ? ESCAPE '\\' OR content LIKE ? ESCAPE '\\'
                         ORDER BY updated_at_ms DESC, chunk_index ASC
                         LIMIT ?"
                            .to_string(),
                        vec![
                            rusqlite::types::Value::Text(like_pat.clone()),
                            rusqlite::types::Value::Text(like_pat),
                            rusqlite::types::Value::Integer(lim),
                        ],
                    ),
                };
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params_from_iter(bind_values), |r| {
                    Ok(DocChunkHit {
                        id: r.get(0)?,
                        source_id: r.get(1)?,
                        path: r.get(2)?,
                        chunk_index: r.get(3)?,
                        content: r.get(4)?,
                        sha256: r.get(5)?,
                        updated_at_ms: r.get(6)?,
                        score: 0.0,
                    })
                })?;
                for row in rows {
                    out.push(row?);
                }
            }
            Ok(out)
        })
    }

    // --- Code Memory (Tree-sitter AST Index & Call Graph) ---

    pub fn get_code_file_hashes(
        &self,
        workspace_root: &str,
    ) -> Result<HashMap<String, String>, StoreError> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT rel_path, sha256 FROM code_files WHERE workspace_root = ?1",
            )?;
            let rows = stmt.query_map(params![workspace_root], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            let mut out = HashMap::new();
            for row in rows {
                let (rel_path, sha) = row?;
                out.insert(rel_path, sha);
            }
            Ok(out)
        })
    }

    pub fn replace_code_file_index(
        &self,
        file: &CodeFileRow,
        symbols: &[CodeSymbolRow],
        edges: &[CodeEdgeRow],
    ) -> Result<(), StoreError> {
        self.run_write_tx(|tx| {
            tx.execute(
                "INSERT INTO code_files (workspace_root, rel_path, sha256, language, line_count, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(workspace_root, rel_path) DO UPDATE SET
                   sha256 = excluded.sha256,
                   language = excluded.language,
                   line_count = excluded.line_count,
                   updated_at_ms = excluded.updated_at_ms",
                params![
                    file.workspace_root,
                    file.rel_path,
                    file.sha256,
                    file.language,
                    file.line_count,
                    file.updated_at_ms,
                ],
            )?;
            tx.execute(
                "DELETE FROM code_symbols_fts
                 WHERE id IN (
                     SELECT id FROM code_symbols WHERE workspace_root = ?1 AND rel_path = ?2
                 )",
                params![file.workspace_root, file.rel_path],
            )?;
            tx.execute(
                "DELETE FROM code_symbols WHERE workspace_root = ?1 AND rel_path = ?2",
                params![file.workspace_root, file.rel_path],
            )?;
            tx.execute(
                "DELETE FROM code_edges WHERE workspace_root = ?1 AND rel_path = ?2",
                params![file.workspace_root, file.rel_path],
            )?;
            for sym in symbols {
                tx.execute(
                    "INSERT INTO code_symbols (id, workspace_root, rel_path, name, qualified_name, kind, language, start_line, end_line, signature)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        sym.id,
                        file.workspace_root,
                        file.rel_path,
                        sym.name,
                        sym.qualified_name,
                        sym.kind,
                        sym.language,
                        sym.start_line,
                        sym.end_line,
                        sym.signature,
                    ],
                )?;
                tx.execute(
                    "INSERT INTO code_symbols_fts (id, workspace_root, rel_path, name, qualified_name, signature)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        sym.id,
                        file.workspace_root,
                        file.rel_path,
                        sym.name,
                        sym.qualified_name,
                        sym.signature,
                    ],
                )?;
            }
            for edge in edges {
                tx.execute(
                    "INSERT INTO code_edges (workspace_root, from_symbol, to_symbol, rel_path, line, edge_kind)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        file.workspace_root,
                        edge.from_symbol,
                        edge.to_symbol,
                        file.rel_path,
                        edge.line,
                        edge.edge_kind,
                    ],
                )?;
            }
            Ok(())
        })
    }

    pub fn prune_missing_code_files(
        &self,
        workspace_root: &str,
        existing_rel_paths: &HashSet<String>,
    ) -> Result<usize, StoreError> {
        self.run_write_tx(|tx| {
            let mut stmt =
                tx.prepare("SELECT rel_path FROM code_files WHERE workspace_root = ?1")?;
            let indexed_paths = stmt
                .query_map(params![workspace_root], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            drop(stmt);

            let mut pruned = 0usize;
            for rel_path in indexed_paths {
                if existing_rel_paths.contains(&rel_path) {
                    continue;
                }
                tx.execute(
                    "DELETE FROM code_symbols_fts
                     WHERE id IN (
                         SELECT id FROM code_symbols WHERE workspace_root = ?1 AND rel_path = ?2
                     )",
                    params![workspace_root, rel_path],
                )?;
                tx.execute(
                    "DELETE FROM code_symbols WHERE workspace_root = ?1 AND rel_path = ?2",
                    params![workspace_root, rel_path],
                )?;
                tx.execute(
                    "DELETE FROM code_edges WHERE workspace_root = ?1 AND rel_path = ?2",
                    params![workspace_root, rel_path],
                )?;
                tx.execute(
                    "DELETE FROM code_files WHERE workspace_root = ?1 AND rel_path = ?2",
                    params![workspace_root, rel_path],
                )?;
                pruned += 1;
            }
            Ok(pruned)
        })
    }

    pub fn search_code_symbols(
        &self,
        workspace_root: &str,
        query: &str,
        kind: Option<&str>,
        limit: usize,
    ) -> Result<Vec<CodeSymbolRow>, StoreError> {
        let sanitized: String = query.chars().filter(|&c| c != '\0').collect();
        let trimmed = sanitized.trim();
        let lim = limit.max(1) as i64;
        self.with_conn(|conn| {
            if trimmed.is_empty() {
                let mut stmt = conn.prepare(
                    "SELECT id, workspace_root, rel_path, name, qualified_name, kind, language, start_line, end_line, signature
                     FROM code_symbols
                     WHERE workspace_root = ?1
                       AND (?2 IS NULL OR kind = ?2)
                     ORDER BY rel_path ASC, start_line ASC
                     LIMIT ?3",
                )?;
                let rows = stmt
                    .query_map(params![workspace_root, kind, lim], |r| {
                        Ok(CodeSymbolRow {
                            id: r.get(0)?,
                            workspace_root: r.get(1)?,
                            rel_path: r.get(2)?,
                            name: r.get(3)?,
                            qualified_name: r.get(4)?,
                            kind: r.get(5)?,
                            language: r.get(6)?,
                            start_line: r.get(7)?,
                            end_line: r.get(8)?,
                            signature: r.get(9)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                return Ok(rows);
            }

            let mut out = Vec::new();
            if let Some(fts_query) = build_fts5_or_query(trimmed) {
                let mut stmt = conn.prepare(
                    "SELECT s.id, s.workspace_root, s.rel_path, s.name, s.qualified_name, s.kind, s.language, s.start_line, s.end_line, s.signature
                     FROM code_symbols_fts f
                     JOIN code_symbols s ON s.id = f.id
                     WHERE code_symbols_fts MATCH ?1
                       AND s.workspace_root = ?2
                       AND (?3 IS NULL OR s.kind = ?3)
                     ORDER BY bm25(code_symbols_fts) ASC, s.rel_path ASC, s.start_line ASC
                     LIMIT ?4",
                )?;
                let rows = stmt.query_map(params![fts_query, workspace_root, kind, lim], |r| {
                    Ok(CodeSymbolRow {
                        id: r.get(0)?,
                        workspace_root: r.get(1)?,
                        rel_path: r.get(2)?,
                        name: r.get(3)?,
                        qualified_name: r.get(4)?,
                        kind: r.get(5)?,
                        language: r.get(6)?,
                        start_line: r.get(7)?,
                        end_line: r.get(8)?,
                        signature: r.get(9)?,
                    })
                })?;
                for row in rows {
                    out.push(row?);
                }
            }

            // Substring fallback handles partial identifier prefixes (e.g. `parse_st`) and CJK.
            if out.is_empty() {
                let like_pat = escape_like_pattern(trimmed);
                let mut stmt = conn.prepare(
                    "SELECT id, workspace_root, rel_path, name, qualified_name, kind, language, start_line, end_line, signature
                     FROM code_symbols
                     WHERE workspace_root = ?1
                       AND (?2 IS NULL OR kind = ?2)
                       AND (name LIKE ?3 ESCAPE '\\'
                            OR qualified_name LIKE ?3 ESCAPE '\\'
                            OR signature LIKE ?3 ESCAPE '\\'
                            OR rel_path LIKE ?3 ESCAPE '\\')
                     ORDER BY rel_path ASC, start_line ASC
                     LIMIT ?4",
                )?;
                let rows = stmt.query_map(params![workspace_root, kind, like_pat, lim], |r| {
                    Ok(CodeSymbolRow {
                        id: r.get(0)?,
                        workspace_root: r.get(1)?,
                        rel_path: r.get(2)?,
                        name: r.get(3)?,
                        qualified_name: r.get(4)?,
                        kind: r.get(5)?,
                        language: r.get(6)?,
                        start_line: r.get(7)?,
                        end_line: r.get(8)?,
                        signature: r.get(9)?,
                    })
                })?;
                for row in rows {
                    out.push(row?);
                }
            }
            Ok(out)
        })
    }

    pub fn list_code_edges_from(
        &self,
        workspace_root: &str,
        from_symbol: &str,
        limit: usize,
    ) -> Result<Vec<CodeEdgeRow>, StoreError> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT workspace_root, from_symbol, to_symbol, rel_path, line, edge_kind
                 FROM code_edges
                 WHERE workspace_root = ?1 AND from_symbol = ?2
                 ORDER BY rel_path ASC, line ASC
                 LIMIT ?3",
            )?;
            let rows = stmt
                .query_map(params![workspace_root, from_symbol, limit.max(1) as i64], |r| {
                    Ok(CodeEdgeRow {
                        workspace_root: r.get(0)?,
                        from_symbol: r.get(1)?,
                        to_symbol: r.get(2)?,
                        rel_path: r.get(3)?,
                        line: r.get(4)?,
                        edge_kind: r.get(5)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn list_code_edges_to(
        &self,
        workspace_root: &str,
        to_symbol: &str,
        limit: usize,
    ) -> Result<Vec<CodeEdgeRow>, StoreError> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT workspace_root, from_symbol, to_symbol, rel_path, line, edge_kind
                 FROM code_edges
                 WHERE workspace_root = ?1 AND to_symbol = ?2
                 ORDER BY rel_path ASC, line ASC
                 LIMIT ?3",
            )?;
            let rows = stmt
                .query_map(params![workspace_root, to_symbol, limit.max(1) as i64], |r| {
                    Ok(CodeEdgeRow {
                        workspace_root: r.get(0)?,
                        from_symbol: r.get(1)?,
                        to_symbol: r.get(2)?,
                        rel_path: r.get(3)?,
                        line: r.get(4)?,
                        edge_kind: r.get(5)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn code_index_stats(&self, workspace_root: &str) -> Result<CodeIndexStats, StoreError> {
        self.with_conn(|conn| {
            let (file_count, last_updated_at_ms): (i64, Option<i64>) = conn.query_row(
                "SELECT COUNT(*), MAX(updated_at_ms) FROM code_files WHERE workspace_root = ?1",
                params![workspace_root],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let symbol_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM code_symbols WHERE workspace_root = ?1",
                params![workspace_root],
                |r| r.get(0),
            )?;
            let edge_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM code_edges WHERE workspace_root = ?1",
                params![workspace_root],
                |r| r.get(0),
            )?;
            Ok(CodeIndexStats {
                file_count,
                symbol_count,
                edge_count,
                last_updated_at_ms,
            })
        })
    }

    // --- Atomic WAL-Safe Backup & Validated Restore ---

    /// Atomically snapshot the live database (including active WAL frames) using `VACUUM INTO`
    /// into a sibling temp file, then atomically rename over `destination` on success.
    pub fn backup_to(&self, destination: &Path) -> Result<(), StoreError> {
        if let Some(parent) = destination.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut nonce = [0u8; 8];
        getrandom::getrandom(&mut nonce)
            .map_err(|e| StoreError::Other(format!("csprng failed: {e}")))?;
        let hex: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let tmp_dest = std::path::PathBuf::from(format!("{}.tmp.{hex}", destination.display()));
        let tmp_str = tmp_dest
            .to_str()
            .ok_or_else(|| StoreError::Other("non-UTF8 backup destination path".into()))?;
        let res = self.with_conn(|conn| {
            conn.execute("VACUUM INTO ?1", params![tmp_str])?;
            Ok(())
        });
        if let Err(e) = res {
            let _ = std::fs::remove_file(&tmp_dest);
            return Err(e);
        }
        if let Err(e) = std::fs::rename(&tmp_dest, destination) {
            let _ = std::fs::remove_file(&tmp_dest);
            return Err(StoreError::Io(e));
        }
        Ok(())
    }

    /// Validate a backup SQLite file (`PRAGMA integrity_check`, Nola `application_id`, and
    /// presence of the `sessions` table) before atomically copying & renaming it to `dest_path`.
    pub fn restore_from_backup(backup_path: &Path, dest_path: &Path) -> Result<(), StoreError> {
        if !backup_path.is_file() {
            return Err(StoreError::Other(format!(
                "backup file does not exist: {}",
                backup_path.display()
            )));
        }
        if let Some(parent) = dest_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut nonce = [0u8; 8];
        getrandom::getrandom(&mut nonce)
            .map_err(|e| StoreError::Other(format!("csprng failed: {e}")))?;
        let hex: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let tmp_dest = std::path::PathBuf::from(format!("{}.restore.{hex}", dest_path.display()));
        if let Err(e) = std::fs::copy(backup_path, &tmp_dest) {
            let _ = std::fs::remove_file(&tmp_dest);
            return Err(StoreError::Io(e));
        }

        // Validate the staged copy (SQLite FTS5 `PRAGMA integrity_check` requires a writable
        // handle to validate virtual-table inverted indices, so validating the staged copy
        // keeps `backup_path` completely untouched while running full FTS5 integrity checks).
        let validation: Result<(), StoreError> = (|| {
            let conn = Connection::open_with_flags(&tmp_dest, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
            let mut stmt = conn.prepare("PRAGMA integrity_check;")?;
            let results = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            if results.is_empty() || !results.iter().all(|s| s.eq_ignore_ascii_case("ok")) {
                return Err(StoreError::Other(format!(
                    "backup integrity_check failed: {}",
                    results.join("; ")
                )));
            }
            let app_id: u32 = conn.query_row("PRAGMA application_id;", [], |r| {
                let v: i64 = r.get(0)?;
                Ok(v as u32)
            })?;
            if app_id != 0 && app_id != NOLA_SQLITE_APPLICATION_ID {
                return Err(StoreError::IncompatibleSchema(format!(
                    "backup has foreign application_id 0x{app_id:08X}"
                )));
            }
            let user_ver: u32 = conn.query_row("PRAGMA user_version;", [], |r| {
                let v: i64 = r.get(0)?;
                Ok(v as u32)
            })?;
            if user_ver > NOLA_SQLITE_USER_VERSION {
                return Err(StoreError::IncompatibleSchema(format!(
                    "backup schema user_version {user_ver} is newer than supported version {NOLA_SQLITE_USER_VERSION}"
                )));
            }
            if user_ver != NOLA_SQLITE_USER_VERSION {
                return Err(StoreError::IncompatibleSchema(format!(
                    "backup schema user_version {user_ver} is incompatible with supported version {NOLA_SQLITE_USER_VERSION}"
                )));
            }
            let has_sessions: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'sessions'",
                [],
                |r| r.get(0),
            )?;
            if has_sessions == 0 {
                return Err(StoreError::IncompatibleSchema(
                    "backup database is missing required 'sessions' table".into(),
                ));
            }
            Ok(())
        })();

        let _ = std::fs::remove_file(format!("{}-wal", tmp_dest.display()));
        let _ = std::fs::remove_file(format!("{}-shm", tmp_dest.display()));
        if let Err(e) = validation {
            let _ = std::fs::remove_file(&tmp_dest);
            return Err(e);
        }

        if let Ok(f) = std::fs::File::open(&tmp_dest) {
            let _ = f.sync_all();
        }
        // Remove stale WAL/SHM sidecars BEFORE and AFTER replacing the database file.
        let _ = std::fs::remove_file(format!("{}-wal", dest_path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", dest_path.display()));
        if let Err(e) = std::fs::rename(&tmp_dest, dest_path) {
            let _ = std::fs::remove_file(&tmp_dest);
            return Err(StoreError::Io(e));
        }
        let _ = std::fs::remove_file(format!("{}-wal", dest_path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", dest_path.display()));
        Ok(())
    }
}

fn build_fts5_or_query(trimmed: &str) -> Option<String> {
    // Bound tokens to 24 to prevent SQLite FTS5 expression-depth errors on long prompts.
    let tokens: Vec<String> = trimmed
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .take(24)
        .map(|tok| format!("\"{}\"", tok.replace('"', "\"\"")))
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" OR "))
    }
}

fn escape_like_pattern(trimmed: &str) -> String {
    format!(
        "%{}%",
        trimmed
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}

fn extract_searchable_event_text(payload: &Value) -> String {
    const MAX_BYTES: usize = 32_768;
    const MAX_DEPTH: usize = 16;

    fn visit(val: &Value, depth: usize, out: &mut String) {
        if depth > MAX_DEPTH || out.len() >= MAX_BYTES {
            return;
        }
        match val {
            Value::String(s) => {
                let t = s.trim();
                if t.is_empty() || t.starts_with("data:") && t.len() > 256 {
                    return;
                }
                if !out.is_empty() {
                    out.push(' ');
                }
                let remaining = MAX_BYTES.saturating_sub(out.len());
                if t.len() <= remaining {
                    out.push_str(t);
                } else {
                    let mut end = remaining;
                    while end > 0 && !t.is_char_boundary(end) {
                        end -= 1;
                    }
                    out.push_str(&t[..end]);
                }
            }
            Value::Array(arr) => {
                for item in arr {
                    visit(item, depth + 1, out);
                    if out.len() >= MAX_BYTES {
                        break;
                    }
                }
            }
            Value::Object(map) => {
                for (k, v) in map {
                    if matches!(
                        k.as_str(),
                        "image_base64" | "audio_base64" | "data_url" | "base64" | "bytes_b64"
                    ) {
                        continue;
                    }
                    visit(v, depth + 1, out);
                    if out.len() >= MAX_BYTES {
                        break;
                    }
                }
            }
            _ => {}
        }
    }

    let mut out = String::new();
    visit(payload, 0, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_types::GoalRecord;
    use nola_types::GoalStatus;
    use serde_json::json;

    #[test]
    fn per_scope_event_pruning_never_evicts_quiet_scopes_and_signals_resync() {
        let store = Store::open_in_memory().unwrap();
        // Quiet scope A writes 2 events
        store
            .append_event("scope_a", "msg", json!({"n": 1}), 5)
            .unwrap();
        store
            .append_event("scope_a", "msg", json!({"n": 2}), 5)
            .unwrap();

        // Noisy scope B writes 10 events with cap = 3
        for i in 1..=10 {
            store
                .append_event("scope_b", "delta", json!({"n": i}), 3)
                .unwrap();
        }

        // Scope A MUST still have all 2 events intact!
        match store.events_after("scope_a", 0, 50).unwrap() {
            EventPage::Events(ev) => assert_eq!(ev.len(), 2),
            other => panic!("unexpected resync on quiet scope A: {other:?}"),
        }

        // Scope B cursor 2 fell behind retention [8..=10] -> ResyncRequired
        assert_eq!(
            store.events_after("scope_b", 2, 50).unwrap(),
            EventPage::ResyncRequired {
                earliest_retained_seq: 8,
                latest_seq: 10
            }
        );

        // Scope B from seq 7 returns 8, 9, 10
        match store.events_after("scope_b", 7, 50).unwrap() {
            EventPage::Events(ev) => {
                let seqs: Vec<i64> = ev.into_iter().map(|e| e.seq).collect();
                assert_eq!(seqs, vec![8, 9, 10]);
            }
            other => panic!("expected events, got {other:?}"),
        }
    }

    #[test]
    fn fts5_memory_and_generic_module_crud_and_atomic_backup() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("nola.db");
        let backup_path = dir.path().join("backup.db");
        let store = Store::open(&db_path).unwrap();

        store
            .upsert_memory(&MemoryItem {
                id: "m1".into(),
                namespace: "project".into(),
                title: "KV cache stability".into(),
                content: "Never emit mid-conversation system role messages".into(),
                tags: vec!["llm".into(), "cache".into()],
                created_at_ms: 1,
                updated_at_ms: 2,
            })
            .unwrap();
        let hits = store.search_memory("KV cache", Some("project"), 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "m1");

        let goal = GoalRecord {
            id: "g1".into(),
            title: "Ship v2".into(),
            objective: "Lean 8-crate rewrite".into(),
            workspace_root: "/tmp/ws".into(),
            status: GoalStatus::Running,
            progress_notes: vec!["nola-store done".into()],
            created_at_ms: 10,
            updated_at_ms: 20,
        };
        store.put_module_record("goal", &goal.id, &goal).unwrap();
        let loaded: GoalRecord = store.get_module_record("goal", "g1").unwrap().unwrap();
        assert_eq!(loaded, goal);

        // Atomic WAL-safe backup preserves active data
        store.backup_to(&backup_path).unwrap();
        let restored = Store::open(&backup_path).unwrap();
        let restored_goal: GoalRecord = restored.get_module_record("goal", "g1").unwrap().unwrap();
        assert_eq!(restored_goal, goal);
    }

    #[test]
    fn sqlite_pragmas_foreign_db_rejection_and_backup_restore() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("nola.db");
        let store = Store::open(&db_path).unwrap();
        store
            .with_conn(|conn| {
                let app_id: i64 = conn.query_row("PRAGMA application_id;", [], |r| r.get(0))?;
                let user_ver: i64 = conn.query_row("PRAGMA user_version;", [], |r| r.get(0))?;
                let sync_mode: i64 = conn.query_row("PRAGMA synchronous;", [], |r| r.get(0))?;
                assert_eq!(app_id as u32, NOLA_SQLITE_APPLICATION_ID);
                assert_eq!(user_ver as u32, NOLA_SQLITE_USER_VERSION);
                assert_eq!(sync_mode, 2, "PRAGMA synchronous must be FULL (2)");
                Ok(())
            })
            .unwrap();

        // Reject foreign SQLite database with non-zero foreign application_id
        let foreign_path = dir.path().join("foreign.db");
        {
            let conn = Connection::open(&foreign_path).unwrap();
            conn.execute_batch("PRAGMA application_id = 0x12345678; CREATE TABLE other (id INT);")
                .unwrap();
        }
        assert!(Store::open(&foreign_path).is_err());

        // Tolerate legacy v2 database with application_id == 0 when sessions table exists
        let legacy_path = dir.path().join("legacy.db");
        {
            let conn = Connection::open(&legacy_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE sessions (id TEXT PRIMARY KEY, title TEXT NOT NULL, workspace_root TEXT NOT NULL, model_connection TEXT, model_id TEXT, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL);",
            )
            .unwrap();
        }
        let legacy_store = Store::open(&legacy_path).unwrap();
        legacy_store
            .with_conn(|conn| {
                let app_id: i64 = conn.query_row("PRAGMA application_id;", [], |r| r.get(0))?;
                assert_eq!(app_id as u32, NOLA_SQLITE_APPLICATION_ID);
                Ok(())
            })
            .unwrap();

        // Backup & validated restore_from_backup
        let backup_path = dir.path().join("snap.db");
        let restored_path = dir.path().join("restored.db");
        store.set_kv("k", &json!({"v": 42})).unwrap();
        store.backup_to(&backup_path).unwrap();
        Store::restore_from_backup(&backup_path, &restored_path).unwrap();
        let reopened = Store::open(&restored_path).unwrap();
        assert_eq!(reopened.get_kv("k").unwrap(), Some(json!({"v": 42})));
        // Reject restoring a foreign DB
        assert!(Store::restore_from_backup(&foreign_path, &restored_path).is_err());
    }

    #[test]
    fn list_module_records_skips_corrupt_rows_and_poison_fence_blocks_writes() {
        let store = Store::open_in_memory().unwrap();
        let good = GoalRecord {
            id: "g_ok".into(),
            title: "Valid".into(),
            objective: "Keep running".into(),
            workspace_root: "/tmp".into(),
            status: GoalStatus::Running,
            progress_notes: vec![],
            created_at_ms: 1,
            updated_at_ms: 2,
        };
        store.put_module_record("goal", &good.id, &good).unwrap();
        // Inject one incompatible/corrupt JSON row under the same kind
        store
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO module_records (kind, id, json, updated_at_ms) VALUES (?1, ?2, ?3, ?4)",
                    params!["goal", "g_bad", r#"{"wrong_schema": true}"#, 3i64],
                )?;
                Ok(())
            })
            .unwrap();

        let listed: Vec<GoalRecord> = store.list_module_records("goal").unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "g_ok");

        // Trigger connection poison fence and verify subsequent writes fail closed while reads work
        store
            .with_conn(|conn| {
                store.poison_connection(conn);
                Ok(())
            })
            .unwrap();
        assert!(store.is_poisoned());
        assert!(store.set_kv("blocked", &json!(1)).is_err());
        assert!(store.append_event("s", "k", json!({}), 10).is_err());
        // Reads still succeed under query_only
        assert_eq!(store.list_module_records::<GoalRecord>("goal").unwrap().len(), 1);
        assert!(store.events_after("s", 0, 10).is_ok());
    }

    #[test]
    fn delete_session_cleans_kv_append_updates_timestamp_and_cjk_escapes_backslash() {
        let store = Store::open_in_memory().unwrap();
        store
            .upsert_session(&Session {
                id: "s1".into(),
                title: "Initial".into(),
                workspace_root: "/tmp".into(),
                model_connection: None,
                model_id: None,
                created_at_ms: 100,
                updated_at_ms: 100,
            })
            .unwrap();
        store.set_kv("session_state:s1", &json!({"ok": true})).unwrap();
        store.set_kv("todos:s1", &json!({"todos": []})).unwrap();
        store.set_kv("plan:s1", &json!({"title": "p"})).unwrap();

        let ev = store.append_event("s1", "msg", json!({"x": 1}), 10).unwrap();
        let sess = store.get_session("s1").unwrap().unwrap();
        assert_eq!(sess.updated_at_ms, ev.created_at_ms);
        assert!(sess.updated_at_ms > 100);

        store.delete_session("s1").unwrap();
        assert!(store.get_session("s1").unwrap().is_none());
        assert!(store.get_kv("session_state:s1").unwrap().is_none());
        assert!(store.get_kv("todos:s1").unwrap().is_none());
        assert!(store.get_kv("plan:s1").unwrap().is_none());

        // CJK LIKE fallback properly escapes backslash before % and _
        store
            .upsert_memory(&MemoryItem {
                id: "cjk1".into(),
                namespace: "default".into(),
                title: "路径".into(),
                content: r"测试\foo%bar_baz内容".into(),
                tags: vec![],
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
        let hits = store.search_memory(r"测试\foo%bar_baz", Some("default"), 10).unwrap();
        assert_eq!(hits.len(), 1);
        let miss = store.search_memory(r"测试\fooXbarYbaz", Some("default"), 10).unwrap();
        assert!(miss.is_empty());
    }

    #[test]
    fn fail_closed_rejects_v1_and_future_schemas_in_open_and_restore_without_modification() {
        let dir = tempfile::tempdir().unwrap();

        // 1. Legacy v1 database (application_id = NOLA, user_version = 1)
        let v1_path = dir.path().join("legacy_v1.db");
        {
            let conn = Connection::open(&v1_path).unwrap();
            conn.execute_batch(&format!(
                "PRAGMA application_id = {NOLA_SQLITE_APPLICATION_ID};
                 PRAGMA user_version = 1;
                 CREATE TABLE sessions (id TEXT PRIMARY KEY, title TEXT NOT NULL);"
            ))
            .unwrap();
        }
        let err = Store::open(&v1_path).err().unwrap();
        assert!(
            matches!(err, StoreError::IncompatibleSchema(_)),
            "expected IncompatibleSchema for v1 db, got {err:?}"
        );
        // Verify v1 DB was NEVER upgraded or modified
        {
            let conn = Connection::open(&v1_path).unwrap();
            let user_ver: i64 = conn
                .query_row("PRAGMA user_version;", [], |r| r.get(0))
                .unwrap();
            assert_eq!(user_ver, 1, "v1 database user_version must remain untouched");
        }

        // 2. Future schema database (user_version = 99)
        let future_path = dir.path().join("future_v99.db");
        {
            let conn = Connection::open(&future_path).unwrap();
            conn.execute_batch(&format!(
                "PRAGMA application_id = {NOLA_SQLITE_APPLICATION_ID};
                 PRAGMA user_version = 99;
                 CREATE TABLE sessions (id TEXT PRIMARY KEY, title TEXT NOT NULL);"
            ))
            .unwrap();
        }
        let err = Store::open(&future_path).err().unwrap();
        assert!(matches!(err, StoreError::IncompatibleSchema(_)));

        // 3. restore_from_backup rejects both v1 and future backups BEFORE overwriting dest_path
        let dest_path = dir.path().join("live_v2.db");
        {
            let live = Store::open(&dest_path).unwrap();
            live.set_kv("sentinel", &json!("intact")).unwrap();
        }
        let err_v1 = Store::restore_from_backup(&v1_path, &dest_path).unwrap_err();
        assert!(matches!(err_v1, StoreError::IncompatibleSchema(_)));
        let err_v99 = Store::restore_from_backup(&future_path, &dest_path).unwrap_err();
        assert!(matches!(err_v99, StoreError::IncompatibleSchema(_)));

        // Verify live_v2.db was untouched
        let reopened = Store::open(&dest_path).unwrap();
        assert_eq!(reopened.get_kv("sentinel").unwrap(), Some(json!("intact")));
    }

    #[test]
    fn events_fts_search_recall_cjk_pruning_truncation_and_session_delete() {
        let store = Store::open_in_memory().unwrap();
        store
            .upsert_session(&Session {
                id: "s1".into(),
                title: "Main".into(),
                workspace_root: "/ws".into(),
                model_connection: None,
                model_id: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();

        let e1 = store
            .append_event(
                "s1",
                "user_message",
                json!({"text": "Investigate SQLite WAL checkpoint contention"}),
                10,
            )
            .unwrap();
        let e2 = store
            .append_event(
                "s1",
                "assistant_message",
                json!({"text": "中文检索测试：数据库索引与召回方案"}),
                10,
            )
            .unwrap();
        let e3 = store
            .append_event(
                "s1",
                "assistant_message",
                json!({"text": "Second step uses truncate cleanup check"}),
                10,
            )
            .unwrap();
        store
            .append_event(
                "s2",
                "user_message",
                json!({"text": "Unrelated scope SQLite checkpoint discussion"}),
                10,
            )
            .unwrap();

        // Scoped FTS5 search
        let hits = store
            .search_events_fts(Some("s1"), "SQLite checkpoint", 10)
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].seq, e1.seq);

        // Cross-scope FTS5 search
        let all_hits = store
            .search_events_fts(None, "SQLite checkpoint", 10)
            .unwrap();
        assert_eq!(all_hits.len(), 2);

        // CJK LIKE fallback
        let cjk_hits = store
            .search_events_fts(Some("s1"), "数据库索引", 10)
            .unwrap();
        assert_eq!(cjk_hits.len(), 1);
        assert_eq!(cjk_hits[0].seq, e2.seq);

        // read_event_range inclusive from_seq
        let range = store.read_event_range("s1", e2.seq, 10).unwrap();
        assert_eq!(
            range.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![e2.seq, e3.seq]
        );

        // Truncation cleans events_fts
        let removed = store.truncate_events_from("s1", e3.seq).unwrap();
        assert_eq!(removed, 1);
        assert!(store
            .search_events_fts(Some("s1"), "truncate cleanup", 10)
            .unwrap()
            .is_empty());

        // Per-scope pruning cleans events_fts
        for i in 0..4 {
            store
                .append_event("s_prune", "msg", json!({"text": format!("prune_token_{i}")}), 2)
                .unwrap();
        }
        assert!(store
            .search_events_fts(Some("s_prune"), "prune_token_0", 10)
            .unwrap()
            .is_empty());
        assert_eq!(
            store
                .search_events_fts(Some("s_prune"), "prune_token_3", 10)
                .unwrap()
                .len(),
            1
        );

        // delete_session cleans events_fts
        store.delete_session("s1").unwrap();
        assert!(store
            .search_events_fts(Some("s1"), "SQLite checkpoint", 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn sync_changes_and_cursors_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.get_sync_cursor("peer-a").unwrap(), None);
        store.save_sync_cursor("peer-a", "cur-100").unwrap();
        assert_eq!(
            store.get_sync_cursor("peer-a").unwrap().as_deref(),
            Some("cur-100")
        );
        store.save_sync_cursor("peer-a", "cur-200").unwrap();
        assert_eq!(
            store.get_sync_cursor("peer-a").unwrap().as_deref(),
            Some("cur-200")
        );

        let c1 = SyncChangeRecord {
            id: "chg-1".into(),
            scope: "memory".into(),
            entity_kind: "memory_item".into(),
            entity_id: "m1".into(),
            payload_json: r#"{"op":"upsert"}"#.into(),
            created_at_ms: 10,
            resolved: false,
        };
        let c2 = SyncChangeRecord {
            id: "chg-2".into(),
            scope: "sessions".into(),
            entity_kind: "session".into(),
            entity_id: "s1".into(),
            payload_json: r#"{"op":"upsert"}"#.into(),
            created_at_ms: 20,
            resolved: false,
        };
        store.record_sync_change(&c1).unwrap();
        store.record_sync_change(&c2).unwrap();

        let pending_all = store.list_pending_sync_changes(None, 10).unwrap();
        assert_eq!(pending_all, vec![c1.clone(), c2.clone()]);
        let pending_mem = store.list_pending_sync_changes(Some("memory"), 10).unwrap();
        assert_eq!(pending_mem, vec![c1]);

        assert!(store.resolve_sync_change("chg-1").unwrap());
        assert!(!store.resolve_sync_change("missing").unwrap());
        let after = store.list_pending_sync_changes(None, 10).unwrap();
        assert_eq!(after, vec![c2]);
    }

    #[test]
    fn doc_chunks_fts5_replace_search_cjk_and_delete() {
        let store = Store::open_in_memory().unwrap();
        let inserted = store
            .replace_source_chunks(
                "src-a",
                &[
                    DocChunkRecord {
                        id: "c1".into(),
                        source_id: "src-a".into(),
                        path: "docs/architecture.md".into(),
                        chunk_index: 0,
                        content: "Nola uses SQLite WAL with FTS5 BM25 ranking".into(),
                        sha256: "aaa".into(),
                        updated_at_ms: 100,
                    },
                    DocChunkRecord {
                        id: "c2".into(),
                        source_id: "src-a".into(),
                        path: "docs/zh.md".into(),
                        chunk_index: 1,
                        content: "文档检索支持中文子串回退策略".into(),
                        sha256: "bbb".into(),
                        updated_at_ms: 101,
                    },
                ],
            )
            .unwrap();
        assert_eq!(inserted, 2);

        store
            .replace_source_chunks(
                "src-b",
                &[DocChunkRecord {
                    id: "c3".into(),
                    source_id: "src-b".into(),
                    path: "notes/wal.md".into(),
                    chunk_index: 0,
                    content: "SQLite WAL notes from source B".into(),
                    sha256: "ccc".into(),
                    updated_at_ms: 102,
                }],
            )
            .unwrap();

        let all_hits = store.search_doc_chunks(None, "SQLite WAL", 10).unwrap();
        assert_eq!(all_hits.len(), 2);

        let only_a = store
            .search_doc_chunks(Some(&["src-a".to_string()]), "SQLite WAL", 10)
            .unwrap();
        assert_eq!(only_a.len(), 1);
        assert_eq!(only_a[0].id, "c1");

        // Explicit empty source_ids filter returns empty
        assert!(store
            .search_doc_chunks(Some(&[]), "SQLite WAL", 10)
            .unwrap()
            .is_empty());

        // CJK LIKE fallback
        let cjk = store
            .search_doc_chunks(Some(&["src-a".to_string()]), "中文子串回退", 10)
            .unwrap();
        assert_eq!(cjk.len(), 1);
        assert_eq!(cjk[0].id, "c2");

        // Replace source-a removes old chunks cleanly
        store
            .replace_source_chunks(
                "src-a",
                &[DocChunkRecord {
                    id: "c4".into(),
                    source_id: "src-a".into(),
                    path: "docs/new.md".into(),
                    chunk_index: 0,
                    content: "Brand new replacement document".into(),
                    sha256: "ddd".into(),
                    updated_at_ms: 200,
                }],
            )
            .unwrap();
        assert!(store
            .search_doc_chunks(Some(&["src-a".to_string()]), "BM25", 10)
            .unwrap()
            .is_empty());
        assert_eq!(
            store
                .search_doc_chunks(Some(&["src-a".to_string()]), "replacement", 10)
                .unwrap()
                .len(),
            1
        );

        let deleted = store.delete_source_chunks("src-a").unwrap();
        assert_eq!(deleted, 1);
        assert!(store
            .search_doc_chunks(Some(&["src-a".to_string()]), "replacement", 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn code_memory_index_search_edges_stats_and_prune() {
        let store = Store::open_in_memory().unwrap();
        let ws = "/repo/nola";

        let file1 = CodeFileRow {
            workspace_root: ws.into(),
            rel_path: "src/policy.rs".into(),
            sha256: "hash1".into(),
            language: "rust".into(),
            line_count: 120,
            updated_at_ms: 1000,
        };
        let sym1 = CodeSymbolRow {
            id: "sym:parse_strict".into(),
            workspace_root: ws.into(),
            rel_path: "src/policy.rs".into(),
            name: "parse_strict".into(),
            qualified_name: "PermissionProfile::parse_strict".into(),
            kind: "function".into(),
            language: "rust".into(),
            start_line: 35,
            end_line: 44,
            signature: "pub fn parse_strict(raw: &str) -> Result<Self, String>".into(),
        };
        let edge1 = CodeEdgeRow {
            workspace_root: ws.into(),
            from_symbol: "daemon::handle_mode".into(),
            to_symbol: "PermissionProfile::parse_strict".into(),
            rel_path: "src/policy.rs".into(),
            line: 40,
            edge_kind: "call".into(),
        };
        store
            .replace_code_file_index(&file1, &[sym1.clone()], &[edge1.clone()])
            .unwrap();

        let file2 = CodeFileRow {
            workspace_root: ws.into(),
            rel_path: "src/legacy.rs".into(),
            sha256: "hash2".into(),
            language: "rust".into(),
            line_count: 45,
            updated_at_ms: 1050,
        };
        let sym2 = CodeSymbolRow {
            id: "sym:old_helper".into(),
            workspace_root: ws.into(),
            rel_path: "src/legacy.rs".into(),
            name: "old_helper".into(),
            qualified_name: "legacy::old_helper".into(),
            kind: "struct".into(),
            language: "rust".into(),
            start_line: 1,
            end_line: 10,
            signature: "pub struct old_helper".into(),
        };
        store
            .replace_code_file_index(&file2, &[sym2], &[])
            .unwrap();

        let hashes = store.get_code_file_hashes(ws).unwrap();
        assert_eq!(hashes.get("src/policy.rs").map(String::as_str), Some("hash1"));
        assert_eq!(hashes.get("src/legacy.rs").map(String::as_str), Some("hash2"));

        // FTS5 symbol search + kind filter
        let hits = store
            .search_code_symbols(ws, "parse_strict", Some("function"), 10)
            .unwrap();
        assert_eq!(hits, vec![sym1.clone()]);

        // Substring prefix fallback
        let prefix_hits = store
            .search_code_symbols(ws, "parse_str", None, 10)
            .unwrap();
        assert_eq!(prefix_hits, vec![sym1]);

        // Call graph edges
        let outgoing = store
            .list_code_edges_from(ws, "daemon::handle_mode", 10)
            .unwrap();
        assert_eq!(outgoing, vec![edge1.clone()]);
        let incoming = store
            .list_code_edges_to(ws, "PermissionProfile::parse_strict", 10)
            .unwrap();
        assert_eq!(incoming, vec![edge1]);

        let stats = store.code_index_stats(ws).unwrap();
        assert_eq!(
            stats,
            CodeIndexStats {
                file_count: 2,
                symbol_count: 2,
                edge_count: 1,
                last_updated_at_ms: Some(1050),
            }
        );

        // Prune missing file removes file, symbols, FTS entries, and edges
        let mut keep = HashSet::new();
        keep.insert("src/policy.rs".to_string());
        let pruned = store.prune_missing_code_files(ws, &keep).unwrap();
        assert_eq!(pruned, 1);
        assert!(store
            .search_code_symbols(ws, "old_helper", None, 10)
            .unwrap()
            .is_empty());
        assert_eq!(store.code_index_stats(ws).unwrap().file_count, 1);
    }
}
