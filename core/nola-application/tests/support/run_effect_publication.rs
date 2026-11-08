// SPDX-License-Identifier: Apache-2.0
//! In-memory publication adapter: the production planner and domain validator run
//! inside a rollback-capable transaction shared with canonical Run mutations.
#![allow(unused_variables)]
use super::{MemoryCreation, MemoryRun};
use nola_domain::assistant_journal::*;
use nola_domain::assistant_journal_operations::*;
use nola_domain::assistant_run_terminal::*;
use nola_store_port::assistant_journal::*;
use nola_store_port::assistant_ownership::*;
use nola_store_port::protocol_journal::*;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct MemoryJournal {
    tasks: BTreeMap<String, nola_store_port::runtime_task::StoredRuntimeTask>,
    task_bindings: BTreeMap<String, nola_store_port::runtime_task::RuntimeTaskRunBinding>,
    task_authorities: BTreeMap<String, nola_store_port::runtime_task::RuntimeAgentTaskAuthority>,
    tool_sources: BTreeMap<String, nola_store_port::tool_source::ToolSourceInput>,
    admission_records: Vec<nola_domain::run_message_admission::RunMessageAdmissionRecord>,
    terminal: Option<(
        AssistantRunTerminalSeal,
        Vec<AssistantRunTerminalManifestEntry>,
    )>,
    heads: BTreeMap<String, AssistantJournalCursor>,
    parts: BTreeMap<(String, String), Option<JournalPartHead>>,
    manifests: BTreeMap<String, Vec<JournalChunkRef>>,
    chunks: BTreeMap<String, AssistantTextChunkV1>,
    orders: BTreeMap<(String, String), JournalOrderKey>,
    records: Vec<StoredAssistantJournalRecord>,
    traces: BTreeMap<String, nola_store_port::trace_journal::AppendTraceJournalEventInput>,
    /// Inject a failure after canonical mutations but before public commit.
    pub(super) fail_publication: bool,
}
impl MemoryJournal {
    pub(super) fn record_count(&self) -> usize {
        self.records.len()
    }
}
fn key(identity: &AssistantJournalIdentity) -> String {
    serde_json::to_string(identity).unwrap()
}
trait Fixture: Clone {
    fn run(&self) -> Result<&MemoryRun, String>;
    fn journal(&self) -> &MemoryJournal;
    fn journal_mut(&mut self) -> &mut MemoryJournal;
    fn backup(&mut self) -> &mut Option<Box<Self>>;
    fn active(&self) -> bool;
    fn owner(&self, conversation: &str, run: &str) -> Result<Option<AssistantOwner>, String> {
        let r = self.run()?;
        Ok(
            (r.checkpoint.conversation_id == conversation && r.run.run_id == run).then(|| {
                AssistantOwner {
                    conversation_id: conversation.into(),
                    run_id: run.into(),
                    assistant_message_id: r.run.assistant_message_id.clone().unwrap(),
                }
            }),
        )
    }
    fn verify_stamp(&self, stamp: &AssistantSourceCommitStamp) -> Result<u64, String> {
        if !self.active() {
            return Err("publication outside canonical transaction".into());
        }
        let run = self.run()?;
        match stamp {
            AssistantSourceCommitStamp::Admission { run_id } if run_id == &run.run.run_id => Ok(1),
            AssistantSourceCommitStamp::Transition { transition_id } => run
                .commits
                .iter()
                .find(|c| &c.transition_id == transition_id)
                .map(|c| c.checkpoint_revision as u64)
                .ok_or_else(|| "publication has no canonical transition".into()),
            _ => Err("publication has no canonical source commit".into()),
        }
    }
    fn verify_owner(&self, identity: &AssistantJournalIdentity) -> Result<(), String> {
        identity.validate().map_err(|e| e.to_string())?;
        let owner = self
            .owner(&identity.conversation_id, &identity.run_id)?
            .ok_or("journal Run absent")?;
        if identity.source_store_epoch != "memory-source"
            || owner.assistant_message_id != identity.assistant_message_id
        {
            return Err("journal owner mismatch".into());
        }
        Ok(())
    }
}
impl Fixture for MemoryRun {
    fn run(&self) -> Result<&MemoryRun, String> {
        Ok(self)
    }
    fn journal(&self) -> &MemoryJournal {
        &self.journal
    }
    fn journal_mut(&mut self) -> &mut MemoryJournal {
        &mut self.journal
    }
    fn backup(&mut self) -> &mut Option<Box<Self>> {
        &mut self.transaction_backup
    }
    fn active(&self) -> bool {
        self.transaction_backup.is_some()
    }
}
impl Fixture for MemoryCreation {
    fn run(&self) -> Result<&MemoryRun, String> {
        self.opened.as_ref().ok_or("no Run".into())
    }
    fn journal(&self) -> &MemoryJournal {
        self.opened
            .as_ref()
            .map_or(&self.journal, |run| &run.journal)
    }
    fn journal_mut(&mut self) -> &mut MemoryJournal {
        match self.opened.as_mut() {
            Some(run) => &mut run.journal,
            None => &mut self.journal,
        }
    }
    fn backup(&mut self) -> &mut Option<Box<Self>> {
        &mut self.transaction_backup
    }
    fn active(&self) -> bool {
        self.transaction_backup.is_some()
    }
}
pub struct Transaction<'a, T>(&'a mut T);
impl<T: Fixture> CommandTransactionRepository for Transaction<'_, T> {
    type Error = String;
    fn has_write_unit_of_work(&self) -> bool {
        self.0.active()
    }
    fn begin_write_unit_of_work(&mut self) -> Result<(), String> {
        if self.0.active() {
            return Err("nested fixture transaction".into());
        }
        let previous = self.0.clone();
        *self.0.backup() = Some(Box::new(previous));
        Ok(())
    }
    fn commit_write_unit_of_work(&mut self) -> Result<(), String> {
        self.0.backup().take().ok_or("no fixture transaction")?;
        Ok(())
    }
    fn rollback_write_unit_of_work(&mut self) -> Result<(), String> {
        let previous = self.0.backup().take().ok_or("no fixture transaction")?;
        *self.0 = *previous;
        Ok(())
    }
    fn begin_command_mutations(&mut self) -> Result<(), String> {
        Err("savepoints outside fixture scope".into())
    }
    fn commit_command_mutations(&mut self) -> Result<(), String> {
        Err("savepoints outside fixture scope".into())
    }
    fn rollback_command_mutations(&mut self) -> Result<(), String> {
        Err("savepoints outside fixture scope".into())
    }
    fn command_revision(&self, _: CommandRevisionEntity<'_>) -> Result<Option<i64>, String> {
        Err("receipt revisions outside fixture scope".into())
    }
}
impl<T: Fixture> CommandReceiptRepository for Transaction<'_, T> {
    type Error = String;
    fn begin_command_receipt(
        &mut self,
        _: &BeginCommandReceiptInput,
    ) -> Result<BeginCommandReceiptResult, ReceiptAdmissionError<String>> {
        Err(ReceiptAdmissionError::Persistence(
            "receipt admission outside direct-call fixture".into(),
        ))
    }
    fn command_receipt(&self, _: &str, _: &str) -> Result<Option<StoredCommandReceipt>, String> {
        Err("receipts outside direct-call fixture".into())
    }
    fn complete_command_receipt(
        &mut self,
        _: &str,
        _: &str,
        _: &str,
        _: &CompleteCommandReceiptOutcome,
    ) -> Result<StoredCommandReceipt, String> {
        Err("receipts outside direct-call fixture".into())
    }
}
struct Reader<'a> {
    journal: &'a MemoryJournal,
    identity: &'a AssistantJournalIdentity,
}
impl AssistantJournalStateReader for Reader<'_> {
    fn load_part_head(&self, part: &str) -> Result<Option<JournalPartHead>, OperationError> {
        match self.journal.parts.get(&(key(self.identity), part.into())) {
            Some(None) => Err(OperationError::RetiredPart),
            Some(Some(head)) => Ok(Some(head.clone())),
            None => Ok(None),
        }
    }
    fn verify_content_reference(
        &self,
        identity: &AssistantJournalIdentity,
        part: &str,
        content: &JournalContentRef,
    ) -> Result<(), OperationError> {
        if identity != self.identity {
            return Err(OperationError::OwnerMismatch);
        }
        let empty =
            empty_journal_content(identity, part, content.field, content.content_generation)?;
        let refs = if content.chunk_count.0 == 0 {
            vec![]
        } else {
            self.journal
                .manifests
                .get(&content.content_root.to_hex())
                .cloned()
                .ok_or(OperationError::ManifestUnavailable)?
        };
        let actual = if refs.is_empty() {
            empty
        } else {
            append_journal_content(&empty, &refs)?
        };
        if actual != *content {
            return Err(OperationError::InvalidContent);
        }
        for chunk in &refs {
            self.verify_chunk_reference(chunk)?;
        }
        Ok(())
    }
    fn verify_chunk_reference(&self, chunk: &JournalChunkRef) -> Result<(), OperationError> {
        let body = self
            .journal
            .chunks
            .get(&chunk.digest.to_hex())
            .ok_or(OperationError::InvalidContent)?;
        body.verify().map_err(|_| OperationError::InvalidContent)?;
        if body.byte_length() as u64 != chunk.utf8_bytes.0 {
            return Err(OperationError::InvalidContent);
        }
        Ok(())
    }
}
fn head<T: Fixture>(
    repo: &T,
    identity: &AssistantJournalIdentity,
) -> Result<AssistantJournalCursor, String> {
    repo.verify_owner(identity)?;
    repo.journal()
        .heads
        .get(&key(identity))
        .cloned()
        .map(Ok)
        .unwrap_or_else(|| {
            AssistantJournalCursor::initial(identity.clone()).map_err(|e| e.to_string())
        })
}
fn prepare<T: Fixture>(
    repo: &mut T,
    identity: &AssistantJournalIdentity,
    part: &str,
    previous: &JournalContentRef,
    supplied: &[AssistantTextChunkV1],
) -> Result<JournalContentRef, String> {
    repo.verify_owner(identity)?;
    if !repo.active() {
        return Err("content preparation outside transaction".into());
    }
    Reader {
        journal: repo.journal(),
        identity,
    }
    .verify_content_reference(identity, part, previous)
    .map_err(|e| e.to_string())?;
    let mut refs = Vec::new();
    for chunk in supplied {
        chunk.verify().map_err(|e| e.to_string())?;
        refs.push(JournalChunkRef {
            digest: chunk.digest(),
            utf8_bytes: JournalNumber(chunk.byte_length() as u64),
        });
    }
    let resulting = append_journal_content(previous, &refs).map_err(|e| e.to_string())?;
    let mut manifest = repo
        .journal()
        .manifests
        .get(&previous.content_root.to_hex())
        .cloned()
        .unwrap_or_default();
    manifest.extend(refs);
    let journal = repo.journal_mut();
    for chunk in supplied {
        journal
            .chunks
            .insert(chunk.digest().to_hex(), chunk.clone());
    }
    journal
        .manifests
        .insert(resulting.content_root.to_hex(), manifest);
    Ok(resulting)
}
fn publish<T: Fixture>(
    repo: &mut T,
    input: &CommitAssistantPublicationSet,
) -> Result<AssistantPublicationCommit, String> {
    let revision = repo.verify_stamp(&input.stamp)?;
    if repo.journal().fail_publication {
        return Err("fixture publication unavailable".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut cursors = Vec::new();
    for group in &input.groups {
        repo.verify_owner(&group.expected.identity)?;
        let identity = &group.expected.identity;
        if !seen.insert(key(identity)) || group.frames.is_empty() {
            return Err("invalid publication group".into());
        }
        let mut cursor = head(repo, identity)?;
        if cursor != group.expected {
            return Err("fixture journal CAS conflict".into());
        }
        for frame in &group.frames {
            if frame.batch.identity != *identity {
                return Err("frame owner mismatch".into());
            }
            for chunk in &frame.supplied_chunks {
                chunk.verify().map_err(|e| e.to_string())?;
                repo.journal_mut()
                    .chunks
                    .insert(chunk.digest().to_hex(), chunk.clone());
            }
            // Appends carry supplied chunk references directly, rather than a
            // separate preparation. Materialize each immutable resulting root.
            for op in &frame.batch.operations {
                if let AssistantJournalOperationV1::Append(append) = op {
                    let mut manifest = repo
                        .journal()
                        .manifests
                        .get(&append.expected.content_root.to_hex())
                        .cloned()
                        .unwrap_or_default();
                    manifest.extend(append.chunks.clone());
                    repo.journal_mut()
                        .manifests
                        .insert(append.resulting.content_root.to_hex(), manifest);
                }
            }
            let changes = validate_assistant_batch(
                &Reader {
                    journal: repo.journal(),
                    identity,
                },
                &frame.batch,
            )
            .map_err(|e| e.to_string())?;
            let header = frame
                .batch
                .seal(
                    cursor.sequence + 1,
                    cursor.record_digest,
                    format!("fixture-record-{}-{}", revision, cursor.sequence + 1),
                    revision,
                )
                .map_err(|e| e.to_string())?;
            cursor.admit(&header).map_err(|e| e.to_string())?;
            for change in changes {
                repo.journal_mut()
                    .parts
                    .insert((key(identity), change.part_id), change.replacement);
            }
            cursor = AssistantJournalCursor {
                identity: identity.clone(),
                sequence: header.sequence,
                record_digest: header.record_digest,
            };
            repo.journal_mut()
                .records
                .push(StoredAssistantJournalRecord {
                    header,
                    batch: frame.batch.clone(),
                });
            repo.journal_mut()
                .heads
                .insert(key(identity), cursor.clone());
        }
        cursors.push(cursor);
    }
    Ok(AssistantPublicationCommit::Applied(cursors))
}
macro_rules! fixture_ports {
    ($type:ty) => {
        // This fixture exercises ordinary Run effects only. Cell persistence
        // uses the SQLite integration suites; never silently discard a write.
        impl nola_store_port::code_cell::CodeCellObservationRepository for $type {
            type Error = String;
            fn read_code_cell(&self, _: &nola_store_port::code_cell::CodeCellQuery)
                -> Result<Option<nola_domain::code_mode::CodeCellView>, String> {
                Err("code cell reads are outside this ordinary-effect fixture".into())
            }
            fn publish_code_cell(&mut self, _: &nola_store_port::code_cell::CodeCellRecord)
                -> Result<(), String> {
                Err("code cell writes are outside this ordinary-effect fixture".into())
            }
        }
        impl nola_store_port::trace_journal::TraceJournalRepository for $type {
            type Error = String;
            fn append_trace_journal_event(&mut self, input: &nola_store_port::trace_journal::AppendTraceJournalEventInput) -> Result<bool, String> {
                if !self.active() { return Err("trace outside canonical transaction".into()); }
                let traces = &mut self.journal_mut().traces;
                if traces.contains_key(&input.event_id) || traces.values().any(|event| event.trace_id == input.trace_id && event.is_terminal) {
                    return Ok(false);
                }
                traces.insert(input.event_id.clone(), input.clone());
                Ok(true)
            }
            fn exact_run_trace_replay(&self, conversation_id: &str, run_id: &str, assistant_message_id: &str,
                after_sequence: i64, through_sequence: Option<i64>, limit: usize,
            ) -> Result<nola_store_port::trace_journal::StoredExactTraceReplay, String> {
                let _ = (conversation_id,run_id,assistant_message_id,after_sequence,through_sequence,limit);
                Err("exact trace replay requires the SQLite recovery fixture".into())
            }
            fn trace_journal_events_after(&self, trace_id: &str, sequence: i64) -> Result<Vec<nola_store_port::trace_journal::StoredTraceJournalEvent>, String> {
                let mut events = self.journal().traces.values().filter(|event| event.trace_id == trace_id && event.sequence > sequence)
                    .map(|event| serde_json::to_value(event).and_then(serde_json::from_value))
                    .collect::<Result<Vec<nola_store_port::trace_journal::StoredTraceJournalEvent>, _>>().map_err(|error| error.to_string())?;
                events.sort_by_key(|event| event.sequence);
                Ok(events)
            }
            fn trace_journal_runs_for_conversation(&self, conversation_id: &str) -> Result<Vec<nola_store_port::trace_journal::StoredTraceJournalRun>, String> {
                let mut events: Vec<_> = self.journal().traces.values().filter(|event| event.conversation_id == conversation_id).collect();
                events.sort_by_key(|event| event.sequence);
                let mut runs = BTreeMap::new();
                for event in events {
                    let run = runs.entry(event.trace_id.clone()).or_insert_with(|| nola_store_port::trace_journal::StoredTraceJournalRun {
                        trace_id: event.trace_id.clone(), run_id: event.run_id.clone(), conversation_id: event.conversation_id.clone(),
                        message_id: event.message_id.clone(), turn_id: event.turn_id.clone(), status: "running".into(), last_sequence: 0,
                        terminal_reason: None, started_at: event.emitted_at.clone(), updated_at: event.emitted_at.clone(), finished_at: None,
                    });
                    run.last_sequence = event.sequence;
                    run.updated_at = event.emitted_at.clone();
                    if let Some(status) = &event.run_status { run.status = status.clone(); }
                    if event.is_terminal { run.finished_at = Some(event.emitted_at.clone()); run.terminal_reason = event.terminal_reason.clone(); }
                }
                Ok(runs.into_values().collect())
            }
            fn latest_trace_journal_id(&self, conversation_id: &str) -> Result<Option<String>, String> {
                Ok(self.trace_journal_runs_for_conversation(conversation_id)?.into_iter()
                    .max_by(|left, right| left.updated_at.cmp(&right.updated_at).then(left.trace_id.cmp(&right.trace_id)))
                    .map(|run| run.trace_id))
            }
        }
        impl CommandProtocolRepositoryProvider for $type {
            type Error = String;
            type Repository<'a>
                = Transaction<'a, Self>
            where
                Self: 'a;
            fn command_protocol_repository(&mut self) -> Self::Repository<'_> {
                Transaction(self)
            }
        }
        impl AssistantJournalPlanningRepository for $type {
            type Error = String;
            fn bind_tool_source(&mut self, stamp: &AssistantSourceCommitStamp,
                identity: &AssistantJournalIdentity, input: &nola_store_port::tool_source::ToolSourceInput) -> Result<(),String> {
                self.verify_stamp(stamp)?;
                self.verify_owner(identity)?;
                if self.assistant_owner_for_part(&identity.conversation_id,&identity.run_id,&input.owner.part_id)?.as_ref()!=Some(&input.owner) {
                    return Err("tool source has foreign fixture Part ownership".into());
                }
                if let Some(previous)=self.journal().tool_sources.get(&input.source_item_id) {
                    if previous != input { return Err("fixture tool source is immutable".into()); }
                }
                self.journal_mut().tool_sources.insert(input.source_item_id.clone(),input.clone());
                Ok(())
            }
            fn assistant_owner_for_core_turn(
                &self,
                conversation: &str,
                turn: &str,
            ) -> Result<Option<AssistantOwner>, String> {
                let run = self.run()?;
                self.assistant_owner_for_turn(conversation, &run.run.run_id, turn)
            }
            fn assistant_owner_for_turn(
                &self,
                conversation: &str,
                run: &str,
                turn: &str,
            ) -> Result<Option<AssistantOwner>, String> {
                let r = self.run()?;
                if r.core_turn_ids.contains(turn) {
                    self.owner(conversation, run)
                } else {
                    Ok(None)
                }
            }
            fn assistant_owner_for_activity(&self, conversation: &str, run: &str, _kind: &str, _id: &str) -> Result<Option<AssistantOwner>,String> { self.owner(conversation,run) }
            fn assistant_owner_for_effect(&self, conversation: &str, run: &str, _effect: &str) -> Result<Option<AssistantOwner>,String> {
                self.owner(conversation,run)
            }
            fn assistant_owner_for_part(
                &self,
                conversation: &str,
                run: &str,
                part: &str,
            ) -> Result<Option<AssistantPartOwner>, String> {
                let r = self.run()?;
                let exists = r.run.assistant_part_id.as_deref() == Some(part)
                    || r.commits
                        .iter()
                        .any(|c| c.part_message_owners.contains_key(part))
                    || r.commits.iter().any(|c| {
                        c.part_mutations.iter().any(|mutation| {
                            use nola_domain::PartMutation;
                            match mutation {
                                PartMutation::Stream { operation, .. } => {
                                    operation.owner().part_id == part
                                }
                                PartMutation::Upsert { part_id, .. }
                                | PartMutation::AppendText { part_id, .. }
                                | PartMutation::Close { part_id, .. }
                                | PartMutation::Metadata { part_id, .. } => part_id == part,
                            }
                        })
                    })
                    || r.attempts
                        .values()
                        .any(|a| a.text_part_id == part || a.reasoning_part_id == part)
                    || r.delegated_invocations
                        .values()
                        .any(|a| a.text_part_id == part || a.reasoning_part_id == part);
                Ok(if exists {
                    self.owner(conversation, run)?
                        .map(|owner| AssistantPartOwner {
                            part_id: part.into(),
                            turn_id: r.core_turn_ids.iter().next().cloned(),
                            owner,
                        })
                } else {
                    None
                })
            }
            fn reserve_assistant_journal_order(
                &mut self,
                stamp: &AssistantSourceCommitStamp,
                identity: &AssistantJournalIdentity,
                section: JournalSection,
                part: &str,
            ) -> Result<JournalOrderKey, String> {
                self.verify_stamp(stamp)?;
                self.verify_owner(identity)?;
                let journal = self.journal_mut();
                let next = journal.orders.len() as u64 + 1;
                let order = journal
                    .orders
                    .entry((key(identity), part.into()))
                    .or_insert(JournalOrderKey {
                        section,
                        position: JournalNumber(next),
                    });
                if order.section != section {
                    return Err("order section mismatch".into());
                }
                Ok(order.clone())
            }
            fn model_attempt_successor(
                &self,
                run: &str,
                effect: &str,
                invocation: &str,
                predecessor: &str,
            ) -> Result<Option<nola_domain::ModelAttemptRecord>, String> {
                Ok(self
                    .run()?
                    .attempts
                    .values()
                    .find(|a| {
                        a.run_id == run
                            && a.effect_id == effect
                            && a.invocation_id == invocation
                            && a.predecessor_id.as_deref() == Some(predecessor)
                    })
                    .cloned())
            }
        }
        impl AssistantJournalRepository for $type {
            type Error = String;
            fn begin_receive_assistant_run_terminal(
                &mut self,
                _: &str,
                _: &AssistantRunTerminalSeal,
            ) -> Result<AssistantRunTerminalReception, String> {
                Err("fixture has no authenticated remote Run".into())
            }
            fn stage_assistant_run_terminal_manifest(
                &mut self,
                _: &str,
                _: &AssistantRunTerminalManifestPage,
            ) -> Result<AssistantRunTerminalReception, String> {
                Err("fixture has no authenticated remote Run".into())
            }
            fn commit_received_assistant_run_terminal(
                &mut self,
                _: &str,
                _: &AssistantRunTerminalSeal,
            ) -> Result<AssistantRunTerminalReception, String> {
                Err("fixture has no authenticated remote Run".into())
            }
            fn seal_assistant_run_terminal(
                &mut self,
                transition: &str,
            ) -> Result<Option<AssistantRunTerminalSeal>, String> {
                if !self.active() {
                    return Err("seal requires outer UoW".into());
                }
                let commit = self.run()?.commits.iter()
                    .find(|commit| commit.transition_id == transition)
                    .ok_or("missing committed transition")?;
                if commit.run_status.as_deref() == Some("sealed") {
                    // This adapter has no authoritative input-prefix, evidence,
                    // requirement or result indexes. Never fabricate a V2 seal.
                    return Err("terminal result sealing requires the real SQLite fixture".into());
                }
                Ok(None)
            }
            fn local_assistant_run_terminal_seal(
                &self,
                conversation: &str,
                run: &str,
            ) -> Result<Option<AssistantRunTerminalSeal>, String> {
                if self.owner(conversation, run)?.is_none() {
                    return Err("terminal owner differs".into());
                }
                Ok(self.journal().terminal.as_ref().map(|(s, _)| s.clone()))
            }
            fn read_local_assistant_run_terminal_manifest(
                &self,
                seal: &AssistantRunTerminalSeal,
                after: JournalNumber,
                max_records: usize,
                max_bytes: usize,
            ) -> Result<AssistantRunTerminalManifestPage, String> {
                let (stored, entries) = self.journal().terminal.as_ref().ok_or("missing seal")?;
                if stored != seal
                    || after.0 > seal.entry_count.0
                    || max_records == 0
                    || max_records > 64
                {
                    return Err("invalid seal read".into());
                }
                let entries: Vec<_> = entries
                    .iter()
                    .skip(after.0 as usize)
                    .take(max_records)
                    .cloned()
                    .collect();
                let next_entry = JournalNumber(after.0 + entries.len() as u64);
                let page = AssistantRunTerminalManifestPage {
                    seal: seal.clone(),
                    after_entry: after,
                    next_entry,
                    complete: next_entry == seal.entry_count,
                    entries,
                };
                if serde_json::to_vec(&page).map_err(|e| e.to_string())?.len() > max_bytes {
                    return Err("manifest exceeds budget".into());
                }
                Ok(page)
            }
            fn assistant_source_epoch(&self) -> Result<String, String> {
                Ok("memory-source".into())
            }
            fn assistant_journal_head(
                &self,
                identity: &AssistantJournalIdentity,
            ) -> Result<AssistantJournalCursor, String> {
                head(self, identity)
            }
            fn local_assistant_journal_head(
                &self,
                identity: &AssistantJournalIdentity,
            ) -> Result<AssistantJournalCursor, String> {
                head(self, identity)
            }
            fn assistant_journal_part(
                &self,
                identity: &AssistantJournalIdentity,
                part: &str,
            ) -> Result<Option<JournalPartHead>, String> {
                self.verify_owner(identity)?;
                Reader {
                    journal: self.journal(),
                    identity,
                }
                .load_part_head(part)
                .map_err(|e| e.to_string())
            }
            fn assistant_journal_part_state(
                &self,
                identity: &AssistantJournalIdentity,
                part: &str,
            ) -> Result<AssistantJournalPartState, String> {
                self.verify_owner(identity)?;
                Ok(
                    match self.journal().parts.get(&(key(identity), part.into())) {
                        Some(Some(head)) => AssistantJournalPartState::Live(head.clone()),
                        Some(None) => AssistantJournalPartState::Retired,
                        None => AssistantJournalPartState::NeverDeclared,
                    },
                )
            }
            fn assistant_journal_content_manifest(
                &self,
                identity: &AssistantJournalIdentity,
                part: &str,
                content: &JournalContentRef,
                after: u64,
                limit: usize,
            ) -> Result<AssistantManifestPage, String> {
                self.verify_owner(identity)?;
                Reader {
                    journal: self.journal(),
                    identity,
                }
                .verify_content_reference(identity, part, content)
                .map_err(|e| e.to_string())?;
                let all = self
                    .journal()
                    .manifests
                    .get(&content.content_root.to_hex())
                    .cloned()
                    .unwrap_or_default();
                let chunks: Vec<_> = all
                    .iter()
                    .skip(after as usize)
                    .take(limit)
                    .cloned()
                    .collect();
                let next_chunk = after + chunks.len() as u64;
                Ok(AssistantManifestPage {
                    chunks,
                    next_chunk,
                    complete: next_chunk == all.len() as u64,
                })
            }
            fn prepare_assistant_content(
                &mut self,
                identity: &AssistantJournalIdentity,
                part: &str,
                previous: &JournalContentRef,
                supplied: &[AssistantTextChunkV1],
            ) -> Result<JournalContentRef, String> {
                prepare(self, identity, part, previous, supplied)
            }
            fn commit_assistant_publication_set(
                &mut self,
                input: &CommitAssistantPublicationSet,
            ) -> Result<AssistantPublicationCommit, String> {
                publish(self, input)
            }
            fn commit_assistant_journal(
                &mut self,
                input: &CommitAssistantJournal,
            ) -> Result<AssistantJournalCommit, String> {
                let commit = publish(
                    self,
                    &CommitAssistantPublicationSet {
                        stamp: input.stamp.clone(),
                        groups: vec![AssistantPublicationGroup {
                            expected: input.expected.clone(),
                            frames: vec![AssistantJournalFrame {
                                batch: input.batch.clone(),
                                supplied_chunks: input.supplied_chunks.clone(),
                            }],
                        }],
                    },
                )?;
                match commit {
                    AssistantPublicationCommit::Applied(mut cursors) => {
                        Ok(AssistantJournalCommit::Applied(cursors.remove(0)))
                    }
                    AssistantPublicationCommit::Replay(mut cursors) => {
                        Ok(AssistantJournalCommit::Replay(cursors.remove(0)))
                    }
                }
            }
            fn assistant_journal_reception_state(
                &self,
                source_host_id: &str,
                through: &AssistantJournalCursor,
            ) -> Result<AssistantJournalReceptionState, Self::Error> {
                Err("operation outside direct Run publication fixture".into())
            }
            fn next_assistant_journal_content(
                &self,
                source_host_id: &str,
                record: &StoredAssistantJournalRecord,
            ) -> Result<Option<AssistantJournalContentNeed>, Self::Error> {
                Err("operation outside direct Run publication fixture".into())
            }
            fn stage_assistant_journal_content(
                &mut self,
                input: &StageAssistantJournalContent,
            ) -> Result<JournalContentRef, Self::Error> {
                Err("operation outside direct Run publication fixture".into())
            }
            fn receive_assistant_journal_record(
                &mut self,
                source_host_id: &str,
                record: &StoredAssistantJournalRecord,
                chunks: &[AssistantTextChunkV1],
            ) -> Result<AssistantJournalCommit, Self::Error> {
                Err("operation outside direct Run publication fixture".into())
            }
            fn local_run_assistant_journal_heads(
                &self,
                conversation_id: &str,
                run_id: &str,
                after_message_id: Option<&str>,
                limit: usize,
            ) -> Result<Vec<AssistantJournalCursor>, Self::Error> {
                Err("operation outside direct Run publication fixture".into())
            }
            fn open_assistant_journal_read(
                &self,
                after: &AssistantJournalCursor,
            ) -> Result<AssistantJournalReadWindow, Self::Error> {
                Err("operation outside direct Run publication fixture".into())
            }
            fn read_assistant_journal_page(
                &self,
                window: &AssistantJournalReadWindow,
                max_records: usize,
                max_bytes: usize,
            ) -> Result<AssistantJournalPage, Self::Error> {
                Err("operation outside direct Run publication fixture".into())
            }
            fn open_assistant_journal_snapshot(
                &mut self,
                identity: &AssistantJournalIdentity,
            ) -> Result<AssistantJournalSnapshot, Self::Error> {
                Err("operation outside direct Run publication fixture".into())
            }
            fn open_assistant_journal_snapshot_at(
                &mut self,
                at: &AssistantJournalCursor,
            ) -> Result<AssistantJournalSnapshot, Self::Error> {
                Err("operation outside direct Run publication fixture".into())
            }
            fn release_assistant_journal_snapshot(
                &mut self,
                snapshot: &AssistantJournalSnapshot,
            ) -> Result<AssistantSnapshotRelease, Self::Error> {
                Err("operation outside direct Run publication fixture".into())
            }
            fn read_assistant_snapshot_parts(
                &self,
                snapshot: &AssistantJournalSnapshot,
                after_part_id: Option<&str>,
                max_records: usize,
                max_bytes: usize,
            ) -> Result<AssistantPartPage, Self::Error> {
                Err("operation outside direct Run publication fixture".into())
            }
            fn read_assistant_snapshot_changes(
                &self,
                snapshot: &AssistantJournalSnapshot,
                base: &AssistantJournalCursor,
                after_part_id: Option<&str>,
                max_records: usize,
                max_bytes: usize,
            ) -> Result<AssistantPartChangePage, Self::Error> {
                Err("operation outside direct Run publication fixture".into())
            }
            fn read_assistant_snapshot_manifest(
                &self,
                snapshot: &AssistantJournalSnapshot,
                part_id: &str,
                content: &JournalContentRef,
                after_chunk: u64,
                max_records: usize,
            ) -> Result<AssistantManifestPage, Self::Error> {
                Err("operation outside direct Run publication fixture".into())
            }
            fn read_assistant_snapshot_chunk(
                &self,
                snapshot: &AssistantJournalSnapshot,
                part_id: &str,
                content: &JournalContentRef,
                chunk_index: u64,
            ) -> Result<AssistantTextChunkV1, Self::Error> {
                Err("operation outside direct Run publication fixture".into())
            }
            fn read_assistant_record_chunk(
                &self,
                snapshot: &AssistantJournalSnapshot,
                part_id: &str,
                content: &JournalContentRef,
                chunk_index: u64,
            ) -> Result<AssistantTextChunkV1, Self::Error> {
                Err("operation outside direct Run publication fixture".into())
            }
            fn read_assistant_snapshot_range(
                &self,
                snapshot: &AssistantJournalSnapshot,
                part_id: &str,
                content: &JournalContentRef,
                offset_bytes: u64,
                limit_bytes: usize,
            ) -> Result<AssistantContentRange, Self::Error> {
                Err("operation outside direct Run publication fixture".into())
            }
        }
    };
}
fixture_ports!(MemoryRun);
fixture_ports!(MemoryCreation);

macro_rules! terminal_read_port {
    ($ty:ty) => {
        impl nola_store_port::remote_session::CanonicalRunTerminalRepository for $ty {
            type Error = String;
            fn read_canonical_run_terminal(&self, conversation: &str, run: &str)
                -> Result<Option<nola_store_port::remote_session::CanonicalRunTerminal>, String> {
                let Some((seal, _)) = self.journal().terminal.as_ref() else { return Ok(None) };
                if seal.identity.conversation_id != conversation || seal.identity.run_id != run { return Ok(None) }
                seal.validate()?;
                Ok(Some(nola_store_port::remote_session::CanonicalRunTerminal {
                    conversation_id:conversation.into(), run_id:run.into(),
                    assistant_message_id:seal.identity.assistant_message_id.clone(),
                    status:nola_store_port::remote_session::CanonicalRunTerminalStatus::Sealed,
                    result:seal.result.clone(), result_digest:seal.result_digest.clone(), completed_at_ms:seal.completed_at_ms,
                }))
            }
        }
    };
}
terminal_read_port!(MemoryRun);
terminal_read_port!(MemoryCreation);

macro_rules! task_authority_read_port {
    ($ty:ty) => {
        impl nola_store_port::runtime_task::TaskAuthorityReadRepository for $ty {
            type Error = String;
            fn read_runtime_task(&self, id: &str) -> Result<Option<nola_store_port::runtime_task::StoredRuntimeTask>, String> {
                Ok(self.journal().tasks.get(id).cloned())
            }
            fn read_runtime_task_run_binding(&self, id: &str) -> Result<Option<nola_store_port::runtime_task::RuntimeTaskRunBinding>, String> {
                Ok(self.journal().task_bindings.get(id).cloned())
            }
            fn read_runtime_agent_task_authority(&self, id: &str) -> Result<Option<nola_store_port::runtime_task::RuntimeAgentTaskAuthority>, String> {
                Ok(self.journal().task_authorities.get(id).cloned())
            }
        }
    };
}
task_authority_read_port!(MemoryRun);
task_authority_read_port!(MemoryCreation);
