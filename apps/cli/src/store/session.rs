use super::{DbSessionInfo, DbSessionInfoWithFirstUserMessage, NolaStore};
use nola_application::{
    ApplicationCommand, Conversation, ConversationMessage, ConversationPart, ConversationSummary,
};
use nola_protocol::{
    ExportBodyField, ExportCommand, ExportPage, ExportRange, PartDerivation, StoredPartDerivation,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(serde::Serialize)]
struct ArchivedToolSource {
    claim: nola_domain::tool_source::ToolSourceClaim,
    body_json: String,
}

impl NolaStore {
    /// Reconstructs a client-owned archive from bounded Service reads. No
    /// complete conversation or large Part crosses a single Application frame.
    pub(super) fn load_canonical_conversation(
        &self,
        id: &str,
    ) -> Result<Option<Conversation>, String> {
        Ok(self
            .load_canonical_export(id, false)?
            .map(|(conversation, _, _)| conversation))
    }

    /// Lossless JSON export keeps original fields and derivation claims. The
    /// existing Web message projection remains a separate presentation format.
    pub fn load_canonical_archive(&self, id: &str) -> Result<Option<Value>, String> {
        let Some((conversation, mut derivations, tool_sources)) = self.load_canonical_export(id, true)? else {
            return Ok(None);
        };
        let mut archive = serde_json::to_value(conversation).map_err(|error| error.to_string())?;
        for message in archive["messages"]
            .as_array_mut()
            .ok_or("invalid archive messages")?
        {
            for part in message["parts"]
                .as_array_mut()
                .ok_or("invalid archive Parts")?
            {
                let id = part["id"].as_str().ok_or("missing archive Part identity")?;
                if let Some(record) = derivations.remove(id) {
                    part["derivation"] =
                        serde_json::to_value(record).map_err(|error| error.to_string())?;
                }
            }
        }
        if !derivations.is_empty() {
            return Err("archive derivation has no target Part".into());
        }
        archive["tool_sources"] = serde_json::to_value(tool_sources).map_err(|error|error.to_string())?;
        Ok(Some(archive))
    }

    fn load_canonical_export(
        &self,
        id: &str,
        include_derivations: bool,
    ) -> Result<Option<(Conversation, BTreeMap<String, PartDerivation>, Vec<ArchivedToolSource>)>, String> {
        let mut derivations = BTreeMap::new();
        let mut tool_sources = Vec::new();
        let mut source_ids = std::collections::BTreeSet::new();
        let mut application = self.application()?;
        let first: Option<ExportPage> =
            application.execute(ExportCommand::LoadConversationExportPage {
                conversation_id: id.into(),
                after_ordinal: None,
                expected_revision: None,
            })?;
        let Some(mut page) = first else {
            return Ok(None);
        };
        let header = page.conversation.clone();
        if header.id != id || header.revision < 1 || header.configuration_revision < 1 {
            return Err("export conversation identity mismatch".into());
        }
        let mut messages = Vec::new();
        let mut after = None;
        loop {
            if page.conversation != header
                || page.messages.len() > 1
                || page.next_ordinal != page.messages.last().map(|message| message.ordinal)
                || (page.has_more && page.messages.is_empty())
            {
                return Err("export metadata page changed identity or cursor".into());
            }
            for message in page.messages {
                if message.ordinal <= after.unwrap_or(0) || message.parts.len() > 512 {
                    return Err("export metadata page does not make bounded progress".into());
                }
                let mut parts = Vec::with_capacity(message.parts.len());
                for part in message.parts {
                    if include_derivations && part.kind == "tool" {
                        let owner=nola_protocol::CanonicalPartOwner::new(id.to_owned(),message.id.clone(),part.id.clone())
                            .map_err(|error|error.to_string())?;
                        let claims:Vec<nola_domain::tool_source::ToolSourceClaim>=application.execute(
                            ExportCommand::ReadConversationToolSources {owner:owner.clone(),expected_conversation_revision:header.revision})?;
                        if claims.len()>2 {return Err("archive tool source metadata exceeds its pair bound".into());}
                        let mut kinds=std::collections::BTreeSet::new();
                        for claim in claims {
                            claim.validate()?;
                            if claim.owner!=owner || !source_ids.insert(claim.source_item_id.clone())
                                || !kinds.insert(serde_json::to_string(&claim.kind).map_err(|error|error.to_string())?) {
                                return Err("archive tool source has foreign or duplicated identity".into());
                            }
                            let body_json=load_export_tool_source(&mut application,&claim,header.revision)?;
                            tool_sources.push(ArchivedToolSource {claim,body_json});
                        }
                    }
                    if include_derivations && part.has_derivation {
                        let owner = nola_protocol::CanonicalPartOwner::new(
                            id.into(),
                            message.id.clone(),
                            part.id.clone(),
                        )
                        .map_err(|error| error.to_string())?;
                        let record: Option<StoredPartDerivation> =
                            application.execute(ExportCommand::ReadConversationPartDerivation {
                                owner: owner.clone(),
                                expected_conversation_revision: header.revision,
                            })?;
                        let record = record.ok_or("declared archive derivation is unavailable")?;
                        if record.derivation.target.owner != owner
                            || serde_json::to_vec(&record)
                                .map_err(|error| error.to_string())?
                                .len()
                                > 24576
                        {
                            return Err("archive derivation owner or byte budget mismatch".into());
                        }
                        if derivations
                            .insert(part.id.clone(), record.derivation)
                            .is_some()
                        {
                            return Err("duplicate archive Part derivation".into());
                        }
                    }
                    let text = load_export_body(
                        &mut application,
                        id,
                        &message.id,
                        &part.id,
                        ExportBodyField::ContentText,
                        header.revision,
                        part.content_text_utf8_bytes,
                    )?;
                    let json = part
                        .content_json_utf8_bytes
                        .map(|bytes| {
                            load_export_body(
                                &mut application,
                                id,
                                &message.id,
                                &part.id,
                                ExportBodyField::ContentJson,
                                header.revision,
                                bytes,
                            )
                        })
                        .transpose()?;
                    parts.push(ConversationPart {
                        id: part.id,
                        kind: part.kind,
                        ordinal: part.ordinal,
                        state: part.state,
                        content_text: text,
                        content_json: json,
                        content_json_deferred: false,
                        content_digest: part.content_digest,
                        created_at_ms: part.created_at_ms,
                        updated_at_ms: part.updated_at_ms,
                    });
                }
                after = Some(message.ordinal);
                messages.push(ConversationMessage {
                    id: message.id,
                    run_id: message.run_id,
                    role: message.role,
                    status: message.status,
                    ordinal: message.ordinal,
                    created_at_ms: message.created_at_ms,
                    completed_at_ms: message.completed_at_ms,
                    parts,
                });
            }
            if !page.has_more {
                break;
            }
            page = application
                .execute::<Option<ExportPage>>(ExportCommand::LoadConversationExportPage {
                    conversation_id: id.into(),
                    after_ordinal: after,
                    expected_revision: Some(header.revision),
                })?
                .ok_or("conversation disappeared during export")?;
        }
        let final_page: Option<ExportPage> =
            application.execute(ExportCommand::LoadConversationExportPage {
                conversation_id: id.into(),
                after_ordinal: None,
                expected_revision: Some(header.revision),
            })?;
        if final_page.as_ref().map(|page| &page.conversation) != Some(&header) {
            return Err("conversation changed during export".into());
        }
        Ok(Some((
            Conversation {
                id: header.id,
                title: header.title,
                revision: header.revision,
                configuration_revision: header.configuration_revision,
                created_at_ms: header.created_at_ms,
                updated_at_ms: header.updated_at_ms,
                settings_json: header.settings_json,
                messages,
            },
            derivations,
            tool_sources,
        )))
    }

    pub fn latest_session_id(&self) -> Result<Option<String>, String> {
        self.latest_session_id_for_project("")
    }

    pub fn latest_session_id_for_project(
        &self,
        project_root: &str,
    ) -> Result<Option<String>, String> {
        Ok(self
            .list_sessions_with_first_user_message_for_project_with_limit(project_root, 1)?
            .into_iter()
            .next()
            .map(|session| session.id))
    }

    pub fn session_metadata(&self, id: &str) -> Result<Option<(String, String)>, String> {
        let mut application = self.application()?;
        let page: Option<ExportPage> =
            application.execute(ExportCommand::LoadConversationExportPage {
                conversation_id: id.into(),
                after_ordinal: None,
                expected_revision: None,
            })?;
        Ok(page.map(|page| {
            (
                timestamp_text(page.conversation.created_at_ms),
                timestamp_text(page.conversation.updated_at_ms),
            )
        }))
    }

    pub fn load_session_messages(&self, id: &str) -> Result<Vec<Value>, String> {
        let conversation = self.load_canonical_conversation(id)?;
        conversation
            .map(|conversation| {
                conversation
                    .messages
                    .iter()
                    .map(canonical_message_value)
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()
            .map(Option::unwrap_or_default)
    }

    pub fn list_sessions(&self) -> Result<Vec<DbSessionInfo>, String> {
        let mut application = self.application()?;
        let summaries: Vec<ConversationSummary> =
            application.list_conversations()?;
        Ok(summaries
            .into_iter()
            .map(|summary| DbSessionInfo {
                id: summary.id,
                title: summary.title,
                created_at: timestamp_text(summary.created_at_ms),
                updated_at: timestamp_text(summary.updated_at_ms),
                message_count: summary.message_count.max(0) as usize,
            })
            .collect())
    }

    #[allow(dead_code)]
    pub fn list_sessions_with_first_user_message(
        &self,
    ) -> Result<Vec<DbSessionInfoWithFirstUserMessage>, String> {
        self.list_sessions_with_first_user_message_with_limit(usize::MAX)
    }

    #[allow(dead_code)]
    pub fn list_sessions_with_first_user_message_with_limit(
        &self,
        limit: usize,
    ) -> Result<Vec<DbSessionInfoWithFirstUserMessage>, String> {
        self.list_sessions_with_first_user_message_for_project_with_limit("", limit)
    }

    #[allow(dead_code)]
    pub fn list_sessions_with_first_user_message_for_project(
        &self,
        project_root: &str,
    ) -> Result<Vec<DbSessionInfoWithFirstUserMessage>, String> {
        self.list_sessions_with_first_user_message_for_project_with_limit(project_root, usize::MAX)
    }

    pub fn list_sessions_with_first_user_message_for_project_with_limit(
        &self,
        project_root: &str,
        limit: usize,
    ) -> Result<Vec<DbSessionInfoWithFirstUserMessage>, String> {
        let mut application = self.application()?;
        let mut summaries = Vec::new();
        while summaries.len() < limit {
            let count = (limit - summaries.len()).min(1024);
            let page = application.list_conversation_page(
                (!project_root.is_empty()).then_some(project_root), true, count, summaries.len(),
            )?;
            let complete = page.len() < count;
            summaries.extend(page);
            if complete { break; }
        }
        let mut result = Vec::new();
        for summary in summaries {
            if summary.message_count <= 0 || result.len() >= limit {
                continue;
            }
            result.push(DbSessionInfoWithFirstUserMessage {
                id: summary.id,
                created_at: timestamp_text(summary.created_at_ms),
                updated_at: timestamp_text(summary.updated_at_ms),
                message_count: summary.message_count as usize,
                first_user_message_json: summary.first_user_message.map(|text|
                    serde_json::json!({"role":"user","content":text}).to_string()),
                last_user_message_at: timestamp_text(summary.last_user_message_at_ms.unwrap_or(summary.updated_at_ms)),
            });
        }
        Ok(result)
    }

    pub fn session_has_messages(&self, id: &str) -> Result<bool, String> {
        let mut application = self.application()?;
        let page: Option<ExportPage> =
            application.execute(ExportCommand::LoadConversationExportPage {
                conversation_id: id.into(),
                after_ordinal: None,
                expected_revision: None,
            })?;
        Ok(page.is_some_and(|page| !page.messages.is_empty()))
    }

    pub fn delete_session(&self, id: &str) -> Result<(), String> {
        let mut application = self.application()?;
        let _: bool = application.execute(ApplicationCommand::DeleteConversation {
            conversation_id: id.to_string(),
        })?;
        Ok(())
    }
}

fn load_export_body(
    application: &mut crate::application_port::CliApplicationPort,
    conversation: &str,
    message: &str,
    part: &str,
    field: ExportBodyField,
    revision: i64,
    total: u64,
) -> Result<String, String> {
    let mut text = String::new();
    let mut offset = 0;
    while offset < total {
        let range: ExportRange =
            application.execute(ExportCommand::ReadConversationExportRange {
                conversation_id: conversation.into(),
                message_id: message.into(),
                part_id: part.into(),
                field,
                expected_revision: revision,
                offset_bytes: offset,
                limit_bytes: 32768,
            })?;
        if range.conversation_id != conversation
            || range.message_id != message
            || range.part_id != part
            || range.field != field
            || range.revision != revision
            || range.offset_bytes != offset
            || range.text.len() > 32768
            || offset.checked_add(range.text.len() as u64) != Some(range.next_offset_bytes)
            || range.total_utf8_bytes != total
            || range.next_offset_bytes <= offset
            || range.next_offset_bytes > total
            || range.end_of_content != (range.next_offset_bytes == total)
        {
            return Err("export original range has invalid owner, revision or byte bounds".into());
        }
        text.push_str(&range.text);
        offset = range.next_offset_bytes;
    }
    Ok(text)
}

fn load_export_tool_source(application:&mut crate::application_port::CliApplicationPort,
    claim:&nola_domain::tool_source::ToolSourceClaim,revision:i64) -> Result<String,String> {
    let total=claim.body.total_utf8_bytes();
    let mut offset=0;
    let mut text=String::new();
    let mut digest=Sha256::new();
    while offset<total {
        let range:nola_domain::tool_source::ToolSourceExportRange=application.execute(
            ExportCommand::ReadConversationToolSourceRange {owner:claim.owner.clone(),source_item_id:claim.source_item_id.clone(),
                expected_conversation_revision:revision,offset_bytes:offset,limit_bytes:32768})?;
        if range.owner!=claim.owner || range.source_item_id!=claim.source_item_id || range.body_sha256!=claim.body_sha256
            || range.revision!=revision || range.offset_bytes!=offset || range.text.len()>32768
            || offset.checked_add(range.text.len() as u64)!=Some(range.next_offset_bytes)
            || range.total_utf8_bytes!=total || range.next_offset_bytes<=offset || range.next_offset_bytes>total
            || range.end_of_content!=(range.next_offset_bytes==total) {
            return Err("archive tool source range changed owner, version or byte bounds".into());
        }
        digest.update(range.text.as_bytes());
        text.push_str(&range.text);
        offset=range.next_offset_bytes;
    }
    if format!("{:x}",digest.finalize())!=claim.body_sha256 {
        return Err("archive tool source original digest mismatch".into());
    }
    Ok(text)
}

fn canonical_message_value(message: &ConversationMessage) -> Result<Value, String> {
    if let Some(value) = message
        .parts
        .first()
        .and_then(|part| part.content_json.as_deref())
        .and_then(|json| serde_json::from_str::<Value>(json).ok())
    {
        return Ok(value);
    }
    Ok(serde_json::json!({
        "role": message.role,
        "content": message.parts.first().map(|part| part.content_text.as_str()).unwrap_or_default()
    }))
}

fn conversation_project_root(conversation: &Conversation) -> String {
    conversation
        .settings_json
        .as_deref()
        .and_then(|settings| serde_json::from_str::<Value>(settings).ok())
        .and_then(|settings| {
            settings
                .get("project_root")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default()
}

fn timestamp_text(timestamp_ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp_ms)
        .map(|date| date.to_rfc3339())
        .unwrap_or_default()
}
