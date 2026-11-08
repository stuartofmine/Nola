// SPDX-License-Identifier: Apache-2.0
//! UI-independent reads of the committed, paginated source journal.
use crate::application_port::CliApplicationPort;
use nola_application::{
    ApplicationCommand as Command, AssistantJournalPartChangePage, AssistantJournalSnapshotHandle,
    AssistantJournalTextRange,
};
use nola_domain::assistant_journal::{AssistantJournalCursor, ASSISTANT_RANGE_MAX_BYTES};
use nola_domain::assistant_journal_operations::*;
use std::collections::{BTreeMap, HashMap};

fn validate_journal_range(
    range: &AssistantJournalTextRange,
    offset: u64,
    content_bytes: u64,
) -> Result<(), String> {
    if range.offset_bytes.0 != offset
        || range.next_offset_bytes.0 <= offset
        || range.next_offset_bytes.0 > content_bytes
        || range.next_offset_bytes.0 - offset != range.text.len() as u64
        || range.text.len() > ASSISTANT_RANGE_MAX_BYTES
        || range.total_utf8_bytes.0 != content_bytes
        || range.end_of_content != (range.next_offset_bytes.0 == content_bytes)
    {
        return Err("journal content range did not advance exactly within its budget".into());
    }
    Ok(())
}

fn decode_structured_journal_field(raw: Option<&str>) -> Result<Option<serde_json::Value>, String> {
    let Some(raw) = raw else { return Ok(None) };
    let envelope: serde_json::Value = serde_json::from_str(raw)
        .map_err(|error| format!("invalid journal structured field: {error}"))?;
    if envelope
        .get("protocol_version")
        .and_then(serde_json::Value::as_u64)
        != Some(1)
    {
        return Err("invalid journal structured field version".into());
    }
    envelope
        .get("value")
        .cloned()
        .map(Some)
        .ok_or_else(|| "journal structured field missing value".into())
}

/// A source-side consumer pinned to a caller-owned Run. Notifications are
/// wakeups only; all content and completion facts come from Application reads.
#[derive(Debug)]
pub(crate) struct SourceRunJournal {
    pub conversation_id: String,
    pub run_id: String,
    pub assistant_message_id: String,
    epoch: Option<String>,
    reader: CommittedJournalReader,
    owner_order: HashMap<String, i64>,
    pub parts: BTreeMap<String, SourceJournalPart>,
    pub seal: Option<nola_domain::assistant_run_terminal::AssistantRunTerminalSeal>,
}

#[derive(Debug)]
pub(crate) struct SourceJournalPart {
    pub head: JournalPartHead,
    pub fields: BTreeMap<JournalContentField, String>,
}

impl SourceJournalPart {
    pub fn text<'a>(&'a self, value: &'a JournalTextValue) -> &'a str {
        match value {
            JournalTextValue::Inline(text) => text,
            JournalTextValue::Field(field) => {
                self.fields.get(field).map(String::as_str).unwrap_or("")
            }
        }
    }

    pub fn structured(
        &self,
        field: JournalContentField,
    ) -> Result<Option<serde_json::Value>, String> {
        decode_structured_journal_field(self.fields.get(&field).map(String::as_str))
    }
}

impl SourceRunJournal {
    pub fn new(conversation_id: String, run_id: String, assistant_message_id: String) -> Self {
        Self {
            conversation_id,
            run_id,
            assistant_message_id,
            epoch: None,
            reader: Default::default(),
            owner_order: HashMap::new(),
            parts: BTreeMap::new(),
            seal: None,
        }
    }

    fn read_head(
        &mut self,
        port: &mut CliApplicationPort,
        through: &AssistantJournalCursor,
    ) -> Result<(), String> {
        through.identity.validate().map_err(|e| e.to_string())?;
        if through.identity.conversation_id != self.conversation_id
            || through.identity.run_id != self.run_id
            || self
                .epoch
                .as_ref()
                .is_some_and(|epoch| epoch != &through.identity.source_store_epoch)
        {
            return Err("source journal Run or store identity changed".into());
        }
        self.epoch = Some(through.identity.source_store_epoch.clone());
        for update in self.reader.read(port, through)? {
            let Some(head) = update.head else {
                self.parts.remove(&update.key);
                continue;
            };
            let mut fields = update.fields;
            let has_markdown = head
                .bodies
                .iter()
                .any(|body| body.field == JournalContentField::Markdown);
            let mut markdown = if update.reset || !has_markdown {
                String::new()
            } else {
                self.parts
                    .get(&update.key)
                    .and_then(|part| part.fields.get(&JournalContentField::Markdown))
                    .cloned()
                    .unwrap_or_default()
            };
            markdown.push_str(&update.markdown_delta);
            if has_markdown {
                fields.insert(JournalContentField::Markdown, markdown);
            }
            self.parts
                .insert(update.key, SourceJournalPart { head, fields });
        }
        Ok(())
    }

    pub fn catch_up(&mut self, port: &mut CliApplicationPort) -> Result<(), String> {
        let seal: Option<nola_domain::assistant_run_terminal::AssistantRunTerminalSeal> = port
            .execute(Command::LocalAssistantRunTerminalSeal {
                conversation_id: self.conversation_id.clone(),
                run_id: self.run_id.clone(),
            })?;
        if let Some(seal) = seal {
            seal.validate()?;
            if seal.identity.conversation_id != self.conversation_id
                || seal.identity.run_id != self.run_id
                || seal.identity.assistant_message_id != self.assistant_message_id
                || self
                    .epoch
                    .as_ref()
                    .is_some_and(|epoch| epoch != &seal.identity.source_store_epoch)
            {
                return Err("source terminal seal does not belong to this Run/root".into());
            }
            if let Some(previous) = &self.seal {
                return if previous == &seal {
                    Ok(())
                } else {
                    Err("source terminal seal changed".into())
                };
            }
            // A live presentation may have observed a later mutable head.
            // Completion is materialized independently at the immutable seal.
            self.reader = Default::default();
            self.parts.clear();
            self.owner_order.clear();
            let mut after = JournalNumber(0);
            let mut entries = Vec::new();
            loop {
                let page: nola_domain::assistant_run_terminal::AssistantRunTerminalManifestPage =
                    port.execute(Command::ReadLocalAssistantRunTerminalManifest {
                        seal: seal.clone(),
                        after_entry: after,
                        max_records: 64,
                        max_encoded_bytes: 262144,
                    })?;
                if page.seal != seal
                    || page.after_entry != after
                    || page.next_entry.0 <= after.0
                    || page.next_entry.0 > seal.entry_count.0
                    || page.entries.len() > 64
                    || page.next_entry.0 - after.0 != page.entries.len() as u64
                    || page.complete != (page.next_entry == seal.entry_count)
                {
                    return Err("source terminal manifest did not advance exactly".into());
                }
                entries.extend(page.entries);
                after = page.next_entry;
                if page.complete {
                    break;
                }
            }
            seal.validate_manifest(&entries)?;
            for entry in entries {
                if let nola_domain::assistant_run_terminal::AssistantRunTerminalManifestEntry::Assistant { through, ordinal, .. } = entry {
                    self.owner_order.insert(through.identity.assistant_message_id.clone(), ordinal);
                    self.read_head(port, &through)?;
                }
            }
            self.seal = Some(seal);
            return Ok(());
        }
        if self.seal.is_some() {
            return Err("source terminal seal disappeared".into());
        }
        let mut after: Option<String> = None;
        for page in 0..64 {
            let heads: Vec<AssistantJournalCursor> =
                port.execute(Command::LocalRunAssistantJournalHeads {
                    conversation_id: self.conversation_id.clone(),
                    run_id: self.run_id.clone(),
                    after_message_id: after.clone(),
                    limit: 32,
                })?;
            if heads.len() > 32 {
                return Err("source head page exceeded its budget".into());
            }
            for head in &heads {
                if after
                    .as_ref()
                    .is_some_and(|old| old >= &head.identity.assistant_message_id)
                {
                    return Err("source head page did not advance".into());
                }
                self.read_head(port, head)?;
                after = Some(head.identity.assistant_message_id.clone());
            }
            if heads.len() < 32 {
                return Ok(());
            }
            if page == 63 {
                return Err("source head directory exceeded its budget".into());
            }
        }
        unreachable!()
    }

    pub fn public_text(&self) -> String {
        let mut parts = self.parts.values().filter(|part| matches!(&part.head.metadata,
            JournalPartMetadata::Text(metadata) if metadata.visibility == JournalVisibility::Public &&
                matches!(metadata.review, None | Some(nola_domain::TextReviewState::Approved)))).collect::<Vec<_>>();
        parts.sort_by_key(|part| {
            (
                self.owner_order
                    .get(&part.head.identity.assistant_message_id)
                    .copied()
                    .unwrap_or(
                        if part.head.identity.assistant_message_id == self.assistant_message_id {
                            0
                        } else {
                            i64::MAX
                        },
                    ),
                part.head.identity.assistant_message_id.clone(),
                part.head.order_key.section,
                part.head.order_key.position,
                part.head.part_id.clone(),
            )
        });
        parts
            .into_iter()
            .filter_map(|part| part.fields.get(&JournalContentField::Markdown))
            .cloned()
            .collect()
    }

    pub(crate) fn ordered_parts(&self) -> Vec<&SourceJournalPart> {
        let mut parts = self.parts.values().collect::<Vec<_>>();
        parts.sort_by_key(|part| {
            (
                self.owner_order
                    .get(&part.head.identity.assistant_message_id)
                    .copied()
                    .unwrap_or(i64::MAX),
                part.head.identity.assistant_message_id.clone(),
                part.head.order_key.section,
                part.head.order_key.position,
                part.head.part_id.clone(),
            )
        });
        parts
    }
}

#[derive(Debug, Default)]
pub(crate) struct CommittedJournalReader {
    owners: HashMap<String, AssistantJournalCursor>,
    parts: HashMap<String, CachedPart>,
}
#[derive(Debug)]
struct CachedPart {
    head: JournalPartHead,
    fields: BTreeMap<JournalContentField, String>,
}
#[derive(Debug)]
pub(crate) struct CommittedJournalUpdate {
    pub key: String,
    pub head: Option<JournalPartHead>,
    pub reset: bool,
    pub markdown_delta: String,
    pub reasoning_delta: String,
    pub fields: BTreeMap<JournalContentField, String>,
}
impl CommittedJournalUpdate {
    pub fn text(&self, value: &JournalTextValue) -> String {
        match value {
            JournalTextValue::Inline(s) => s.clone(),
            JournalTextValue::Field(f) => self.fields.get(f).cloned().unwrap_or_default(),
        }
    }
    pub fn field(&self, field: JournalContentField) -> &str {
        self.fields.get(&field).map(String::as_str).unwrap_or("")
    }
    pub fn structured(
        &self,
        field: JournalContentField,
    ) -> Result<Option<serde_json::Value>, String> {
        decode_structured_journal_field(self.fields.get(&field).map(String::as_str))
    }
}
impl CommittedJournalReader {
    pub fn read(
        &mut self,
        port: &mut CliApplicationPort,
        through: &AssistantJournalCursor,
    ) -> Result<Vec<CommittedJournalUpdate>, String> {
        self.read_with_ranges(port, through, |port, command| port.execute(command))
    }

    fn read_with_ranges(
        &mut self,
        port: &mut CliApplicationPort,
        through: &AssistantJournalCursor,
        mut read_range: impl FnMut(
            &mut CliApplicationPort,
            Command,
        ) -> Result<AssistantJournalTextRange, String>,
    ) -> Result<Vec<CommittedJournalUpdate>, String> {
        let owner = serde_json::to_string(&through.identity).map_err(|e| e.to_string())?;
        let base = self.owners.get(&owner).cloned().unwrap_or(
            AssistantJournalCursor::initial(through.identity.clone()).map_err(|e| e.to_string())?,
        );
        if base.sequence >= through.sequence {
            return if base.sequence == through.sequence && base != *through {
                Err("journal cursor digest changed".into())
            } else {
                Ok(vec![])
            };
        }
        let snapshot: AssistantJournalSnapshotHandle =
            port.execute(Command::OpenAssistantJournalSnapshot {
                at: through.clone(),
            })?;
        let result = (|| -> Result<_, String> {
            let mut after = None;
            let mut changes = vec![];
            loop {
                let page: AssistantJournalPartChangePage =
                    port.execute(Command::ReadAssistantJournalPartChanges {
                        snapshot: snapshot.clone(),
                        base: base.clone(),
                        after_part_id: after.clone(),
                        max_records: 64,
                        max_encoded_bytes: 262144,
                    })?;
                if page.base != base || page.at != *through || page.changes.len() > 64 {
                    return Err("journal change frontier mismatch".into());
                }
                changes.extend(page.changes);
                let Some(next) = page.next_part_id else { break };
                if after.as_ref().is_some_and(|old| old >= &next) {
                    return Err("journal directory did not advance".into());
                }
                after = Some(next);
            }
            // Change-page continuations are lexicographic Part IDs, but display
            // order is the independent section/position/ID tuple.
            changes.sort_by_key(|change| {
                (
                    change
                        .head
                        .as_ref()
                        .map(|head| (head.order_key.section, head.order_key.position)),
                    change.part_id.clone(),
                )
            });
            let mut updates = vec![];
            let mut cache = vec![];
            for change in changes {
                let key = format!("{owner}/{}", change.part_id);
                let Some(head) = change.head else {
                    updates.push(CommittedJournalUpdate {
                        key: key.clone(),
                        head: None,
                        reset: true,
                        markdown_delta: String::new(),
                        reasoning_delta: String::new(),
                        fields: BTreeMap::new(),
                    });
                    cache.push((key, None));
                    continue;
                };
                if head.identity != through.identity || head.part_id != change.part_id {
                    return Err("journal Part owner mismatch".into());
                }
                let previous = self.parts.get(&key);
                let reset = previous.is_some_and(|p| {
                    p.head.generation != head.generation
                        || p.head
                            .bodies
                            .iter()
                            .find(|body| body.field == JournalContentField::Markdown)
                            .is_some_and(|old| {
                                !head.bodies.iter().any(|body| {
                                    body.field == JournalContentField::Markdown
                                        && body.content_generation == old.content_generation
                                        && body.utf8_bytes >= old.utf8_bytes
                                })
                            })
                });
                let mut fields = BTreeMap::new();
                let mut markdown = String::new();
                let mut reasoning = String::new();
                for content in &head.bodies {
                    let old = previous
                        .filter(|_| !reset)
                        .and_then(|p| p.head.bodies.iter().find(|v| v.field == content.field));
                    let append = old.is_some_and(|v| {
                        v.content_generation == content.content_generation
                            && v.utf8_bytes <= content.utf8_bytes
                    });
                    let mut offset = if append { old.unwrap().utf8_bytes.0 } else { 0 };
                    let mut text = String::new();
                    while offset < content.utf8_bytes.0 {
                        let page = read_range(
                            port,
                            Command::ReadAssistantJournalRange {
                                snapshot: snapshot.clone(),
                                part_id: head.part_id.clone(),
                                content: content.clone(),
                                offset_bytes: JournalNumber(offset),
                                limit_bytes: ASSISTANT_RANGE_MAX_BYTES as u32,
                            },
                        )?;
                        validate_journal_range(&page, offset, content.utf8_bytes.0)?;
                        text.push_str(&page.text);
                        offset = page.next_offset_bytes.0;
                    }
                    match content.field {
                        JournalContentField::Markdown => markdown = text,
                        JournalContentField::ReasoningSummary => reasoning = text,
                        field => {
                            let mut value = if append {
                                previous
                                    .and_then(|p| p.fields.get(&field))
                                    .cloned()
                                    .unwrap_or_default()
                            } else {
                                String::new()
                            };
                            value.push_str(&text);
                            fields.insert(field, value);
                        }
                    }
                }
                updates.push(CommittedJournalUpdate {
                    key: key.clone(),
                    head: Some(head.clone()),
                    reset,
                    markdown_delta: markdown,
                    reasoning_delta: reasoning,
                    fields: fields.clone(),
                });
                cache.push((key, Some(CachedPart { head, fields })));
            }
            Ok((updates, cache))
        })();
        let released: Result<serde_json::Value, String> =
            port.execute(Command::ReleaseAssistantJournalSnapshot { snapshot });
        let (updates, cache) = result?;
        released?;
        for (key, value) in cache {
            match value {
                Some(value) => {
                    self.parts.insert(key, value);
                }
                None => {
                    self.parts.remove(&key);
                }
            }
        }
        self.owners.insert(owner, through.clone());
        Ok(updates)
    }
}

#[cfg(test)]
pub(crate) mod source_tests {
    use super::*;
    use nola_core::agent::host::*;
    use nola_core::agent::protocol::*;
    use serde_json::json;

    /// Core reduces real Host deliveries and Application publishes every
    /// journal and terminal fact; tests never handcraft committed seals.
    pub fn completed_source(
        tools: &[(&str, serde_json::Value)],
        text: &str,
    ) -> (CliApplicationPort, SourceRunJournal) {
        completed_source_with_port(CliApplicationPort::ephemeral().unwrap(), tools, text)
    }

    pub fn completed_source_with_port(
        mut port: CliApplicationPort,
        tools: &[(&str, serde_json::Value)],
        text: &str,
    ) -> (CliApplicationPort, SourceRunJournal) {
        let descriptor = |id: &str, route| HostCapabilityDescriptor {
            id: id.into(),
            title: id.into(),
            route,
            availability: CapabilityAvailability::Available,
            exposure: HostCapabilityExposure::Visible,
            permission_domain: None,
            input_schema: json!({"type":"object"}),
            input_normalization: HostInputNormalization {
                permission_contract: nola_domain::permission_resource_plan::PermissionOperationContract {
                    // These fixtures deliver synthetic goal declarations or
                    // ordinary read results, never real filesystem content.
                    risk: if matches!(id, "goal_complete" | "read_file") {
                        nola_domain::permission_resource_plan::PermissionOperationRisk::OrdinaryRead
                    } else {
                        nola_domain::permission_resource_plan::PermissionOperationRisk::Unknown
                    },
                    ..Default::default()
                },
                ..Default::default()
            },
            output_policy: HostOutputPolicy {
                preview_char_limit: 4000,
                persist_full_output: true,
                allow_artifacts: true,
            },
            supports_streaming: id == "model.stream",
            supports_argument_streaming: false,
            parallelism: HostCapabilityParallelism::Serial,
            cancellation: HostCancellationMode::Cooperative,
            replay_safety: HostReplaySafety::Recoverable,
        };
        let mut capabilities = BTreeMap::new();
        capabilities.insert(
            "model.stream".into(),
            descriptor("model.stream", HostCapabilityRoute::Model),
        );
        for (id, _) in tools {
            capabilities.insert(id.to_string(), descriptor(id, HostCapabilityRoute::Tool));
        }
        let begun = port.begin_assistant_turn(nola_application::BeginAssistantTurn {
            expected_configuration_revision: None, expected_parent_message_id: None,
            conversation_id: "source-conversation".into(), conversation_title: "Source fixture".into(),
            user_message_id: "source-user".into(), user_part_id: "source-user:text".into(), user_text: "Work".into(),
            user_content_json: None, assistant_message_id: "source-root".into(), context_checkpoint_id: None,
            core_config: AgentCoreConfig {
                turn_identity: crate::test_model_identity::identity(),
                session_id: Some(SessionId::from_string("actual-source-core".into())), model: Some("fixture-model".into()),
                // Calls may repeat; the model catalog contains each capability once.
                tools: tools.iter().map(|(id, _)| *id).collect::<std::collections::BTreeSet<_>>()
                    .into_iter().map(|id| json!({"type":"function","function":{"name":id,"parameters":{"type":"object"}}})).collect(),
                host_catalog: Some(HostCapabilityCatalog { host_id: HostId("fixture-host".into()), revision: 1,
                    availability_epoch: 1, lease: None, capabilities }), ..Default::default()
            },
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(ItemId::from_string("source-user".into()), "Work")],
                turn_context_items: vec![], turn_context_platform: Default::default(), turn_context_components: vec![],
                submission_id: Some("source-run".into()), submitted_at_epoch_ms: Some(1000), delivery: AgentInputDelivery::CurrentTurn,
            }, created_at_ms: 1000,
        }).unwrap();
        let digest = begun.invocation_config_digest;
        let mut requested_tools = false;
        for round in 0..16 {
            let now = 1100 + round * 100;
            let claimed = port
                .claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
                    run_id: "source-run".into(),
                    worker_generation: "source-generation".into(),
                    lease_owner: "source-worker".into(),
                    claimed_at_ms: now,
                    lease_expires_at_ms: now + 60_000,
                    limit: 1,
                })
                .unwrap();
            let Some(request) = claimed.requests.into_iter().next() else {
                break;
            };
            let mut sequence = request.resume_after_sequence.unwrap_or(0);
            let mut responses = vec![HostResponse::ExecutionStarted];
            match &request.payload {
                HostRequest::ModelStream { .. } => {
                    responses.push(HostResponse::ModelAttemptStarted {
                        provider: "fixture-provider".into(),
                        model: "fixture-model".into(),
                        reason: "initial".into(),
                    });
                    let tool_calls = if !requested_tools {
                        requested_tools = true;
                        tools.iter().enumerate().map(|(index, (id, _))| json!({"call_id":format!("call-{index}"),"name":id,"arguments":{}})).collect()
                    } else {
                        vec![]
                    };
                    let items = if tool_calls.is_empty() {
                        vec![serde_json::to_value(AgentItem::assistant_message(
                            ItemId::from_string("source-answer".into()),
                            text,
                        ))
                        .unwrap()]
                    } else {
                        vec![]
                    };
                    responses.push(HostResponse::ModelCompleted {
                        items,
                        tool_calls,
                        usage: json!({}),
                    });
                }
                HostRequest::ExecuteTool {
                    capability_id,
                    call_id,
                    ..
                } => {
                    let output = tools
                        .iter()
                        .find(|(id, _)| *id == capability_id)
                        .unwrap()
                        .1
                        .clone();
                    responses.push(HostResponse::ToolCompleted {
                        call_id: call_id.clone(),
                        output,
                        outcome: HostToolOutcome::Succeeded,
                        activation: None,
                        full_output_ref: None,
                        artifacts: vec![],
                    });
                }
                other => panic!("Unexpected fixture Host operation: {other:?}"),
            }
            for (index, payload) in responses.into_iter().enumerate() {
                sequence += 1;
                port.reduce_and_accept_host_response(
                    nola_application::ReduceAndAcceptHostResponse {
                        conversation_id: "source-conversation".into(),
                        run_id: "source-run".into(),
                        invocation_config_digest: digest.clone(),
                        response: HostResponseEnvelope {
                            attempt_id: request.attempt_id.clone(),
                            delegated_invocation_id: request.delegated_invocation_id.clone(),
                            request_id: request.request_id.clone(),
                            effect_id: request.effect_id.clone(),
                            host_id: request.host_id.clone(),
                            lease_id: request.lease_id.clone(),
                            session_id: request.session_id.clone(),
                            turn_id: request.turn_id.clone(),
                            catalog_revision: request.catalog_revision,
                            availability_epoch: request.availability_epoch,
                            delivery: request.delivery.clone(),
                            sequence,
                            payload,
                        },
                        received_at_ms: now + index as i64 + 1,
                    },
                )
                .unwrap();
            }
        }
        let mut source = SourceRunJournal::new(
            "source-conversation".into(),
            "source-run".into(),
            "source-root".into(),
        );
        source.catch_up(&mut port).unwrap();
        assert!(matches!(
            source.seal.as_ref().unwrap().result.value,
            nola_domain::run_result::RunResultValue::Answered { .. }
        ));
        (port, source)
    }

    #[test]
    fn source_reader_requires_real_seal_and_replays_body_exactly_once() {
        let (mut port, mut source) = completed_source(&[], "Verified source body🙂");
        assert_eq!(source.public_text(), "Verified source body🙂");
        source.catch_up(&mut port).unwrap();
        assert_eq!(source.public_text(), "Verified source body🙂");
        let mut foreign = SourceRunJournal::new(
            source.conversation_id.clone(),
            source.run_id.clone(),
            "foreign-root".into(),
        );
        assert!(foreign.catch_up(&mut port).is_err());
        let mut missing = SourceRunJournal::new(
            source.conversation_id.clone(),
            "missing-run".into(),
            source.assistant_message_id.clone(),
        );
        if missing.catch_up(&mut port).is_ok() {
            assert!(missing.seal.is_none());
            assert!(missing.public_text().is_empty());
        }
    }

    fn unicode_body() -> String {
        (0..8_000)
            .map(|index| format!("行{index:05}🙂é｜committed\n"))
            .collect()
    }

    fn root_head(port: &mut CliApplicationPort) -> AssistantJournalCursor {
        let heads: Vec<AssistantJournalCursor> = port
            .execute(Command::LocalRunAssistantJournalHeads {
                conversation_id: "source-conversation".into(),
                run_id: "source-run".into(),
                after_message_id: None,
                limit: 32,
            })
            .unwrap();
        heads
            .into_iter()
            .find(|head| head.identity.assistant_message_id == "source-root")
            .unwrap()
    }

    fn public_update_text(updates: &[CommittedJournalUpdate]) -> String {
        updates
            .iter()
            .filter(|update| {
                matches!(update.head.as_ref().map(|head| &head.metadata),
                    Some(JournalPartMetadata::Text(metadata))
                    if metadata.visibility == JournalVisibility::Public
                        && matches!(metadata.review,
                            None | Some(nola_domain::TextReviewState::Approved)))
            })
            .map(|update| update.markdown_delta.as_str())
            .collect()
    }

    #[test]
    fn committed_unicode_ranges_are_bounded_exactly_once_and_survive_cold_read() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("unicode-source.sqlite");
        let text = unicode_body();
        let (mut port, source) = completed_source_with_port(
            CliApplicationPort::persistent(&database).unwrap(),
            &[],
            &text,
        );
        assert_eq!(source.public_text(), text);
        assert!(source.parts.values().any(|part| part
            .head
            .bodies
            .iter()
            .any(|body| body.field == JournalContentField::Markdown
                && body.utf8_bytes.0 > 64 * 1024
                && body.chunk_count.0 > 1)));
        let through = root_head(&mut port);
        let mut reader = CommittedJournalReader::default();
        let mut range_count = 0;
        let updates = reader
            .read_with_ranges(&mut port, &through, |port, command| {
                let (limit_bytes, offset_bytes, markdown) = match &command {
                    Command::ReadAssistantJournalRange {
                        limit_bytes,
                        offset_bytes,
                        content,
                        ..
                    } => (
                        *limit_bytes,
                        *offset_bytes,
                        content.field == JournalContentField::Markdown,
                    ),
                    _ => unreachable!(),
                };
                assert_eq!(limit_bytes, ASSISTANT_RANGE_MAX_BYTES as u32);
                let range: AssistantJournalTextRange = port.execute(command)?;
                assert_eq!(range.offset_bytes, offset_bytes);
                assert!(range.text.len() <= ASSISTANT_RANGE_MAX_BYTES);
                if markdown {
                    range_count += 1;
                }
                Ok(range)
            })
            .unwrap();
        assert!(
            range_count > 4,
            "source content must cross actual UTF-8 ranges"
        );
        assert_eq!(public_update_text(&updates), text);
        let display_keys = updates
            .iter()
            .filter_map(|update| update.head.as_ref())
            .map(|head| {
                (
                    head.order_key.section,
                    head.order_key.position,
                    &head.part_id,
                )
            })
            .collect::<Vec<_>>();
        assert!(display_keys.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(reader.read(&mut port, &through).unwrap().is_empty());
        drop(port);

        let mut cold = CliApplicationPort::persistent(&database).unwrap();
        assert_eq!(root_head(&mut cold), through);
        let mut cold_reader = CommittedJournalReader::default();
        assert_eq!(
            public_update_text(&cold_reader.read(&mut cold, &through).unwrap()),
            text
        );
        assert!(cold_reader.read(&mut cold, &through).unwrap().is_empty());
    }

    #[test]
    fn interrupted_or_invalid_source_range_does_not_consume_the_retry_cursor() {
        let text = unicode_body();
        let (mut port, _) = completed_source(&[], &text);
        let through = root_head(&mut port);
        let mut reader = CommittedJournalReader::default();
        let mut ranges = 0;
        let error = reader
            .read_with_ranges(&mut port, &through, |port, command| {
                if matches!(&command, Command::ReadAssistantJournalRange { content, .. }
                    if content.field == JournalContentField::Markdown)
                {
                    ranges += 1;
                    if ranges == 2 {
                        return Err("injected range transport interruption".into());
                    }
                }
                port.execute(command)
            })
            .unwrap_err();
        assert_eq!(error, "injected range transport interruption");
        assert_eq!(
            ranges, 2,
            "first actual source range must succeed before failure"
        );
        assert!(reader.owners.is_empty());
        assert!(reader.parts.is_empty());

        let mut ranges = 0;
        assert!(reader
            .read_with_ranges(&mut port, &through, |port, command| {
                let markdown = matches!(&command, Command::ReadAssistantJournalRange { content, .. }
                    if content.field == JournalContentField::Markdown);
                let mut range: AssistantJournalTextRange = port.execute(command)?;
                if markdown {
                    ranges += 1;
                    if ranges == 2 {
                        // A transport must not contradict the real source frontier.
                        range.end_of_content = !range.end_of_content;
                    }
                }
                Ok(range)
            })
            .is_err());
        assert_eq!(ranges, 2);
        assert!(reader.owners.is_empty());
        assert!(reader.parts.is_empty());
        assert_eq!(
            public_update_text(&reader.read(&mut port, &through).unwrap()),
            text
        );
        assert!(reader.read(&mut port, &through).unwrap().is_empty());
    }

    #[test]
    fn range_validation_rejects_wrong_length_budget_and_completion_frontier() {
        let first = AssistantJournalTextRange {
            offset_bytes: JournalNumber(0),
            next_offset_bytes: JournalNumber(4),
            total_utf8_bytes: JournalNumber(8),
            text: "🙂".into(),
            end_of_content: false,
        };
        validate_journal_range(&first, 0, 8).unwrap();
        let mut wrong = first.clone();
        wrong.end_of_content = true;
        assert!(validate_journal_range(&wrong, 0, 8).is_err());
        wrong = first.clone();
        wrong.text = "é".into();
        assert!(validate_journal_range(&wrong, 0, 8).is_err());
        wrong = first.clone();
        wrong.offset_bytes = JournalNumber(1);
        assert!(validate_journal_range(&wrong, 0, 8).is_err());
        wrong = first.clone();
        wrong.total_utf8_bytes = JournalNumber(9);
        assert!(validate_journal_range(&wrong, 0, 8).is_err());
        let final_range = AssistantJournalTextRange {
            offset_bytes: JournalNumber(4),
            next_offset_bytes: JournalNumber(8),
            text: "éé".into(),
            end_of_content: true,
            ..first
        };
        validate_journal_range(&final_range, 4, 8).unwrap();
        wrong = final_range;
        wrong.end_of_content = false;
        assert!(validate_journal_range(&wrong, 4, 8).is_err());
        let bytes = ASSISTANT_RANGE_MAX_BYTES as u64 + 1;
        let oversized = AssistantJournalTextRange {
            offset_bytes: JournalNumber(0),
            next_offset_bytes: JournalNumber(bytes),
            total_utf8_bytes: JournalNumber(bytes),
            text: "a".repeat(bytes as usize),
            end_of_content: true,
        };
        assert!(validate_journal_range(&oversized, 0, bytes).is_err());
    }

    #[test]
    fn missing_structured_body_is_distinct_from_an_actual_json_null() {
        let (_, source) = completed_source(&[], "actual source");
        let head = source.parts.values().next().unwrap().head.clone();
        let mut part = SourceJournalPart {
            head,
            fields: BTreeMap::new(),
        };
        assert_eq!(
            part.structured(JournalContentField::ToolOutput).unwrap(),
            None
        );
        part.fields.insert(
            JournalContentField::ToolOutput,
            json!({"protocol_version":1,"value":null}).to_string(),
        );
        assert_eq!(
            part.structured(JournalContentField::ToolOutput).unwrap(),
            Some(serde_json::Value::Null)
        );
        part.fields.insert(
            JournalContentField::ToolOutput,
            json!({"protocol_version":1}).to_string(),
        );
        assert!(part.structured(JournalContentField::ToolOutput).is_err());
        let mut update = CommittedJournalUpdate {
            key: "value-semantics".into(),
            head: None,
            reset: false,
            markdown_delta: String::new(),
            reasoning_delta: String::new(),
            fields: BTreeMap::new(),
        };
        assert_eq!(
            update.structured(JournalContentField::ToolOutput).unwrap(),
            None
        );
        update.fields.insert(
            JournalContentField::ToolOutput,
            json!({"protocol_version":1,"value":null}).to_string(),
        );
        assert_eq!(
            update.structured(JournalContentField::ToolOutput).unwrap(),
            Some(serde_json::Value::Null)
        );
        assert_eq!(
            update.structured(JournalContentField::ToolInput).unwrap(),
            None
        );
        update
            .fields
            .insert(JournalContentField::ToolOutput, String::new());
        assert!(update.structured(JournalContentField::ToolOutput).is_err());
        update.fields.insert(
            JournalContentField::ToolOutput,
            json!({"protocol_version":1}).to_string(),
        );
        assert!(update.structured(JournalContentField::ToolOutput).is_err());
    }
}
