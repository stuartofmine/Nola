// SPDX-License-Identifier: Apache-2.0
//! Source-owned, ordered, atomically sealed complete publication sets.
use super::*;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

struct Source {
    kind: &'static str,
    id: String,
    epoch: String,
    conversation: String,
    run: Option<String>,
    transition_revision: Option<u64>,
    idle_event_cursor: Option<i64>,
    key: String,
}

#[derive(Default, Debug, PartialEq, Eq)]
struct Counts {
    groups: u64,
    frames: u64,
    operations: u64,
    chunks: u64,
    utf8_bytes: u64,
}

struct Receipt {
    revision: u64,
    digest: String,
    counts: Counts,
    cursors: Vec<AssistantJournalCursor>,
}

fn count(value: usize) -> StoreResult<u64> {
    let value = u64::try_from(value).map_err(invalid)?;
    number(value)?;
    Ok(value)
}
fn add(total: &mut u64, value: u64) -> StoreResult<()> {
    *total = total
        .checked_add(value)
        .ok_or_else(|| invalid("publication count overflow"))?;
    number(*total)?;
    Ok(())
}
fn hash_bytes(hash: &mut Sha256, bytes: &[u8]) -> StoreResult<()> {
    hash.update(count(bytes.len())?.to_be_bytes());
    hash.update(bytes);
    Ok(())
}
fn hash_identity(hash: &mut Sha256, id: &AssistantJournalIdentity) -> StoreResult<()> {
    id.validate().map_err(invalid)?;
    for value in [
        &id.source_store_epoch,
        &id.conversation_id,
        &id.run_id,
        &id.assistant_message_id,
    ] {
        hash_bytes(hash, value.as_bytes())?;
    }
    Ok(())
}

fn source(c: &Connection, stamp: &AssistantSourceCommitStamp) -> StoreResult<Source> {
    let epoch: String = c.query_row(
        "SELECT value FROM store_metadata WHERE key='assistant_source_epoch'",
        [],
        |r| r.get(0),
    )?;
    let (kind, id, run, conversation, transition_revision) = match stamp {
        AssistantSourceCommitStamp::UiSurface { surface } => {
            surface.validate().map_err(invalid)?;
            let key = format!("{:x}", Sha256::digest(serde_json::to_vec(&surface.identity)?));
            let exists: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM ui_surface_revisions r JOIN ui_surfaces s ON s.identity_key=r.identity_key WHERE r.identity_key=?1 AND r.revision=?2 AND s.authority_id=?3)",
                params![key, number(surface.revision)?, epoch], |row| row.get(0))?;
            if !exists || surface.identity.owner.authority_id != epoch { return Err(invalid("unknown local Surface source")); }
            ("ui_surface", format!("{key}:{}", surface.revision), Some(surface.identity.owner.originating_run_id.clone()),
                surface.identity.owner.conversation_id.clone(), Some(surface.revision))
        }
        AssistantSourceCommitStamp::TaskRunBinding { task_id, child_run_id } => {
            let raw: String = c.query_row("SELECT a.admission_json FROM runtime_task_run_bindings b JOIN runtime_agent_task_authorities a ON a.task_id=b.task_id WHERE b.task_id=?1 AND b.run_id=?2 AND a.phase='admitted'", params![task_id,child_run_id], |r|r.get(0))?;
            let admission: nola_domain::agent_task_admission::AgentTaskAdmission = serde_json::from_str(&raw)?;
            admission.validate().map_err(invalid)?;
            let exact: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM runtime_task_run_bindings b JOIN runs r ON r.id=b.run_id WHERE b.task_id=?1 AND b.run_id=?2 AND b.owner_session_id=?3 AND b.child_session_id=?4 AND r.conversation_id=b.child_session_id AND r.created_at_ms=b.bound_at_ms)",params![task_id,child_run_id,admission.parent_conversation_id,admission.child_conversation_id],|r|r.get(0))?;
            if !exact || admission.task_id != *task_id { return Err(invalid("Task binding publication owner differs")); }
            ("task_run_binding",serde_json::to_string(&(task_id,child_run_id))?,Some(admission.parent_run_id),admission.parent_conversation_id,None)
        }
        AssistantSourceCommitStamp::Admission { run_id } => {
            let conversation: Option<String> = c.query_row(
                "SELECT conversation_id FROM runs r WHERE id=?1 AND NOT EXISTS(SELECT 1 FROM remote_run_sources s WHERE s.run_id=r.id)",
                [run_id], |r|r.get(0),
            ).optional()?;
            (
                "admission",
                run_id.clone(),
                Some(run_id.clone()),
                conversation.ok_or_else(|| invalid("unknown local source admission"))?,
                None,
            )
        }
        AssistantSourceCommitStamp::Transition { transition_id } => {
            let row: Option<(String, String, u64)> = c.query_row(
                "SELECT t.run_id,r.conversation_id,t.checkpoint_revision FROM committed_transitions t JOIN runs r ON r.id=t.run_id WHERE t.transition_id=?1 AND NOT EXISTS(SELECT 1 FROM remote_run_sources s WHERE s.run_id=r.id)",
                [transition_id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
            ).optional()?;
            let (run, conversation, revision) =
                row.ok_or_else(|| invalid("unknown local source transition"))?;
            (
                "transition",
                transition_id.clone(),
                Some(run),
                conversation,
                Some(revision),
            )
        }
        AssistantSourceCommitStamp::IdleCheckpoint { conversation_id, checkpoint_revision, event_cursor } => {
            crate::require_id("idle_source.conversation_id", conversation_id)?;
            if *checkpoint_revision <= 0 || *event_cursor < 0 {
                return Err(invalid("invalid idle source checkpoint coordinates"));
            }
            ("idle_checkpoint", serde_json::to_string(&(conversation_id,checkpoint_revision,event_cursor))?,
                None, conversation_id.clone(), Some(*checkpoint_revision as u64))
        }
    };
    if id.trim().is_empty() {
        return Err(invalid("empty publication source identity"));
    }
    // This key identifies the actual source commit independently of its set.
    // A changed manifest can never acquire a fresh publication slot.
    let mut hash = Sha256::new();
    hash.update(b"NOLA-AJ-PUBLICATION-SOURCE\0\x01");
    for value in [epoch.as_str(), conversation.as_str(), run.as_deref().unwrap_or(""), id.as_str()] {
        hash_bytes(&mut hash, value.as_bytes())?;
    }
    hash_bytes(&mut hash, kind.as_bytes())?;
    Ok(Source {
        kind,
        id,
        epoch,
        conversation,
        run,
        transition_revision,
        idle_event_cursor: match stamp { AssistantSourceCommitStamp::IdleCheckpoint {event_cursor,..} => Some(*event_cursor), _ => None },
        key: format!("{:x}", hash.finalize()),
    })
}

fn publication_receipt(c: &Connection, source: &Source) -> StoreResult<Option<Receipt>> {
    let stored: Option<(u64, String, u64, u64, u64, u64, u64, String)> = c.query_row(
        "SELECT source_revision,set_digest,group_count,frame_count,operation_count,supplied_chunk_count,supplied_utf8_bytes,result_json FROM assistant_journal_publications WHERE source_key=?1 AND source_store_epoch=?2 AND source_kind=?3 AND source_id=?4 AND conversation_id=?5 AND run_id IS ?6",
        params![source.key,source.epoch,source.kind,source.id,source.conversation,source.run],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?)),
    ).optional()?;
    stored
        .map(
            |(revision, digest, groups, frames, operations, chunks, utf8_bytes, json)| {
                Ok(Receipt {
                    revision,
                    digest,
                    counts: Counts {
                        groups,
                        frames,
                        operations,
                        chunks,
                        utf8_bytes,
                    },
                    cursors: serde_json::from_str(&json)?,
                })
            },
        )
        .transpose()
}

fn first_revision(store: &NolaStore, source: &Source) -> StoreResult<u64> {
    if source.kind == "task_run_binding" {
        let (_, child_run): (String,String) = serde_json::from_str(&source.id)?;
        return store.applied_admission_stamps.get(&child_run).copied()
            .and_then(|value|u64::try_from(value).ok()).filter(|value|*value>0)
            .ok_or_else(||invalid("Task binding publication requires its child admission in this unit of work"));
    }
    if source.kind == "ui_surface" {
        if !store.applied_ui_surface_stamps.contains(&source.id) {
            return Err(invalid("Surface publication needs its exact source write in this unit of work"));
        }
        return source.transition_revision.ok_or_else(|| invalid("Surface revision missing"));
    }
    let revision = match source.kind {
        "admission" => store
            .applied_admission_stamps
            .get(source.run.as_deref().ok_or_else(|| invalid("admission source has no Run"))?)
            .copied()
            .and_then(|v| u64::try_from(v).ok()),
        "transition" if store.applied_transition_stamps.contains(&source.id) => {
            source.transition_revision
        }
        "idle_checkpoint" if source.transition_revision.is_some_and(|revision|
            store.applied_idle_checkpoint_stamps.get(&(source.conversation.clone(), revision as i64))
                .copied() == source.idle_event_cursor) => source.transition_revision,
        _ => None,
    }
    .ok_or_else(|| {
        invalid("publication needs its source commit applied in this write unit of work")
    })?;
    if revision == 0 {
        return Err(invalid("publication source revision must be positive"));
    }
    number(revision)?;
    let exact: bool = store.connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM core_checkpoints WHERE conversation_id=?1 AND revision=?2 AND (?3 IS NULL OR event_cursor=?3))",
        params![source.conversation, number(revision)?, source.idle_event_cursor],
        |r| r.get(0),
    )?;
    if !exact {
        return Err(invalid(
            "publication source is not the current exact Run commit",
        ));
    }
    Ok(revision)
}
pub(super) fn validate_applied_planning_source(
    store: &NolaStore, stamp: &AssistantSourceCommitStamp, identity: &AssistantJournalIdentity,
) -> StoreResult<()> {
    let actual = source(&store.connection, stamp)?;
    if actual.conversation != identity.conversation_id
        || actual.epoch != identity.source_store_epoch {
        return Err(invalid("journal order source and target owner differ"));
    }
    // A persisted or replayed receipt alone never authorizes a new reservation.
    first_revision(store, &actual)?;
    validate_target(&store.connection, &actual, identity)?;
    Ok(())
}

/// A source checkpoint proves a commit, never an assistant owner. Cross-Run
/// and idle updates require their own immutable Turn/Message membership.
fn validate_target(c: &Connection, source: &Source, identity: &AssistantJournalIdentity) -> StoreResult<()> {
    validate_owner(c, identity)?;
    if source.kind == "task_run_binding" && source.run.as_deref()!=Some(identity.run_id.as_str()) {
        return Err(invalid("Task binding publication belongs to a different parent Run"));
    }
    if source.run.as_deref() != Some(identity.run_id.as_str()) {
        let bound: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_turn_message_bindings b JOIN runs r ON r.id=b.run_id
             WHERE b.conversation_id=?1 AND b.run_id=?2 AND b.message_id=?3
             AND r.conversation_id=?1 AND r.kind='conversation')",
            params![identity.conversation_id,identity.run_id,identity.assistant_message_id], |row|row.get(0),
        )?;
        if !bound { return Err(invalid("cross-Run journal target lacks persisted assistant Turn binding")); }
    }
    Ok(())
}

fn manifest(
    input: &CommitAssistantPublicationSet,
    source: &Source,
    revision: u64,
) -> StoreResult<(String, Counts)> {
    if input.groups.is_empty() {
        return Err(invalid("publication set must contain an owner"));
    }
    let mut counts = Counts {
        groups: count(input.groups.len())?,
        ..Counts::default()
    };
    let mut owners = HashSet::new();
    let mut hash = Sha256::new();
    hash.update(b"NOLA-AJ-PUBLICATION-SET\0\x01");
    hash_bytes(&mut hash, source.key.as_bytes())?;
    hash.update(revision.to_be_bytes());
    hash.update(counts.groups.to_be_bytes());
    for (group_index, group) in input.groups.iter().enumerate() {
        let identity = &group.expected.identity;
        if identity.source_store_epoch != source.epoch
            || identity.conversation_id != source.conversation
            || !owners.insert(owner(identity)?)
        {
            return Err(invalid(
                "publication owner is duplicated or outside exact source scope",
            ));
        }
        if group.frames.is_empty() {
            return Err(invalid("publication group must contain a bounded frame"));
        }
        number(group.expected.sequence)?;
        if group.expected.sequence == 0
            && AssistantJournalCursor::initial(identity.clone()).map_err(invalid)? != group.expected
        {
            return Err(invalid("publication genesis cursor differs"));
        }
        hash.update(count(group_index)?.to_be_bytes());
        hash_identity(&mut hash, identity)?;
        hash.update(group.expected.sequence.to_be_bytes());
        hash_bytes(&mut hash, group.expected.record_digest.to_hex().as_bytes())?;
        hash.update(count(group.frames.len())?.to_be_bytes());
        add(&mut counts.frames, count(group.frames.len())?)?;
        for (frame_index, frame) in group.frames.iter().enumerate() {
            if frame.batch.identity != *identity {
                return Err(invalid("publication frame/cursor owner mismatch"));
            }
            hash.update(count(frame_index)?.to_be_bytes());
            hash_bytes(&mut hash, &frame.batch.canonical_bytes().map_err(invalid)?)?;
            add(&mut counts.operations, count(frame.batch.operations.len())?)?;
            if frame.supplied_chunks.len() > ASSISTANT_PAGE_MAX_RECORDS {
                return Err(invalid("supplied chunk budget"));
            }
            hash.update(count(frame.supplied_chunks.len())?.to_be_bytes());
            let mut frame_bytes = 0u64;
            for chunk in &frame.supplied_chunks {
                chunk.verify().map_err(invalid)?;
                let length = count(chunk.byte_length())?;
                add(&mut frame_bytes, length)?;
                hash.update(length.to_be_bytes());
                hash_bytes(&mut hash, chunk.digest().to_hex().as_bytes())?;
            }
            if frame_bytes > ASSISTANT_FRAME_MAX_BYTES as u64 {
                return Err(invalid("supplied chunk budget"));
            }
            add(&mut counts.chunks, count(frame.supplied_chunks.len())?)?;
            add(&mut counts.utf8_bytes, frame_bytes)?;
        }
    }
    for value in [
        counts.groups,
        counts.frames,
        counts.operations,
        counts.chunks,
        counts.utf8_bytes,
    ] {
        hash.update(value.to_be_bytes());
    }
    Ok((format!("{:x}", hash.finalize()), counts))
}

fn operation_id(source: &Source, group: usize, frame: usize) -> String {
    format!("publication:{}:{group}:{frame}", source.key)
}

fn replay(
    c: &Connection,
    input: &CommitAssistantPublicationSet,
    source: &Source,
    receipt: &Receipt,
) -> StoreResult<Vec<AssistantJournalCursor>> {
    let mut cursors = Vec::with_capacity(input.groups.len());
    let records: u64 = c.query_row(
        "SELECT COUNT(*) FROM assistant_journal_publication_frames WHERE source_key=?1",
        [&source.key],
        |r| r.get(0),
    )?;
    if records != receipt.counts.frames {
        return Err(invalid(
            "publication receipt has incomplete frame membership",
        ));
    }
    for (group_index, group) in input.groups.iter().enumerate() {
        let key = validate_read_owner(c, &group.expected.identity)?;
        let mut cursor = group.expected.clone();
        for (frame_index, frame) in group.frames.iter().enumerate() {
            // Verify only bytes introduced by this complete set, never an old
            // prepared field's cumulative body. The digest PK seek and exact
            // incoming length bound the fetched BLOB to one validated chunk.
            // This is strictly read-only: replay must not repair missing bytes.
            for chunk in &frame.supplied_chunks {
                let matches: Option<bool> = c
                    .query_row(
                        "SELECT bytes FROM assistant_journal_chunks WHERE digest=?1 AND length(bytes)=?2",
                        params![chunk.digest().to_hex(), count(chunk.byte_length())?],
                        |row| {
                            let stored: Vec<u8> = row.get(0)?;
                            Ok(stored == chunk.text().as_bytes())
                        },
                    )
                    .optional()?;
                if matches != Some(true) {
                    return Err(invalid(
                        "publication stored supplied chunk integrity failure",
                    ));
                }
            }
            let stored: Option<(String,Vec<u8>,String)> = c.query_row(
                "SELECT r.header_json,r.batch,m.record_digest FROM assistant_journal_publication_frames m JOIN assistant_journal_records r ON r.owner=m.owner AND r.sequence=m.sequence WHERE m.source_key=?1 AND m.group_ordinal=?2 AND m.frame_ordinal=?3 AND m.owner=?4 AND r.operation_id=?5 AND r.source_revision=?6",
                params![source.key,count(group_index)?,count(frame_index)?,key,operation_id(source,group_index,frame_index),number(receipt.revision)?],
                |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
            ).optional()?;
            let (json, bytes, digest) =
                stored.ok_or_else(|| invalid("publication record membership unavailable"))?;
            let header: AssistantJournalRecordHeaderV1 = serde_json::from_str(&json)?;
            frame.batch.verify_header(&header).map_err(invalid)?;
            if bytes != frame.batch.canonical_bytes().map_err(invalid)?
                || header.record_digest.to_hex() != digest
                || header.operation_id != operation_id(source, group_index, frame_index)
                || header.source_commit_revision != receipt.revision
                || cursor.admit(&header).map_err(invalid)? != AssistantRecordAdmission::Apply
            {
                return Err(invalid("publication replay record proof differs"));
            }
            cursor.sequence = header.sequence;
            cursor.record_digest = header.record_digest;
        }
        cursors.push(cursor);
    }
    if cursors != receipt.cursors {
        return Err(invalid(
            "publication replay receipt differs from complete records",
        ));
    }
    Ok(cursors)
}

pub(super) fn commit(
    store: &mut NolaStore,
    input: &CommitAssistantPublicationSet,
) -> StoreResult<AssistantPublicationCommit> {
    require_uow(store)?;
    if let AssistantSourceCommitStamp::UiSurface { surface } = &input.stamp {
        if input.groups.len() != 1 || input.groups[0].frames.len() != 1
            || input.groups[0].frames[0].batch.operations.len() != 1 {
            return Err(invalid("Surface publication must contain its single reference mutation"));
        }
        let operation = &input.groups[0].frames[0].batch.operations[0];
        let (part_id, metadata) = match operation {
            AssistantJournalOperationV1::Declare(value) => (&value.part.part_id, &value.part.metadata),
            AssistantJournalOperationV1::Update(value) => (&value.part_id, &value.metadata),
            _ => return Err(invalid("Surface source cannot publish other mutation kinds")),
        };
        let JournalPartMetadata::UiSurface(metadata) = metadata else {
            return Err(invalid("Surface source cannot publish other Part kinds"));
        };
        let key = format!("{:x}", Sha256::digest(serde_json::to_vec(&surface.identity)?));
        if metadata.surface != *surface || *part_id != format!("ui-surface:{key}") {
            return Err(invalid("Surface journal reference differs from source"));
        }
        let raw: String = store.connection.query_row("SELECT content_json FROM message_parts WHERE id=?1 AND message_id=?2 AND kind='ui_surface'",
            params![part_id, surface.identity.owner.message_id], |row| row.get(0))?;
        let nola_domain::MessagePartView::UiSurface { surface: stored, summary, .. } = serde_json::from_str(&raw)? else {
            return Err(invalid("Surface Part source is invalid"));
        };
        if stored != *surface || summary != metadata.summary { return Err(invalid("Surface journal metadata differs from committed Part")); }
    }
    let source = source(&store.connection, &input.stamp)?;
    let existing = publication_receipt(&store.connection, &source)?;
    if existing.is_none() {
        if let AssistantSourceCommitStamp::TaskRunBinding { task_id, .. } = &input.stamp {
            if input.groups.len()!=1 || input.groups[0].frames.len()!=1
                || input.groups[0].frames[0].batch.operations.len()!=1
                || !input.groups[0].frames[0].supplied_chunks.is_empty() {
                return Err(invalid("Task binding must publish one metadata update"));
            }
            let operation=&input.groups[0].frames[0].batch.operations[0];
            let AssistantJournalOperationV1::Update(update)=operation else { return Err(invalid("Task binding cannot publish content")); };
            if update.part_id != format!("task:{task_id}") { return Err(invalid("Task binding cannot update another Part")); }
            let previous=store.assistant_journal_part_state(&input.groups[0].expected.identity,&update.part_id)?;
            let nola_store_port::assistant_journal::AssistantJournalPartState::Live(previous)=previous else { return Err(invalid("Task binding needs an existing Task Part")); };
            let JournalPartMetadata::TaskControl(mut expected)=previous.metadata else { return Err(invalid("Task binding needs Task control metadata")); };
            let JournalPartMetadata::TaskControl(actual)=&update.metadata else { return Err(invalid("Task binding cannot change Part kind")); };
            // Initial binding is admitted only while the exact Task and child
            // Run is admitted and nonterminal. This is execution evidence, not a UI inference
            // from the presence of a guidance button.
            let running: bool = store.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM durable_tasks t JOIN runtime_task_run_bindings b ON b.task_id=t.id JOIN runs r ON r.id=b.run_id WHERE t.id=?1 AND t.status='running' AND r.status IN ('queued','running') AND b.owner_session_id=t.owner_session_id AND b.child_session_id=t.child_session_id)",
                [task_id], |row|row.get(0))?;
            if !running || actual.task.state != nola_domain::ActivityState::Running {
                return Err(invalid("Task binding state differs from its running Task and Run"));
            }
            expected.task.state=actual.task.state.clone();
            expected.supports_guidance=actual.supports_guidance;
            if JournalPartMetadata::TaskControl(expected)!=update.metadata { return Err(invalid("Task binding can only change its observed state and guidance availability")); }
        }
    }
    let revision = match &existing {
        Some(receipt) => receipt.revision,
        None => first_revision(store, &source)?,
    };
    if source
        .transition_revision
        .is_some_and(|actual| actual != revision)
    {
        return Err(invalid("publication receipt/source revision differs"));
    }
    let (digest, counts) = manifest(input, &source, revision)?;
    // The savepoint includes every supplied byte/head/version/record/membership
    // and receipt. Earlier caller preparations remain owned by its outer UoW.
    let tx = store.write_transaction()?;
    if let Some(receipt) = existing {
        if digest != receipt.digest || counts != receipt.counts {
            return Err(StoreError::Conflict(
                "publication whole-set replay differs".into(),
            ));
        }
        let cursors = replay(&tx, input, &source, &receipt)?;
        tx.commit()?;
        return Ok(AssistantPublicationCommit::Replay(cursors));
    }
    for group in &input.groups {
        validate_target(&tx, &source, &group.expected.identity)?;
        if head(&tx, &group.expected.identity)? != group.expected {
            return Err(StoreError::Conflict(
                "publication initial head CAS mismatch".into(),
            ));
        }
    }
    let mut cursors = Vec::with_capacity(input.groups.len());
    let mut membership = Vec::new();
    for (group_index, group) in input.groups.iter().enumerate() {
        let mut cursor = group.expected.clone();
        for (frame_index, frame) in group.frames.iter().enumerate() {
            cursor = write::apply_frame(
                &tx,
                &cursor,
                frame,
                operation_id(&source, group_index, frame_index),
                revision,
            )?;
            membership.push((
                group_index,
                frame_index,
                owner(&cursor.identity)?,
                cursor.sequence,
                cursor.record_digest,
            ));
        }
        cursors.push(cursor);
    }
    tx.execute(
        "INSERT INTO assistant_journal_publications(source_key,source_store_epoch,source_kind,source_id,conversation_id,run_id,source_revision,set_digest,group_count,frame_count,operation_count,supplied_chunk_count,supplied_utf8_bytes,result_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![source.key,source.epoch,source.kind,source.id,source.conversation,source.run,number(revision)?,digest,number(counts.groups)?,number(counts.frames)?,number(counts.operations)?,number(counts.chunks)?,number(counts.utf8_bytes)?,serde_json::to_string(&cursors)?],
    )?;
    for (group, frame, owner, sequence, digest) in membership {
        tx.execute(
            "INSERT INTO assistant_journal_publication_frames(source_key,group_ordinal,frame_ordinal,owner,sequence,record_digest) VALUES(?1,?2,?3,?4,?5,?6)",
            params![source.key,count(group)?,count(frame)?,owner,number(sequence)?,digest.to_hex()],
        )?;
    }
    tx.commit()?;
    Ok(AssistantPublicationCommit::Applied(cursors))
}
