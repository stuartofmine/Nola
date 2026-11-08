// SPDX-License-Identifier: Apache-2.0
//! Ordered domain projection changes emitted after canonical command commits.

use nola_kernel::conversation_projection_diff as projection_diff;
#[cfg(test)]
use nola_kernel::text_part_mutations;

use nola_domain::ConversationProjectionMutation;
#[cfg(test)]
use nola_domain::MessagePartView;
use nola_domain::{
    ConversationId, ConversationListProjectionEvent, ConversationListProjectionEventIdentity,
    ConversationListProjectionMutation, ConversationProjection, ConversationProjectionEvent,
    ConversationProjectionEventIdentity, ConversationSummaryView, ProjectionExecutionOwner,
    RunEventId, RunId,
};
use nola_store_port::protocol_journal::{
    AppendProjectionEventInput, ProjectionJournalRepository, ProjectionScopeKey,
};
use nola_store_port::{
    conversation_history::ConversationProjectionRepository,
    conversation_query::ConversationQueryRepository,
};

use crate::{
    conversation_projection::ConversationProjectionContractError, load_conversation_projection_v1,
    ApplicationCommand, ApplicationRuntime,
};

pub(crate) fn conversation_id_for_command(command: &ApplicationCommand) -> Option<&str> {
    match command {
        ApplicationCommand::SubmitUiInteraction { input } => Some(&input.interaction.identity.owner.conversation_id),
        ApplicationCommand::PreviewUiSurface { input } => Some(&input.identity.owner.conversation_id),
        ApplicationCommand::ReadUiSurface { identity } | ApplicationCommand::ReadUiSurfaceChanges { identity, .. } | ApplicationCommand::ReadUiSurfaceRevision { identity, .. } => Some(&identity.owner.conversation_id),
        ApplicationCommand::ListUiSurfaces { conversation_id } => Some(conversation_id),
        ApplicationCommand::BeginReceiveAssistantRunTerminal { seal, .. }
        | ApplicationCommand::CommitReceivedAssistantRunTerminal { seal, .. } => {
            Some(&seal.identity.conversation_id)
        }
        ApplicationCommand::StageAssistantRunTerminalManifest { page, .. } => {
            Some(&page.seal.identity.conversation_id)
        }
        ApplicationCommand::ReceiveAssistantJournalRecord { record, .. } => {
            Some(&record.header.identity.conversation_id)
        }
        ApplicationCommand::ReceiveRunMessageAdmissions { page, .. } => {
            Some(&page.through.identity.conversation_id)
        }
        ApplicationCommand::AcceptRemoteRunContextUsage { record, .. } => Some(&record.identity.conversation_id),
        ApplicationCommand::BeginAssistantTurn { input } => Some(&input.conversation_id),
        ApplicationCommand::PromoteQueuedAssistantTurn { input } => Some(&input.conversation_id),
        ApplicationCommand::BeginQueuedUserTurn { input } => Some(&input.conversation_id),
        ApplicationCommand::BeginTaskAssistantTurn { input, .. } => Some(&input.conversation_id),
        ApplicationCommand::BeginAutomationAssistantTurn { input, .. } => Some(&input.conversation_id),
        ApplicationCommand::RequestRunCancellation { input } => Some(&input.conversation_id),
        ApplicationCommand::ResolveUnknownEffectRecovery { input } => Some(&input.conversation_id),
        ApplicationCommand::ClaimCancellationHostRequests { input } => {
            Some(&input.identity.conversation_id)
        }
        ApplicationCommand::ReduceAndAcceptHostResponse { input } => Some(&input.conversation_id),
        ApplicationCommand::ReduceAndCommitActiveSubmission { input } => {
            Some(&input.conversation_id)
        }
        ApplicationCommand::ReduceAndCommitIdleSubmission { input } => Some(&input.conversation_id),
        ApplicationCommand::RevertConversation { input } => Some(&input.conversation_id),
        ApplicationCommand::CreateConversationBranch { input } => Some(&input.conversation.id),
        ApplicationCommand::PrepareRemoteSubmission { input } => Some(&input.conversation_id),
        ApplicationCommand::AbandonPreparedRemoteSubmission { identity } => {
            Some(&identity.conversation_id)
        }
        ApplicationCommand::AdoptObservedRemoteRun { receipt } => {
            Some(&receipt.identity.conversation_id)
        }
        ApplicationCommand::BindRemoteSubmissionRequest { identity, .. } => {
            Some(&identity.conversation_id)
        }
        ApplicationCommand::AcceptRemoteSubmissionReceipt { receipt } => {
            Some(&receipt.identity.conversation_id)
        }
        ApplicationCommand::CreateConversation { input } => Some(&input.id),
        ApplicationCommand::AdmitConversationUserMessage { input } => Some(&input.conversation_id),
        ApplicationCommand::ImportConversationTranscript { input } => Some(&input.conversation_id),
        ApplicationCommand::PublishUploadedTranscript { input } => {
            Some(&input.transcript.conversation_id)
        }
        ApplicationCommand::EditConversationMessage { input } => Some(&input.conversation_id),
        ApplicationCommand::DeleteConversationMessage { input } => Some(&input.conversation_id),
        ApplicationCommand::Context { command } => command.conversation_id(),
        ApplicationCommand::PatchConversationMetadata { input } => Some(&input.conversation_id),
        ApplicationCommand::PatchConversationConfiguration { input } => Some(&input.conversation_id),
        ApplicationCommand::SelectConversationExecution { input } => Some(&input.conversation_id),
        ApplicationCommand::SetConversationPermission { conversation_id, .. } => Some(conversation_id),
        ApplicationCommand::ApplyActivePermissionPolicy { conversation_id, .. } => Some(conversation_id),
        ApplicationCommand::RenameConversation {
            conversation_id, ..
        }
        | ApplicationCommand::DeleteConversation { conversation_id } => Some(conversation_id),
        _ => None,
    }
}

impl<R> ApplicationRuntime<R>
where
    R: ConversationProjectionRepository + ConversationQueryRepository + ProjectionJournalRepository,
{
    pub(crate) fn journal_conversation_invalidation(
        &mut self,
        client_id: &str,
        request_id: &str,
        conversation_id: &str,
        occurred_at: &str,
    ) -> Result<(), String> {
        let summary = self
            .load_conversation_summary_view(conversation_id)?
            .ok_or("conversation disappeared before projection invalidation")?;
        let scope = ProjectionScopeKey {
            scope_type: "conversation".into(),
            scope_id: conversation_id.into(),
        };
        let sequence = self
            .store
            .projection_cursor(&scope)
            .map_err(|e| e.to_string())?
            .map(|cursor| cursor.current_sequence)
            .unwrap_or(0);
        let event = ConversationProjectionEvent {
            identity: ConversationProjectionEventIdentity {
                event_id: RunEventId(scoped_projection_event_id(
                    "projection",
                    client_id,
                    request_id,
                    conversation_id,
                )),
                conversation_id: ConversationId(conversation_id.into()),
                run_id: None,
                attempt_id: None,
                delegated_invocation_id: None,
                sequence: sequence
                    .checked_add(1)
                    .ok_or("projection sequence overflow")?,
            },
            mutation: ConversationProjectionMutation::InvalidateProjection {
                revision: summary.revision,
            },
        };
        self.store
            .append_projection_event(&AppendProjectionEventInput {
                event_id: event.identity.event_id.0.clone(),
                scope,
                expected_after_sequence: sequence,
                payload_json: serde_json::to_string(&event).map_err(|e| e.to_string())?,
                occurred_at: occurred_at.into(),
            })
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub(crate) fn load_conversation_summary_view(
        &self,
        conversation_id: &str,
    ) -> Result<Option<ConversationSummaryView>, String> {
        self.store
            .conversation_summary(conversation_id)
            .map_err(|error| format!("load conversation list summary failed: {error}"))?
            .map(|conversation| {
                Ok(ConversationSummaryView {
                    conversation_id: ConversationId(conversation.id),
                    title: conversation.title,
                    revision: u64::try_from(conversation.revision)
                        .map_err(|_| "conversation summary revision is negative".to_string())?,
                    message_count: u64::try_from(conversation.message_count).map_err(|_| {
                        "conversation summary message count is negative".to_string()
                    })?,
                    created_at_ms: conversation.created_at_ms,
                    updated_at_ms: conversation.updated_at_ms,
                    project_root: conversation.project_root,
                    first_user_message: conversation.first_user_message,
                    last_user_message_at_ms: conversation.last_user_message_at_ms,
                })
            })
            .transpose()
    }

    pub(crate) fn journal_conversation_list_change(
        &mut self,
        client_id: &str,
        request_id: &str,
        conversation_id: &str,
        before: Option<&ConversationSummaryView>,
        occurred_at: &str,
    ) -> Result<(), String> {
        let after = self.load_conversation_summary_view(conversation_id)?;
        if before == after.as_ref() {
            return Ok(());
        }
        let scope = ProjectionScopeKey {
            scope_type: "conversations".to_string(),
            scope_id: String::new(),
        };
        let sequence = self
            .store
            .projection_cursor(&scope)
            .map_err(|error| format!("load conversation list journal cursor failed: {error}"))?
            .map(|cursor| cursor.current_sequence)
            .unwrap_or(0);
        let mutation = match after {
            Some(conversation) => ConversationListProjectionMutation::Upsert { conversation },
            None => ConversationListProjectionMutation::Remove {
                conversation_id: ConversationId(conversation_id.to_string()),
                revision: before
                    .map(|value| value.revision.saturating_add(1))
                    .unwrap_or(1),
            },
        };
        let event = ConversationListProjectionEvent {
            identity: ConversationListProjectionEventIdentity {
                event_id: RunEventId(scoped_projection_event_id(
                    "projection-list",
                    client_id,
                    request_id,
                    conversation_id,
                )),
                conversation_id: ConversationId(conversation_id.to_string()),
                sequence: sequence
                    .checked_add(1)
                    .ok_or_else(|| "conversation list event sequence overflow".to_string())?,
            },
            mutation,
        };
        self.store
            .append_projection_event(&AppendProjectionEventInput {
                event_id: event.identity.event_id.0.clone(),
                scope,
                expected_after_sequence: sequence,
                payload_json: serde_json::to_string(&event).map_err(|error| {
                    format!("encode conversation list projection event failed: {error}")
                })?,
                occurred_at: occurred_at.to_string(),
            })
            .map_err(|error| {
                format!("append conversation list projection event failed: {error}")
            })?;
        Ok(())
    }

    pub(crate) fn load_optional_conversation_projection(
        &self,
        conversation_id: &str,
    ) -> Result<Option<ConversationProjection>, String> {
        match load_conversation_projection_v1(&self.store, conversation_id) {
            Ok(projection) => Ok(Some(projection)),
            Err(ConversationProjectionContractError::MissingConversation(_)) => Ok(None),
            Err(error) => Err(format!(
                "load canonical projection before command failed: {error}"
            )),
        }
    }

    pub(crate) fn journal_conversation_projection_change(
        &mut self,
        client_id: &str,
        request_id: &str,
        conversation_id: &str,
        before: Option<&ConversationProjection>,
        stream_identity: Option<&(RunId, ProjectionExecutionOwner)>,
        occurred_at: &str,
    ) -> Result<(), String> {
        let after = match load_conversation_projection_v1(&self.store, conversation_id) {
            Ok(projection) => projection,
            Err(ConversationProjectionContractError::MissingConversation(_)) => return Ok(()),
            Err(error) => {
                return Err(format!(
                    "load canonical projection after command failed: {error}"
                ))
            }
        };
        append_projection_change(
            &mut self.store,
            client_id,
            request_id,
            conversation_id,
            before,
            &after,
            stream_identity,
            occurred_at,
        )
    }
}

/// Application computes the diff; the port enforces cursor CAS in the same
/// command transaction. This use case does not select a SQLite implementation.
fn append_projection_change<J: ProjectionJournalRepository + ?Sized>(
    journal: &mut J,
    client_id: &str,
    request_id: &str,
    conversation_id: &str,
    before: Option<&ConversationProjection>,
    after: &ConversationProjection,
    stream_identity: Option<&(RunId, ProjectionExecutionOwner)>,
    occurred_at: &str,
) -> Result<(), String> {
    let mutations = projection_diff(before, after);
    if mutations.is_empty() {
        return Ok(());
    }
    let scope = ProjectionScopeKey {
        scope_type: "conversation".to_string(),
        scope_id: conversation_id.to_string(),
    };
    let mut sequence = journal
        .projection_cursor(&scope)
        .map_err(|error| format!("load projection journal cursor failed: {error}"))?
        .map(|cursor| cursor.current_sequence)
        .unwrap_or(0);
    let run_id = stream_identity
        .map(|(run_id, _)| run_id.clone())
        .or_else(|| after.active_run.as_ref().map(|run| run.run_id.clone()))
        .or_else(|| {
            before
                .and_then(|projection| projection.active_run.as_ref().map(|run| run.run_id.clone()))
        });
    let attempt_id = stream_identity.and_then(|(_, owner)| match owner {
        ProjectionExecutionOwner::ModelAttempt { attempt_id } => Some(attempt_id.clone()),
        _ => None,
    });
    let delegated_invocation_id = stream_identity.and_then(|(_, owner)| match owner {
        ProjectionExecutionOwner::DelegatedInvocation { invocation_id } => {
            Some(invocation_id.clone())
        }
        _ => None,
    });
    for (index, mutation) in mutations.into_iter().enumerate() {
        let next_sequence = sequence
            .checked_add(1)
            .ok_or_else(|| "projection event sequence overflow".to_string())?;
        let event = ConversationProjectionEvent {
            identity: ConversationProjectionEventIdentity {
                event_id: RunEventId(projection_event_id(
                    "projection",
                    client_id,
                    request_id,
                    index,
                )),
                conversation_id: ConversationId(conversation_id.to_string()),
                run_id: run_id.clone(),
                attempt_id: attempt_id.clone(),
                delegated_invocation_id: delegated_invocation_id.clone(),
                sequence: next_sequence,
            },
            mutation,
        };
        let payload_json = serde_json::to_string(&event)
            .map_err(|error| format!("encode domain projection event failed: {error}"))?;
        journal
            .append_projection_event(&AppendProjectionEventInput {
                event_id: event.identity.event_id.0.clone(),
                scope: scope.clone(),
                expected_after_sequence: sequence,
                payload_json,
                occurred_at: occurred_at.to_string(),
            })
            .map_err(|error| format!("append projection event failed: {error}"))?;
        sequence = next_sequence;
    }
    Ok(())
}

// Length-prefixed components preserve the receipt's tuple identity even when
// callers use delimiters inside their opaque IDs. No hash or normalization is needed.
fn projection_event_id(prefix: &str, client_id: &str, request_id: &str, index: usize) -> String {
    format!(
        "{prefix}:{}:{client_id}:{}:{request_id}:{index}",
        client_id.len(),
        request_id.len()
    )
}

fn scoped_projection_event_id(
    prefix: &str,
    client_id: &str,
    request_id: &str,
    conversation_id: &str,
) -> String {
    format!(
        "scoped-{}:{}:{conversation_id}",
        projection_event_id(prefix, client_id, request_id, 0),
        conversation_id.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_domain::{ContextUsageView, ConversationSurfaceView};

    fn projection(title: &str, revision: u64) -> ConversationProjection {
        ConversationProjection {
            execution_ownership: nola_domain::ConversationExecutionOwnership::User,
            contract_version: 1,
            conversation_id: ConversationId("conversation-1".into()),
            revision,
            title: title.into(),
            messages: Vec::new(),
            active_run: None,
            surface: ConversationSurfaceView {
                revision,
                ..ConversationSurfaceView::default()
            },
            context_usage: ContextUsageView {
                measurement: None,
                prompt_cache_measurement: None,
                prompt_cache_diagnostic: None,
                conversation_cache_usage: None,
                cache_usage: None,
                used_tokens: 0,
                context_window_tokens: 65_536,
                prompt_token_limit: 61_440,
                estimated: true,
                measured_at_revision: revision,
            },
        }
    }

    struct MemoryJournal {
        sequence: u64,
        rejected: bool,
        events: Vec<nola_store_port::protocol_journal::StoredProjectionEvent>,
    }
    impl ProjectionJournalRepository for MemoryJournal {
        type Error = &'static str;
        fn projection_cursor(
            &self,
            scope: &ProjectionScopeKey,
        ) -> Result<Option<nola_store_port::protocol_journal::StoredProjectionCursor>, Self::Error>
        {
            Ok(Some(
                nola_store_port::protocol_journal::StoredProjectionCursor {
                    scope: scope.clone(),
                    current_sequence: self.sequence,
                    revision: 1,
                    updated_at: "now".into(),
                },
            ))
        }
        fn append_projection_event(
            &mut self,
            input: &AppendProjectionEventInput,
        ) -> Result<nola_store_port::protocol_journal::StoredProjectionEvent, Self::Error> {
            if self.rejected {
                return Err("journal write rejected");
            }
            assert_eq!(input.expected_after_sequence, self.sequence);
            let event: ConversationProjectionEvent =
                serde_json::from_str(&input.payload_json).unwrap();
            assert_eq!(event.identity.sequence, self.sequence + 1);
            assert_eq!(event.identity.conversation_id.0, input.scope.scope_id);
            self.sequence += 1;
            let stored = nola_store_port::protocol_journal::StoredProjectionEvent {
                event_id: input.event_id.clone(),
                scope: input.scope.clone(),
                sequence: self.sequence,
                payload_json: input.payload_json.clone(),
                occurred_at: input.occurred_at.clone(),
            };
            self.events.push(stored.clone());
            Ok(stored)
        }
        fn projection_events_after(
            &self,
            _: &ProjectionScopeKey,
            _: u64,
            _: usize,
        ) -> Result<Vec<nola_store_port::protocol_journal::StoredProjectionEvent>, Self::Error>
        {
            panic!("append use case must not read full journal history")
        }
    }

    #[test]
    fn non_sqlite_projection_journal_preserves_order_and_propagates_failed_append() {
        let before = projection("Before", 1);
        let after = projection("After", 2);
        let mut journal = MemoryJournal {
            sequence: 41,
            rejected: true,
            events: Vec::new(),
        };
        let error = append_projection_change(
            &mut journal,
            "client",
            "request",
            "conversation-1",
            Some(&before),
            &after,
            None,
            "now",
        )
        .unwrap_err();
        assert!(error.contains("journal write rejected"));
        assert_eq!(journal.sequence, 41);
        assert!(journal.events.is_empty());
        journal.rejected = false;
        append_projection_change(
            &mut journal,
            "client",
            "request",
            "conversation-1",
            Some(&before),
            &after,
            None,
            "now",
        )
        .unwrap();
        assert!(!journal.events.is_empty());
        assert_eq!(journal.sequence, 41 + journal.events.len() as u64);
    }

    #[test]
    fn projection_diff_emits_metadata_without_appending_a_transcript() {
        let before = projection("Before", 1);
        let after = projection("After", 2);
        let changes = projection_diff(Some(&before), &after);
        assert!(changes.iter().any(|change| matches!(
            change,
            ConversationProjectionMutation::SetTitle { title, revision: 2 }
                if title == "After"
        )));
        assert!(!changes
            .iter()
            .any(|change| matches!(change, ConversationProjectionMutation::UpsertMessage { .. })));
    }

    #[test]
    fn targeted_summary_matches_sidebar_after_database_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("summary.db");
        let mut runtime = ApplicationRuntime::open(&path).unwrap();
        for (index, id) in ["first", "quoted'id", "last"].into_iter().enumerate() {
            runtime
                .store
                .create_conversation(&nola_store::CreateConversationInput {
                    id: id.into(),
                    title: format!("Conversation {index}"),
                    revision: 1,
                    settings_json: Some(r#"{"project_root":"/tmp/workspace"}"#.into()),
                    created_at_ms: index as i64 + 1,
                    updated_at_ms: index as i64 + 1,
                })
                .unwrap();
        }
        drop(runtime);
        let runtime = ApplicationRuntime::open(&path).unwrap();
        let summaries = runtime.store.list_conversations().unwrap();
        assert_eq!(summaries.len(), 3);
        for summary in summaries {
            let targeted = runtime
                .store
                .conversation_summary(&summary.id)
                .unwrap()
                .unwrap();
            assert_eq!(
                serde_json::to_value(&targeted).unwrap(),
                serde_json::to_value(&summary).unwrap()
            );
            let projected = runtime
                .load_conversation_summary_view(&summary.id)
                .unwrap()
                .unwrap();
            assert_eq!(projected.conversation_id.0, summary.id);
            assert_eq!(projected.project_root.as_deref(), Some("/tmp/workspace"));
        }
        assert!(runtime
            .load_conversation_summary_view("missing")
            .unwrap()
            .is_none());
        assert!(runtime
            .load_conversation_summary_view("' OR 1=1 --")
            .unwrap()
            .is_none());
    }
}

#[cfg(test)]
mod text_stream_projection_tests {
    use super::*;
    use nola_domain::{MessageId, MessagePartId, MessageRole, MessageState, MessageView};

    fn message(markdown: &str, revision: u64, is_final: bool) -> MessageView {
        MessageView {
            message_id: MessageId("assistant-1".into()),
            role: MessageRole::Assistant,
            state: MessageState::Running,
            revision,
            created_at_ms: 1_000,
            parts: vec![MessagePartView::Text {
                part_id: MessagePartId("assistant-1:text".into()),
                revision,
                markdown: markdown.into(),
                is_final,
                review: None,
            }],
        }
    }

    #[test]
    fn growing_text_uses_a_compact_preview_delta() {
        let mutations = text_part_mutations(
            &message("Hello", 1, false),
            &message("Hello world", 2, false),
        )
        .expect("text-only growth should be compact");
        assert_eq!(mutations.len(), 1);
        assert!(matches!(
            &mutations[0],
            ConversationProjectionMutation::AppendTextPreview {
                message_id,
                part_id,
                revision: 2,
                delta,
            } if message_id.0 == "assistant-1"
                && part_id.0 == "assistant-1:text"
                && delta == " world"
        ));
    }

    #[test]
    fn completed_text_replaces_the_preview_with_the_full_document() {
        let mutations = text_part_mutations(
            &message("draft", 2, false),
            &message("canonical answer", 3, true),
        )
        .expect("completion should be a typed text mutation");
        assert!(matches!(
            &mutations[0],
            ConversationProjectionMutation::CompleteTextPart {
                revision: 3,
                markdown,
                ..
            } if markdown == "canonical answer"
        ));
    }

    #[test]
    fn completed_text_is_not_republished_for_an_unrelated_revision() {
        let mutations = text_part_mutations(
            &message("canonical answer", 3, true),
            &message("canonical answer", 4, true),
        )
        .expect("revision-only text changes remain representable");
        assert!(mutations.is_empty());
    }

    #[test]
    fn message_state_changes_fall_back_to_an_authoritative_upsert() {
        let before = message("answer", 3, false);
        let mut after = message("answer", 4, true);
        after.state = MessageState::Completed;
        assert!(text_part_mutations(&before, &after).is_none());
    }

    #[test]
    fn ten_million_character_preview_emits_only_the_new_suffix() {
        let text = "a".repeat(10_000_000);
        let before = message(&text, 1, false);
        let after = message(&(text + "\nnext"), 2, false);
        let mutations = text_part_mutations(&before, &after).unwrap();
        assert_eq!(mutations.len(), 1);
        assert!(matches!(&mutations[0],
            ConversationProjectionMutation::AppendTextPreview { delta, .. }
            if delta == "\nnext"
        ));
        assert!(serde_json::to_vec(&mutations).unwrap().len() < 256);
    }

    #[test]
    fn text_growth_must_not_hide_a_changed_message_identity_or_timestamp() {
        let before = message("draft", 1, false);
        let mut after = message("draft more", 2, false);
        after.message_id = MessageId("another-message".into());
        assert!(text_part_mutations(&before, &after).is_none());
        after.message_id = before.message_id.clone();
        after.created_at_ms += 1;
        assert!(text_part_mutations(&before, &after).is_none());
    }
}
