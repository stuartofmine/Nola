// SPDX-License-Identifier: Apache-2.0
//! Immutable staged restore facts; only the final commit changes visible history.
use super::*;
use nola_domain::archive_restore::{
    ArchiveRestoreEntry, ArchiveRestoreTarget, BeginArchiveRestore, CommitArchiveRestore,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryFact {
    restore_id: String,
    sequence: u64,
    entry: ArchiveRestoreEntry,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetFact {
    restore_id: String,
    target: ArchiveRestoreTarget,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommitFact {
    input: CommitArchiveRestore,
    target_count: u64,
}

fn fact_id(restore_id: &str, suffix: impl std::fmt::Display) -> String {
    format!("{}:{restore_id}:{suffix}", restore_id.len())
}
fn publish(
    connection: &Connection,
    kind: &str,
    id: &str,
    payload: &impl Serialize,
    at: i64,
) -> StoreResult<()> {
    enqueue_sync_change(
        connection,
        kind,
        id,
        1,
        false,
        &serde_json::to_string(payload)?,
        at,
    )
}

pub(super) fn enqueue(connection: &Connection, restore_id: &str, at: i64) -> StoreResult<()> {
    let (begin, commit): (String, String) = connection.query_row(
        "SELECT CASE WHEN length(CAST(begin_json AS BLOB))<=4096 THEN begin_json END,
         CASE WHEN length(CAST(commit_json AS BLOB))<=4096 THEN commit_json END
         FROM archive_restore_sessions WHERE restore_id=?1",
        params![restore_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let begin: BeginArchiveRestore = serde_json::from_str(&begin)?;
    let commit: CommitArchiveRestore = serde_json::from_str(&commit)?;
    publish(connection, "archive_restore_begin", restore_id, &begin, at)?;
    let mut statement = connection.prepare(
        "SELECT sequence,CASE WHEN length(CAST(entry_json AS BLOB))<=131072 THEN entry_json END
         FROM archive_restore_entries WHERE restore_id=?1 ORDER BY sequence",
    )?;
    let mut rows = statement.query(params![restore_id])?;
    while let Some(row) = rows.next()? {
        let sequence = row.get::<_, i64>(0)? as u64;
        let entry: ArchiveRestoreEntry = serde_json::from_str(&row.get::<_, String>(1)?)?;
        let references = entry_references(&entry);
        for reference in references {
            canonical_text_replica::enqueue(connection, reference, at)?;
        }
        publish(
            connection,
            "archive_restore_entry",
            &fact_id(restore_id, sequence),
            &EntryFact {
                restore_id: restore_id.into(),
                sequence,
                entry,
            },
            at,
        )?;
    }
    let mut statement = connection.prepare(
        "SELECT CASE WHEN length(CAST(target_json AS BLOB))<=4096 THEN target_json END
         FROM archive_restore_targets WHERE restore_id=?1 ORDER BY conversation_id",
    )?;
    let mut rows = statement.query(params![restore_id])?;
    let mut target_count = 0;
    while let Some(row) = rows.next()? {
        let target: ArchiveRestoreTarget = serde_json::from_str(&row.get::<_, String>(0)?)?;
        publish(
            connection,
            "archive_restore_target",
            &fact_id(restore_id, &target.conversation_id),
            &TargetFact {
                restore_id: restore_id.into(),
                target,
            },
            at,
        )?;
        target_count += 1;
    }
    publish(
        connection,
        "archive_restore_commit",
        restore_id,
        &CommitFact {
            input: commit,
            target_count,
        },
        at,
    )
}

/// The remote log must acknowledge all immutable staging facts and body seals
/// before it is offered an irreversible final visibility change.
pub(super) fn dependencies_acknowledged(
    connection: &Connection,
    payload: &str,
) -> StoreResult<bool> {
    let commit: CommitFact = serde_json::from_str(payload)?;
    let expected = commit
        .input
        .expected_entry_count
        .checked_add(commit.target_count)
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| StoreError::Invariant("archive dependency count overflow".into()))?;
    let acknowledged:i64=connection.query_row(
        "SELECT COUNT(*) FROM sync_outbox WHERE entity_type IN ('archive_restore_begin','archive_restore_entry','archive_restore_target')
         AND json_extract(payload_json,'$.restore_id')=?1 AND status IN ('acknowledged','unchanged')",
        params![commit.input.restore_id],|row|row.get(0))?;
    if u64::try_from(acknowledged).ok() != Some(expected) {
        return Ok(false);
    }
    let mut statement = connection.prepare(
        "SELECT CASE WHEN length(CAST(entry_json AS BLOB))<=131072 THEN entry_json END
         FROM archive_restore_entries WHERE restore_id=?1 ORDER BY sequence",
    )?;
    let mut rows = statement.query(params![commit.input.restore_id])?;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        count += 1;
        let entry: ArchiveRestoreEntry = serde_json::from_str(&row.get::<_, String>(0)?)?;
        for reference in entry_references(&entry) {
                let acknowledged:bool=connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sync_outbox WHERE entity_type='canonical_text_seal'
                     AND entity_id=?1 AND revision=1 AND status IN ('acknowledged','unchanged') AND payload_json=?2)",
                    params![reference.content_id(),serde_json::to_string(&reference)?],|row|row.get(0))?;
                if !acknowledged {
                    return Ok(false);
                }
        }
    }
    Ok(count == commit.input.expected_entry_count)
}

fn entry_references(entry: &ArchiveRestoreEntry) -> &[nola_domain::CanonicalBodyReference] {
    match entry {
        ArchiveRestoreEntry::Message { references, .. } => references,
        ArchiveRestoreEntry::ToolSource { claim } => std::slice::from_ref(&claim.body),
        ArchiveRestoreEntry::Profile { .. } | ArchiveRestoreEntry::PartDerivation { .. } => &[],
    }
}

pub(super) fn apply(connection: &Connection, change: &RemoteSyncChangeInput) -> StoreResult<()> {
    if change.deleted || change.revision != 1 || change.payload_json.len() > 256 * 1024 {
        return Err(StoreError::Invariant(
            "invalid immutable archive restore fact".into(),
        ));
    }
    match change.entity_type.as_str() {
        "archive_restore_begin" => {
            let begin: BeginArchiveRestore = serde_json::from_str(&change.payload_json)?;
            if change.entity_id != begin.restore_id {
                return Err(StoreError::Invariant(
                    "archive begin identity mismatch".into(),
                ));
            }
            archive_restore::begin_on_connection(connection, &begin, true)?;
        }
        "archive_restore_entry" => {
            let fact: EntryFact = serde_json::from_str(&change.payload_json)?;
            if change.entity_id != fact_id(&fact.restore_id, fact.sequence) {
                return Err(StoreError::Invariant(
                    "archive entry identity mismatch".into(),
                ));
            }
            archive_restore::stage_on_connection(
                connection,
                &fact.restore_id,
                fact.sequence,
                &fact.entry,
                false,
            )?;
        }
        "archive_restore_target" => {
            let fact: TargetFact = serde_json::from_str(&change.payload_json)?;
            if change.entity_id != fact_id(&fact.restore_id, &fact.target.conversation_id) {
                return Err(StoreError::Invariant(
                    "archive target identity mismatch".into(),
                ));
            }
            let encoded = serde_json::to_string(&fact.target)?;
            for identity in [&fact.restore_id, &fact.target.conversation_id] {
                if identity.is_empty()
                    || identity.len() > 128
                    || identity.trim() != identity
                    || identity.chars().any(char::is_control)
                {
                    return Err(StoreError::Invariant(
                        "invalid archive target identity".into(),
                    ));
                }
            }
            if encoded.len() > 4096 {
                return Err(StoreError::Invariant(
                    "archive target exceeds byte budget".into(),
                ));
            }
            let previous: Option<String> = connection
                .query_row(
                    "SELECT CASE WHEN length(CAST(target_json AS BLOB))<=4096 THEN target_json END
                 FROM archive_restore_targets WHERE restore_id=?1 AND conversation_id=?2",
                    params![fact.restore_id, fact.target.conversation_id],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(previous) = previous {
                if previous != encoded {
                    return Err(StoreError::Conflict("archive target replay differs".into()));
                }
            } else {
                if archive_restore::progress_on_connection(connection, &fact.restore_id)?.completed
                {
                    return Err(StoreError::Conflict(
                        "completed archive cannot accept a new target".into(),
                    ));
                }
                connection.execute("INSERT INTO archive_restore_targets(restore_id,conversation_id,target_json) VALUES(?1,?2,?3)",
                    params![fact.restore_id,fact.target.conversation_id,encoded])?;
            }
        }
        "archive_restore_commit" => {
            let fact: CommitFact = serde_json::from_str(&change.payload_json)?;
            if change.entity_id != fact.input.restore_id {
                return Err(StoreError::Invariant(
                    "archive commit identity mismatch".into(),
                ));
            }
            let count: i64 = connection.query_row(
                "SELECT COUNT(*) FROM archive_restore_targets WHERE restore_id=?1",
                params![fact.input.restore_id],
                |row| row.get(0),
            )?;
            if u64::try_from(count).ok() != Some(fact.target_count) {
                return Err(StoreError::Conflict(
                    "archive commit is missing target facts".into(),
                ));
            }
            archive_restore::commit_replica_on_connection(connection, &fact.input)?;
        }
        _ => return Err(StoreError::Invariant("unknown archive restore fact".into())),
    }
    Ok(())
}

impl NolaStore {
    /// Borrow wire facts instead of cloning whole replica pages into Store DTOs.
    /// Call before page application while computing bounded projection updates.
    pub fn archive_restore_projection_ids<'a>(
        &self,
        changes: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> StoreResult<Vec<String>> {
        let changes: Vec<_> = changes.into_iter().collect();
        let mut restores = std::collections::BTreeSet::new();
        for (kind, payload) in &changes {
            if *kind == "archive_restore_commit" {
                let fact: CommitFact = serde_json::from_str(payload)?;
                restores.insert(fact.input.restore_id);
            }
        }
        if restores.is_empty() {
            return Ok(Vec::new());
        }
        let mut identities = std::collections::BTreeSet::new();
        let mut statement = self.connection.prepare("SELECT id FROM conversations")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        for id in rows {
            identities.insert(id?);
        }
        for restore in &restores {
            let mut statement = self.connection.prepare(
                "SELECT conversation_id FROM archive_restore_targets WHERE restore_id=?1
                 UNION SELECT conversation_id FROM archive_restore_profiles WHERE restore_id=?1",
            )?;
            let rows = statement.query_map(params![restore], |row| row.get::<_, String>(0))?;
            for id in rows {
                identities.insert(id?);
            }
        }
        for (kind, payload) in changes {
            match kind {
                "archive_restore_target" => {
                    let fact: TargetFact = serde_json::from_str(payload)?;
                    if restores.contains(&fact.restore_id) {
                        identities.insert(fact.target.conversation_id);
                    }
                }
                "archive_restore_entry" => {
                    let fact: EntryFact = serde_json::from_str(payload)?;
                    if restores.contains(&fact.restore_id) {
                        if let ArchiveRestoreEntry::Profile { profile } = fact.entry {
                            identities.insert(profile.id);
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(identities.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_domain::archive_restore::{ArchiveConversationProfile, ArchiveMessage};
    use nola_domain::CanonicalBodyReference;
    use nola_store_port::archive_restore::ArchiveRestoreRepository;

    fn transfer(
        destination: &mut NolaStore,
        entries: &[StoredSyncChange],
        cursor: i64,
    ) -> StoreResult<ApplyRemoteSyncPageResult> {
        destination.apply_remote_sync_page(&ApplyRemoteSyncPageInput {
            replica_id: "remote".into(),
            next_cursor: cursor + entries.len() as i64,
            applied_at_ms: 10,
            changes: entries
                .iter()
                .enumerate()
                .map(|(index, entry)| RemoteSyncChangeInput {
                    cursor: cursor + index as i64 + 1,
                    operation_id: entry.operation_id.clone(),
                    entity_type: entry.entity_type.clone(),
                    entity_id: entry.entity_id.clone(),
                    revision: entry.revision,
                    deleted: entry.deleted,
                    payload_json: entry.payload_json.clone(),
                    created_at_ms: entry.created_at_ms,
                })
                .collect(),
        })
    }
    fn acknowledge(source: &mut NolaStore, entries: &[StoredSyncChange], cursor: i64) {
        for entry in entries {
            source
                .resolve_sync_change(&entry.operation_id, "acknowledged", Some(cursor), 10)
                .unwrap();
        }
    }
    fn seed(source: &mut NolaStore, destination: &mut NolaStore) -> i64 {
        for id in ["c", "obsolete"] {
            source
                .create_conversation(&CreateConversationInput {
                    id: id.into(),
                    title: id.into(),
                    revision: 1,
                    settings_json: None,
                    created_at_ms: 1,
                    updated_at_ms: 1,
                })
                .unwrap();
        }
        source
            .append_messages(&AppendMessagesInput {
                conversation_id: "c".into(),
                expected_message_count: 0,
                updated_at_ms: 2,
                messages: vec![MessageInput {
                    id: "m".into(),
                    run_id: None,
                    role: "user".into(),
                    status: "completed".into(),
                    ordinal: 1,
                    created_at_ms: 1,
                    completed_at_ms: Some(2),
                    parts: vec![MessagePartInput {
                        id: "p".into(),
                        kind: "text".into(),
                        ordinal: 1,
                        state: "completed".into(),
                        content_text: "OLD ORIGINAL".into(),
                        content_json: None,
                        content_digest: None,
                        created_at_ms: 1,
                        updated_at_ms: 2,
                    }],
                }],
            })
            .unwrap();
        let entries = source.pending_sync_changes(100).unwrap();
        let cursor = transfer(destination, &entries, 0).unwrap().cursor;
        acknowledge(source, &entries, cursor);
        cursor
    }
    fn restore(source: &mut NolaStore, length: u64) -> CanonicalBodyReference {
        restore_fields(source, length, None)
    }

    fn restore_fields(
        source: &mut NolaStore,
        length: u64,
        json: Option<&str>,
    ) -> CanonicalBodyReference {
        let revision = source.archive_directory_revision().unwrap();
        source
            .begin_archive_restore(&BeginArchiveRestore {
                restore_id: "restore".into(),
                expected_directory_revision: revision,
                created_at_ms: 3,
            })
            .unwrap();
        source
            .stage_archive_entry(
                "restore",
                0,
                &ArchiveRestoreEntry::Profile {
                    profile: ArchiveConversationProfile {
                        id: "c".into(),
                        title: "Restored".into(),
                        settings_json: None,
                        created_at_ms: 1,
                        updated_at_ms: 3,
                    },
                },
            )
            .unwrap();
        let reference = CanonicalBodyReference::for_field(
            "archive-body".into(),
            "c".into(),
            "m".into(),
            "p".into(),
            nola_domain::PartBodyField::ContentText,
            length,
            length.div_ceil(32768) as u32,
        )
        .unwrap();
        let mut chunks = (0..reference.chunk_count()).map(|index| {
            let offset = u64::from(index) * 32768;
            let size = (length - offset).min(32768) as usize;
            let text = if offset + size as u64 == length {
                "z".repeat(size - 7) + "TAILNEW"
            } else {
                "z".repeat(size)
            };
            Ok((offset, text))
        });
        source
            .prepare_archive_content("restore", &reference, &mut chunks, 256 * 1024 * 1024)
            .unwrap();
        let mut references = vec![reference.clone()];
        if let Some(json) = json {
            let json_reference = CanonicalBodyReference::for_field(
                "archive-json".into(),
                "c".into(),
                "m".into(),
                "p".into(),
                nola_domain::PartBodyField::ContentJson,
                json.len() as u64,
                1,
            )
            .unwrap();
            source
                .prepare_archive_content(
                    "restore",
                    &json_reference,
                    &mut std::iter::once(Ok((0, json.to_string()))),
                    256 * 1024 * 1024,
                )
                .unwrap();
            references.push(json_reference);
        }
        source
            .stage_archive_entry(
                "restore",
                1,
                &ArchiveRestoreEntry::Message {
                    conversation_id: "c".into(),
                    message: ArchiveMessage {
                        id: "m".into(),
                        run_id: None,
                        role: "user".into(),
                        status: "completed".into(),
                        ordinal: 1,
                        created_at_ms: 1,
                        completed_at_ms: Some(3),
                        parts: vec![MessagePartInput {
                            id: "p".into(),
                            kind: "text".into(),
                            ordinal: 1,
                            state: "completed".into(),
                            content_text: String::new(),
                            content_json: None,
                            content_digest: None,
                            created_at_ms: 1,
                            updated_at_ms: 3,
                        }],
                    },
                    references,
                },
            )
            .unwrap();
        let progress = source
            .stage_archive_entry(
                "restore",
                2,
                &ArchiveRestoreEntry::Profile {
                    profile: ArchiveConversationProfile {
                        id: "new".into(),
                        title: "New conversation".into(),
                        settings_json: None,
                        created_at_ms: 3,
                        updated_at_ms: 3,
                    },
                },
            )
            .unwrap();
        source.begin_write_unit_of_work().unwrap();
        source
            .commit_archive_restore(&CommitArchiveRestore {
                restore_id: "restore".into(),
                expected_directory_revision: revision,
                expected_entry_count: progress.next_sequence,
                manifest_digest: progress.manifest_digest,
                committed_at_ms: 3,
            })
            .unwrap();
        source.commit_write_unit_of_work().unwrap();
        reference
    }
    fn assert_old(destination: &NolaStore) {
        let (revision,text):(i64,String)=destination.connection.query_row(
            "SELECT c.revision,p.content_text FROM conversations c JOIN messages m ON m.conversation_id=c.id JOIN message_parts p ON p.message_id=m.id WHERE c.id='c'",[],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
        assert_eq!(revision, 2);
        assert_eq!(text, "OLD ORIGINAL");
        let obsolete: bool = destination
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM conversations WHERE id='obsolete')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(obsolete);
    }

    #[test]
    fn archive_json_preparation_validates_structure_and_quota_without_partial_receipt() {
        let root = tempfile::tempdir().unwrap();
        let mut store = NolaStore::open(root.path().join("prepare.db")).unwrap();
        store
            .begin_archive_restore(&BeginArchiveRestore {
                restore_id: "prepare".into(),
                expected_directory_revision: store.archive_directory_revision().unwrap(),
                created_at_ms: 1,
            })
            .unwrap();
        store
            .stage_archive_entry(
                "prepare",
                0,
                &ArchiveRestoreEntry::Profile {
                    profile: nola_domain::archive_restore::ArchiveConversationProfile {
                        id: "c".into(),
                        title: "Prepare".into(),
                        settings_json: None,
                        created_at_ms: 1,
                        updated_at_ms: 1,
                    },
                },
            )
            .unwrap();
        for (index, raw, budget) in [(0, "{broken", 100), (1, "{\"valid\":true}", 2)] {
            let reference = CanonicalBodyReference::for_field(
                format!("bad-{index}"),
                "c".into(),
                "m".into(),
                "p".into(),
                nola_domain::PartBodyField::ContentJson,
                raw.len() as u64,
                1,
            )
            .unwrap();
            assert!(store
                .prepare_archive_content(
                    "prepare",
                    &reference,
                    &mut std::iter::once(Ok((0, raw.to_string()))),
                    budget
                )
                .is_err());
            let count: i64 = store
                .connection
                .query_row("SELECT count(*) FROM canonical_text_objects", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0);
            let receipts: i64 = store
                .connection
                .query_row("SELECT count(*) FROM archive_restore_prepared", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(receipts, 0);
        }
    }

    #[test]
    fn two_field_archive_replays_after_restart_and_missing_json_prevents_partial_commit() {
        let root = tempfile::tempdir().unwrap();
        let source_path = root.path().join("two-field-source.db");
        let destination_path = root.path().join("two-field-target.db");
        let mut source = NolaStore::open(&source_path).unwrap();
        let mut destination = NolaStore::open(&destination_path).unwrap();
        let mut cursor = seed(&mut source, &mut destination);
        let json = " { \"text\":\"你好🙂 original\", \"n\":9223372036854775807 } ";
        restore_fields(&mut source, 100, Some(json));
        loop {
            let entries = source.pending_sync_changes(37).unwrap();
            assert!(!entries.is_empty());
            if entries[0].entity_type == "archive_restore_commit" {
                destination
                    .connection
                    .execute(
                        "DELETE FROM canonical_text_chunks WHERE content_id='archive-json'",
                        [],
                    )
                    .unwrap();
                assert!(transfer(&mut destination, &entries, cursor).is_err());
                assert_old(&destination);
                assert_eq!(
                    destination.sync_cursor("remote").unwrap().unwrap().cursor,
                    cursor
                );
                let bindings: i64 = destination
                    .connection
                    .query_row(
                        "SELECT count(*) FROM message_part_body_references WHERE part_id='p'",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(
                    bindings, 0,
                    "neither restored field can become visible alone"
                );
                // Repair the staged immutable dependency, then retry the exact final fact.
                destination
                    .connection
                    .execute(
                        "INSERT INTO canonical_text_chunks VALUES('archive-json',0,?1)",
                        [json],
                    )
                    .unwrap();
                cursor = transfer(&mut destination, &entries, cursor).unwrap().cursor;
                acknowledge(&mut source, &entries, cursor);
                break;
            }
            cursor = transfer(&mut destination, &entries, cursor).unwrap().cursor;
            acknowledge(&mut source, &entries, cursor);
            assert_old(&destination);
            drop(source);
            drop(destination);
            source = NolaStore::open(&source_path).unwrap();
            destination = NolaStore::open(&destination_path).unwrap();
        }
        drop(source);
        drop(destination);
        for path in [&source_path, &destination_path] {
            let reopened = NolaStore::open(path).unwrap();
            let conversation = reopened.load_conversation("c").unwrap().unwrap();
            let part = &conversation.messages[0].parts[0];
            assert_eq!(part.content_text, "z".repeat(93) + "TAILNEW");
            assert_eq!(part.content_json.as_deref(), Some(json));
            let fields: i64 = reopened.connection.query_row(
                "SELECT count(*) FROM message_part_body_references WHERE part_id='p' AND field IN ('content_text','content_json')",
                [], |row| row.get(0)).unwrap();
            assert_eq!(fields, 2);
        }
    }

    #[test]
    fn staged_ten_million_byte_restore_is_atomic_across_database_restarts() {
        let root = tempfile::tempdir().unwrap();
        let source_path = root.path().join("source.db");
        let destination_path = root.path().join("destination.db");
        let mut source = NolaStore::open(&source_path).unwrap();
        let mut destination = NolaStore::open(&destination_path).unwrap();
        let mut cursor = seed(&mut source, &mut destination);
        let reference = restore(&mut source, 10_000_000);
        let mut committed = false;
        let mut last = Vec::new();
        let mut rounds = 0;
        loop {
            let entries = source.pending_sync_changes(37).unwrap();
            if entries.is_empty() {
                break;
            }
            rounds += 1;
            let final_page = entries
                .iter()
                .any(|entry| entry.entity_type == "archive_restore_commit");
            if final_page {
                assert_eq!(
                    entries.len(),
                    1,
                    "final commit waits for acknowledgement of every stage fact"
                );
            }
            cursor = transfer(&mut destination, &entries, cursor).unwrap().cursor;
            acknowledge(&mut source, &entries, cursor);
            committed |= final_page;
            if !committed {
                assert_old(&destination);
            }
            last = entries;
            drop(source);
            drop(destination);
            source = NolaStore::open(&source_path).unwrap();
            destination = NolaStore::open(&destination_path).unwrap();
        }
        assert!(committed && rounds > 8);
        let (revision,inline,reference_json):(i64,String,String)=destination.connection.query_row(
            "SELECT c.revision,p.content_text,r.reference_json FROM conversations c JOIN messages m ON m.conversation_id=c.id JOIN message_parts p ON p.message_id=m.id JOIN message_part_body_references r ON r.part_id=p.id AND r.field='content_text' WHERE c.id='c'",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        assert_eq!(revision, 3);
        assert!(inline.is_empty());
        assert_eq!(
            serde_json::from_str::<CanonicalBodyReference>(&reference_json).unwrap(),
            reference
        );
        canonical_text_replica::require_complete(&destination.connection, &reference).unwrap();
        let tail:String=destination.connection.query_row("SELECT text FROM canonical_text_chunks WHERE content_id='archive-body' ORDER BY offset_bytes DESC LIMIT 1",[],|row|row.get(0)).unwrap();
        assert!(tail.ends_with("TAILNEW"));
        let mut statement=destination.connection.prepare("SELECT offset_bytes,text FROM canonical_text_chunks WHERE content_id='archive-body' ORDER BY offset_bytes").unwrap();
        let mut rows = statement.query([]).unwrap();
        let mut length = 0_u64;
        while let Some(row) = rows.next().unwrap() {
            let offset: i64 = row.get(0).unwrap();
            let text: String = row.get(1).unwrap();
            assert_eq!(offset as u64, length);
            length += text.len() as u64;
            if length == reference.total_utf8_bytes() {
                assert!(text[..text.len() - 7].bytes().all(|byte| byte == b'z'));
                assert!(text.ends_with("TAILNEW"));
            } else {
                assert!(text.bytes().all(|byte| byte == b'z'));
            }
        }
        assert_eq!(length, reference.total_utf8_bytes());
        drop(rows);
        drop(statement);
        let obsolete:(i64,bool)=destination.connection.query_row("SELECT revision,deleted FROM sync_entity_revisions WHERE entity_type='conversation' AND entity_id='obsolete'",[],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
        assert_eq!(obsolete, (2, true));
        assert!(
            destination.pending_sync_changes(10).unwrap().is_empty(),
            "remote restore creates no echo"
        );
        let replay = transfer(&mut destination, &last, cursor).unwrap();
        assert_eq!(replay.skipped, 1);
        let revision_after: i64 = destination
            .connection
            .query_row(
                "SELECT revision FROM conversations WHERE id='c'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(revision_after, 3);
        let mut late_target = last[0].clone();
        late_target.operation_id = "late-target".into();
        late_target.entity_type = "archive_restore_target".into();
        late_target.entity_id = fact_id("restore", "late");
        late_target.payload_json = serde_json::to_string(&TargetFact {
            restore_id: "restore".into(),
            target: ArchiveRestoreTarget {
                conversation_id: "late".into(),
                base_revision: None,
                base_content_revision: None,
                resulting_revision: 1,
                deleted: false,
            },
        })
        .unwrap();
        assert!(transfer(&mut destination, &[late_target], replay.cursor).is_err());
        assert_eq!(
            destination.sync_cursor("remote").unwrap().unwrap().cursor,
            replay.cursor
        );
    }

    #[test]
    fn omitted_target_or_remote_new_conversation_rejects_final_without_losing_old_history() {
        for omit_target in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let mut source = NolaStore::open(root.path().join("source.db")).unwrap();
            let mut destination = NolaStore::open(root.path().join("destination.db")).unwrap();
            let mut cursor = seed(&mut source, &mut destination);
            restore(&mut source, 100);
            if !omit_target {
                destination
                    .create_conversation(&CreateConversationInput {
                        id: "concurrent".into(),
                        title: "Keep this".into(),
                        revision: 1,
                        settings_json: None,
                        created_at_ms: 3,
                        updated_at_ms: 3,
                    })
                    .unwrap();
            }
            loop {
                let entries = source.pending_sync_changes(37).unwrap();
                assert!(!entries.is_empty());
                if entries[0].entity_type == "archive_restore_commit" {
                    assert!(transfer(&mut destination, &entries, cursor).is_err());
                    assert_eq!(
                        destination.sync_cursor("remote").unwrap().unwrap().cursor,
                        cursor
                    );
                    assert_old(&destination);
                    break;
                }
                let transmitted: Vec<_> = entries
                    .iter()
                    .filter(|entry| {
                        !omit_target
                            || entry.entity_type != "archive_restore_target"
                            || !entry.entity_id.ends_with(":obsolete")
                    })
                    .cloned()
                    .collect();
                cursor = transfer(&mut destination, &transmitted, cursor)
                    .unwrap()
                    .cursor;
                acknowledge(&mut source, &entries, cursor);
                assert_old(&destination);
            }
        }
    }

    #[test]
    fn changed_stage_replay_and_concurrent_rename_cannot_replace_committed_history() {
        let root = tempfile::tempdir().unwrap();
        let mut source = NolaStore::open(root.path().join("source.db")).unwrap();
        let mut destination = NolaStore::open(root.path().join("destination.db")).unwrap();
        let mut cursor = seed(&mut source, &mut destination);
        restore(&mut source, 100);
        let staged = source.pending_sync_changes(100).unwrap();
        assert!(!staged
            .iter()
            .any(|entry| entry.entity_type == "archive_restore_commit"));
        cursor = transfer(&mut destination, &staged, cursor).unwrap().cursor;
        acknowledge(&mut source, &staged, cursor);
        let mut altered = staged
            .iter()
            .find(|entry| entry.entity_type == "archive_restore_entry")
            .unwrap()
            .clone();
        let mut payload: serde_json::Value = serde_json::from_str(&altered.payload_json).unwrap();
        payload["entry"]["profile"]["title"] = "altered replay".into();
        altered.payload_json = serde_json::to_string(&payload).unwrap();
        assert!(transfer(&mut destination, &[altered], cursor).is_err());
        assert_eq!(
            destination.sync_cursor("remote").unwrap().unwrap().cursor,
            cursor
        );
        assert_old(&destination);
        destination
            .rename_conversation("c", "Concurrent rename", None, 4)
            .unwrap();
        let final_fact = source.pending_sync_changes(100).unwrap();
        assert_eq!(final_fact.len(), 1);
        assert_eq!(final_fact[0].entity_type, "archive_restore_commit");
        assert!(transfer(&mut destination, &final_fact, cursor).is_err());
        let (title,text):(String,String)=destination.connection.query_row(
            "SELECT c.title,p.content_text FROM conversations c JOIN messages m ON m.conversation_id=c.id JOIN message_parts p ON p.message_id=m.id WHERE c.id='c'",[],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
        assert_eq!(title, "Concurrent rename");
        assert_eq!(text, "OLD ORIGINAL");
        assert_eq!(
            destination.sync_cursor("remote").unwrap().unwrap().cursor,
            cursor
        );
    }

    #[test]
    fn projection_scope_includes_same_page_profiles_and_staged_targets_only_for_final_commit() {
        let root = tempfile::tempdir().unwrap();
        let mut source = NolaStore::open(root.path().join("source.db")).unwrap();
        let mut destination = NolaStore::open(root.path().join("destination.db")).unwrap();
        let cursor = seed(&mut source, &mut destination);
        restore(&mut source, 100);
        let staged = source.pending_sync_changes(100).unwrap();
        assert!(destination
            .archive_restore_projection_ids(
                staged
                    .iter()
                    .map(|entry| (entry.entity_type.as_str(), entry.payload_json.as_str()))
            )
            .unwrap()
            .is_empty());
        let final_json: String = source
            .connection
            .query_row(
                "SELECT payload_json FROM sync_outbox WHERE entity_type='archive_restore_commit'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let mut before: Vec<_> = staged
            .iter()
            .filter(|entry| entry.entity_type != "archive_restore_target")
            .map(|entry| (entry.entity_type.as_str(), entry.payload_json.as_str()))
            .collect();
        before.push(("archive_restore_commit", final_json.as_str()));
        assert_eq!(
            destination.archive_restore_projection_ids(before).unwrap(),
            ["c", "new", "obsolete"]
        );
        transfer(&mut destination, &staged, cursor).unwrap();
        assert_eq!(
            destination
                .archive_restore_projection_ids([("archive_restore_commit", final_json.as_str())])
                .unwrap(),
            ["c", "new", "obsolete"]
        );
    }

    #[test]
    fn target_fact_identity_is_unambiguous_when_ids_contain_colons() {
        assert_ne!(fact_id("a:b", "c"), fact_id("a", "b:c"));
    }
}
