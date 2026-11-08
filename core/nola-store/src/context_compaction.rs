// SPDX-License-Identifier: Apache-2.0
//! Source-conditional context publication; never replaces a conversation snapshot.
use crate::{params, require_id, NolaStore, StoreError, StoreResult};
use nola_store_port::context::{CommitContextCompactionInput, ContextCompactionSource};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn fingerprint(value: &Value) -> StoreResult<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

fn settings(connection: &Connection, owner: &str) -> StoreResult<Value> {
    let raw: Option<Option<String>> = connection.query_row(
        "SELECT s.settings_json FROM conversations c LEFT JOIN conversation_settings s ON s.conversation_id=c.id WHERE c.id=?1",
        params![owner], |r| r.get(0),
    ).optional()?;
    let raw = raw.ok_or_else(|| StoreError::Conflict("context owner no longer exists".into()))?;
    Ok(match raw {
        Some(raw) => serde_json::from_str(&raw)?,
        None => json!({}),
    })
}

fn source(
    connection: &Connection,
    owner: &str,
    limit: Option<u64>,
) -> StoreResult<ContextCompactionSource> {
    require_id("context.conversation_id", owner)?;
    let fields = settings(connection, owner)?;
    let mut query = connection.prepare("SELECT id,role,status,ordinal,created_at_ms FROM messages WHERE conversation_id=?1 ORDER BY ordinal LIMIT ?2")?;
    let messages = query
        .query_map(params![owner, limit.map(|n| n as i64).unwrap_or(-1)], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut values = Vec::with_capacity(messages.len());
    for (id, role, status, ordinal, created) in messages {
        let mut parts = connection.prepare("SELECT id,kind,ordinal,state,content_text,content_json,content_digest FROM message_parts WHERE message_id=?1 ORDER BY ordinal")?;
        let parts = parts
            .query_map(params![id], |r| {
                Ok(json!([
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<String>>(6)?
                ]))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut references = connection.prepare("SELECT r.part_id,r.field,r.reference_json FROM message_part_body_references r JOIN message_parts p ON p.id=r.part_id WHERE p.message_id=?1 ORDER BY p.ordinal,r.field")?;
        let references = references
            .query_map(params![id], |r| {
                Ok(json!([
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?
                ]))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        values.push(json!([
            id, role, status, ordinal, created, parts, references
        ]));
    }
    Ok(ContextCompactionSource {
        conversation_id: owner.into(),
        message_count: values.len() as u64,
        source_fingerprint: fingerprint(&json!(values))?,
        context_fingerprint: fingerprint(&json!([
            fields.get("sessionMemory"),
            fields.get("compactedMemoryEntries"),
            fields.get("lastCompactedAt")
        ]))?,
    })
}

impl NolaStore {
    pub fn prepare_context_compaction(&self, owner: &str) -> StoreResult<ContextCompactionSource> {
        // A single pinned snapshot covers all message parts and context lineage.
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        source(&self.connection, owner, None)
    }

    pub fn commit_context_compaction(
        &mut self,
        input: &CommitContextCompactionInput,
    ) -> StoreResult<()> {
        let owner = &input.source.conversation_id;
        if (!input.entry.is_null() && input.summary.trim().is_empty())
            || input.validated_source.conversation_id != *owner
            || input.validated_source.message_count < input.source.message_count
        {
            return Err(StoreError::Invariant(
                "invalid context compaction candidate".into(),
            ));
        }
        let transaction = self.write_transaction()?;
        if source(&transaction, owner, Some(input.source.message_count))? != input.source
            || source(&transaction, owner, None)? != input.validated_source
        {
            return Err(StoreError::Conflict(
                "context source changed; replan compaction".into(),
            ));
        }
        if !input.entry.is_null() {
            let entry = input
                .entry
                .as_object()
                .ok_or_else(|| StoreError::Invariant("context entry must be an object".into()))?;
            if entry.get("summary").and_then(Value::as_str) != Some(input.summary.as_str())
                || entry
                    .get("archivedMessageIDs")
                    .and_then(Value::as_array)
                    .is_none_or(|ids| ids.is_empty())
            {
                return Err(StoreError::Invariant("invalid context lineage".into()));
            }
            let ids = entry["archivedMessageIDs"]
                .as_array()
                .expect("validated above");
            let mut query = transaction.prepare(
                "SELECT id FROM messages WHERE conversation_id=?1 ORDER BY ordinal LIMIT ?2",
            )?;
            let prefix = query
                .query_map(params![owner, input.source.message_count as i64], |r| {
                    r.get::<_, String>(0)
                })?
                .collect::<Result<std::collections::HashSet<_>, _>>()?;
            let mut unique = std::collections::HashSet::new();
            for id in ids {
                let id = id.as_str().ok_or_else(|| {
                    StoreError::Invariant("invalid archive message identity".into())
                })?;
                if !prefix.contains(id) || !unique.insert(id) {
                    return Err(StoreError::Conflict(
                        "archive source is outside prepared prefix".into(),
                    ));
                }
            }
        }
        let mut fields = settings(&transaction, owner)?;
        let fields = fields
            .as_object_mut()
            .ok_or_else(|| StoreError::Invariant("invalid context settings".into()))?;
        if !input.entry.is_null() {
            let entries = fields
                .entry("compactedMemoryEntries")
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .ok_or_else(|| StoreError::Invariant("invalid context lineage".into()))?;
            entries.push(input.entry.clone());
        }
        fields.insert("sessionMemory".into(), json!(input.summary));
        let committed_at: String = transaction.query_row(
            "SELECT strftime('%Y-%m-%dT%H:%M:%SZ',?1/1000.0,'unixepoch')",
            params![input.committed_at_ms],
            |r| r.get(0),
        )?;
        fields.insert("lastCompactedAt".into(), json!(committed_at));
        let raw = serde_json::to_string(fields)?;
        transaction.execute("INSERT INTO conversation_settings(conversation_id,settings_json,updated_at_ms) VALUES(?1,?2,?3) ON CONFLICT(conversation_id) DO UPDATE SET settings_json=excluded.settings_json,updated_at_ms=excluded.updated_at_ms",params![owner,raw,input.committed_at_ms])?;
        transaction.execute(
            "UPDATE conversations SET revision=revision+1,updated_at_ms=?2 WHERE id=?1",
            params![owner, input.committed_at_ms],
        )?;
        crate::conversation_metadata_sync::enqueue(&transaction, owner)?;
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> NolaStore {
        let mut store = NolaStore::open(":memory:").unwrap();
        store.create_conversation(&nola_store_port::conversation_profile::CreateConversationInput {
            id: "owner".into(), title: "Title".into(), revision: 1,
            settings_json: Some(json!({"sessionMemory":"old","compactedMemoryEntries":[],"permissionModeRawValue":"cautious"}).to_string()),
            created_at_ms: 1, updated_at_ms: 1,
        }).unwrap();
        append(&store, "first", 1);
        store
    }
    fn append(store: &NolaStore, id: &str, ordinal: i64) {
        store.connection.execute("INSERT INTO messages(id,conversation_id,role,status,ordinal,created_at_ms) VALUES(?1,'owner','user','completed',?2,1)",params![id,ordinal]).unwrap();
        store.connection.execute("INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,created_at_ms,updated_at_ms) VALUES(?1,?1,'text',0,'completed','original',1,1)",params![id]).unwrap();
    }
    fn candidate(
        source: ContextCompactionSource,
        validated_source: ContextCompactionSource,
    ) -> CommitContextCompactionInput {
        CommitContextCompactionInput {
            source,
            validated_source,
            summary: "summary".into(),
            entry: json!({"summary":"summary","archivedMessageIDs":["first"]}),
            committed_at_ms: 10,
        }
    }
    #[test]
    fn append_and_unrelated_configuration_survive_context_commit() {
        let mut store = fixture();
        let original = store.prepare_context_compaction("owner").unwrap();
        append(&store, "tail", 2);
        store.connection.execute("UPDATE conversation_settings SET settings_json=json_set(settings_json,'$.permissionModeRawValue','read_only') WHERE conversation_id='owner'",[]).unwrap();
        let validated = store.prepare_context_compaction("owner").unwrap();
        store
            .commit_context_compaction(&candidate(original, validated))
            .unwrap();
        let fields = settings(&store.connection, "owner").unwrap();
        assert_eq!(fields["permissionModeRawValue"], "read_only");
        assert_eq!(fields["sessionMemory"], "summary");
        assert_eq!(
            store
                .prepare_context_compaction("owner")
                .unwrap()
                .message_count,
            2
        );
        let body: String = store
            .connection
            .query_row(
                "SELECT content_text FROM message_parts WHERE id='first'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(body, "original");
    }
    #[test]
    fn edited_prefix_or_tail_after_budget_check_rejects_without_context_write() {
        for edit_prefix in [true, false] {
            let mut store = fixture();
            let original = store.prepare_context_compaction("owner").unwrap();
            let input = candidate(original.clone(), original);
            if edit_prefix {
                store
                    .connection
                    .execute(
                        "UPDATE message_parts SET content_text='edited' WHERE id='first'",
                        [],
                    )
                    .unwrap();
            } else {
                append(&store, "late-tail", 2);
            }
            assert!(store.commit_context_compaction(&input).is_err());
            assert_eq!(
                settings(&store.connection, "owner").unwrap()["sessionMemory"],
                "old"
            );
        }
    }
    #[test]
    fn competing_summary_commit_invalidates_candidate() {
        let mut store = fixture();
        let original = store.prepare_context_compaction("owner").unwrap();
        let input = candidate(original.clone(), original);
        store.commit_context_compaction(&input).unwrap();
        assert!(store.commit_context_compaction(&input).is_err());
        assert_eq!(
            settings(&store.connection, "owner").unwrap()["compactedMemoryEntries"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}
