// SPDX-License-Identifier: Apache-2.0
//! Same-UoW completion frontier, captured only after final journal publication.
use super::*;
use nola_domain::assistant_run_terminal::*;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct TaskEvidence {
    owner_session_id: String,
    owner_turn_id: String,
    state: String,
    durability: TaskDurabilityEvidence,
}
#[derive(Deserialize)]
struct TaskDurabilityEvidence {
    #[serde(default)]
    terminal_receipt_id: Option<String>,
}
#[derive(Deserialize)]
struct CheckpointEvidence {
    session_id: String,
    revision: u64,
    last_event_cursor: CursorEvidence,
    state: StateEvidence,
}
#[derive(Deserialize)]
struct CursorEvidence {
    sequence: u64,
}
#[derive(Deserialize)]
struct StateEvidence {
    session_id: String,
    task_projection: TasksEvidence,
}
#[derive(Deserialize)]
struct TasksEvidence {
    tasks: BTreeMap<String, TaskEvidence>,
}

fn local_run(c: &Connection, conversation: &str, run: &str) -> StoreResult<()> {
    crate::require_id("terminal.conversation_id", conversation)?;
    crate::require_id("terminal.run_id", run)?;
    let local: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM runs r
        WHERE r.id=?1 AND r.conversation_id=?2 AND r.kind='conversation'
        AND NOT EXISTS(SELECT 1 FROM remote_run_sources s WHERE s.run_id=r.id))",
        params![run, conversation],
        |r| r.get(0),
    )?;
    if !local {
        return Err(invalid("terminal Run is not authored by this source Host"));
    }
    Ok(())
}
pub(super) fn load(
    store: &NolaStore,
    conversation: &str,
    run: &str,
) -> StoreResult<Option<AssistantRunTerminalSeal>> {
    let _snapshot = crate::read_snapshot::pin(&store.connection)?;
    local_run(&store.connection, conversation, run)?;
    let raw: Option<String> = store.connection.query_row(
        "SELECT seal_json FROM assistant_run_terminal_seals WHERE run_id=?1 AND conversation_id=?2",
        params![run,conversation], |r|r.get(0)).optional()?;
    raw.map(|raw| {
        let seal: AssistantRunTerminalSeal = serde_json::from_str(&raw)?;
        seal.validate().map_err(invalid)?;
        if seal.identity.conversation_id != conversation
            || seal.identity.run_id != run
            || seal.identity.source_store_epoch != store.assistant_source_epoch()?
        {
            return Err(invalid("terminal seal identity differs"));
        }
        Ok(seal)
    })
    .transpose()
}
pub(super) fn freeze(
    store: &mut NolaStore,
    transition_id: &str,
) -> StoreResult<Option<AssistantRunTerminalSeal>> {
    require_uow(store)?;
    let row: Option<(String,String,String,String,i64,i64,i64)> = store.connection.query_row(
        "SELECT r.conversation_id,r.id,r.assistant_message_id,r.status,r.completed_at_ms,t.checkpoint_revision,t.committed_at_ms
         FROM committed_transitions t JOIN runs r ON r.id=t.run_id
         WHERE t.transition_id=?1 AND r.kind='conversation'
         AND r.status='sealed'", [transition_id],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional()?;
    let Some((conversation, run, assistant, _status, completed, revision, committed)) = row else {
        return Ok(None);
    };
    local_run(&store.connection, &conversation, &run)?;
    if let Some(seal) = load(store, &conversation, &run)? {
        if seal.transition_id != transition_id {
            return Err(invalid("Run already sealed by another terminal transition"));
        }
        return Ok(Some(seal)); // Never recapture newer background presentation heads.
    }
    if !store.applied_transition_stamps.contains(transition_id) || completed != committed {
        return Err(invalid(
            "terminal seal requires its applied canonical transition UoW",
        ));
    }
    let (checkpoint_revision,event_cursor,checkpoint_json):(i64,i64,String)=store.connection.query_row(
        "SELECT revision,event_cursor,checkpoint_json FROM core_checkpoints JOIN core_checkpoint_bodies USING(conversation_id) WHERE conversation_id=?1",
        [&conversation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    if checkpoint_revision != revision || revision <= 0 || event_cursor < 0 {
        return Err(invalid(
            "terminal seal checkpoint is not its committed frontier",
        ));
    }
    let checkpoint: CheckpointEvidence = serde_json::from_str(&checkpoint_json)?;
    if checkpoint.session_id.is_empty()
        || checkpoint.session_id != checkpoint.state.session_id
        || checkpoint.last_event_cursor.sequence != event_cursor as u64
    {
        return Err(invalid("terminal seal Core session differs"));
    }
    let result = crate::run_requirements::seal_result(&store.connection, &conversation, &run)?;
    let epoch = store.assistant_source_epoch()?;
    let mut entries = Vec::new();
    // Every assistant admitted into the exact Run is required, not only the
    // owners modified by the final frame. The real canonical ordinal is retained.
    let mut query = store.connection.prepare(
        "SELECT m.id,m.ordinal,m.created_at_ms,m.status,m.completed_at_ms FROM messages m WHERE m.conversation_id=?1
         AND m.run_id=?2 AND m.role='assistant' ORDER BY m.ordinal,m.id",
    )?;
    let owners = query
        .query_map(params![conversation, run], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<i64>>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(query);
    for (message, ordinal, created_at_ms, state, completed_at_ms) in owners {
        let state = match state.as_str() {
            "completed" => AssistantRunTerminalStatus::Completed,
            "failed" => AssistantRunTerminalStatus::Failed,
            "cancelled" => AssistantRunTerminalStatus::Cancelled,
            _ => {
                return Err(invalid(
                    "terminal assistant has no canonical terminal state",
                ))
            }
        };
        let completed_at_ms = completed_at_ms
            .ok_or_else(|| invalid("terminal assistant has no canonical completion time"))?;
        let identity = AssistantJournalIdentity {
            source_store_epoch: epoch.clone(),
            conversation_id: conversation.clone(),
            run_id: run.clone(),
            assistant_message_id: message,
        };
        let through = store.local_assistant_journal_head(&identity)?;
        if through.sequence == 0 {
            // An admitted but never dispatched blocked Run has no visible
            // Parts. Its authenticated genesis cursor is the exact frontier;
            // requiring a dummy Text record would invent assistant content.
            let empty_admission: bool = store.connection.query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM effect_outbox WHERE run_id=?1)
                 AND NOT EXISTS(SELECT 1 FROM message_parts WHERE message_id=?2
                    AND (COALESCE(content_text,'')<>'' OR content_json IS NOT NULL OR kind<>'text'))",
                params![run, identity.assistant_message_id], |row| row.get(0),
            )?;
            let empty_segment: bool = store.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM assistant_message_segments s JOIN assistant_turn_message_bindings b
                    ON b.run_id=s.run_id AND b.message_id=s.message_id AND b.turn_id=s.turn_id
                    WHERE s.run_id=?1 AND s.message_id=?2 AND b.conversation_id=?3)
                 AND NOT EXISTS(SELECT 1 FROM message_parts WHERE message_id=?2)",
                params![run,identity.assistant_message_id,conversation], |row|row.get(0),
            )?;
            if !empty_segment && (!empty_admission || !matches!(&result.value, nola_domain::run_result::RunResultValue::Blocked { .. })) {
                return Err(invalid("terminal assistant has no published journal frontier"));
            }
        }
        entries.push(AssistantRunTerminalManifestEntry::Assistant {
            through,
            ordinal,
            created_at_ms,
            state,
            completed_at_ms,
        });
    }
    let heads_count = entries.len() as u64;
    if heads_count == 0 {
        return Err(invalid("terminal Run has no published assistant owner"));
    }
    // Capture only exact receipt consumption already accepted by Core. An
    // independent queued/running background task adds no new completion policy.
    for (task_id, task) in checkpoint.state.task_projection.tasks {
        let Some(receipt_id) = task.durability.terminal_receipt_id else {
            continue;
        };
        let belongs: bool = store.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_turn_message_bindings
             WHERE conversation_id=?1 AND run_id=?2 AND turn_id=?3)",
            params![conversation, run, task.owner_turn_id],
            |r| r.get(0),
        )?;
        if !belongs {
            continue;
        }
        let bound_agent: bool = store.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM durable_tasks t JOIN runtime_task_run_bindings b
             ON b.task_id=t.id WHERE t.id=?1 AND t.kind='agent')",
            [&task_id], |r| r.get(0),
        )?;
        // Shell execution and pre-admission task failure retain their Task
        // receipt, but do not claim a child Run result that never existed.
        if !bound_agent { continue; }
        if task.owner_session_id != checkpoint.session_id {
            return Err(invalid("consumed task belongs to another Core session"));
        }
        let raw: String=store.connection.query_row(
            "SELECT receipt_json FROM runtime_task_terminal_receipts WHERE task_id=?1 AND receipt_id=?2",
            params![task_id,receipt_id],|r|r.get(0))?;
        let receipt: nola_store_port::runtime_task::RuntimeTaskTerminalReceipt =
            serde_json::from_str(&raw)?;
        if receipt.task_id != task_id
            || receipt.receipt_id != receipt_id
            || receipt.owner_session_id != conversation
            || receipt.status != task.state
        {
            return Err(invalid(
                "consumed child receipt differs from canonical parent evidence",
            ));
        }
        let child_run_id: Option<String> = store
            .connection
            .query_row(
                "SELECT run_id FROM runtime_task_run_bindings WHERE task_id=?1",
                [&task_id],
                |r| r.get(0),
            )
            .optional()?;
        let receipt_digest =
            AssistantDigest::parse_hex(&format!("{:x}", Sha256::digest(raw.as_bytes())))
                .map_err(invalid)?;
        let child_run = child_run_id.as_deref().ok_or_else(|| invalid("consumed task receipt has no child Run"))?;
        let child_seal: String = store.connection.query_row(
            "SELECT seal_json FROM assistant_run_terminal_seals WHERE run_id=?1",
            [child_run], |row| row.get(0))?;
        let child_seal: AssistantRunTerminalSeal = serde_json::from_str(&child_seal)?;
        child_seal.validate().map_err(invalid)?;
        if child_seal.identity.run_id!=child_run
            || receipt.child_session_id.as_deref()!=Some(child_seal.identity.conversation_id.as_str())
            || receipt.journal_result.as_ref().map(|result|&result.seal)!=Some(&child_seal) {
            return Err(invalid("child receipt result differs from exact child Run seal"));
        }
        entries.push(AssistantRunTerminalManifestEntry::ChildReceipt {
            task_id,
            receipt_id,
            parent_turn_id: task.owner_turn_id,
            child_session_id: receipt.child_session_id,
            child_run_id,
            input_watermark: receipt.input_watermark,
            completion_watermark: receipt.completion_watermark,
            status: receipt.status,
            receipt_digest,
            result_digest: child_seal.result_digest,
        });
    }
    let manifest_digest = terminal_manifest_digest(&entries).map_err(invalid)?;
    let mut seal = AssistantRunTerminalSeal {
        protocol_version: 2,
        identity: AssistantJournalIdentity {
            source_store_epoch: epoch,
            conversation_id: conversation.clone(),
            run_id: run.clone(),
            assistant_message_id: assistant,
        },
        core_session_id: checkpoint.session_id,
        transition_id: transition_id.into(),
        checkpoint_revision: JournalNumber(revision as u64),
        core_revision: JournalNumber(checkpoint.revision),
        event_cursor: JournalNumber(event_cursor as u64),
        result_digest: result.computed_digest().map_err(invalid)?,
        result,
        completed_at_ms: completed,
        admission: nola_store_port::run_message_admission::RunMessageAdmissionRepository::local_run_message_admission_head(store, &conversation, &run)?,
        entry_count: JournalNumber(entries.len() as u64),
        heads_count: JournalNumber(heads_count),
        child_receipt_count: JournalNumber(entries.len() as u64 - heads_count),
        manifest_digest,
        seal_digest: manifest_digest,
    };
    seal.seal_digest = seal.computed_digest().map_err(invalid)?;
    seal.validate_manifest(&entries).map_err(invalid)?;
    let tx = store.write_transaction()?;
    for (ordinal, entry) in entries.iter().enumerate() {
        tx.execute("INSERT INTO assistant_run_terminal_manifest(run_id,ordinal,entry_json) VALUES(?1,?2,?3)",
            params![run,ordinal as i64,serde_json::to_string(entry)?])?;
    }
    tx.execute("INSERT INTO assistant_run_terminal_seals(run_id,conversation_id,transition_id,seal_json,seal_digest) VALUES(?1,?2,?3,?4,?5)",
        params![run,conversation,transition_id,serde_json::to_string(&seal)?,seal.seal_digest.to_hex()])?;
    tx.commit()?;
    Ok(Some(seal))
}

pub(super) fn page(
    store: &NolaStore,
    seal: &AssistantRunTerminalSeal,
    after: JournalNumber,
    max_records: usize,
    max_bytes: usize,
) -> StoreResult<AssistantRunTerminalManifestPage> {
    seal.validate().map_err(invalid)?;
    if max_records == 0
        || max_records > 64
        || !(1024..=262144).contains(&max_bytes)
        || after.0 > seal.entry_count.0
    {
        return Err(invalid("terminal manifest read budget/cursor is invalid"));
    }
    let _snapshot = crate::read_snapshot::pin(&store.connection)?;
    if load(store, &seal.identity.conversation_id, &seal.identity.run_id)?.as_ref() != Some(seal) {
        return Err(invalid("terminal manifest seal identity differs"));
    }
    let mut result = AssistantRunTerminalManifestPage {
        seal: seal.clone(),
        after_entry: after,
        next_entry: after,
        complete: false,
        entries: vec![],
    };
    let mut query=store.connection.prepare("SELECT ordinal,entry_json FROM assistant_run_terminal_manifest WHERE run_id=?1 AND ordinal>=?2 ORDER BY ordinal LIMIT ?3")?;
    let rows = query
        .query_map(
            params![
                seal.identity.run_id,
                i64::try_from(after.0).map_err(invalid)?,
                max_records as i64
            ],
            |r| Ok((r.get::<_, u64>(0)?, r.get::<_, String>(1)?)),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    for (ordinal, raw) in rows {
        if ordinal != result.next_entry.0 {
            return Err(invalid("terminal manifest contains a gap"));
        }
        result.entries.push(serde_json::from_str(&raw)?);
        result.next_entry.0 += 1;
        result.complete = result.next_entry == seal.entry_count;
        if serde_json::to_vec(&result)?.len() > max_bytes {
            result.entries.pop();
            result.next_entry.0 -= 1;
            result.complete = false;
            if result.entries.is_empty() {
                return Err(invalid("terminal manifest entry exceeds read budget"));
            }
            break;
        }
    }
    result.complete = result.next_entry == seal.entry_count;
    if result.entries.is_empty() && !result.complete {
        return Err(invalid("terminal manifest ended before its sealed count"));
    }
    if serde_json::to_vec(&result)?.len() > max_bytes {
        return Err(invalid("terminal manifest header exceeds read budget"));
    }
    Ok(result)
}
