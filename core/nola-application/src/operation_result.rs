// SPDX-License-Identifier: Apache-2.0
//! Application-owned results for product transactions.
//!
//! Store transaction records are persistence details. Public use cases return
//! these objects so clients depend on product outcomes rather than Store DTOs.

use nola_store_port::conversation_profile::CreateConversationResult;
use nola_store_port::conversation_revert::ApplyConversationProjectionResult;
use nola_store_port::run_creation::BeginTurnResult;
use nola_store_port::run_effect::CommitTransitionResult;
use nola_store_port::transcript_write::AppendMessagesResult;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationCreation {
    pub created: bool,
    pub initial_configuration_revision: Option<i64>,
}

impl From<CreateConversationResult> for ConversationCreation {
    fn from(value: CreateConversationResult) -> Self {
        Self {
            created: value.created,
            initial_configuration_revision: value.initial_configuration_revision,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptImportResult {
    pub appended_message_count: usize,
    pub message_count: usize,
    pub conversation_revision: i64,
}

impl From<AppendMessagesResult> for TranscriptImportResult {
    fn from(value: AppendMessagesResult) -> Self {
        Self {
            appended_message_count: value.appended_message_count,
            message_count: value.message_count,
            conversation_revision: value.conversation_revision,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationProjectionChange {
    pub applied: bool,
    pub conversation_revision: i64,
    pub ledger_message_count: usize,
}

impl From<ApplyConversationProjectionResult> for ConversationProjectionChange {
    fn from(value: ApplyConversationProjectionResult) -> Self {
        Self {
            applied: value.applied,
            conversation_revision: value.conversation_revision,
            ledger_message_count: value.ledger_message_count,
        }
    }
}

pub use nola_domain::CanonicalBranchResult as ConversationBranchCreation;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnOpened {
    pub conversation_id: String,
    pub user_message_id: String,
    pub assistant_message_id: String,
    pub run_id: String,
    pub user_ordinal: i64,
    pub assistant_ordinal: i64,
}

impl From<BeginTurnResult> for TurnOpened {
    fn from(value: BeginTurnResult) -> Self {
        Self {
            conversation_id: value.conversation_id,
            user_message_id: value.user_message_id,
            assistant_message_id: value.assistant_message_id,
            run_id: value.run_id,
            user_ordinal: value.user_ordinal,
            assistant_ordinal: value.assistant_ordinal,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionCommit {
    pub applied: bool,
    pub checkpoint_revision: i64,
    pub next_event_sequence: i64,
    /// Exact changed-owner watermarks. Deliver only after the outer transaction
    /// commits; these are not a snapshot or proof of current execution authority.
    pub assistant_publications: Vec<nola_domain::assistant_journal::AssistantJournalCursor>,
}

impl From<CommitTransitionResult> for TransitionCommit {
    fn from(value: CommitTransitionResult) -> Self {
        Self {
            applied: value.applied,
            checkpoint_revision: value.checkpoint_revision,
            next_event_sequence: value.next_event_sequence,
            assistant_publications: Vec::new(),
        }
    }
}
