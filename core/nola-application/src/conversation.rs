// SPDX-License-Identifier: Apache-2.0
//! Conversation use cases and invariants.
//!
//! Store DTOs are deliberately constructed here. Clients express product
//! intent and cannot choose database revisions or bypass active-Run rules.

use crate::transcript_input::FinalizedTranscriptInput;
#[cfg(test)]
use nola_protocol::ConversationTranscriptImportSource;
use nola_protocol::{ImportConversationTranscript, TranscriptMessageInput as MessageInput};
use nola_store_port::conversation_profile::{
    ConversationProfileRepository, CreateConversationInput,
};
use nola_store_port::conversation_revert::{
    ApplyConversationProjectionInput, MessageProjectionMutation,
};
use nola_store_port::transcript_write::TranscriptEditRepository;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ConversationCreation, ConversationProjectionChange, TranscriptImportResult};
use nola_store_port::transcript_write::{AppendMessagesInput, TranscriptWriteRepository};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CreateConversation {
    pub id: String,
    pub title: String,
    pub settings_json: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EditConversationMessage {
    pub operation_id: String,
    pub conversation_id: String,
    pub expected_revision: i64,
    pub parent_message_id: String,
    pub replacement_message: MessageInput,
    pub committed_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeleteConversationMessage {
    pub operation_id: String,
    pub conversation_id: String,
    pub expected_revision: i64,
    pub message_id: String,
    pub committed_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalUserMessageAdmission {
    pub conversation_id: String,
    pub message_id: String,
    pub message_count: usize,
    pub conversation_revision: i64,
    pub already_admitted: bool,
}

/// Admit one canonical user message without creating an assistant Run.
/// The surrounding command receipt transaction makes the client request ID
/// idempotent; this message identity also protects duplicate retries made with
/// a new transport request ID.
pub fn admit_user_message<R: TranscriptWriteRepository + ?Sized>(
    store: &mut R,
    conversation: &nola_store_port::conversation_history::StoredConversation,
    command: nola_protocol::AdmitConversationUserMessage,
) -> Result<CanonicalUserMessageAdmission, String> {
    if command.conversation_id != conversation.id {
        return Err("canonical_message_conversation_mismatch".into());
    }
    let id = command.message_id.trim();
    if id.is_empty() || id != command.message_id || id.len() > 128 || id.chars().any(char::is_control) {
        return Err("invalid canonical user message id".into());
    }
    if command.text.trim().is_empty() || command.text.len() > 1_000_000 || command.text.contains('\0') {
        return Err("canonical user message text is empty or too large".into());
    }
    if command.created_at_ms < 0 {
        return Err("canonical user message timestamp is invalid".into());
    }
    if let Some(existing) = conversation.messages.iter().find(|message| message.id == command.message_id) {
        let exact = existing.role == "user"
            && existing.run_id.is_none()
            && existing.status == "completed"
            && existing.parts.len() == 1
            && existing.parts[0].kind == "text"
            && existing.parts[0].content_text == command.text
            && existing.parts[0].created_at_ms == command.created_at_ms
            && existing.parts[0].updated_at_ms == command.created_at_ms
            && existing.created_at_ms == command.created_at_ms;
        if !exact {
            return Err("canonical_user_message_idempotency_conflict".into());
        }
        return Ok(CanonicalUserMessageAdmission {
            conversation_id: conversation.id.clone(),
            message_id: command.message_id,
            message_count: conversation.messages.len(),
            conversation_revision: conversation.revision,
            already_admitted: true,
        });
    }
    let ordinal = conversation.messages.last().map_or(Some(1), |message| message.ordinal.checked_add(1))
        .ok_or("canonical_message_ordinal_overflow")?;
    let input = AppendMessagesInput {
        conversation_id: command.conversation_id.clone(),
        expected_message_count: conversation.messages.len(),
        messages: vec![nola_store_port::transcript_write::MessageInput {
            id: command.message_id.clone(),
            run_id: None,
            role: "user".into(),
            status: "completed".into(),
            ordinal,
            created_at_ms: command.created_at_ms,
            completed_at_ms: Some(command.created_at_ms),
            parts: vec![nola_domain::MessagePartInput {
                id: format!("{}:text", command.message_id),
                kind: "text".into(),
                ordinal: 1,
                state: "completed".into(),
                content_text: command.text,
                content_json: None,
                content_digest: None,
                created_at_ms: command.created_at_ms,
                updated_at_ms: command.created_at_ms,
            }],
        }],
        updated_at_ms: command.created_at_ms,
    };
    let result = store.append_messages(&input).map_err(|error| error.to_string())?;
    Ok(CanonicalUserMessageAdmission {
        conversation_id: command.conversation_id,
        message_id: command.message_id,
        message_count: result.message_count,
        conversation_revision: result.conversation_revision,
        already_admitted: false,
    })
}

#[cfg(test)]
mod canonical_user_admission_tests {
    use super::*;
    use nola_store_port::conversation_history::{StoredConversation, StoredMessage, StoredPart};
    use nola_store_port::transcript_write::{AppendMessagesResult, MessageInput};

    #[derive(Default)]
    struct Writer { messages: Vec<MessageInput>, appends: usize }
    impl TranscriptWriteRepository for Writer {
        type Error = String;
        fn has_active_run(&self, _: &str) -> Result<bool, Self::Error> { Ok(false) }
        fn conversation_id_for_run(&self, _: &str) -> Result<Option<String>, Self::Error> { Ok(None) }
        fn append_messages(&mut self, input: &AppendMessagesInput) -> Result<AppendMessagesResult, Self::Error> {
            if input.expected_message_count != self.messages.len() { return Err("message_count_conflict".into()); }
            self.appends += 1;
            self.messages.extend(input.messages.clone());
            Ok(AppendMessagesResult { appended_message_count: input.messages.len(), message_count: self.messages.len(), conversation_revision: 2 })
        }
    }
    fn snapshot(writer: &Writer) -> StoredConversation {
        StoredConversation { id: "conversation".into(), title: "Chat".into(), revision: if writer.messages.is_empty() { 1 } else { 2 },
            configuration_revision: 1, created_at_ms: 1, updated_at_ms: 1, settings_json: None,
            messages: writer.messages.iter().map(|message| StoredMessage { id: message.id.clone(), run_id: message.run_id.clone(),
                role: message.role.clone(), status: message.status.clone(), ordinal: message.ordinal,
                created_at_ms: message.created_at_ms, completed_at_ms: message.completed_at_ms,
                parts: message.parts.iter().map(|part| StoredPart { id: part.id.clone(), kind: part.kind.clone(), ordinal: part.ordinal,
                    state: part.state.clone(), content_text: part.content_text.clone(), content_json: part.content_json.clone(),
                    content_json_deferred: false, content_digest: part.content_digest.clone(), created_at_ms: part.created_at_ms,
                    updated_at_ms: part.updated_at_ms }).collect() }).collect() }
    }
    fn command() -> nola_protocol::AdmitConversationUserMessage {
        nola_protocol::AdmitConversationUserMessage { conversation_id: "conversation".into(), message_id: "stable-client-message".into(),
            text: "Ask the two experts".into(), created_at_ms: 12 }
    }
    #[test]
    fn canonical_admission_is_visible_and_same_identity_retry_is_idempotent() {
        let mut writer = Writer::default();
        let initial = snapshot(&writer);
        let first = admit_user_message(&mut writer, &initial, command()).unwrap();
        assert!(!first.already_admitted);
        assert_eq!(writer.appends, 1);
        let current = snapshot(&writer);
        let second = admit_user_message(&mut writer, &current, command()).unwrap();
        assert!(second.already_admitted);
        assert_eq!(second.message_id, first.message_id);
        assert_eq!(writer.appends, 1);
        assert_eq!(writer.messages[0].role, "user");
        assert_eq!(writer.messages[0].run_id, None);
        assert_eq!(writer.messages[0].parts[0].content_text, "Ask the two experts");
    }
    #[test]
    fn reusing_message_identity_with_changed_input_conflicts() {
        let mut writer = Writer::default();
        let initial = snapshot(&writer);
        admit_user_message(&mut writer, &initial, command()).unwrap();
        let mut changed = command(); changed.text.push_str(" changed");
        let current = snapshot(&writer);
        let error = admit_user_message(&mut writer, &current, changed).unwrap_err();
        assert_eq!(error, "canonical_user_message_idempotency_conflict");
        assert_eq!(writer.appends, 1);
    }
    #[test]
    fn admission_rejects_foreign_snapshot_and_invalid_text() {
        let mut writer = Writer::default();
        let mut foreign = snapshot(&writer); foreign.id = "other".into();
        assert_eq!(admit_user_message(&mut writer, &foreign, command()).unwrap_err(), "canonical_message_conversation_mismatch");
        let mut invalid = command(); invalid.text = "  ".into();
        let initial = snapshot(&writer);
        assert!(admit_user_message(&mut writer, &initial, invalid).is_err());
        assert_eq!(writer.appends, 0);
    }
}

pub fn create_conversation<R: ConversationProfileRepository + ?Sized>(
    store: &mut R,
    command: CreateConversation,
) -> Result<ConversationCreation, String> {
    if command.updated_at_ms < command.created_at_ms {
        return Err("conversation updated_at_ms cannot precede created_at_ms".to_string());
    }
    if let Some(settings) = command.settings_json.as_deref() {
        validate_settings_object(settings)?;
    }
    store
        .create_conversation(&CreateConversationInput {
            id: command.id,
            title: command.title,
            revision: 1,
            settings_json: command.settings_json,
            created_at_ms: command.created_at_ms,
            updated_at_ms: command.updated_at_ms,
        })
        .map(Into::into)
        .map_err(|error| error.to_string())
}

pub fn rename_conversation<R: ConversationProfileRepository + ?Sized>(
    store: &mut R,
    conversation_id: &str,
    title: &str,
    title_source: Option<nola_domain::conversation_configuration::ConversationTitleSource>,
    updated_at_ms: i64,
) -> Result<bool, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("conversation title must not be empty".to_string());
    }
    store
        .rename_conversation(conversation_id, title,
            Some(title_source.unwrap_or(nola_domain::conversation_configuration::ConversationTitleSource::User)), updated_at_ms)
        .map_err(|error| error.to_string())
}

pub fn delete_conversation<
    R: nola_store_port::conversation_deletion::ConversationDeletionRepository + ?Sized,
>(
    store: &mut R,
    conversation_id: &str,
) -> Result<bool, String> {
    store
        .delete_conversation(conversation_id)
        .map_err(|error| error.to_string())
}

pub fn edit_conversation_message<R: TranscriptEditRepository + ?Sized>(
    store: &mut R,
    command: EditConversationMessage,
) -> Result<ConversationProjectionChange, String> {
    ensure_conversation_is_idle(store, &command.conversation_id)?;
    validate_replacement_message(&command.replacement_message)?;
    validate_known_run_scope(
        store,
        &command.conversation_id,
        std::slice::from_ref(&command.replacement_message),
    )?;
    store
        .apply_conversation_projection(&ApplyConversationProjectionInput {
            operation_id: command.operation_id.clone(),
            conversation_id: command.conversation_id,
            expected_revision: command.expected_revision,
            operation_kind: "edit".to_string(),
            parent_message_id: Some(command.parent_message_id.clone()),
            mutations: vec![MessageProjectionMutation {
                event_id: format!("{}:hide", command.operation_id),
                message_id: command.parent_message_id,
                visible: false,
                reason: "superseded_by_edit".to_string(),
            }],
            appended_messages: vec![command.replacement_message.into_store()],
            created_at_ms: command.committed_at_ms,
        })
        .map(Into::into)
        .map_err(|error| error.to_string())
}

pub fn delete_conversation_message<R: TranscriptEditRepository + ?Sized>(
    store: &mut R,
    command: DeleteConversationMessage,
) -> Result<ConversationProjectionChange, String> {
    ensure_conversation_is_idle(store, &command.conversation_id)?;
    store
        .apply_conversation_projection(&ApplyConversationProjectionInput {
            operation_id: command.operation_id.clone(),
            conversation_id: command.conversation_id,
            expected_revision: command.expected_revision,
            operation_kind: "delete".to_string(),
            parent_message_id: Some(command.message_id.clone()),
            mutations: vec![MessageProjectionMutation {
                event_id: format!("{}:hide", command.operation_id),
                message_id: command.message_id,
                visible: false,
                reason: "user_deleted".to_string(),
            }],
            appended_messages: Vec::new(),
            created_at_ms: command.committed_at_ms,
        })
        .map(Into::into)
        .map_err(|error| error.to_string())
}

pub fn import_conversation_transcript<R: TranscriptWriteRepository + ?Sized>(
    store: &mut R,
    command: ImportConversationTranscript,
) -> Result<TranscriptImportResult, String> {
    ensure_conversation_is_idle(store, &command.conversation_id)?;
    if command.import_id.trim().is_empty() {
        return Err("conversation transcript import_id must not be empty".to_string());
    }
    validate_imported_messages(command.expected_message_count, &command.messages)?;
    validate_known_run_scope(store, &command.conversation_id, &command.messages)?;
    store
        .append_messages(&AppendMessagesInput {
            conversation_id: command.conversation_id,
            expected_message_count: command.expected_message_count,
            messages: command
                .messages
                .into_iter()
                .map(MessageInput::into_store)
                .collect(),
            updated_at_ms: command.imported_at_ms,
        })
        .map(Into::into)
        .map_err(|error| error.to_string())
}

fn ensure_conversation_is_idle<R: TranscriptWriteRepository + ?Sized>(
    store: &R,
    conversation_id: &str,
) -> Result<(), String> {
    if store
        .has_active_run(conversation_id)
        .map_err(|error| error.to_string())?
    {
        return Err("cannot edit conversation history while its Run is active".to_string());
    }
    Ok(())
}

fn validate_known_run_scope<R: TranscriptWriteRepository + ?Sized>(
    store: &R,
    conversation_id: &str,
    messages: &[MessageInput],
) -> Result<(), String> {
    let mut checked = HashSet::new();
    for run_id in messages
        .iter()
        .filter_map(|message| message.run_id.as_deref())
    {
        if !checked.insert(run_id) {
            continue;
        }
        if let Some(owner) = store
            .conversation_id_for_run(run_id)
            .map_err(|error| error.to_string())?
        {
            if owner != conversation_id {
                return Err("transcript Run belongs to another conversation".into());
            }
        }
    }
    Ok(())
}

fn validate_replacement_message(message: &MessageInput) -> Result<(), String> {
    message.validate_finalized()
}

fn validate_imported_messages(
    expected_message_count: usize,
    messages: &[MessageInput],
) -> Result<(), String> {
    if messages.is_empty() {
        return Err("imported transcript must contain at least one message".into());
    }
    expected_message_count
        .checked_add(messages.len())
        .and_then(|count| i64::try_from(count).ok())
        .ok_or_else(|| {
            "imported transcript message count exceeds canonical ordinal range".to_string()
        })?;
    let mut message_ids = HashSet::new();
    let mut part_ids = HashSet::new();
    for (offset, message) in messages.iter().enumerate() {
        validate_replacement_message(message)?;
        if message.id.trim().is_empty() || !message_ids.insert(message.id.as_str()) {
            return Err("imported transcript message IDs must be non-empty and unique".to_string());
        }
        let expected_ordinal = (expected_message_count + offset + 1) as i64;
        if message.ordinal != expected_ordinal {
            return Err(format!(
                "imported transcript ordinal {} must be {}",
                message.ordinal, expected_ordinal
            ));
        }
        for part in &message.parts {
            if part.id.trim().is_empty() || !part_ids.insert(part.id.as_str()) {
                return Err("imported transcript Part IDs must be non-empty and unique".to_string());
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_metadata_patch(patch: &str) -> Result<(), String> {
    let value: Value = serde_json::from_str(patch).map_err(|error| error.to_string())?;
    nola_store_port::conversation_profile::validate_metadata_fields(&value).map_err(str::to_owned)
}

fn validate_settings_object(settings: &str) -> Result<(), String> {
    match serde_json::from_str::<Value>(settings) {
        Ok(Value::Object(settings)) => {
            let mut configuration = serde_json::Map::new();
            for key in ["modelSelectionID", "executionHostRawValue", "executionHostBinding", "clientHostParameters", "clientModelPresentation",
                "memoryEnabledOverride", "retrievalEnabledOverride", "isPinned", "language", "selectedPromptTemplateID", "promptResponseStyle",
                "executionBackend", "externalExecutionSessions", "persistenceEnabled", "dataSourceOverrides", "dataSourceIds", "executionPolicy",
                "promptTemplateSelectionIsExplicit", "contextLimitOverride", "externalExecutionModelIDs", "externalExecutionModelProviderIDs"] {
                if let Some(value) = settings.get(key).filter(|value| !value.is_null()) {
                    configuration.insert(key.into(), value.clone());
                }
            }
            if let Some(permission) = settings.get("permissionModeRawValue") {
                configuration.insert("permissionProfile".into(),permission.clone());
            }
            crate::conversation_configuration::validate_patch(&Value::Object(configuration).to_string())?;
            // Full native snapshots and sparse Web patches share the model
            // override contract. Unrelated presentation metadata stays opaque.
            if let Some(overrides) = settings.get("modelOverrides") {
                crate::conversation_configuration::validate_patch(
                    &serde_json::json!({"modelOverrides": overrides}).to_string(),
                )?;
                if overrides.as_object().is_some_and(|models| {
                    models
                        .values()
                        .any(|model| model.get("seed").is_some_and(Value::is_null))
                }) {
                    return Err("persisted seed inheritance must omit the field; null is only a patch reset".into());
                }
            }
            Ok(())
        }
        Ok(_) => Err("conversation settings must be a JSON object".to_string()),
        Err(error) => Err(format!("conversation settings are invalid JSON: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_cannot_write_or_clear_model_identity_but_retains_valid_existing_clients() {
        for host in [serde_json::json!("local"), serde_json::Value::Null] {
            assert!(validate_metadata_patch(&serde_json::json!({"executionHostRawValue":host}).to_string()).is_err());
        }
        for selection in [serde_json::json!("configured:model"), Value::Null] {
            assert!(validate_metadata_patch(&serde_json::json!({"modelSelectionID":selection}).to_string()).is_err());
        }
        assert!(validate_metadata_patch(&serde_json::json!({"modelOverrides":{"configured:model":{"temperature":0.2}},
            "clientModelPresentation":{"configured:model":{"streamOutput":false}},"client_note":"ok"}).to_string()).is_err());
        assert!(validate_metadata_patch(&serde_json::json!({"clientNote":"ok","isPinned":true}).to_string()).is_ok());
        assert!(validate_settings_object(&serde_json::json!({"modelSelectionID":"not-scoped"}).to_string()).is_err());
        assert!(validate_settings_object(&serde_json::json!({"clientModelPresentation":{"configured:model":{"showThinking":1}}}).to_string()).is_err());
    }
    use nola_protocol::TranscriptPartInput as MessagePartInput;

    #[test]
    fn deletion_uses_atomic_repository_outcome_without_loading_a_run() {
        struct Repository(Result<bool, &'static str>);
        impl nola_store_port::conversation_deletion::ConversationDeletionRepository for Repository {
            type Error = &'static str;
            fn delete_conversation(&mut self, id: &str) -> Result<bool, Self::Error> {
                assert_eq!(id, "conversation");
                self.0
            }
        }
        for outcome in [
            Ok(true),
            Ok(false),
            Err("active Run"),
            Err("publication failed"),
        ] {
            assert_eq!(
                delete_conversation(&mut Repository(outcome), "conversation"),
                outcome.map_err(str::to_string)
            );
        }
    }

    fn completed_message(id: &str, ordinal: i64) -> MessageInput {
        MessageInput {
            id: id.to_string(),
            run_id: None,
            role: "user".to_string(),
            status: "completed".to_string(),
            ordinal,
            created_at_ms: ordinal,
            completed_at_ms: Some(ordinal),
            parts: vec![MessagePartInput {
                id: format!("{id}:text"),
                kind: "text".to_string(),
                ordinal: 1,
                state: "completed".to_string(),
                content_text: id.to_string(),
                content_json: None,
                content_digest: None,
                created_at_ms: ordinal,
                updated_at_ms: ordinal,
            }],
        }
    }

    struct ImportRepository {
        active: bool,
        run_owner: Option<String>,
        writes: usize,
        conflict: bool,
    }

    impl TranscriptWriteRepository for ImportRepository {
        type Error = &'static str;
        fn has_active_run(&self, _: &str) -> Result<bool, Self::Error> {
            Ok(self.active)
        }
        fn conversation_id_for_run(&self, _: &str) -> Result<Option<String>, Self::Error> {
            Ok(self.run_owner.clone())
        }
        fn append_messages(
            &mut self,
            input: &AppendMessagesInput,
        ) -> Result<nola_store_port::transcript_write::AppendMessagesResult, Self::Error> {
            self.writes += 1;
            if self.conflict {
                return Err("message count conflict");
            }
            assert_eq!(input.expected_message_count, 0);
            assert_eq!(input.messages[0].parts[0].content_text, "message");
            Ok(nola_store_port::transcript_write::AppendMessagesResult {
                appended_message_count: 1,
                message_count: 1,
                conversation_revision: 2,
            })
        }
    }

    #[test]
    fn transcript_import_admission_and_commit_use_a_non_sqlite_repository() {
        let mut message = completed_message("message", 1);
        message.run_id = Some("run".into());
        let command = ImportConversationTranscript {
            import_id: "import".into(),
            source: ConversationTranscriptImportSource::BackupRestore,
            conversation_id: "conversation".into(),
            expected_message_count: 0,
            imported_at_ms: 2,
            messages: vec![message],
        };
        let mut repository = ImportRepository {
            active: true,
            run_owner: None,
            writes: 0,
            conflict: false,
        };
        assert!(import_conversation_transcript(&mut repository, command.clone()).is_err());
        assert_eq!(repository.writes, 0);
        repository.active = false;
        repository.run_owner = Some("foreign".into());
        assert!(import_conversation_transcript(&mut repository, command.clone()).is_err());
        assert_eq!(repository.writes, 0);
        repository.run_owner = Some("conversation".into());
        repository.conflict = true;
        assert_eq!(
            import_conversation_transcript(&mut repository, command.clone()).unwrap_err(),
            "message count conflict"
        );
        repository.conflict = false;
        let result = import_conversation_transcript(&mut repository, command).unwrap();
        assert_eq!(result.message_count, 1);
        assert_eq!(result.conversation_revision, 2);
    }

    #[test]
    fn transcript_import_rejects_overflow_without_panicking() {
        let message = completed_message("message", 1);
        assert!(
            validate_imported_messages(usize::MAX, std::slice::from_ref(&message))
                .unwrap_err()
                .contains("ordinal range")
        );
        if let Ok(maximum) = usize::try_from(i64::MAX) {
            assert!(
                validate_imported_messages(maximum, std::slice::from_ref(&message))
                    .unwrap_err()
                    .contains("ordinal range")
            );
        }
    }

    #[test]
    fn transcript_import_source_uses_shared_backup_contract() {
        let source: ConversationTranscriptImportSource =
            serde_json::from_str("\"backup_restore\"").expect("shared backup source");
        assert!(matches!(
            source,
            ConversationTranscriptImportSource::BackupRestore
        ));
        assert_eq!(
            serde_json::to_string(&source).unwrap(),
            "\"backup_restore\""
        );
        assert!(serde_json::from_str::<ConversationTranscriptImportSource>(
            "\"canonical_backup_restore\""
        )
        .is_err());
    }

    #[test]
    fn transcript_import_requires_contiguous_canonical_ordinals() {
        let error = validate_imported_messages(2, &[completed_message("message-3", 4)])
            .expect_err("a client must not invent a non-contiguous projection");
        assert!(error.contains("must be 3"), "{error}");
    }

    #[test]
    fn transcript_import_rejects_duplicate_part_identity() {
        let first = completed_message("message-1", 1);
        let mut second = completed_message("message-2", 2);
        second.parts[0].id = first.parts[0].id.clone();
        let error = validate_imported_messages(0, &[first, second])
            .expect_err("Part identity must be globally unique inside one import");
        assert!(error.contains("Part IDs"), "{error}");
    }

    #[test]
    fn transcript_import_rejects_incomplete_projection() {
        let mut message = completed_message("message-1", 1);
        message.status = "streaming".to_string();
        message.completed_at_ms = None;
        let error = validate_imported_messages(0, &[message])
            .expect_err("streaming state belongs to a Run, not transcript import");
        assert!(error.contains("must be completed"), "{error}");
    }
}

#[cfg(test)]
mod edit_port_tests {
    use super::*;
    use nola_store_port::conversation_revert::ApplyConversationProjectionResult;
    use nola_store_port::transcript_write::AppendMessagesResult;
    struct Facts {
        active: bool,
        mutation: Option<ApplyConversationProjectionInput>,
    }
    impl TranscriptWriteRepository for Facts {
        type Error = &'static str;
        fn has_active_run(&self, _: &str) -> Result<bool, Self::Error> {
            Ok(self.active)
        }
        fn conversation_id_for_run(&self, _: &str) -> Result<Option<String>, Self::Error> {
            panic!("delete must not read unrelated Runs")
        }
        fn append_messages(
            &mut self,
            _: &AppendMessagesInput,
        ) -> Result<AppendMessagesResult, Self::Error> {
            panic!("delete must use atomic projection mutation")
        }
    }
    impl TranscriptEditRepository for Facts {
        fn apply_conversation_projection(
            &mut self,
            input: &ApplyConversationProjectionInput,
        ) -> Result<ApplyConversationProjectionResult, Self::Error> {
            self.mutation = Some(input.clone());
            Err("concurrent revision conflict")
        }
    }
    #[test]
    fn delete_checks_idle_then_preserves_revision_identity_and_cas_error() {
        let mut facts = Facts {
            active: true,
            mutation: None,
        };
        let command = || DeleteConversationMessage {
            operation_id: "delete-1".into(),
            conversation_id: "c".into(),
            expected_revision: 42,
            message_id: "m".into(),
            committed_at_ms: 50,
        };
        assert!(delete_conversation_message(&mut facts, command())
            .unwrap_err()
            .contains("Run is active"));
        assert!(facts.mutation.is_none());
        facts.active = false;
        assert!(delete_conversation_message(&mut facts, command())
            .unwrap_err()
            .contains("concurrent revision conflict"));
        let mutation = facts.mutation.unwrap();
        assert_eq!(mutation.expected_revision, 42);
        assert_eq!(mutation.conversation_id, "c");
        assert_eq!(mutation.mutations.len(), 1);
        assert_eq!(mutation.mutations[0].message_id, "m");
        assert!(!mutation.mutations[0].visible);
        assert!(mutation.appended_messages.is_empty());
    }
}
