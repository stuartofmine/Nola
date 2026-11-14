// SPDX-License-Identifier: Apache-2.0
//! Cursor-based access to durable projection events.

use nola_domain::{
    ConversationListProjectionEvent, ConversationProjection, ConversationProjectionEvent,
};
use nola_store_port::conversation_history::ConversationProjectionRepository;
use nola_store_port::conversation_query::ConversationQueryRepository;
use nola_store_port::protocol_journal::{ProjectionJournalRepository, ProjectionScopeKey};

use crate::conversation_read::ConversationSummary;
use crate::ApplicationRuntime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationProjectionScope {
    pub scope_type: String,
    pub scope_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationProjectionEvent {
    pub event: ConversationProjectionEvent,
    pub occurred_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationProjectionSuffix {
    pub events: Vec<ApplicationProjectionEvent>,
    pub current_sequence: u64,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationConversationListProjectionEvent {
    pub event: ConversationListProjectionEvent,
    pub occurred_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationConversationListProjectionSuffix {
    pub events: Vec<ApplicationConversationListProjectionEvent>,
    pub current_sequence: u64,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationConversationProjectionSnapshot {
    pub projection: Option<ConversationProjection>,
    pub current_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationConversationListProjectionSnapshot {
    pub conversations: Vec<ConversationSummary>,
    pub current_sequence: u64,
}

impl<R> ApplicationRuntime<R>
where
    R: ConversationProjectionRepository + ConversationQueryRepository + ProjectionJournalRepository,
{
    pub fn projection_cursor(&self, scope: ApplicationProjectionScope) -> Result<u64, String> {
        self.store
            .projection_cursor(&ProjectionScopeKey {
                scope_type: scope.scope_type,
                scope_id: scope.scope_id,
            })
            .map(|cursor| cursor.map(|cursor| cursor.current_sequence).unwrap_or(0))
            .map_err(|error| format!("load projection cursor failed: {error}"))
    }

    pub fn conversation_list_projection_snapshot(
        &self,
    ) -> Result<ApplicationConversationListProjectionSnapshot, String> {
        let conversations = self
            .store
            .list_conversations()
            .map_err(|error| format!("load conversation list projection failed: {error}"))?
            .into_iter()
            .map(ConversationSummary::from)
            .collect();
        let scope = ProjectionScopeKey {
            scope_type: "conversations".to_string(),
            scope_id: String::new(),
        };
        let current_sequence = self
            .store
            .projection_cursor(&scope)
            .map_err(|error| format!("load conversation list projection cursor failed: {error}"))?
            .map(|cursor| cursor.current_sequence)
            .unwrap_or(0);
        Ok(ApplicationConversationListProjectionSnapshot {
            conversations,
            current_sequence,
        })
    }

    pub fn conversation_projection_snapshot(
        &self,
        conversation_id: &str,
    ) -> Result<ApplicationConversationProjectionSnapshot, String> {
        let projection = self.load_optional_conversation_projection(conversation_id)?;
        let scope = ProjectionScopeKey {
            scope_type: "conversation".to_string(),
            scope_id: conversation_id.to_string(),
        };
        let current_sequence = self
            .store
            .projection_cursor(&scope)
            .map_err(|error| format!("load projection cursor failed: {error}"))?
            .map(|cursor| cursor.current_sequence)
            .unwrap_or(0);
        Ok(ApplicationConversationProjectionSnapshot {
            projection,
            current_sequence,
        })
    }

    pub fn conversation_projection_page_snapshot(
        &self,
        conversation_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<ApplicationConversationProjectionSnapshot, String> {
        let projection = crate::load_conversation_projection_page_v1(
            &self.store,
            conversation_id,
            offset,
            limit,
        )
        .map_err(|error| format!("load conversation projection page failed: {error}"))?;
        let scope = ProjectionScopeKey {
            scope_type: "conversation".to_string(),
            scope_id: conversation_id.to_string(),
        };
        let current_sequence = self
            .store
            .projection_cursor(&scope)
            .map_err(|error| format!("load projection page cursor failed: {error}"))?
            .map(|cursor| cursor.current_sequence)
            .unwrap_or(0);
        Ok(ApplicationConversationProjectionSnapshot {
            projection: Some(projection),
            current_sequence,
        })
    }

    pub fn projection_event_suffix(
        &self,
        scope: ApplicationProjectionScope,
        after_sequence: u64,
        limit: usize,
    ) -> Result<ApplicationProjectionSuffix, String> {
        if scope.scope_type != "conversation" || scope.scope_id.is_empty() {
            return Err(
                "conversation projection events require a nonempty conversation scope".into(),
            );
        }
        if limit == 0 {
            return Err("conversation projection event page limit must be positive".into());
        }
        let scope = ProjectionScopeKey {
            scope_type: scope.scope_type,
            scope_id: scope.scope_id,
        };
        let mut expected_after = after_sequence;
        let events = self
            .store
            .projection_events_after(&scope, after_sequence, limit)
            .map_err(|error| format!("load projection event suffix failed: {error}"))?
            .into_iter()
            .map(|stored| {
                let event: ConversationProjectionEvent = serde_json::from_str(&stored.payload_json)
                    .map_err(|error| {
                        format!(
                            "decode projection event {} failed: {error}",
                            stored.event_id
                        )
                    })?;
                if event.identity.event_id.0.as_str() != stored.event_id.as_str()
                    || event.identity.sequence != stored.sequence
                {
                    return Err(format!(
                        "projection event {} identity or sequence does not match its journal row",
                        stored.event_id
                    ));
                }
                if event.identity.conversation_id.0 != scope.scope_id {
                    return Err(format!(
                        "projection event {} belongs to another conversation",
                        stored.event_id
                    ));
                }
                let expected = expected_after
                    .checked_add(1)
                    .ok_or_else(|| "projection event sequence overflow".to_string())?;
                if stored.sequence != expected {
                    return Err(format!(
                        "projection journal sequence gap: expected {expected}, received {}",
                        stored.sequence
                    ));
                }
                expected_after = stored.sequence;
                Ok(ApplicationProjectionEvent {
                    event,
                    occurred_at: stored.occurred_at,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let current_sequence = self
            .store
            .projection_cursor(&scope)
            .map_err(|error| format!("load projection cursor failed: {error}"))?
            .map(|cursor| cursor.current_sequence)
            .unwrap_or(0);
        let delivered_sequence = events
            .last()
            .map(|event| event.event.identity.sequence)
            .unwrap_or(after_sequence);
        if after_sequence > current_sequence || delivered_sequence > current_sequence {
            return Err("projection event cursor is ahead of the journal cursor".into());
        }
        if events.is_empty() && after_sequence < current_sequence {
            return Err(format!(
                "projection journal suffix is missing after {after_sequence}; current cursor is {current_sequence}"
            ));
        }
        Ok(ApplicationProjectionSuffix {
            events,
            current_sequence,
            has_more: current_sequence > delivered_sequence,
        })
    }

    pub fn conversation_list_projection_event_suffix(
        &self,
        after_sequence: u64,
        limit: usize,
    ) -> Result<ApplicationConversationListProjectionSuffix, String> {
        let scope = ProjectionScopeKey {
            scope_type: "conversations".to_string(),
            scope_id: String::new(),
        };
        let events = self
            .store
            .projection_events_after(&scope, after_sequence, limit)
            .map_err(|error| format!("load conversation list event suffix failed: {error}"))?
            .into_iter()
            .map(|stored| {
                let event: ConversationListProjectionEvent =
                    serde_json::from_str(&stored.payload_json).map_err(|error| {
                        format!("decode conversation list event {} failed: {error}", stored.event_id)
                    })?;
                if event.identity.event_id.0.as_str() != stored.event_id.as_str()
                    || event.identity.sequence != stored.sequence
                {
                    return Err(format!(
                        "conversation list event {} identity or sequence does not match its journal row",
                        stored.event_id
                    ));
                }
                Ok(ApplicationConversationListProjectionEvent {
                    event,
                    occurred_at: stored.occurred_at,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let current_sequence = self
            .store
            .projection_cursor(&scope)
            .map_err(|error| format!("load conversation list event cursor failed: {error}"))?
            .map(|cursor| cursor.current_sequence)
            .unwrap_or(0);
        let delivered_sequence = events
            .last()
            .map(|event| event.event.identity.sequence)
            .unwrap_or(after_sequence);
        Ok(ApplicationConversationListProjectionSuffix {
            events,
            current_sequence,
            has_more: current_sequence > delivered_sequence,
        })
    }
}

impl<R: nola_store_port::assistant_journal::AssistantJournalRepository> ApplicationRuntime<R> {
    pub fn local_journal_head(&self, identity: &nola_domain::assistant_journal::AssistantJournalIdentity)
        -> Result<nola_domain::assistant_journal::AssistantJournalCursor, String> {
        self.store.local_assistant_journal_head(identity).map_err(|error| error.to_string())
    }
}

pub fn conversation_list_projection_snapshot_from_store<R>(
    store: &mut R,
) -> Result<ApplicationConversationListProjectionSnapshot, String>
where
    R: ConversationQueryRepository
        + ProjectionJournalRepository
        + nola_store_port::consistent_read::ConsistentReadRepository,
{
    store.with_consistent_read(|store| {
        let conversations = store
            .list_conversations()
            .map_err(|error| format!("load conversation list projection failed: {error}"))?
            .into_iter()
            .map(ConversationSummary::from)
            .collect();
        let scope = ProjectionScopeKey {
            scope_type: "conversations".to_string(),
            scope_id: String::new(),
        };
        let current_sequence = store
            .projection_cursor(&scope)
            .map_err(|error| format!("load conversation list projection cursor failed: {error}"))?
            .map(|cursor| cursor.current_sequence)
            .unwrap_or(0);
        Ok(ApplicationConversationListProjectionSnapshot {
            conversations,
            current_sequence,
        })
    })
}

pub fn conversation_projection_snapshot_from_store<R>(
    store: &mut R,
    conversation_id: &str,
) -> Result<ApplicationConversationProjectionSnapshot, String>
where
    R: ConversationProjectionRepository
        + ProjectionJournalRepository
        + nola_store_port::consistent_read::ConsistentReadRepository,
{
    store.with_consistent_read(|store| {
        let projection = match crate::load_conversation_projection_v1(store, conversation_id) {
            Ok(projection) => Some(projection),
            Err(crate::ConversationProjectionContractError::MissingConversation(_)) => None,
            Err(error) => {
                return Err(format!(
                    "load canonical projection before command failed: {error}"
                ))
            }
        };
        let scope = ProjectionScopeKey {
            scope_type: "conversation".to_string(),
            scope_id: conversation_id.to_string(),
        };
        let current_sequence = store
            .projection_cursor(&scope)
            .map_err(|error| format!("load projection cursor failed: {error}"))?
            .map(|cursor| cursor.current_sequence)
            .unwrap_or(0);
        Ok(ApplicationConversationProjectionSnapshot {
            projection,
            current_sequence,
        })
    })
}

pub fn conversation_projection_page_snapshot_from_store<R>(
    store: &mut R,
    conversation_id: &str,
    offset: usize,
    limit: usize,
) -> Result<ApplicationConversationProjectionSnapshot, String>
where
    R: ConversationProjectionRepository
        + ProjectionJournalRepository
        + nola_store_port::consistent_read::ConsistentReadRepository,
{
    store.with_consistent_read(|store| {
        let projection =
            crate::load_conversation_projection_page_v1(store, conversation_id, offset, limit)
                .map_err(|error| format!("load conversation projection page failed: {error}"))?;
        let scope = ProjectionScopeKey {
            scope_type: "conversation".to_string(),
            scope_id: conversation_id.to_string(),
        };
        let current_sequence = store
            .projection_cursor(&scope)
            .map_err(|error| format!("load projection page cursor failed: {error}"))?
            .map(|cursor| cursor.current_sequence)
            .unwrap_or(0);
        Ok(ApplicationConversationProjectionSnapshot {
            projection: Some(projection),
            current_sequence,
        })
    })
}
