// SPDX-License-Identifier: Apache-2.0
//! Terminal evidence is authored only by successful lifecycle transactions.
use super::*;
use nola_store_port::assistant_journal::AssistantJournalRepository;
use nola_store_port::runtime_task::RuntimeTaskJournalResult;

pub(super) fn journal_result(
    store: &mut NolaStore,
    binding: &RuntimeTaskRunBinding,
) -> StoreResult<Option<RuntimeTaskJournalResult>> {
    use nola_domain::assistant_journal_operations::{
        JournalContentField, JournalNumber, JournalPartMetadata, JournalVisibility,
    };
    use nola_domain::assistant_run_terminal::{
        AssistantRunTerminalManifestEntry, TerminalManifestJsonLength, TerminalManifestValidator,
    };
    if store.connection.is_autocommit() {
        return Err(StoreError::Invariant(
            "task result requires the Application write unit of work".into(),
        ));
    }
    let Some(seal) =
        store.local_assistant_run_terminal_seal(&binding.child_session_id, &binding.run_id)?
    else {
        return Ok(None);
    };
    seal.validate().map_err(StoreError::Invariant)?;
    let mut length = TerminalManifestJsonLength::default();
    let mut after = JournalNumber(0);
    loop {
        let page = store.read_local_assistant_run_terminal_manifest(&seal, after, 32, 65536)?;
        if page.seal != seal
            || page.after_entry != after
            || (!page.complete && page.next_entry <= after)
        {
            return Err(StoreError::Invariant(
                "task result terminal manifest did not advance".into(),
            ));
        }
        for entry in &page.entries {
            length.observe(entry).map_err(StoreError::Invariant)?;
        }
        after = page.next_entry;
        if page.complete {
            break;
        }
    }
    let mut validator =
        TerminalManifestValidator::new(&seal, length.bytes()).map_err(StoreError::Invariant)?;
    let mut bytes = 0u64;
    let mut preview = String::new();
    let mut preview_open = true;
    let mut after = JournalNumber(0);
    loop {
        let page = store.read_local_assistant_run_terminal_manifest(&seal, after, 32, 65536)?;
        if page.seal != seal
            || page.after_entry != after
            || (!page.complete && page.next_entry <= after)
        {
            return Err(StoreError::Invariant(
                "task result terminal manifest did not advance".into(),
            ));
        }
        for entry in page.entries {
            validator.observe(&entry).map_err(StoreError::Invariant)?;
            let AssistantRunTerminalManifestEntry::Assistant { through, .. } = entry else {
                continue;
            };
            let snapshot = store.open_assistant_journal_snapshot_at(&through)?;
            let read = (|| -> StoreResult<()> {
                let mut after = None;
                let mut seen = std::collections::HashSet::new();
                let mut preview_parts = std::collections::BTreeMap::new();
                loop {
                    let page = store.read_assistant_snapshot_parts(
                        &snapshot,
                        after.as_deref(),
                        32,
                        65536,
                    )?;
                    for part in page.parts {
                        let JournalPartMetadata::Text(metadata) = &part.metadata else {
                            continue;
                        };
                        if metadata.visibility != JournalVisibility::Public
                            || !matches!(
                                metadata.review,
                                None | Some(nola_domain::TextReviewState::Approved)
                            )
                        {
                            continue;
                        }
                        let body = part
                            .bodies
                            .iter()
                            .find(|body| body.field == JournalContentField::Markdown)
                            .ok_or_else(|| {
                                StoreError::Invariant(
                                    "public task text has no markdown reference".into(),
                                )
                            })?;
                        bytes = bytes.checked_add(body.utf8_bytes.0).ok_or_else(|| {
                            StoreError::Invariant("task public byte count overflow".into())
                        })?;
                        if preview_open && preview.len() < 4096 && body.utf8_bytes.0 > 0 {
                            use nola_domain::assistant_journal_operations::JournalSection;
                            let section = match part.order_key.section {
                                JournalSection::Transcript => 0,
                                JournalSection::Tasks => 1,
                                JournalSection::Skills => 2,
                                JournalSection::ModelAttempts => 3,
                            };
                            preview_parts.insert(
                                (section, part.order_key.position.0, part.part_id),
                                body.clone(),
                            );
                            // Nonempty parts each contribute at least one byte. Only
                            // this bounded prefix can contribute to a 4KiB preview.
                            if preview_parts.len() > 4096 - preview.len() {
                                preview_parts.pop_last();
                            }
                        }
                    }
                    after = page.next_part_id;
                    match &after {
                        Some(cursor) if seen.insert(cursor.clone()) => {}
                        Some(_) => {
                            return Err(StoreError::Invariant("task result directory cycle".into()))
                        }
                        None => break,
                    }
                }
                // Opaque directory order is not display order.
                for ((_, _, part_id), body) in preview_parts {
                    if preview_open && preview.len() < 4096 && body.utf8_bytes.0 > 0 {
                        let remaining = 4096 - preview.len();
                        let range = store.read_assistant_snapshot_range(
                            &snapshot,
                            &part_id,
                            &body,
                            0,
                            remaining.max(4),
                        )?;
                        if range.offset_bytes != 0
                            || range.total_utf8_bytes != body.utf8_bytes.0
                            || range.next_offset_bytes != range.text.len() as u64
                        {
                            return Err(StoreError::Invariant("task preview range differs".into()));
                        }
                        let mut end = remaining.min(range.text.len());
                        while !range.text.is_char_boundary(end) {
                            end -= 1;
                        }
                        preview.push_str(&range.text[..end]);
                        preview_open = end as u64 == body.utf8_bytes.0;
                    }
                }
                Ok(())
            })();
            let release = store.release_assistant_journal_snapshot(&snapshot);
            read?;
            release?;
        }
        after = page.next_entry;
        if page.complete {
            break;
        }
    }
    validator.finish().map_err(StoreError::Invariant)?;
    Ok(Some(RuntimeTaskJournalResult {
        seal,
        public_utf8_bytes: JournalNumber(bytes),
        preview_truncated: (preview.len() as u64) < bytes,
        preview,
    }))
}

pub(super) fn record_terminal_receipt(
    tx: &Connection,
    task_id: &str,
    source: RuntimeTaskTerminalSource,
    now_ms: i64,
) -> StoreResult<()> {
    record_terminal_receipt_with_journal(tx, task_id, source, now_ms, None)
}

pub(super) fn record_terminal_receipt_with_journal(
    tx: &Connection,
    task_id: &str,
    source: RuntimeTaskTerminalSource,
    now_ms: i64,
    journal_result: Option<RuntimeTaskJournalResult>,
) -> StoreResult<()> {
    let fact: Option<(String,Option<String>,Option<String>,Option<i64>,Option<i64>,Option<String>,i64)> = tx.query_row(
        "SELECT status,owner_session_id,child_session_id,input_watermark,completion_watermark,output_path,ended_at_ms
         FROM durable_tasks WHERE id=?1 AND kind IN ('agent','shell_process') AND status IN ('sealed','completed','failed','cancelled')",
        [task_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)),
    ).optional()?;
    let Some((status, owner, child, input, mut completion, output, ended)) = fact else {
        return Ok(());
    };
    // Every terminal writer shares this boundary, including expiry and
    // cancellation. Losing a worker lease is not evidence that its admitted
    // Run has stopped. Retain the binding even if that Run is unavailable.
    let bound: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM runtime_task_run_bindings WHERE task_id=?1)",
        [task_id], |row| row.get(0),
    )?;
    if bound && !matches!(&source, RuntimeTaskTerminalSource::BoundRunTerminal { .. }) {
        return Err(StoreError::Conflict(
            "bound agent task must settle from its canonical Run seal".into(),
        ));
    }
    if let RuntimeTaskTerminalSource::BoundRunTerminal { binding, .. } = &source {
        if binding.task_id != task_id || owner.as_deref() != Some(binding.owner_session_id.as_str())
            || input != binding.input_watermark {
            return Err(StoreError::Invariant("terminal receipt binding does not own this task".into()));
        }
    }
    validate_bound_result(&source, &status, child.as_deref(), journal_result.as_ref())?;
    if matches!(source, RuntimeTaskTerminalSource::WorkerFinish { .. }) {
        if completion != input {
            return Err(StoreError::Conflict(
                "worker terminal must commit its exact input watermark".into(),
            ));
        }
    } else {
        // Operator cancellation/expiry settles the task's exact input version;
        // it is not a successful worker execution or an inferred new version.
        tx.execute(
            "UPDATE durable_tasks SET completion_watermark=input_watermark WHERE id=?1",
            [task_id],
        )?;
        completion = input;
    }
    let mut stmt=tx.prepare("SELECT id,task_id,attempt_number,worker_id,worker_generation,lease_token,lease_expires_at_ms,state,revision,claimed_at_ms
        FROM runtime_task_attempts WHERE task_id=?1 ORDER BY attempt_number")?;
    let attempts = stmt
        .query_map([task_id], |r| {
            Ok(nola_store_port::runtime_task::RuntimeTaskAttempt {
                id: r.get(0)?,
                task_id: r.get(1)?,
                attempt_number: r.get(2)?,
                worker_id: r.get(3)?,
                worker_generation: r.get(4)?,
                lease_token: r.get(5)?,
                lease_expires_at_ms: r.get(6)?,
                state: r.get(7)?,
                revision: r.get(8)?,
                claimed_at_ms: r.get(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut guidance_query = tx
        .prepare("SELECT record_json FROM runtime_task_guidance WHERE task_id=?1 ORDER BY rowid")?;
    let mut guidance = guidance_query
        .query_map([task_id], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|json| serde_json::from_str::<RuntimeTaskGuidance>(&json).map_err(StoreError::from))
        .collect::<StoreResult<Vec<_>>>()?;
    for message in &mut guidance {
        if matches!(message.status.as_str(), "submitted" | "accepted") {
            if status == "completed" {
                return Err(StoreError::Conflict(
                    "completed task still has unresolved durable guidance".into(),
                ));
            }
            message.status = "rejected".into();
            message.reason = Some(format!("task_{status}_before_delivery"));
            message.resolved_at_ms = Some(now_ms);
            tx.execute("UPDATE runtime_task_guidance SET record_json=?3 WHERE task_id=?1 AND message_id=?2",
                params![task_id,message.message_id,serde_json::to_string(message)?])?;
        }
    }
    if matches!(source, RuntimeTaskTerminalSource::BoundRunTerminal { .. })
        && journal_result.is_none()
    {
        return Err(StoreError::Invariant(
            "bound Run terminal requires sealed journal result".into(),
        ));
    }
    let result_ref = journal_result
        .as_ref()
        .map(|result| {
            format!(
                "nola-journal://{}/{}",
                result.seal.identity.run_id,
                result.seal.seal_digest.to_hex()
            )
        })
        .or(output);
    let receipt = RuntimeTaskTerminalReceipt {
        journal_result,
        guidance,
        receipt_id: uuid::Uuid::new_v4().to_string(),
        task_id: task_id.into(),
        owner_session_id: owner.unwrap_or_default(),
        child_session_id: child,
        input_watermark: input,
        completion_watermark: completion,
        status: status.clone(),
        result_ref,
        ended_at_ms: ended,
        recorded_at_ms: now_ms,
        attempts,
        source,
    };
    tx.execute("INSERT INTO runtime_task_terminal_receipts(task_id,receipt_id,receipt_json) VALUES(?1,?2,?3)",
        params![task_id,receipt.receipt_id,serde_json::to_string(&receipt).map_err(|e|StoreError::Invariant(e.to_string()))?])?;
    if !bound {
        crate::permission_source::close_from_unbound_terminal(tx, &receipt)?;
    }
    Ok(())
}

impl NolaStore {
    pub fn runtime_task_terminal_receipt(
        &self,
        task_id: &str,
    ) -> StoreResult<Option<RuntimeTaskTerminalReceipt>> {
        self.connection
            .query_row(
                "SELECT receipt_json FROM runtime_task_terminal_receipts WHERE task_id=?1",
                [task_id],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(|json| {
                let receipt: RuntimeTaskTerminalReceipt = serde_json::from_str(&json)
                    .map_err(|e| StoreError::Invariant(e.to_string()))?;
                if receipt.task_id != task_id {
                    return Err(StoreError::Invariant("terminal receipt task identity differs".into()));
                }
                if let RuntimeTaskTerminalSource::BoundRunTerminal { binding, .. } = &receipt.source {
                    if binding.task_id != receipt.task_id || binding.owner_session_id != receipt.owner_session_id
                        || binding.input_watermark != receipt.input_watermark {
                        return Err(StoreError::Invariant("terminal receipt binding differs".into()));
                    }
                }
                validate_bound_result(&receipt.source, &receipt.status,
                    receipt.child_session_id.as_deref(), receipt.journal_result.as_ref())?;
                Ok(receipt)
            })
            .transpose()
    }
}

fn validate_bound_result(
    source: &RuntimeTaskTerminalSource,
    status: &str,
    child: Option<&str>,
    result: Option<&RuntimeTaskJournalResult>,
) -> StoreResult<()> {
    match source {
        RuntimeTaskTerminalSource::BoundRunTerminal { binding, assistant_message_id, .. } => {
            let result = result.ok_or_else(|| StoreError::Invariant(
                "bound task terminal requires its exact Run result".into()))?;
            result.seal.validate().map_err(StoreError::Invariant)?;
            if status != "sealed"
                || child != Some(binding.child_session_id.as_str())
                || result.seal.identity.run_id != binding.run_id
                || result.seal.identity.assistant_message_id != *assistant_message_id
                || result.seal.result.conversation_id != binding.child_session_id
            {
                return Err(StoreError::Invariant("task receipt and sealed Run identity differ".into()));
            }
        }
        _ if status == "sealed" || result.is_some() => return Err(StoreError::Invariant(
            "only a bound Run seal can settle an agent task as sealed".into())),
        _ => {}
    }
    Ok(())
}
