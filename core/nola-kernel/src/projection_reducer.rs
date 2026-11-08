// SPDX-License-Identifier: Apache-2.0

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::error::Error;
use std::fmt::{Display, Formatter};

use nola_domain::{
    ConversationProjection, MessageId, MessagePartId, MessagePartView, MessageView, RunEventId,
    RunId,
};
pub use nola_domain::{
    ConversationProjectionEvent as ProjectionEvent,
    ConversationProjectionEventIdentity as ProjectionEventIdentity,
    ConversationProjectionMutation as ProjectionMutation, ProjectionExecutionOwner,
    ProjectionStreamOwner,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionApplyOutcome {
    Applied,
    Duplicate,
    Stale,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectionApplyError {
    SnapshotRequired { revision: u64 },
    ConversationMismatch,
    EventIdentityConflict,
    DuplicateMessageIdentity,
    DuplicatePartIdentity,
    BatchTooLarge { limit: usize, received: usize },
    SequenceGap { expected: u64, received: u64 },
    MissingRunIdentity,
    MissingAttemptIdentity,
    RunMismatch,
    AttemptMismatch,
    MessageNotFound,
    PartKindMismatch,
    InvalidPublicSemantics(nola_domain::PublicPartContractError),
}

impl Display for ProjectionApplyError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SnapshotRequired { revision } => write!(
                formatter,
                "projection snapshot required at revision {revision}"
            ),
            Self::ConversationMismatch => {
                formatter.write_str("projection event belongs to another conversation")
            }
            Self::EventIdentityConflict => {
                formatter.write_str("projection event ID was reused with a different identity")
            }
            Self::DuplicateMessageIdentity => {
                formatter.write_str("projection contains duplicate message identities")
            }
            Self::DuplicatePartIdentity => {
                formatter.write_str("projection contains duplicate Part identities")
            }
            Self::BatchTooLarge { limit, received } => {
                write!(
                    formatter,
                    "projection batch has {received} events; maximum is {limit}"
                )
            }
            Self::SequenceGap { expected, received } => {
                write!(
                    formatter,
                    "projection sequence gap: expected {expected}, received {received}"
                )
            }
            Self::MissingRunIdentity => {
                formatter.write_str("projection event is missing its run identity")
            }
            Self::MissingAttemptIdentity => {
                formatter.write_str("streaming projection event is missing its execution identity")
            }
            Self::RunMismatch => {
                formatter.write_str("projection event run identity does not match its payload")
            }
            Self::AttemptMismatch => formatter
                .write_str("a message Part cannot be mutated by different execution owners"),
            Self::MessageNotFound => formatter.write_str("projection message does not exist"),
            Self::PartKindMismatch => {
                formatter.write_str("projection Part identity changed content kind")
            }
            Self::InvalidPublicSemantics(error) => Display::fmt(error, formatter),
        }
    }
}

impl Error for ProjectionApplyError {}

/// Shared idempotent reducer for platform presenter adapters.
///
/// The reducer accepts a strict contiguous event suffix. A duplicate event ID
/// is a no-op. Text completion replaces the preview for the same Part ID and
/// permanently closes that Part to subsequent deltas.
/// A snapshot covers the entire prefix through its supplied sequence. Deliveries
/// in that prefix are no-ops. Only recent suffix identities are retained; older
/// suffix deliveries require the caller to resume from its acknowledged cursor.
#[derive(Clone, Debug)]
pub struct ConversationProjectionReducer {
    projection: ConversationProjection,
    last_sequence: u64,
    snapshot_sequence: u64,
    seen_events: BTreeMap<RunEventId, ProjectionEventIdentity>,
    seen_event_order: VecDeque<RunEventId>,
    part_attempts: BTreeMap<MessagePartId, (RunId, ProjectionExecutionOwner)>,
    completed_text_parts: BTreeSet<MessagePartId>,
}

/// An in-process projection checkpoint, including cursor and stream ownership.
/// Private state prevents callers from pairing a cursor with another projection.
/// This is not a durable transcript or a serialized cross-process checkpoint.
#[derive(Clone, Debug)]
pub struct ConversationProjectionCheckpoint(ConversationProjectionReducer);

impl ConversationProjectionReducer {
    const RECENT_EVENT_LIMIT: usize = 256;
    pub const MAX_BATCH_EVENTS: usize = 256;

    pub fn restore(projection: ConversationProjection, last_sequence: u64) -> Self {
        let completed_text_parts = projection
            .messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .filter_map(|part| match part {
                MessagePartView::Text {
                    part_id,
                    is_final: true,
                    ..
                } => Some(part_id.clone()),
                _ => None,
            })
            .collect();
        Self {
            projection,
            last_sequence,
            snapshot_sequence: last_sequence,
            seen_events: BTreeMap::new(),
            seen_event_order: VecDeque::new(),
            part_attempts: BTreeMap::new(),
            completed_text_parts,
        }
    }

    pub fn projection(&self) -> &ConversationProjection {
        &self.projection
    }

    /// Restores a wire snapshot without allowing live text to lose its owner.
    /// Ownership must come from Application, never inferred by the presenter.
    pub fn restore_owned(
        projection: ConversationProjection,
        last_sequence: u64,
        owners: Vec<ProjectionStreamOwner>,
    ) -> Result<Self, ProjectionApplyError> {
        let mut reducer = Self::restore(projection, last_sequence);
        let mut message_ids = BTreeSet::new();
        let mut part_ids = BTreeSet::new();
        for message in &reducer.projection.messages {
            if !message_ids.insert(message.message_id.clone()) {
                return Err(ProjectionApplyError::DuplicateMessageIdentity);
            }
            for part in &message.parts {
                part.validate_public_semantics(&message.message_id)
                    .map_err(ProjectionApplyError::InvalidPublicSemantics)?;
                if !part_ids.insert(message_part_id(part).clone()) {
                    return Err(ProjectionApplyError::DuplicatePartIdentity);
                }
            }
        }
        for owner in owners {
            let message = reducer
                .projection
                .messages
                .iter()
                .find(|message| message.message_id == owner.message_id)
                .ok_or(ProjectionApplyError::MessageNotFound)?;
            if !message.parts.iter().any(|part| {
                matches!(part,
                MessagePartView::Text { part_id, .. } if part_id == &owner.part_id)
            }) {
                return Err(ProjectionApplyError::PartKindMismatch);
            }
            if owner.run_id.0.trim().is_empty() {
                return Err(ProjectionApplyError::MissingRunIdentity);
            }
            if owner.execution.identity().trim().is_empty() {
                return Err(ProjectionApplyError::MissingAttemptIdentity);
            }
            if reducer
                .part_attempts
                .insert(owner.part_id, (owner.run_id, owner.execution))
                .is_some()
            {
                return Err(ProjectionApplyError::EventIdentityConflict);
            }
        }
        for message in &reducer.projection.messages {
            for part in &message.parts {
                if let MessagePartView::Text {
                    part_id,
                    is_final: false,
                    ..
                } = part
                {
                    if !reducer.part_attempts.contains_key(part_id) {
                        return Err(ProjectionApplyError::MissingAttemptIdentity);
                    }
                }
            }
        }
        Ok(reducer)
    }

    pub fn last_sequence(&self) -> u64 {
        self.last_sequence
    }

    /// Moves a bounded presenter window into a checkpoint without copying text.
    pub fn into_checkpoint(self) -> ConversationProjectionCheckpoint {
        ConversationProjectionCheckpoint(self)
    }

    /// Resumes with duplicate and stream identity tracking intact.
    pub fn resume(checkpoint: ConversationProjectionCheckpoint) -> Self {
        checkpoint.0
    }

    /// Applies one bounded suffix atomically, including cursor and attempt state.
    /// The caller must bound its projection window and decoded payload bytes.
    /// Staging clones that window, not an unbounded canonical transcript.
    pub fn apply_batch(
        &mut self,
        events: Vec<ProjectionEvent>,
    ) -> Result<Vec<ProjectionApplyOutcome>, ProjectionApplyError> {
        if events.len() > Self::MAX_BATCH_EVENTS {
            return Err(ProjectionApplyError::BatchTooLarge {
                limit: Self::MAX_BATCH_EVENTS,
                received: events.len(),
            });
        }
        if events.is_empty() {
            return Ok(Vec::new());
        }
        let mut staged = self.clone();
        let outcomes = events
            .into_iter()
            .map(|event| staged.apply(event))
            .collect::<Result<Vec<_>, _>>()?;
        *self = staged;
        Ok(outcomes)
    }

    pub fn apply(
        &mut self,
        event: ProjectionEvent,
    ) -> Result<ProjectionApplyOutcome, ProjectionApplyError> {
        if event.identity.conversation_id != self.projection.conversation_id {
            return Err(ProjectionApplyError::ConversationMismatch);
        }
        if let Some(previous) = self.seen_events.get(&event.identity.event_id) {
            return if previous == &event.identity {
                Ok(ProjectionApplyOutcome::Duplicate)
            } else {
                Err(ProjectionApplyError::EventIdentityConflict)
            };
        }
        // The authoritative snapshot already includes this prefix. Do not replay
        // its mutations (including invalidations), retain its payloads, or advance
        // the cursor. Scope and retained identity conflicts take precedence above.
        if event.identity.sequence > 0 && event.identity.sequence <= self.snapshot_sequence {
            return Ok(ProjectionApplyOutcome::Duplicate);
        }
        let expected = self.last_sequence.saturating_add(1);
        if event.identity.sequence != expected {
            return Err(ProjectionApplyError::SequenceGap {
                expected,
                received: event.identity.sequence,
            });
        }

        let outcome = self.apply_mutation(&event.identity, event.mutation)?;
        self.last_sequence = event.identity.sequence;
        self.seen_event_order
            .push_back(event.identity.event_id.clone());
        self.seen_events
            .insert(event.identity.event_id.clone(), event.identity);
        while self.seen_event_order.len() > Self::RECENT_EVENT_LIMIT {
            if let Some(expired) = self.seen_event_order.pop_front() {
                self.seen_events.remove(&expired);
            }
        }
        Ok(outcome)
    }

    fn apply_mutation(
        &mut self,
        identity: &ProjectionEventIdentity,
        mutation: ProjectionMutation,
    ) -> Result<ProjectionApplyOutcome, ProjectionApplyError> {
        match mutation {
            ProjectionMutation::InvalidateProjection { revision } => {
                // Do not acknowledge this sequence or mutate an incomplete
                // window. The replacement snapshot supplies its own cursor.
                Err(ProjectionApplyError::SnapshotRequired { revision })
            }
            ProjectionMutation::SetTitle { title, revision } => {
                if revision < self.projection.revision {
                    return Ok(ProjectionApplyOutcome::Stale);
                }
                self.projection.title = title;
                self.projection.revision = revision;
                Ok(ProjectionApplyOutcome::Applied)
            }
            ProjectionMutation::UpsertMessage { message } => {
                for part in &message.parts {
                    part.validate_public_semantics(&message.message_id)
                        .map_err(ProjectionApplyError::InvalidPublicSemantics)?;
                }
                let revision = message.revision;
                let retained_parts: BTreeSet<_> = message
                    .parts
                    .iter()
                    .map(|part| message_part_id(part).clone())
                    .collect();
                let final_parts: Vec<_> = message
                    .parts
                    .iter()
                    .filter_map(|part| match part {
                        MessagePartView::Text {
                            part_id,
                            is_final: true,
                            ..
                        } => Some(part_id.clone()),
                        _ => None,
                    })
                    .collect();
                if let Some(index) = self
                    .projection
                    .messages
                    .iter()
                    .position(|current| current.message_id == message.message_id)
                {
                    if message.revision < self.projection.messages[index].revision {
                        return Ok(ProjectionApplyOutcome::Stale);
                    }
                    let previous = std::mem::replace(&mut self.projection.messages[index], message);
                    for part in &previous.parts {
                        let id = message_part_id(part);
                        if !retained_parts.contains(id) {
                            self.part_attempts.remove(id);
                            self.completed_text_parts.remove(id);
                        }
                    }
                } else {
                    self.projection.messages.push(message);
                    self.projection.messages.sort_by(|left, right| {
                        left.created_at_ms
                            .cmp(&right.created_at_ms)
                            .then_with(|| left.message_id.cmp(&right.message_id))
                    });
                }
                self.completed_text_parts.extend(final_parts);
                self.projection.revision = self.projection.revision.max(revision);
                Ok(ProjectionApplyOutcome::Applied)
            }
            ProjectionMutation::RemoveMessage {
                message_id,
                revision,
            } => {
                let Some(index) = self
                    .projection
                    .messages
                    .iter()
                    .position(|message| message.message_id == message_id)
                else {
                    return Ok(ProjectionApplyOutcome::Stale);
                };
                if revision < self.projection.messages[index].revision {
                    return Ok(ProjectionApplyOutcome::Stale);
                }
                let removed = self.projection.messages.remove(index);
                for part in &removed.parts {
                    let part_id = message_part_id(part);
                    self.part_attempts.remove(part_id);
                    self.completed_text_parts.remove(part_id);
                }
                self.projection.revision = self.projection.revision.max(revision);
                Ok(ProjectionApplyOutcome::Applied)
            }
            ProjectionMutation::AppendTextPreview {
                message_id,
                part_id,
                revision,
                delta,
            } => {
                self.validate_text_target(&message_id, &part_id)?;
                self.validate_text_disclosure(&message_id, &part_id, &delta)?;
                self.bind_stream_identity(identity, &part_id)?;
                if self.completed_text_parts.contains(&part_id) {
                    return Ok(ProjectionApplyOutcome::Stale);
                }
                let message = self.message_mut(&message_id)?;
                match message
                    .parts
                    .iter_mut()
                    .find(|part| message_part_id(part) == &part_id)
                {
                    Some(MessagePartView::Text {
                        revision: current_revision,
                        markdown,
                        is_final,
                        ..
                    }) => {
                        if *is_final || revision <= *current_revision {
                            return Ok(ProjectionApplyOutcome::Stale);
                        }
                        markdown.push_str(&delta);
                        *current_revision = revision;
                    }
                    Some(_) => return Err(ProjectionApplyError::PartKindMismatch),
                    None => message.parts.push(MessagePartView::Text {
                        part_id,
                        revision,
                        markdown: delta,
                        is_final: false,
                        review: None,
                    }),
                }
                message.revision = message.revision.max(revision);
                Ok(ProjectionApplyOutcome::Applied)
            }
            ProjectionMutation::CompleteTextPart {
                message_id,
                part_id,
                revision,
                markdown,
            } => {
                self.validate_text_target(&message_id, &part_id)?;
                self.validate_text_disclosure(&message_id, &part_id, &markdown)?;
                self.bind_stream_identity(identity, &part_id)?;
                let message = self.message_mut(&message_id)?;
                match message
                    .parts
                    .iter_mut()
                    .find(|part| message_part_id(part) == &part_id)
                {
                    Some(MessagePartView::Text {
                        revision: current_revision,
                        markdown: current_markdown,
                        is_final,
                        ..
                    }) => {
                        if revision <= *current_revision && *is_final {
                            return Ok(ProjectionApplyOutcome::Stale);
                        }
                        if revision < *current_revision {
                            return Ok(ProjectionApplyOutcome::Stale);
                        }
                        *current_revision = revision;
                        *current_markdown = markdown;
                        *is_final = true;
                    }
                    Some(_) => return Err(ProjectionApplyError::PartKindMismatch),
                    None => message.parts.push(MessagePartView::Text {
                        part_id: part_id.clone(),
                        revision,
                        markdown,
                        is_final: true,
                        review: None,
                    }),
                }
                message.revision = message.revision.max(revision);
                self.completed_text_parts.insert(part_id);
                Ok(ProjectionApplyOutcome::Applied)
            }
            ProjectionMutation::SetActiveRun { active_run } => {
                let event_run = identity
                    .run_id
                    .as_ref()
                    .ok_or(ProjectionApplyError::MissingRunIdentity)?;
                if active_run
                    .as_ref()
                    .is_some_and(|run| &run.run_id != event_run)
                {
                    return Err(ProjectionApplyError::RunMismatch);
                }
                self.projection.active_run = active_run;
                Ok(ProjectionApplyOutcome::Applied)
            }
            ProjectionMutation::ReplaceSurface { surface } => {
                if surface.revision < self.projection.surface.revision {
                    return Ok(ProjectionApplyOutcome::Stale);
                }
                self.projection.surface = surface;
                self.projection.revision = self
                    .projection
                    .revision
                    .max(self.projection.surface.revision);
                Ok(ProjectionApplyOutcome::Applied)
            }
            ProjectionMutation::SetContextUsage { context_usage } => {
                // The contiguous Application event sequence orders this update.
                // Measurement revisions belong to their execution source and
                // may restart; they are not a cross-source freshness clock.
                self.projection.context_usage = context_usage;
                Ok(ProjectionApplyOutcome::Applied)
            }
        }
    }

    fn validate_text_target(
        &self,
        message_id: &MessageId,
        part_id: &MessagePartId,
    ) -> Result<(), ProjectionApplyError> {
        let message = self
            .projection
            .messages
            .iter()
            .find(|message| &message.message_id == message_id)
            .ok_or(ProjectionApplyError::MessageNotFound)?;
        if message.parts.iter().any(|part| {
            message_part_id(part) == part_id && !matches!(part, MessagePartView::Text { .. })
        }) {
            return Err(ProjectionApplyError::PartKindMismatch);
        }
        Ok(())
    }

    fn validate_text_disclosure(
        &self,
        message_id: &MessageId,
        part_id: &MessagePartId,
        incoming: &str,
    ) -> Result<(), ProjectionApplyError> {
        if incoming.is_empty() {
            return Ok(());
        }
        let withheld = self
            .projection
            .messages
            .iter()
            .find(|message| &message.message_id == message_id)
            .into_iter()
            .flat_map(|message| &message.parts)
            .any(|part| {
                matches!(part, MessagePartView::Text { part_id: id, review: Some(review), .. }
                if id == part_id && *review != nola_domain::TextReviewState::Approved)
            });
        if withheld {
            return Err(ProjectionApplyError::InvalidPublicSemantics(
                nola_domain::PublicPartContractError::WithheldText,
            ));
        }
        Ok(())
    }

    fn bind_stream_identity(
        &mut self,
        identity: &ProjectionEventIdentity,
        part_id: &MessagePartId,
    ) -> Result<(), ProjectionApplyError> {
        let execution = match (&identity.attempt_id, &identity.delegated_invocation_id) {
            (Some(attempt_id), None) => ProjectionExecutionOwner::ModelAttempt {
                attempt_id: attempt_id.clone(),
            },
            (None, Some(invocation_id)) => ProjectionExecutionOwner::DelegatedInvocation {
                invocation_id: invocation_id.clone(),
            },
            (None, None) => return Err(ProjectionApplyError::MissingAttemptIdentity),
            (Some(_), Some(_)) => return Err(ProjectionApplyError::EventIdentityConflict),
        };
        let run_id = identity
            .run_id
            .as_ref()
            .ok_or(ProjectionApplyError::MissingRunIdentity)?;
        if run_id.0.trim().is_empty() {
            return Err(ProjectionApplyError::MissingRunIdentity);
        }
        if execution.identity().trim().is_empty() {
            return Err(ProjectionApplyError::MissingAttemptIdentity);
        }
        match self.part_attempts.get(part_id) {
            Some((bound_run, _)) if bound_run != run_id => Err(ProjectionApplyError::RunMismatch),
            Some((_, bound_attempt)) if bound_attempt != &execution => {
                Err(ProjectionApplyError::AttemptMismatch)
            }
            Some(_) => Ok(()),
            None => {
                self.part_attempts
                    .insert(part_id.clone(), (run_id.clone(), execution));
                Ok(())
            }
        }
    }

    fn message_mut(
        &mut self,
        message_id: &MessageId,
    ) -> Result<&mut MessageView, ProjectionApplyError> {
        self.projection
            .messages
            .iter_mut()
            .find(|message| &message.message_id == message_id)
            .ok_or(ProjectionApplyError::MessageNotFound)
    }
}

fn message_part_id(part: &MessagePartView) -> &MessagePartId {
    match part {
        MessagePartView::UiAction { part_id, .. }
        | MessagePartView::UiSurface { part_id, .. }
        | MessagePartView::Historical { part_id, .. }
        | MessagePartView::Text { part_id, .. }
        | MessagePartView::Reasoning { part_id, .. }
        | MessagePartView::Tool { part_id, .. }
        | MessagePartView::Source { part_id, .. }
        | MessagePartView::Citation { part_id, .. }
        | MessagePartView::Attachment { part_id, .. }
        | MessagePartView::Task { part_id, .. }
        | MessagePartView::Skill { part_id, .. }
        | MessagePartView::ModelAttempt { part_id, .. }
        | MessagePartView::Compaction { part_id, .. }
        | MessagePartView::Error { part_id, .. } => part_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_domain::{
        ContextUsageView, ConversationId, ConversationSurfaceView, MessageRole, MessageState,
        RunAttemptId,
    };

    fn reducer() -> ConversationProjectionReducer {
        ConversationProjectionReducer::restore(
            ConversationProjection {
                execution_ownership: nola_domain::ConversationExecutionOwnership::User,
            contract_version: 2,
                conversation_id: ConversationId("conversation-1".into()),
                revision: 0,
                title: "Architecture V2".into(),
                messages: vec![MessageView {
                    message_id: MessageId("assistant-1".into()),
                    role: MessageRole::Assistant,
                    state: MessageState::Running,
                    revision: 1,
                    created_at_ms: 1,
                    parts: Vec::new(),
                }],
                active_run: None,
                surface: ConversationSurfaceView::default(),
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
                    measured_at_revision: 0,
                },
            },
            0,
        )
    }

    fn identity(event: &str, sequence: u64, attempt: &str) -> ProjectionEventIdentity {
        ProjectionEventIdentity {
            event_id: RunEventId(event.into()),
            conversation_id: ConversationId("conversation-1".into()),
            run_id: Some(RunId("run-1".into())),
            attempt_id: Some(RunAttemptId(attempt.into())),
            delegated_invocation_id: None,
            sequence,
        }
    }

    #[test]
    fn delegated_snapshot_restores_exact_owner_and_rejects_model_alias() {
        let mut projection = reducer().projection().clone();
        projection.messages[0].parts.push(MessagePartView::Text {
            part_id: MessagePartId("text-1".into()),
            revision: 1,
            markdown: "hello".into(),
            is_final: false,
            review: None,
        });
        let owner = ProjectionStreamOwner {
            message_id: MessageId("assistant-1".into()),
            part_id: MessagePartId("text-1".into()),
            run_id: RunId("run-1".into()),
            execution: ProjectionExecutionOwner::DelegatedInvocation {
                invocation_id: "external-1".into(),
            },
        };
        let encoded = serde_json::to_value(&owner).unwrap();
        assert_eq!(encoded["execution"]["type"], "delegated_invocation");
        assert!(encoded.get("attempt_id").is_none());
        assert!(encoded["execution"].get("attempt_id").is_none());
        let mut restored =
            ConversationProjectionReducer::restore_owned(projection, 5, vec![owner]).unwrap();
        let mut event = ProjectionEvent {
            identity: identity("next", 6, "external-1"),
            mutation: ProjectionMutation::AppendTextPreview {
                message_id: MessageId("assistant-1".into()),
                part_id: MessagePartId("text-1".into()),
                revision: 2,
                delta: " world".into(),
            },
        };
        // Identical strings in different execution namespaces cannot take ownership.
        assert_eq!(
            restored.apply(event.clone()),
            Err(ProjectionApplyError::AttemptMismatch)
        );
        assert_eq!(restored.last_sequence(), 5);
        event.identity.attempt_id = None;
        event.identity.delegated_invocation_id = Some("other-external".into());
        assert_eq!(
            restored.apply(event.clone()),
            Err(ProjectionApplyError::AttemptMismatch)
        );
        event.identity.delegated_invocation_id = Some("external-1".into());
        assert_eq!(restored.apply(event), Ok(ProjectionApplyOutcome::Applied));
        assert_eq!(restored.last_sequence(), 6);
    }

    #[test]
    fn ambiguous_stream_identity_cannot_mutate_or_advance_cursor() {
        let mut reducer = reducer();
        let mut id = identity("ambiguous", 1, "attempt-1");
        id.delegated_invocation_id = Some("external-1".into());
        assert_eq!(
            reducer.apply(ProjectionEvent {
                identity: id,
                mutation: ProjectionMutation::AppendTextPreview {
                    message_id: MessageId("assistant-1".into()),
                    part_id: MessagePartId("text-1".into()),
                    revision: 2,
                    delta: "must not appear".into(),
                },
            }),
            Err(ProjectionApplyError::EventIdentityConflict)
        );
        assert_eq!(reducer.last_sequence(), 0);
        assert!(reducer.projection().messages[0].parts.is_empty());
    }

    #[test]
    fn owned_restore_rejects_duplicate_messages_and_parts() {
        let mut projection = reducer().projection().clone();
        projection.messages.push(projection.messages[0].clone());
        assert!(matches!(
            ConversationProjectionReducer::restore_owned(projection.clone(), 0, vec![]),
            Err(ProjectionApplyError::DuplicateMessageIdentity)
        ));
        projection.messages[1].message_id = MessageId("assistant-2".into());
        for message in &mut projection.messages {
            message.parts.push(MessagePartView::Text {
                part_id: MessagePartId("shared-part".into()),
                revision: 1,
                markdown: "closed".into(),
                is_final: true,
                review: None,
            });
        }
        assert!(matches!(
            ConversationProjectionReducer::restore_owned(projection, 0, vec![]),
            Err(ProjectionApplyError::DuplicatePartIdentity)
        ));
    }

    #[test]
    fn blank_stream_identity_does_not_poison_cursor_or_ownership() {
        let mut reducer = reducer();
        for (run, attempt, expected) in [
            ("  ", "attempt-1", ProjectionApplyError::MissingRunIdentity),
            ("run-1", "\t", ProjectionApplyError::MissingAttemptIdentity),
        ] {
            let mut id = identity("event-1", 1, attempt);
            id.run_id = Some(RunId(run.into()));
            assert_eq!(
                reducer.apply(ProjectionEvent {
                    identity: id,
                    mutation: ProjectionMutation::AppendTextPreview {
                        message_id: MessageId("assistant-1".into()),
                        part_id: MessagePartId("text-1".into()),
                        revision: 2,
                        delta: "Hello".into(),
                    },
                }),
                Err(expected)
            );
            assert_eq!(reducer.last_sequence(), 0);
            assert!(reducer.part_attempts.is_empty());
            assert!(reducer.projection().messages[0].parts.is_empty());
        }
        assert_eq!(
            reducer.apply(ProjectionEvent {
                identity: identity("event-1", 1, "attempt-1"),
                mutation: ProjectionMutation::AppendTextPreview {
                    message_id: MessageId("assistant-1".into()),
                    part_id: MessagePartId("text-1".into()),
                    revision: 2,
                    delta: "Hello".into(),
                },
            }),
            Ok(ProjectionApplyOutcome::Applied)
        );
        assert_eq!(reducer.last_sequence(), 1);
    }

    #[test]
    fn review_metadata_change_requires_authoritative_upsert() {
        let mut before = reducer().projection().clone();
        before.messages[0].parts = vec![MessagePartView::Text {
            part_id: MessagePartId("reviewed-text".into()),
            revision: 1,
            markdown: String::new(),
            is_final: true,
            review: Some(nola_domain::TextReviewState::Pending),
        }];
        let mut after = before.clone();
        after.messages[0].revision = 2;
        let MessagePartView::Text {
            review, revision, ..
        } = &mut after.messages[0].parts[0]
        else {
            panic!("expected Text")
        };
        *review = Some(nola_domain::TextReviewState::Rejected);
        *revision = 2;
        let mutations = crate::conversation_projection_diff(Some(&before), &after);
        assert!(matches!(
            mutations.as_slice(),
            [ProjectionMutation::UpsertMessage { .. }]
        ));
        let mut consumer = ConversationProjectionReducer::restore(before, 0);
        consumer
            .apply_batch(vec![ProjectionEvent {
                identity: identity("review-change", 1, "attempt-1"),
                mutation: mutations.into_iter().next().unwrap(),
            }])
            .unwrap();
        assert_eq!(consumer.projection().messages[0], after.messages[0]);
    }

    #[test]
    fn withheld_text_cannot_be_disclosed_by_snapshot_upsert_or_incremental_events() {
        for review in [
            nola_domain::TextReviewState::Pending,
            nola_domain::TextReviewState::Rejected,
            nola_domain::TextReviewState::Interrupted,
        ] {
            let mut source = reducer().projection().clone();
            source.messages[0].parts = vec![MessagePartView::Text {
                part_id: MessagePartId("reviewed".into()),
                revision: 1,
                markdown: String::new(),
                is_final: false,
                review: Some(review),
            }];
            for mutation in [
                ProjectionMutation::AppendTextPreview {
                    message_id: source.messages[0].message_id.clone(),
                    part_id: MessagePartId("reviewed".into()),
                    revision: 2,
                    delta: "candidate".into(),
                },
                ProjectionMutation::CompleteTextPart {
                    message_id: source.messages[0].message_id.clone(),
                    part_id: MessagePartId("reviewed".into()),
                    revision: 2,
                    markdown: "candidate".into(),
                },
            ] {
                let mut consumer = ConversationProjectionReducer::restore(source.clone(), 0);
                assert_eq!(
                    consumer.apply(ProjectionEvent {
                        identity: identity("disclose", 1, "attempt-1"),
                        mutation
                    }),
                    Err(ProjectionApplyError::InvalidPublicSemantics(
                        nola_domain::PublicPartContractError::WithheldText
                    ))
                );
                assert_eq!(consumer.projection(), &source);
                assert_eq!(consumer.last_sequence(), 0);
                assert!(consumer.part_attempts.is_empty());
            }
            let mut invalid = source.messages[0].clone();
            if let MessagePartView::Text { markdown, .. } = &mut invalid.parts[0] {
                *markdown = "candidate".into();
            }
            let mut consumer = ConversationProjectionReducer::restore(source.clone(), 0);
            assert_eq!(
                consumer.apply(ProjectionEvent {
                    identity: identity("upsert", 1, "attempt-1"),
                    mutation: ProjectionMutation::UpsertMessage {
                        message: invalid.clone()
                    }
                }),
                Err(ProjectionApplyError::InvalidPublicSemantics(
                    nola_domain::PublicPartContractError::WithheldText
                ))
            );
            let mut invalid_snapshot = source;
            invalid_snapshot.messages[0] = invalid;
            assert!(matches!(
                ConversationProjectionReducer::restore_owned(invalid_snapshot, 0, vec![]),
                Err(ProjectionApplyError::InvalidPublicSemantics(
                    nola_domain::PublicPartContractError::WithheldText
                ))
            ));
        }
    }

    #[test]
    fn independent_message_revision_uses_authoritative_upsert() {
        let mut reducer = reducer();
        let mut before = reducer.projection().clone();
        before.messages[0].parts = vec![MessagePartView::Text {
            part_id: MessagePartId("independent-part".into()),
            revision: 1,
            markdown: "first".into(),
            is_final: false,
            review: None,
        }];
        reducer = ConversationProjectionReducer::restore(before.clone(), 0);
        let mut after = before.clone();
        after.messages[0].revision = 9;
        after.messages[0].parts = vec![MessagePartView::Text {
            part_id: MessagePartId("independent-part".into()),
            revision: 2,
            markdown: "first second".into(),
            is_final: false,
            review: None,
        }];
        let mutations = crate::conversation_projection_diff(Some(&before), &after);
        assert!(matches!(
            mutations.as_slice(),
            [ProjectionMutation::UpsertMessage { .. }]
        ));
        reducer
            .apply_batch(
                mutations
                    .into_iter()
                    .map(|mutation| ProjectionEvent {
                        identity: identity("independent-envelope", 1, "attempt-1"),
                        mutation,
                    })
                    .collect(),
            )
            .unwrap();
        assert_eq!(reducer.projection().messages, after.messages);
    }

    #[test]
    fn generated_stream_events_reconstruct_the_authoritative_message() {
        let mut reducer = reducer();
        let mut sequence = 0;
        for (revision, text, final_text) in [
            (1, "Hello", false),
            (2, "Hello\nworld", false),
            (3, "# Canonical\n\nHello world", true),
        ] {
            let mut target = reducer.projection().clone();
            target.messages[0].revision = revision;
            target.messages[0].parts = vec![MessagePartView::Text {
                part_id: MessagePartId("roundtrip-part".into()),
                revision,
                markdown: text.into(),
                is_final: final_text,
                review: None,
            }];
            let mutations =
                crate::conversation_projection_diff(Some(reducer.projection()), &target);
            let events = mutations
                .into_iter()
                .map(|mutation| {
                    sequence += 1;
                    ProjectionEvent {
                        identity: identity(&format!("roundtrip-{sequence}"), sequence, "attempt-1"),
                        mutation,
                    }
                })
                .collect();
            reducer.apply_batch(events).unwrap();
            assert_eq!(reducer.projection().messages, target.messages,
                "generated events must preserve both content and message envelope at revision {revision}");
        }
    }

    #[test]
    fn checkpoint_resume_preserves_cursor_duplicates_and_stream_ownership() {
        let mut reducer = reducer();
        let first = ProjectionEvent {
            identity: identity("checkpoint-preview", 1, "attempt-1"),
            mutation: ProjectionMutation::AppendTextPreview {
                message_id: MessageId("assistant-1".into()),
                part_id: MessagePartId("checkpoint-part".into()),
                revision: 1,
                delta: "Hello".into(),
            },
        };
        reducer.apply(first.clone()).unwrap();
        let before = reducer.projection().clone();
        let mut resumed = ConversationProjectionReducer::resume(reducer.into_checkpoint());
        assert_eq!(resumed.last_sequence(), 1);
        assert_eq!(
            resumed.apply(first.clone()),
            Ok(ProjectionApplyOutcome::Duplicate)
        );
        let mut next = first;
        next.identity = identity("checkpoint-next", 2, "attempt-2");
        assert_eq!(
            resumed.apply(next.clone()),
            Err(ProjectionApplyError::AttemptMismatch)
        );
        next.identity.attempt_id = Some(RunAttemptId("attempt-1".into()));
        next.identity.run_id = Some(RunId("another-run".into()));
        assert_eq!(
            resumed.apply(next.clone()),
            Err(ProjectionApplyError::RunMismatch)
        );
        assert_eq!(resumed.projection(), &before);
        assert_eq!(resumed.last_sequence(), 1);
        next.identity.run_id = Some(RunId("run-1".into()));
        next.mutation = ProjectionMutation::CompleteTextPart {
            message_id: MessageId("assistant-1".into()),
            part_id: MessagePartId("checkpoint-part".into()),
            revision: 2,
            markdown: "Hello world".into(),
        };
        assert_eq!(resumed.apply(next), Ok(ProjectionApplyOutcome::Applied));
        let mut resumed = ConversationProjectionReducer::resume(resumed.into_checkpoint());
        assert_eq!(
            resumed.apply(ProjectionEvent {
                identity: identity("late-preview", 3, "attempt-1"),
                mutation: ProjectionMutation::AppendTextPreview {
                    message_id: MessageId("assistant-1".into()),
                    part_id: MessagePartId("checkpoint-part".into()),
                    revision: 3,
                    delta: "Must not append".into(),
                },
            }),
            Ok(ProjectionApplyOutcome::Stale)
        );
    }

    #[test]
    fn batch_failure_rolls_back_projection_cursor_and_attempt_state() {
        let mut reducer = reducer();
        let before = reducer.projection.clone();
        let preview = ProjectionEvent {
            identity: identity("batch-preview", 1, "attempt-1"),
            mutation: ProjectionMutation::AppendTextPreview {
                message_id: MessageId("assistant-1".into()),
                part_id: MessagePartId("batch-part".into()),
                revision: 1,
                delta: "Must roll back".into(),
            },
        };
        let invalid = ProjectionEvent {
            identity: identity("batch-invalid", 2, "attempt-2"),
            mutation: ProjectionMutation::CompleteTextPart {
                message_id: MessageId("assistant-1".into()),
                part_id: MessagePartId("batch-part".into()),
                revision: 2,
                markdown: "Wrong attempt".into(),
            },
        };
        assert_eq!(
            reducer.apply_batch(vec![preview.clone(), invalid]),
            Err(ProjectionApplyError::AttemptMismatch)
        );
        assert_eq!(reducer.projection, before);
        assert_eq!(reducer.last_sequence(), 0);
        assert!(reducer.seen_events.is_empty());
        assert!(reducer.seen_event_order.is_empty());
        assert!(reducer.part_attempts.is_empty());
        assert!(reducer.completed_text_parts.is_empty());
        assert_eq!(
            reducer.apply_batch(vec![preview]),
            Ok(vec![ProjectionApplyOutcome::Applied])
        );
    }

    #[test]
    fn successful_batch_commits_once_and_duplicate_delivery_is_a_no_op() {
        let mut reducer = reducer();
        let events = (1..=2)
            .map(|sequence| ProjectionEvent {
                identity: identity(&format!("batch-{sequence}"), sequence, "attempt-1"),
                mutation: ProjectionMutation::SetTitle {
                    title: format!("Title {sequence}"),
                    revision: 1,
                },
            })
            .collect::<Vec<_>>();
        assert_eq!(
            reducer.apply_batch(events.clone()),
            Ok(vec![ProjectionApplyOutcome::Applied; 2])
        );
        assert_eq!(reducer.last_sequence(), 2);
        assert_eq!(reducer.projection.title, "Title 2");
        let committed = reducer.projection.clone();
        assert_eq!(
            reducer.apply_batch(events),
            Ok(vec![ProjectionApplyOutcome::Duplicate; 2])
        );
        assert_eq!(reducer.projection, committed);
        assert_eq!(reducer.last_sequence(), 2);
    }

    #[test]
    fn oversized_batch_is_rejected_before_any_event_is_applied() {
        let mut reducer = reducer();
        let before = reducer.projection.clone();
        let count = ConversationProjectionReducer::MAX_BATCH_EVENTS + 1;
        let events = (1..=count)
            .map(|sequence| ProjectionEvent {
                identity: identity(
                    &format!("too-large-{sequence}"),
                    sequence as u64,
                    "attempt-1",
                ),
                mutation: ProjectionMutation::SetTitle {
                    title: "Must not change".into(),
                    revision: 1,
                },
            })
            .collect();
        assert_eq!(
            reducer.apply_batch(events),
            Err(ProjectionApplyError::BatchTooLarge {
                limit: ConversationProjectionReducer::MAX_BATCH_EVENTS,
                received: count,
            })
        );
        assert_eq!(reducer.projection, before);
        assert_eq!(reducer.last_sequence(), 0);
        assert!(reducer.apply_batch(Vec::new()).unwrap().is_empty());
    }

    #[test]
    fn replacing_parts_releases_old_tracking_and_tracks_new_final_text() {
        let mut reducer = reducer();
        let template = reducer.projection.messages[0].clone();
        for cycle in 0..1_000_u64 {
            let part_id = MessagePartId(format!("replacement-{cycle}"));
            let mut message = template.clone();
            message.revision = cycle + 1;
            message.parts = vec![MessagePartView::Text {
                part_id: part_id.clone(),
                revision: cycle + 1,
                markdown: "Final snapshot".into(),
                is_final: true,
                review: None,
            }];
            assert_eq!(
                reducer.apply(ProjectionEvent {
                    identity: identity(&format!("replace-{cycle}"), cycle * 2 + 1, "attempt-1"),
                    mutation: ProjectionMutation::UpsertMessage { message },
                }),
                Ok(ProjectionApplyOutcome::Applied)
            );
            assert_eq!(reducer.completed_text_parts.len(), 1);
            assert!(reducer.part_attempts.is_empty());
            assert_eq!(
                reducer.apply(ProjectionEvent {
                    identity: identity(&format!("late-{cycle}"), cycle * 2 + 2, "attempt-1"),
                    mutation: ProjectionMutation::AppendTextPreview {
                        message_id: MessageId("assistant-1".into()),
                        part_id,
                        revision: cycle + 2,
                        delta: "Must not append".into(),
                    },
                }),
                Ok(ProjectionApplyOutcome::Stale)
            );
            let MessagePartView::Text { markdown, .. } = &reducer.projection.messages[0].parts[0]
            else {
                panic!("expected text");
            };
            assert_eq!(markdown, "Final snapshot");
            assert!(reducer.part_attempts.len() <= 1);
        }
    }

    #[test]
    fn missing_message_does_not_poison_attempt_binding() {
        for complete in [false, true] {
            let mut reducer = reducer();
            let mutation = |message: &str| {
                if complete {
                    ProjectionMutation::CompleteTextPart {
                        message_id: MessageId(message.into()),
                        part_id: MessagePartId("part".into()),
                        revision: 2,
                        markdown: "Final".into(),
                    }
                } else {
                    ProjectionMutation::AppendTextPreview {
                        message_id: MessageId(message.into()),
                        part_id: MessagePartId("part".into()),
                        revision: 2,
                        delta: "Preview".into(),
                    }
                }
            };
            let before = reducer.projection.clone();
            assert_eq!(
                reducer.apply(ProjectionEvent {
                    identity: identity("rejected", 1, "wrong-attempt"),
                    mutation: mutation("missing"),
                }),
                Err(ProjectionApplyError::MessageNotFound)
            );
            assert_eq!(reducer.projection, before);
            assert_eq!(reducer.last_sequence(), 0);
            assert!(reducer.part_attempts.is_empty());
            assert!(reducer.seen_events.is_empty());
            assert_eq!(
                reducer.apply(ProjectionEvent {
                    identity: identity("accepted", 1, "correct-attempt"),
                    mutation: mutation("assistant-1"),
                }),
                Ok(ProjectionApplyOutcome::Applied)
            );
        }
    }

    #[test]
    fn removed_messages_release_part_bookkeeping_across_long_streams() {
        let mut reducer = reducer();
        let template = reducer.projection.messages[0].clone();
        for cycle in 0..1_000_u64 {
            let mut message = template.clone();
            message.message_id = MessageId(format!("transient-{cycle}"));
            let id = message.message_id.clone();
            let part = MessagePartId(format!("text-{cycle}"));
            let mutations = [
                ProjectionMutation::UpsertMessage { message },
                ProjectionMutation::AppendTextPreview {
                    message_id: id.clone(),
                    part_id: part.clone(),
                    revision: 1,
                    delta: "Text".into(),
                },
                ProjectionMutation::CompleteTextPart {
                    message_id: id.clone(),
                    part_id: part,
                    revision: 2,
                    markdown: "Text complete".into(),
                },
                ProjectionMutation::RemoveMessage {
                    message_id: id,
                    revision: 2,
                },
            ];
            for (offset, mutation) in mutations.into_iter().enumerate() {
                let sequence = cycle * 4 + offset as u64 + 1;
                assert_eq!(
                    reducer.apply(ProjectionEvent {
                        identity: identity(&format!("lifecycle-{sequence}"), sequence, "attempt-1"),
                        mutation,
                    }),
                    Ok(ProjectionApplyOutcome::Applied)
                );
            }
            assert_eq!(reducer.projection.messages.len(), 1);
            assert!(reducer.part_attempts.is_empty());
            assert!(reducer.completed_text_parts.is_empty());
        }
    }

    #[test]
    fn older_remove_does_not_delete_a_newer_message() {
        let mut reducer = reducer();
        assert_eq!(
            reducer.apply(ProjectionEvent {
                identity: identity("old-remove", 1, "attempt-1"),
                mutation: ProjectionMutation::RemoveMessage {
                    message_id: MessageId("assistant-1".into()),
                    revision: 0,
                },
            }),
            Ok(ProjectionApplyOutcome::Stale)
        );
        assert_eq!(reducer.projection.messages.len(), 1);
    }

    #[test]
    fn duplicate_checks_do_not_hide_scope_or_identity_conflicts() {
        let mut reducer = reducer();
        let event = ProjectionEvent {
            identity: identity("shared-id", 1, "attempt-1"),
            mutation: ProjectionMutation::SetTitle {
                title: "Accepted".into(),
                revision: 1,
            },
        };
        reducer.apply(event.clone()).unwrap();
        let before = reducer.projection.clone();
        let mut foreign = event.clone();
        foreign.identity.conversation_id = ConversationId("another-conversation".into());
        assert_eq!(
            reducer.apply(foreign),
            Err(ProjectionApplyError::ConversationMismatch)
        );
        for change in 0..3 {
            let mut conflicting = event.clone();
            match change {
                0 => conflicting.identity.sequence = 2,
                1 => conflicting.identity.run_id = Some(RunId("another-run".into())),
                _ => conflicting.identity.attempt_id = Some(RunAttemptId("another-attempt".into())),
            }
            assert_eq!(
                reducer.apply(conflicting),
                Err(ProjectionApplyError::EventIdentityConflict)
            );
            assert_eq!(reducer.projection, before);
            assert_eq!(reducer.last_sequence(), 1);
        }
        assert_eq!(reducer.apply(event), Ok(ProjectionApplyOutcome::Duplicate));
    }

    #[test]
    fn snapshot_prefix_redelivery_is_a_no_op_after_checkpoint_resume() {
        let projection = reducer().projection.clone();
        let mut restored = ConversationProjectionReducer::restore(projection.clone(), 10);
        for sequence in [1, 10, 4] {
            let event = ProjectionEvent {
                identity: identity(&format!("covered-{sequence}"), sequence, "attempt-1"),
                mutation: ProjectionMutation::InvalidateProjection { revision: 999 },
            };
            assert_eq!(restored.apply(event), Ok(ProjectionApplyOutcome::Duplicate));
        }
        assert_eq!(restored.projection, projection);
        assert_eq!(restored.last_sequence(), 10);
        assert!(restored.seen_events.is_empty());
        restored = ConversationProjectionReducer::resume(restored.into_checkpoint());
        let suffix = ProjectionEvent {
            identity: identity("suffix", 11, "attempt-1"),
            mutation: ProjectionMutation::SetTitle {
                title: "New title".into(),
                revision: 11,
            },
        };
        assert_eq!(
            restored.apply(suffix.clone()),
            Ok(ProjectionApplyOutcome::Applied)
        );
        let covered = ProjectionEvent {
            identity: identity("covered-again", 10, "attempt-1"),
            mutation: ProjectionMutation::SetTitle {
                title: "Old title".into(),
                revision: 999,
            },
        };
        assert_eq!(
            restored.apply(covered),
            Ok(ProjectionApplyOutcome::Duplicate)
        );
        assert_eq!(restored.projection.title, "New title");
        assert_eq!(restored.last_sequence(), 11);
        assert_eq!(
            restored.apply(suffix.clone()),
            Ok(ProjectionApplyOutcome::Duplicate)
        );
        let mut conflict = suffix;
        conflict.identity.sequence = 10;
        assert_eq!(
            restored.apply(conflict),
            Err(ProjectionApplyError::EventIdentityConflict)
        );
    }

    #[test]
    fn snapshot_prefix_does_not_hide_foreign_scope_zero_or_suffix_gaps() {
        let mut restored = ConversationProjectionReducer::restore(reducer().projection.clone(), 10);
        let mut event = ProjectionEvent {
            identity: identity("covered", 10, "attempt-1"),
            mutation: ProjectionMutation::InvalidateProjection { revision: 999 },
        };
        event.identity.conversation_id = ConversationId("foreign".into());
        assert_eq!(
            restored.apply(event),
            Err(ProjectionApplyError::ConversationMismatch)
        );
        for sequence in [0, 12] {
            assert_eq!(
                restored.apply(ProjectionEvent {
                    identity: identity("invalid-sequence", sequence, "attempt-1"),
                    mutation: ProjectionMutation::InvalidateProjection { revision: 999 },
                }),
                Err(ProjectionApplyError::SequenceGap {
                    expected: 11,
                    received: sequence
                })
            );
        }
        assert_eq!(restored.last_sequence(), 10);
    }

    #[test]
    fn long_event_stream_keeps_a_bounded_duplicate_window() {
        let mut reducer = reducer();
        for sequence in 1..=10_000 {
            reducer
                .apply(ProjectionEvent {
                    identity: identity(&format!("event-{sequence}"), sequence, "attempt-1"),
                    mutation: ProjectionMutation::SetTitle {
                        title: "Stable".into(),
                        revision: sequence,
                    },
                })
                .unwrap();
            assert!(reducer.seen_events.len() <= ConversationProjectionReducer::RECENT_EVENT_LIMIT);
            assert!(
                reducer.seen_event_order.len() <= ConversationProjectionReducer::RECENT_EVENT_LIMIT
            );
        }
        assert_eq!(
            reducer.seen_events.len(),
            ConversationProjectionReducer::RECENT_EVENT_LIMIT
        );
        let expired = ProjectionEvent {
            identity: identity("event-1", 1, "attempt-1"),
            mutation: ProjectionMutation::SetTitle {
                title: "Must not reappear".into(),
                revision: 1,
            },
        };
        assert_eq!(
            reducer.apply(expired),
            Err(ProjectionApplyError::SequenceGap {
                expected: 10_001,
                received: 1,
            })
        );
        assert_eq!(reducer.projection.title, "Stable");
        assert_eq!(reducer.last_sequence(), 10_000);
    }

    #[test]
    fn ordered_context_update_can_reset_measurement_revision_but_not_event_sequence() {
        let mut reducer = reducer();
        reducer.projection.context_usage.used_tokens = 9_000;
        reducer.projection.context_usage.measured_at_revision = 900;
        reducer.projection.context_usage.estimated = false;
        let usage = ContextUsageView {
            measurement: None,
            prompt_cache_measurement: None,
            prompt_cache_diagnostic: None,
            conversation_cache_usage: None,
            cache_usage: None,
            used_tokens: 0,
            context_window_tokens: 32_768,
            prompt_token_limit: 24_576,
            estimated: true,
            measured_at_revision: 2,
        };
        let event = ProjectionEvent {
            identity: identity("usage-reset", 1, "attempt-2"),
            mutation: ProjectionMutation::SetContextUsage {
                context_usage: usage.clone(),
            },
        };
        assert_eq!(
            reducer.apply(event.clone()),
            Ok(ProjectionApplyOutcome::Applied)
        );
        assert_eq!(reducer.projection.context_usage, usage);
        assert_eq!(reducer.apply(event), Ok(ProjectionApplyOutcome::Duplicate));
        let before = reducer.projection.clone();
        for sequence in [1, 3] {
            let stale = ProjectionEvent {
                identity: identity(&format!("wrong-order-{sequence}"), sequence, "attempt-1"),
                mutation: ProjectionMutation::SetContextUsage {
                    context_usage: ContextUsageView {
                        used_tokens: 99_999,
                        ..usage.clone()
                    },
                },
            };
            assert_eq!(
                reducer.apply(stale),
                Err(ProjectionApplyError::SequenceGap {
                    expected: 2,
                    received: sequence,
                })
            );
            assert_eq!(reducer.projection, before);
            assert_eq!(reducer.last_sequence(), 1);
        }
    }

    #[test]
    fn same_revision_sibling_events_update_message_and_surface() {
        let mut reducer = reducer();
        let mut message = reducer.projection.messages[0].clone();
        message.state = MessageState::Completed;
        assert_eq!(
            reducer.apply(ProjectionEvent {
                identity: identity("message-sibling", 1, "attempt-1"),
                mutation: ProjectionMutation::UpsertMessage {
                    message: message.clone()
                },
            }),
            Ok(ProjectionApplyOutcome::Applied)
        );
        assert_eq!(reducer.projection.messages[0], message);
        let mut surface = reducer.projection.surface.clone();
        surface.committed_message_ids = vec![message.message_id];
        assert_eq!(
            reducer.apply(ProjectionEvent {
                identity: identity("surface-sibling", 2, "attempt-1"),
                mutation: ProjectionMutation::ReplaceSurface {
                    surface: surface.clone()
                },
            }),
            Ok(ProjectionApplyOutcome::Applied)
        );
        assert_eq!(reducer.projection.surface, surface);
        assert_eq!(reducer.last_sequence(), 2);
    }

    #[test]
    fn completed_text_replaces_preview_instead_of_appending_full_text() {
        let mut reducer = reducer();
        let preview = ProjectionEvent {
            identity: identity("event-1", 1, "attempt-1"),
            mutation: ProjectionMutation::AppendTextPreview {
                message_id: MessageId("assistant-1".into()),
                part_id: MessagePartId("text-1".into()),
                revision: 1,
                delta: "Hello".into(),
            },
        };
        assert_eq!(reducer.apply(preview), Ok(ProjectionApplyOutcome::Applied));

        let completed = ProjectionEvent {
            identity: identity("event-2", 2, "attempt-1"),
            mutation: ProjectionMutation::CompleteTextPart {
                message_id: MessageId("assistant-1".into()),
                part_id: MessagePartId("text-1".into()),
                revision: 2,
                markdown: "Hello world".into(),
            },
        };
        assert_eq!(
            reducer.apply(completed),
            Ok(ProjectionApplyOutcome::Applied)
        );

        let MessagePartView::Text {
            markdown, is_final, ..
        } = &reducer.projection().messages[0].parts[0]
        else {
            panic!("expected text Part");
        };
        assert_eq!(markdown, "Hello world");
        assert!(*is_final);
    }

    #[test]
    fn duplicate_event_is_an_idempotent_no_op() {
        let mut reducer = reducer();
        let event = ProjectionEvent {
            identity: identity("event-1", 1, "attempt-1"),
            mutation: ProjectionMutation::AppendTextPreview {
                message_id: MessageId("assistant-1".into()),
                part_id: MessagePartId("text-1".into()),
                revision: 1,
                delta: "one".into(),
            },
        };
        assert_eq!(
            reducer.apply(event.clone()),
            Ok(ProjectionApplyOutcome::Applied)
        );
        assert_eq!(reducer.apply(event), Ok(ProjectionApplyOutcome::Duplicate));
    }

    #[test]
    fn another_attempt_cannot_continue_the_same_part() {
        let mut reducer = reducer();
        reducer
            .apply(ProjectionEvent {
                identity: identity("event-1", 1, "attempt-1"),
                mutation: ProjectionMutation::AppendTextPreview {
                    message_id: MessageId("assistant-1".into()),
                    part_id: MessagePartId("text-1".into()),
                    revision: 1,
                    delta: "one".into(),
                },
            })
            .unwrap();
        let result = reducer.apply(ProjectionEvent {
            identity: identity("event-2", 2, "attempt-2"),
            mutation: ProjectionMutation::AppendTextPreview {
                message_id: MessageId("assistant-1".into()),
                part_id: MessagePartId("text-1".into()),
                revision: 2,
                delta: "two".into(),
            },
        });
        assert_eq!(result, Err(ProjectionApplyError::AttemptMismatch));
        assert_eq!(reducer.last_sequence(), 1);
    }

    #[test]
    fn sequence_gaps_are_not_assigned_to_the_current_message() {
        let mut reducer = reducer();
        let result = reducer.apply(ProjectionEvent {
            identity: identity("event-2", 2, "attempt-1"),
            mutation: ProjectionMutation::AppendTextPreview {
                message_id: MessageId("assistant-1".into()),
                part_id: MessagePartId("text-1".into()),
                revision: 1,
                delta: "out of order".into(),
            },
        });
        assert_eq!(
            result,
            Err(ProjectionApplyError::SequenceGap {
                expected: 1,
                received: 2,
            })
        );
        assert!(reducer.projection().messages[0].parts.is_empty());
    }
}
