// SPDX-License-Identifier: Apache-2.0
//! Canonical-private stream transaction implementation shared by transition
//! persistence and replica delivery.
//! The caller owns the transaction: this module never commits or retries it.
use crate::{params, Connection, OptionalExtension, StoreError, StoreResult};
use nola_domain::{
    apply_canonical_stream, CanonicalStreamHead, CanonicalStreamOperation, CanonicalStreamOwner,
};

// Logical bytes of the current field generation, matching replica content
// admission. Aggregate Store usage (including retained generations) requires a
// separate physical quota/GC policy; this is not an aggregate storage budget.
const MAX_STREAM_FIELD_BYTES: u64 = 256 * 1024 * 1024;

fn validate_field_quota(total_utf8_bytes: u64) -> StoreResult<()> {
    if total_utf8_bytes > MAX_STREAM_FIELD_BYTES {
        return Err(invalid("canonical stream field exceeds 256 MiB quota"));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> StoreError {
    StoreError::Invariant(message.into())
}

fn owner(operation: &CanonicalStreamOperation) -> &CanonicalStreamOwner {
    match operation {
        CanonicalStreamOperation::Declare { owner }
        | CanonicalStreamOperation::Append { owner, .. }
        | CanonicalStreamOperation::Replace { owner, .. }
        | CanonicalStreamOperation::Close { owner, .. } => owner,
    }
}

/// The persisted run/message/Part relationship is the authority, not any
/// caller-supplied owner, root or global digest. No body is hydrated here.
fn require_owner(connection: &Connection, owner: &CanonicalStreamOwner) -> StoreResult<()> {
    let eligible: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM message_parts p
         JOIN messages m ON m.id=p.message_id
         WHERE p.id=?1 AND m.id=?2 AND m.conversation_id=?3 AND m.run_id=?4
           AND m.role='assistant'
           AND p.kind='text' AND p.state='open'
           AND COALESCE((SELECT e.visible FROM message_projection_events e
             JOIN conversation_projection_operations o ON o.operation_id=e.operation_id
             WHERE e.conversation_id=m.conversation_id AND e.message_id=m.id
             ORDER BY o.resulting_revision DESC LIMIT 1),1)=1)",
        params![
            owner.part_id,
            owner.message_id,
            owner.conversation_id,
            owner.run_id
        ],
        |row| row.get(0),
    )?;
    if !eligible {
        return Err(invalid(
            "canonical stream owner is not an open assistant Text Part",
        ));
    }
    Ok(())
}

fn load_head(connection: &Connection, part: &str) -> StoreResult<Option<CanonicalStreamHead>> {
    let encoded: Option<String> = connection
        .query_row(
            "SELECT head_json FROM canonical_stream_heads WHERE part_id=?1",
            [part],
            |row| row.get(0),
        )
        .optional()?;
    encoded
        .map(|value| {
            serde_json::from_str(&value)
                .map_err(|error| invalid(format!("invalid canonical stream head: {error}")))
        })
        .transpose()
}

fn sql_integer(value: u64) -> StoreResult<i64> {
    i64::try_from(value).map_err(|_| invalid("canonical stream geometry exceeds SQLite integer"))
}

pub(super) fn load_state(
    connection: &Connection,
    owner: &CanonicalStreamOwner,
) -> StoreResult<nola_domain::CanonicalTextState> {
    use nola_domain::CanonicalTextState;
    owner
        .validate()
        .map_err(|error| invalid(format!("invalid stream owner: {error:?}")))?;
    let _snapshot = crate::read_snapshot::pin(connection)?;
    let message: Option<(String, Option<String>, String, bool)> = connection.query_row(
        "SELECT m.conversation_id,m.run_id,m.role,COALESCE((SELECT e.visible
         FROM message_projection_events e JOIN conversation_projection_operations o ON o.operation_id=e.operation_id
         WHERE e.conversation_id=m.conversation_id AND e.message_id=m.id
         ORDER BY o.resulting_revision DESC LIMIT 1),1) FROM messages m WHERE m.id=?1",
        [&owner.message_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).optional()?;
    if message
        .as_ref()
        .is_some_and(|(conversation, run, role, visible)| {
            conversation != &owner.conversation_id
                || run.as_deref() != Some(owner.run_id.as_str())
                || role != "assistant"
                || !visible
        })
    {
        return Err(invalid(
            "canonical text Message owner or visibility mismatch",
        ));
    }
    let part: Option<(String, String, String)> = connection
        .query_row(
            "SELECT message_id,CASE WHEN length(CAST(kind AS BLOB))<=128 THEN kind END,
         CASE WHEN length(CAST(state AS BLOB))<=128 THEN state END FROM message_parts WHERE id=?1",
            [&owner.part_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((message_id, kind, state)) = part else {
        return Ok(CanonicalTextState::Missing);
    };
    if message_id != owner.message_id || message.is_none() {
        return Err(invalid("canonical text Part owner mismatch"));
    }
    if let Some(head) = crate::canonical_stream_read::resolve(
        connection,
        &owner.conversation_id,
        &owner.message_id,
        &owner.part_id,
    )? {
        if head.owner != *owner {
            return Err(invalid("canonical text head owner mismatch"));
        }
        return Ok(CanonicalTextState::Stream(head));
    }
    let field = crate::published_part_body::resolve_field(
        connection,
        &owner.conversation_id,
        &owner.message_id,
        &owner.part_id,
        nola_domain::PartBodyField::ContentText,
    )?;
    Ok(CanonicalTextState::Existing {
        kind,
        state,
        total_utf8_bytes: field
            .total_utf8_bytes()
            .ok_or_else(|| invalid("canonical text field is absent"))?,
    })
}

pub(super) fn apply(
    transaction: &Connection,
    operation: &CanonicalStreamOperation,
) -> StoreResult<CanonicalStreamHead> {
    if transaction.is_autocommit() {
        return Err(invalid(
            "canonical stream write requires an outer transaction",
        ));
    }
    let owner = owner(operation);
    owner
        .validate()
        .map_err(|error| invalid(format!("invalid stream owner: {error:?}")))?;
    require_owner(transaction, owner)?;
    let current = load_head(transaction, &owner.part_id)?;
    if current.is_some() {
        let resolved = crate::canonical_stream_read::resolve(
            transaction,
            &owner.conversation_id,
            &owner.message_id,
            &owner.part_id,
        )?;
        if resolved != current {
            return Err(invalid(
                "stream head disagrees with the committed original field",
            ));
        }
    }
    let next = apply_canonical_stream(current.as_ref(), operation)
        .map_err(|error| StoreError::Conflict(format!("canonical stream rejected: {error:?}")))?;
    // Constant-time head admission: never SUM or read the existing chunks on
    // append merely to enforce this per-field limit. No writes precede this.
    validate_field_quota(next.total_utf8_bytes)?;
    if matches!(operation, CanonicalStreamOperation::Close { .. }) {
        crate::canonical_stream_read::require_complete(
            transaction,
            current.as_ref().expect("validated close head"),
        )?;
    }
    // Never allow a second source of truth beside the stream head. Check this
    // on every mutation, so an unrelated writer cannot silently recreate it.
    let conflicts: bool = transaction.query_row(
        "SELECT content_text<>'' OR EXISTS(
             SELECT 1 FROM message_part_body_references b
             WHERE b.part_id=message_parts.id AND b.field='content_text')
         FROM message_parts WHERE id=?1",
        [&owner.part_id],
        |row| row.get(0),
    )?;
    if conflicts {
        return Err(invalid(
            "canonical stream conflicts with existing original text backing",
        ));
    }
    let (kind, text, offset) = match operation {
        CanonicalStreamOperation::Declare { .. } => ("declare", None, 0),
        CanonicalStreamOperation::Append { delta, .. } => (
            "append",
            Some(delta.as_str()),
            current
                .as_ref()
                .expect("validated append head")
                .total_utf8_bytes,
        ),
        CanonicalStreamOperation::Replace { text, .. } => {
            ("replace", (!text.is_empty()).then_some(text.as_str()), 0)
        }
        CanonicalStreamOperation::Close { .. } => ("close", None, 0),
    };
    // Bound the numeric fields before writing any rows, not just fields used
    // by the chunk insert. JSON u64 otherwise exceeds the storage geometry.
    let generation = sql_integer(next.generation)?;
    sql_integer(next.revision)?;
    sql_integer(next.total_utf8_bytes)?;
    let chunk_count = sql_integer(next.chunk_count.into())?;
    let encoded = serde_json::to_string(&next).map_err(|error| invalid(error.to_string()))?;
    transaction.execute(
        "INSERT INTO canonical_stream_manifests(part_id,root,previous_root,operation,head_json)
         VALUES(?1,?2,?3,?4,?5)",
        params![
            owner.part_id,
            next.root,
            current.as_ref().map(|head| &head.root),
            kind,
            encoded
        ],
    )?;
    if let Some(text) = text {
        transaction.execute(
            "INSERT INTO canonical_stream_chunks(part_id,generation,chunk_index,offset_bytes,text,manifest_root)
             VALUES(?1,?2,?3,?4,?5,?6)",
            params![owner.part_id, generation, chunk_count-1, sql_integer(offset)?, text, next.root],
        )?;
    }
    match current {
        None => {
            transaction.execute(
                "INSERT INTO canonical_stream_heads(part_id,head_json) VALUES(?1,?2)",
                params![owner.part_id, encoded],
            )?;
        }
        Some(previous) => {
            let expected =
                serde_json::to_string(&previous).map_err(|error| invalid(error.to_string()))?;
            let changed = transaction.execute(
                "UPDATE canonical_stream_heads SET head_json=?2 WHERE part_id=?1 AND head_json=?3",
                params![owner.part_id, encoded, expected],
            )?;
            if changed != 1 {
                return Err(StoreError::Conflict("canonical stream head changed".into()));
            }
        }
    }
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_domain::PartBodyField;

    #[test]
    fn logical_field_quota_accepts_exact_limit_and_rejects_overflow() {
        for bytes in [0, MAX_STREAM_FIELD_BYTES - 1, MAX_STREAM_FIELD_BYTES] {
            assert!(validate_field_quota(bytes).is_ok());
        }
        for bytes in [MAX_STREAM_FIELD_BYTES + 1, u64::MAX] {
            assert!(validate_field_quota(bytes).is_err());
        }
    }

    fn setup(path: &std::path::Path) -> crate::NolaStore {
        let store = crate::NolaStore::open(path).unwrap();
        store.connection.execute_batch(
            "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','stream',0,1,1);
             INSERT INTO messages(id,conversation_id,run_id,role,status,ordinal,created_at_ms)
             VALUES('m','c','r','assistant','streaming',0,1);
             INSERT INTO runs(id,conversation_id,kind,assistant_message_id,status,next_event_sequence,created_at_ms,updated_at_ms)
             VALUES('r','c','conversation','m','running',1,1,1);
             INSERT INTO message_parts(id,message_id,kind,ordinal,state,created_at_ms,updated_at_ms)
             VALUES('p','m','text',0,'open',1,1);"
        ).unwrap();
        store
    }

    fn owner() -> CanonicalStreamOwner {
        CanonicalStreamOwner {
            conversation_id: "c".into(),
            run_id: "r".into(),
            message_id: "m".into(),
            part_id: "p".into(),
            field: PartBodyField::ContentText,
        }
    }

    fn append(head: &CanonicalStreamHead, text: &str) -> CanonicalStreamOperation {
        CanonicalStreamOperation::Append {
            owner: owner(),
            expected_head: head.clone(),
            delta: text.into(),
        }
    }

    #[test]
    fn stream_commit_reopen_replace_and_close_preserve_immutable_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stream.sqlite");
        let mut store = setup(&path);
        let tx = store.connection.transaction().unwrap();
        let empty = apply(&tx, &CanonicalStreamOperation::Declare { owner: owner() }).unwrap();
        let first = apply(&tx, &append(&empty, "中😀")).unwrap();
        let second = apply(&tx, &append(&first, "tail")).unwrap();
        assert_eq!(second.total_utf8_bytes, 11);
        tx.commit().unwrap();
        drop(store);
        let mut store = crate::NolaStore::open(&path).unwrap();
        assert_eq!(
            load_head(&store.connection, "p").unwrap(),
            Some(second.clone())
        );
        let tx = store.connection.transaction().unwrap();
        assert!(apply(&tx, &append(&first, "stale")).is_err());
        let replaced = apply(
            &tx,
            &CanonicalStreamOperation::Replace {
                owner: owner(),
                expected_head: second.clone(),
                text: "new".into(),
            },
        )
        .unwrap();
        assert_eq!(replaced.generation, second.generation + 1);
        assert_eq!(replaced.total_utf8_bytes, 3);
        let closed = apply(
            &tx,
            &CanonicalStreamOperation::Close {
                owner: owner(),
                expected_head: replaced.clone(),
            },
        )
        .unwrap();
        assert!(apply(&tx, &append(&closed, "late")).is_err());
        tx.commit().unwrap();
        let chunks: Vec<String> = store
            .connection
            .prepare("SELECT text FROM canonical_stream_chunks ORDER BY generation,chunk_index")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(chunks, ["中😀", "tail", "new"]);
        let inline: String = store
            .connection
            .query_row(
                "SELECT content_text FROM message_parts WHERE id='p'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(inline.is_empty());
    }

    #[test]
    fn late_failure_rolls_back_head_chunks_manifest_and_never_borrows_owner() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = setup(&dir.path().join("rollback.sqlite"));
        {
            let tx = store.connection.transaction().unwrap();
            let empty = apply(&tx, &CanonicalStreamOperation::Declare { owner: owner() }).unwrap();
            let first = apply(&tx, &append(&empty, "first")).unwrap();
            let mut foreign = owner();
            foreign.run_id = "not-the-owner".into();
            assert!(apply(
                &tx,
                &CanonicalStreamOperation::Append {
                    owner: foreign,
                    expected_head: first,
                    delta: "denied".into(),
                }
            )
            .is_err());
            // The enclosing transition must abort the entire batch.
        }
        for table in [
            "canonical_stream_heads",
            "canonical_stream_chunks",
            "canonical_stream_manifests",
        ] {
            let count: i64 = store
                .connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0, "{table}");
        }
        store
            .connection
            .execute(
                "UPDATE message_parts SET content_text='other backing' WHERE id='p'",
                [],
            )
            .unwrap();
        let tx = store.connection.transaction().unwrap();
        assert!(apply(&tx, &CanonicalStreamOperation::Declare { owner: owner() }).is_err());
    }

    #[test]
    fn successor_message_uses_its_actual_run_not_the_initial_assistant() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = setup(&dir.path().join("successor.sqlite"));
        store
            .connection
            .execute_batch(
                "INSERT INTO messages(id,conversation_id,run_id,role,status,ordinal,created_at_ms)
             VALUES('next','c','r','assistant','streaming',1,2);
             INSERT INTO message_parts(id,message_id,kind,ordinal,state,created_at_ms,updated_at_ms)
             VALUES('next-part','next','text',0,'open',2,2);",
            )
            .unwrap();
        let mut next_owner = owner();
        next_owner.message_id = "next".into();
        next_owner.part_id = "next-part".into();
        let tx = store.connection.transaction().unwrap();
        let head = apply(
            &tx,
            &CanonicalStreamOperation::Declare {
                owner: next_owner.clone(),
            },
        )
        .unwrap();
        let mut wrong = next_owner.clone();
        wrong.message_id = "m".into();
        assert!(apply(
            &tx,
            &CanonicalStreamOperation::Append {
                owner: wrong,
                expected_head: head.clone(),
                delta: "denied".into(),
            }
        )
        .is_err());
        apply(
            &tx,
            &CanonicalStreamOperation::Append {
                owner: next_owner,
                expected_head: head,
                delta: "successor".into(),
            },
        )
        .unwrap();
        tx.commit().unwrap();
    }

    #[test]
    fn sql_failure_after_manifest_insert_aborts_the_enclosing_transition() {
        for failure_table in ["canonical_stream_chunks", "canonical_stream_heads"] {
            let dir = tempfile::tempdir().unwrap();
            let mut store = setup(&dir.path().join("sql-failure.sqlite"));
            let tx = store.connection.transaction().unwrap();
            let first = apply(&tx, &CanonicalStreamOperation::Declare { owner: owner() }).unwrap();
            tx.commit().unwrap();
            let action = if failure_table == "canonical_stream_heads" {
                "UPDATE"
            } else {
                "INSERT"
            };
            store
                .connection
                .execute_batch(&format!(
                    "CREATE TRIGGER fail_write BEFORE {action} ON {failure_table}
                 BEGIN SELECT RAISE(ABORT,'injected failure'); END;"
                ))
                .unwrap();
            {
                let tx = store.connection.transaction().unwrap();
                assert!(apply(&tx, &append(&first, "cannot commit")).is_err());
            }
            assert_eq!(load_head(&store.connection, "p").unwrap(), Some(first));
            let counts: (i64, i64) = store
                .connection
                .query_row(
                    "SELECT (SELECT count(*) FROM canonical_stream_manifests),
                        (SELECT count(*) FROM canonical_stream_chunks)",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(counts, (1, 0));
        }
    }

    #[test]
    fn sealed_or_reintroduced_inline_backing_is_never_silently_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = setup(&dir.path().join("backing-conflict.sqlite"));
        // The presence of another authoritative backing is enough to reject;
        // this writer must not load or reinterpret that reference as a stream.
        store
            .connection
            .execute(
                "INSERT INTO message_part_body_references VALUES('p','content_text','{}')",
                [],
            )
            .unwrap();
        {
            let tx = store.connection.transaction().unwrap();
            assert!(apply(&tx, &CanonicalStreamOperation::Declare { owner: owner() }).is_err());
        }
        store
            .connection
            .execute(
                "DELETE FROM message_part_body_references WHERE part_id='p'",
                [],
            )
            .unwrap();
        let tx = store.connection.transaction().unwrap();
        let empty = apply(&tx, &CanonicalStreamOperation::Declare { owner: owner() }).unwrap();
        tx.commit().unwrap();
        store
            .connection
            .execute(
                "UPDATE message_parts SET content_text='old writer' WHERE id='p'",
                [],
            )
            .unwrap();
        {
            let tx = store.connection.transaction().unwrap();
            assert!(apply(&tx, &append(&empty, "new writer")).is_err());
        }
        assert_eq!(load_head(&store.connection, "p").unwrap(), Some(empty));
    }

    #[test]
    fn close_never_certifies_missing_or_changed_earlier_chunks() {
        for damage in [
            "DELETE FROM canonical_stream_chunks WHERE part_id='p' AND chunk_index=0",
            "UPDATE canonical_stream_chunks SET text='xxxxx' WHERE part_id='p' AND chunk_index=0",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut store = setup(&dir.path().join("close-corrupt.sqlite"));
            let tx = store.connection.transaction().unwrap();
            let empty = apply(&tx, &CanonicalStreamOperation::Declare { owner: owner() }).unwrap();
            let first = apply(&tx, &append(&empty, "first")).unwrap();
            let second = apply(&tx, &append(&first, "last")).unwrap();
            tx.commit().unwrap();
            store.connection.execute_batch(damage).unwrap();
            {
                let tx = store.connection.transaction().unwrap();
                assert!(apply(
                    &tx,
                    &CanonicalStreamOperation::Close {
                        owner: owner(),
                        expected_head: second.clone(),
                    }
                )
                .is_err());
            }
            assert_eq!(load_head(&store.connection, "p").unwrap(), Some(second));
            let count: i64 = store
                .connection
                .query_row(
                    "SELECT count(*) FROM canonical_stream_manifests WHERE operation='close'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0);
        }
    }
}
