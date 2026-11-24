// SPDX-License-Identifier: Apache-2.0
//! SQLite WAL + FTS5 unified store with per-scope event retention,
//! atomic `VACUUM INTO` backup, and generic typed business-module CRUD.

use std::path::Path;
use std::sync::{Arc, Mutex};

use nola_types::{now_ms, McpServerConfig, MemoryItem, Session, SessionEvent};
use rusqlite::{params, Connection, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

/// Default maximum retained events per scope before oldest events in that scope are pruned.
pub const DEFAULT_MAX_EVENTS_PER_SCOPE: i64 = 10_000;

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
    #[error("store error: {0}")]
    Other(String),
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
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA foreign_keys = ON;

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
                 ON module_records (kind, updated_at_ms DESC);",
        )?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn with_conn<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T, StoreError>) -> Result<T, StoreError> {
        let mut guard = self
            .conn
            .lock()
            .map_err(|_| StoreError::Other("store mutex poisoned".into()))?;
        f(&mut guard)
    }

    // --- Sessions ---

    pub fn upsert_session(&self, session: &Session) -> Result<(), StoreError> {
        self.with_conn(|conn| {
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
        self.with_conn(|conn| {
            let tx = conn.transaction()?;
            tx.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
            tx.execute("DELETE FROM events WHERE scope = ?1", params![id])?;
            tx.execute("DELETE FROM event_watermarks WHERE scope = ?1", params![id])?;
            tx.commit()?;
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
        let ts = now_ms();
        self.with_conn(|conn| {
            let tx = conn.transaction()?;
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
            // Per-scope pruning: NEVER evicts events belonging to any other scope!
            let cutoff_seq = next_seq.saturating_sub(cap);
            if cutoff_seq > 0 {
                tx.execute(
                    "DELETE FROM events WHERE scope = ?1 AND seq <= ?2",
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
            tx.commit()?;
            Ok(SessionEvent {
                scope: scope.to_string(),
                seq: next_seq,
                kind: kind.to_string(),
                payload,
                created_at_ms: ts,
            })
        })
    }

    pub fn events_after(
        &self,
        scope: &str,
        after_seq: i64,
        limit: usize,
    ) -> Result<EventPage, StoreError> {
        self.with_conn(|conn| {
            let watermark: Option<(i64, i64)> = conn
                .query_row(
                    "SELECT min_retained_seq, max_seq FROM event_watermarks WHERE scope = ?1",
                    params![scope],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let Some((min_retained, max_seq)) = watermark else {
                return Ok(EventPage::Events(Vec::new()));
            };
            if after_seq > 0 && after_seq + 1 < min_retained {
                return Ok(EventPage::ResyncRequired {
                    earliest_retained_seq: min_retained,
                    latest_seq: max_seq,
                });
            }
            let mut stmt = conn.prepare(
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
        self.with_conn(|conn| {
            let tx = conn.transaction()?;
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
            tx.commit()?;
            Ok(())
        })
    }

    pub fn search_memory(
        &self,
        query: &str,
        namespace: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryItem>, StoreError> {
        let trimmed = query.trim();
        self.with_conn(|conn| {
            if trimmed.is_empty() {
                return Self::list_memory_inner(conn, namespace, limit);
            }
            // Quote terms safely for FTS5 so punctuation never triggers syntax errors.
            let fts_query = trimmed
                .split_whitespace()
                .map(|tok| format!("\"{}\"", tok.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(" OR ");
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
        self.with_conn(|conn| {
            let tx = conn.transaction()?;
            let n = tx.execute("DELETE FROM memory_items WHERE id = ?1", params![id])?;
            tx.execute("DELETE FROM memory_fts WHERE id = ?1", params![id])?;
            tx.commit()?;
            Ok(n > 0)
        })
    }

    // --- KV Settings / Model Config ---

    pub fn set_kv(&self, key: &str, value: &Value) -> Result<(), StoreError> {
        let raw = serde_json::to_string(value)?;
        let ts = now_ms();
        self.with_conn(|conn| {
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
        self.with_conn(|conn| {
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
        self.with_conn(|conn| {
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
        self.with_conn(|conn| {
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
                "SELECT json FROM module_records WHERE kind = ?1 ORDER BY updated_at_ms DESC",
            )?;
            let raws = stmt
                .query_map(params![kind], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            raws.into_iter()
                .map(|s| serde_json::from_str(&s).map_err(StoreError::from))
                .collect()
        })
    }

    pub fn delete_module_record(&self, kind: &str, id: &str) -> Result<bool, StoreError> {
        self.with_conn(|conn| {
            let n = conn.execute(
                "DELETE FROM module_records WHERE kind = ?1 AND id = ?2",
                params![kind, id],
            )?;
            Ok(n > 0)
        })
    }

    // --- Atomic WAL-Safe Backup ---

    /// Atomically snapshot the live database (including active WAL frames) using `VACUUM INTO`.
    pub fn backup_to(&self, destination: &Path) -> Result<(), StoreError> {
        if destination.exists() {
            std::fs::remove_file(destination)?;
        }
        if let Some(parent) = destination.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let dest_str = destination
            .to_str()
            .ok_or_else(|| StoreError::Other("non-UTF8 backup destination path".into()))?;
        self.with_conn(|conn| {
            conn.execute("VACUUM INTO ?1", params![dest_str])?;
            Ok(())
        })
    }
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
}
