// SPDX-License-Identifier: Apache-2.0
//! Authenticated replica completion. Staged evidence never grants execution.
use super::*;
use nola_domain::assistant_run_terminal::*;

fn authorize(c: &Connection, host: &str, seal: &AssistantRunTerminalSeal) -> StoreResult<()> {
    seal.validate().map_err(invalid)?;
    crate::require_id("terminal source Host", host)?;
    number(seal.entry_count.0)?;
    number(seal.checkpoint_revision.0)?;
    number(seal.event_cursor.0)?;
    let id = &seal.identity;
    let valid: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM runs r JOIN remote_run_sources s ON s.run_id=r.id
        JOIN messages m ON m.id=r.assistant_message_id WHERE r.id=?1 AND r.conversation_id=?2
        AND r.kind='conversation' AND r.assistant_message_id=?3 AND s.source_host_id=?4
        AND m.conversation_id=?2 AND m.run_id=r.id AND m.role='assistant')",
        params![id.run_id, id.conversation_id, id.assistant_message_id, host],
        |r| r.get(0),
    )?;
    if !valid {
        return Err(invalid(
            "terminal seal has no exact authenticated remote Run/root binding",
        ));
    }
    let mismatch: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM remote_assistant_journal_sources
        WHERE run_id=?1 AND (source_host_id<>?2 OR source_store_epoch<>?3))",
        params![id.run_id, host, id.source_store_epoch],
        |r| r.get(0),
    )?;
    if mismatch {
        return Err(invalid("terminal source incarnation differs"));
    }
    Ok(())
}
pub(super) fn state(
    c: &Connection,
    host: &str,
    seal: &AssistantRunTerminalSeal,
) -> StoreResult<AssistantRunTerminalReception> {
    authorize(c, host, seal)?;
    let (stored_host,raw,next,complete,committed):(String,String,u64,bool,bool)=c.query_row(
        "SELECT source_host_id,seal_json,next_entry,manifest_complete,committed FROM remote_assistant_run_terminal_stages WHERE run_id=?1",
        [&seal.identity.run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
    if stored_host != host || serde_json::from_str::<AssistantRunTerminalSeal>(&raw)? != *seal {
        return Err(invalid("remote terminal seal changed after staging"));
    }
    Ok(AssistantRunTerminalReception {
        seal: seal.clone(),
        next_entry: JournalNumber(next),
        manifest_complete: complete,
        committed,
    })
}
pub(super) fn begin(
    store: &mut NolaStore,
    host: &str,
    seal: &AssistantRunTerminalSeal,
) -> StoreResult<AssistantRunTerminalReception> {
    require_uow(store)?;
    let tx = store.write_transaction()?;
    authorize(&tx, host, seal)?;
    tx.execute("INSERT OR IGNORE INTO remote_assistant_run_terminal_stages(run_id,source_host_id,seal_json) VALUES(?1,?2,?3)",
        params![seal.identity.run_id,host,serde_json::to_string(seal)?])?;
    let result = state(&tx, host, seal)?;
    tx.commit()?;
    Ok(result)
}
fn entries(
    c: &Connection,
    seal: &AssistantRunTerminalSeal,
) -> StoreResult<Vec<AssistantRunTerminalManifestEntry>> {
    let mut statement=c.prepare("SELECT ordinal,entry_json FROM remote_assistant_run_terminal_entries WHERE run_id=?1 ORDER BY ordinal")?;
    let rows = statement
        .query_map([&seal.identity.run_id], |r| {
            Ok((r.get::<_, u64>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .enumerate()
        .map(|(index, (ordinal, raw))| {
            if index as u64 != ordinal {
                return Err(invalid("staged terminal manifest gap"));
            }
            serde_json::from_str(&raw).map_err(Into::into)
        })
        .collect()
}
pub(super) fn stage(
    store: &mut NolaStore,
    host: &str,
    page: &AssistantRunTerminalManifestPage,
) -> StoreResult<AssistantRunTerminalReception> {
    require_uow(store)?;
    if page.entries.len() > 64
        || page.entries.is_empty()
        || serde_json::to_vec(page)?.len() > 262144
        || page.after_entry.0.checked_add(page.entries.len() as u64) != Some(page.next_entry.0)
        || page.next_entry.0 > page.seal.entry_count.0
        || page.complete != (page.next_entry == page.seal.entry_count)
    {
        return Err(invalid("invalid terminal manifest transport page"));
    }
    let tx = store.write_transaction()?;
    let current = state(&tx, host, &page.seal)?;
    if page.after_entry.0 > current.next_entry.0 {
        return Err(invalid("terminal manifest page gap"));
    }
    for (offset, entry) in page.entries.iter().enumerate() {
        let ordinal = page.after_entry.0 + offset as u64;
        let raw = serde_json::to_string(entry)?;
        if ordinal < current.next_entry.0 {
            let old:String=tx.query_row("SELECT entry_json FROM remote_assistant_run_terminal_entries WHERE run_id=?1 AND ordinal=?2",
                params![page.seal.identity.run_id,number(ordinal)?],|r|r.get(0))?;
            if old != raw {
                return Err(invalid("terminal manifest replay differs"));
            }
        } else {
            if current.manifest_complete {
                return Err(invalid("terminal manifest already sealed"));
            }
            tx.execute(
                "INSERT INTO remote_assistant_run_terminal_entries VALUES(?1,?2,?3)",
                params![page.seal.identity.run_id, number(ordinal)?, raw],
            )?;
        }
    }
    let next = current.next_entry.0.max(page.next_entry.0);
    if next == page.seal.entry_count.0 && !current.manifest_complete {
        let manifest = entries(&tx, &page.seal)?;
        page.seal.validate_manifest(&manifest).map_err(invalid)?;
        crate::run_message_admission::require_frontier(&tx, host, &page.seal.admission)?;
        for entry in manifest {
            if let AssistantRunTerminalManifestEntry::Assistant {
                through, ordinal, ..
            } = entry
            {
                receive::authorize(&tx, host, &through.identity)?;
                let declared: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM run_message_admission_owners WHERE owner=?1 AND message_id=?2 AND source_ordinal=?3)",
                    params![serde_json::to_string(&page.seal.identity)?,through.identity.assistant_message_id,ordinal],
                    |r|r.get(0))?;
                if !declared {
                    return Err(invalid("terminal owner lacks exact message declaration"));
                }
            }
        }
    }
    tx.execute("UPDATE remote_assistant_run_terminal_stages SET next_entry=?2,manifest_complete=?3 WHERE run_id=?1",
        params![page.seal.identity.run_id,number(next)?,next==page.seal.entry_count.0])?;
    let result = state(&tx, host, &page.seal)?;
    tx.commit()?;
    Ok(result)
}
pub(super) fn commit(
    store: &mut NolaStore,
    host: &str,
    seal: &AssistantRunTerminalSeal,
) -> StoreResult<AssistantRunTerminalReception> {
    require_uow(store)?;
    let tx = store.write_transaction()?;
    let current = state(&tx, host, seal)?;
    if current.committed {
        tx.commit()?;
        return Ok(current);
    }
    if !current.manifest_complete {
        return Err(invalid("terminal manifest is incomplete"));
    }
    let manifest = entries(&tx, seal)?;
    seal.validate_manifest(&manifest).map_err(invalid)?;
    crate::run_message_admission::require_frontier(&tx, host, &seal.admission)?;
    for entry in &manifest {
        if let AssistantRunTerminalManifestEntry::Assistant { through, .. } = entry {
            if !receive::reception_state(&tx, host, through)?.complete {
                return Err(invalid("terminal assistant frontier is not yet durable"));
            }
        }
        // Child entries are authenticated source receipt attestations frozen in
        // the manifest, not local worker receipts and never execution authority.
    }
    let status = "sealed";
    let (old, completed): (String, Option<i64>) = tx.query_row(
        "SELECT status,completed_at_ms FROM runs WHERE id=?1",
        [&seal.identity.run_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if old == "sealed"
        && (old != status || completed != Some(seal.completed_at_ms))
    {
        return Err(invalid(
            "remote terminal conflicts with canonical Run status",
        ));
    }
    tx.execute(
        "UPDATE runs SET status=?2,completed_at_ms=?3,updated_at_ms=?3 WHERE id=?1",
        params![seal.identity.run_id, status, seal.completed_at_ms],
    )?;
    for entry in manifest {
        if let AssistantRunTerminalManifestEntry::Assistant {
            through,
            state,
            completed_at_ms,
            ..
        } = entry
        {
            let message_status = match state {
                AssistantRunTerminalStatus::Completed => "completed",
                AssistantRunTerminalStatus::Failed => "failed",
                AssistantRunTerminalStatus::Cancelled => "cancelled",
            };
            tx.execute(
                "UPDATE messages SET status=?2,completed_at_ms=?3 WHERE id=?1",
                params![
                    through.identity.assistant_message_id,
                    message_status,
                    completed_at_ms
                ],
            )?;
        }
    }
    tx.execute("UPDATE conversations SET revision=revision+1,updated_at_ms=MAX(updated_at_ms,?2) WHERE id=?1",params![seal.identity.conversation_id,seal.completed_at_ms])?;
    tx.execute(
        "UPDATE remote_assistant_run_terminal_stages SET committed=1 WHERE run_id=?1",
        [&seal.identity.run_id],
    )?;
    let result = state(&tx, host, seal)?;
    tx.commit()?;
    Ok(result)
}
