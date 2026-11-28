// SPDX-License-Identifier: Apache-2.0
//! Transactional session branching, rewinding and idempotent resend preparation.

use super::{Store, StoreError};
use nola_types::{now_ms, Session};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::Value;

/// A client can address a turn by event sequence or by its opaque message ID.
#[derive(Clone, Copy)]
pub enum HistoryBoundary<'a> {
    Sequence(i64),
    Message(&'a str),
}

impl HistoryBoundary<'_> {
    fn resolve(self, tx: &Transaction<'_>, scope: &str) -> Result<i64, StoreError> {
        match self {
            Self::Sequence(seq) => Ok(seq),
            Self::Message(id) => Ok(tx.query_row(
                "SELECT seq FROM events WHERE scope=?1
                 AND kind IN ('user_message','chat_user','assistant_message','chat_assistant','turn_completed')
                 AND COALESCE(NULLIF(COALESCE(json_extract(payload_json,'$.id'),
                     json_extract(payload_json,'$.message_id')),''),'msg-' || seq)=?2
                 ORDER BY seq LIMIT 1",
                params![scope, id], |row| row.get(0),
            )?),
        }
    }
}

impl Store {
    /// Event scope for the canonical projection of a conversation.
    pub fn conversation_projection_scope(conversation_id: &str) -> String {
        format!("__proj__:{conversation_id}")
    }

    /// Copy a transcript and its model state from the same database snapshot.
    pub fn branch_session(
        &self,
        source: &str,
        dest: &Session,
        through: Option<HistoryBoundary<'_>>,
    ) -> Result<(), StoreError> {
        self.run_write_tx(|tx| {
            tx.execute(
                "INSERT INTO sessions (id,title,workspace_root,model_connection,model_id,created_at_ms,updated_at_ms)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![dest.id, dest.title, dest.workspace_root, dest.model_connection, dest.model_id,
                        dest.created_at_ms, dest.updated_at_ms],
            )?;
            let last = through.map(|boundary| boundary.resolve(tx, source)).transpose()?.unwrap_or(i64::MAX);
            tx.execute("INSERT INTO events SELECT ?2,seq,kind,payload_json,created_at_ms FROM events
                        WHERE scope=?1 AND seq<=?3", params![source, dest.id, last])?;
            tx.execute("INSERT INTO events_fts(scope_id,seq,kind,text)
                        SELECT ?2,seq,kind,text FROM events_fts WHERE scope_id=?1 AND CAST(seq AS INTEGER)<=?3",
                        params![source, dest.id, last])?;
            tx.execute("INSERT INTO event_watermarks SELECT ?1,MIN(seq),MAX(seq) FROM events
                        WHERE scope=?1 HAVING COUNT(*)>0", [&dest.id])?;
            for prefix in ["session_state", "conv_meta", "conv_knowledge"] {
                tx.execute("INSERT INTO kv(key,value_json,updated_at_ms)
                            SELECT ?2,value_json,updated_at_ms FROM kv WHERE key=?1",
                           params![format!("{prefix}:{source}"), format!("{prefix}:{}", dest.id)])?;
            }
            if let Some(from) = last.checked_add(1) {
                trim_model_history(tx, source, &dest.id, from)?;
            }
            Ok(())
        })
    }

    /// Delete events at or after `from_seq` within `scope` (for Edit & Resend / Regenerate).
    pub fn truncate_events_from(&self, scope: &str, from_seq: i64) -> Result<usize, StoreError> {
        self.truncate_at(scope, HistoryBoundary::Sequence(from_seq), false)
            .map(|(_, count)| count)
    }

    pub fn truncate_at(
        &self,
        scope: &str,
        boundary: HistoryBoundary<'_>,
        keep: bool,
    ) -> Result<(i64, usize), StoreError> {
        self.run_write_tx(|tx| {
            let seq = boundary.resolve(tx, scope)?;
            let from = if keep { seq.saturating_add(1) } else { seq };
            Ok((from, truncate(tx, scope, from)?))
        })
    }

    /// Publish the prepared operation and its rewind together, so retries never
    /// truncate the replacement turn after a partial write.
    pub fn prepare_resend(
        &self,
        scope: &str,
        from: Option<HistoryBoundary<'_>>,
        operation_key: &str,
        latest_key: &str,
        prepared: Value,
    ) -> Result<Value, StoreError> {
        self.run_write_tx(|tx| {
            if let Some(raw) = tx
                .query_row(
                    "SELECT value_json FROM kv WHERE key=?1",
                    [operation_key],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
            {
                return Ok(serde_json::from_str(&raw)?);
            }
            if let Some(boundary) = from {
                truncate(tx, scope, boundary.resolve(tx, scope)?)?;
            }
            let raw = serde_json::to_string(&prepared)?;
            for key in [operation_key, latest_key] {
                tx.execute(
                    "INSERT INTO kv(key,value_json,updated_at_ms) VALUES (?1,?2,?3)
                            ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json,
                            updated_at_ms=excluded.updated_at_ms",
                    params![key, raw, now_ms()],
                )?;
            }
            Ok(prepared)
        })
    }
}

fn truncate(tx: &Transaction<'_>, scope: &str, from_seq: i64) -> Result<usize, StoreError> {
    trim_model_history(tx, scope, scope, from_seq)?;
    let deleted = tx.execute(
        "DELETE FROM events WHERE scope = ?1 AND seq >= ?2",
        params![scope, from_seq],
    )?;
    tx.execute(
        "DELETE FROM events_fts WHERE scope_id = ?1 AND CAST(seq AS INTEGER) >= ?2",
        params![scope, from_seq],
    )?;
    if deleted > 0 {
        // A reader of the removed suffix must reload the retained prefix.
        // Window clients observe the maximum of history and projection cursors.
        // Invalidate both in the same transaction even if the projection is
        // ahead of history. Cursor zero still reads the full retained prefix.
        tx.execute(
            "UPDATE event_watermarks SET min_retained_seq=max_seq+2, max_seq=max_seq+1
             WHERE scope IN (?1,?2)",
            params![scope, Store::conversation_projection_scope(scope)],
        )?;
    }
    Ok(deleted)
}

/// Locate the edited turn in the original model history. Keep its full prefix
/// (including media, tool results and provider continuations). Walking back from
/// the newest turn distinguishes repeated prompts. An unavailable compacted
/// boundary aborts the surrounding transaction before any changes are committed.
fn trim_model_history(
    conn: &Connection,
    source: &str,
    dest: &str,
    from: i64,
) -> Result<(), StoreError> {
    let key = format!("session_state:{dest}");
    let Some(raw) = conn
        .query_row("SELECT value_json FROM kv WHERE key=?1", [&key], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
    else {
        return Ok(());
    };
    let mut state: Value = serde_json::from_str(&raw)?;
    let Some(history) = state.get_mut("history").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    let cut = retained_history_len(conn, source, from, history)?;
    if cut == history.len() {
        return Ok(());
    }
    history.truncate(cut);
    conn.execute(
        "UPDATE kv SET value_json=?2,updated_at_ms=?3 WHERE key=?1",
        params![key, serde_json::to_string(&state)?, now_ms()],
    )?;
    Ok(())
}

fn retained_history_len(
    conn: &Connection,
    source: &str,
    from: i64,
    history: &[Value],
) -> Result<usize, StoreError> {
    if history.is_empty() {
        return Ok(0);
    }
    let first_removed: Option<String> = conn
        .query_row(
            "SELECT kind FROM events WHERE scope=?1 AND seq>=?2 AND kind IN
         ('user_message','chat_user','assistant_message','chat_assistant','turn_completed')
         ORDER BY seq LIMIT 1",
            params![source, from],
            |r| r.get(0),
        )
        .optional()?;
    let Some(kind) = first_removed else {
        return Ok(history.len());
    };
    let previous: Option<String> = conn
        .query_row(
            "SELECT kind FROM events WHERE scope=?1 AND seq<?2 AND kind IN
         ('user_message','chat_user','assistant_message','chat_assistant','turn_completed')
         ORDER BY seq DESC LIMIT 1",
            params![source, from],
            |r| r.get(0),
        )
        .optional()?;
    if previous.is_none() {
        let earliest: Option<i64> = conn
            .query_row(
                "SELECT min_retained_seq FROM event_watermarks WHERE scope=?1",
                [source],
                |row| row.get(0),
            )
            .optional()?;
        if earliest == Some(1) {
            return Ok(0);
        }
    }
    // turn_completed and chat_assistant can describe the same reply.
    let keep_user = !matches!(kind.as_str(), "chat_user" | "user_message")
        && matches!(previous.as_deref(), Some("chat_user" | "user_message"));
    let mut stmt = conn.prepare(
        "SELECT seq, COALESCE(json_extract(payload_json,'$.content'),json_extract(payload_json,'$.text'),'')
         FROM events WHERE scope=?1 AND kind IN ('user_message','chat_user') ORDER BY seq DESC")?;
    let mut rows = stmt.query([source])?;
    let mut end = history.len();
    let mut boundary = None;
    while let Some(row) = rows.next()? {
        let seq: i64 = row.get(0)?;
        if seq < from && !keep_user && boundary.is_none() {
            break;
        }
        let prompt: String = row.get(1)?;
        let index = history[..end]
            .iter()
            .rposition(|message| matches_prompt(message, &prompt))
            .ok_or_else(|| {
                StoreError::Other("edit point is unavailable in the saved model history".into())
            })?;
        // Verify the nearest surviving user turn as well. A failed repeated prompt
        // can exist in the transcript without ever reaching persisted model history.
        if seq < from && !keep_user {
            break;
        }
        end = index;
        if seq < from {
            boundary = Some(index + 1);
            break;
        }
        boundary = Some(index);
    }
    match boundary {
        Some(cut) => Ok(cut),
        None if previous.is_some() => Ok(history.len()),
        None => Err(StoreError::Other("missing model history boundary".into())),
    }
}

fn matches_prompt(message: &Value, prompt: &str) -> bool {
    message["role"] == "user"
        && message["content"].as_str().is_some_and(|text| {
            text == prompt
                || text
                    .strip_prefix(prompt.trim_end())
                    .is_some_and(|tail| tail.starts_with("\n\n["))
        })
}
